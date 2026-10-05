/** Bounded image admission and a lightweight wire view of persisted images. */
import { createHash } from "node:crypto";
import type { ImageContent, UserMessage } from "@earendil-works/pi-ai";

export const MAX_IMAGE_BYTES = 1024 * 1024;
export const MAX_IMAGES = 4;

export function promptContent(message: string, images: unknown, acceptsImages: boolean): UserMessage["content"] {
  if (images === undefined) return message;
  if (!Array.isArray(images) || images.length > MAX_IMAGES) throw new Error(`Attach up to ${MAX_IMAGES} images per message`);
  if (!images.length) return message;
  if (!acceptsImages) throw new Error("This model does not accept images. Choose a vision-capable model.");
  const validated: ImageContent[] = images.map((image: unknown) => {
    if (!image || typeof image !== "object") throw new Error("Invalid image attachment");
    const { type, data, mimeType } = image as Record<string, unknown>;
    if (type !== "image" || typeof data !== "string" || typeof mimeType !== "string") throw new Error("Invalid image attachment");
    if (data.length > Math.ceil(MAX_IMAGE_BYTES / 3) * 4 || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(data)) throw new Error("Image must be valid base64, at most 1 MiB");
    const bytes = Buffer.from(data, "base64");
    if (!bytes.length || bytes.length > MAX_IMAGE_BYTES || bytes.toString("base64") !== data) throw new Error("Image must be valid base64, at most 1 MiB");
    const signature = mimeType === "image/png" ? bytes.subarray(0, 8).equals(Buffer.from([137,80,78,71,13,10,26,10]))
      : mimeType === "image/jpeg" ? bytes[0] === 255 && bytes[1] === 216 && bytes[2] === 255
      : mimeType === "image/webp" ? bytes.toString("ascii", 0, 4) === "RIFF" && bytes.toString("ascii", 8, 12) === "WEBP"
      : mimeType === "image/gif" ? ["GIF87a", "GIF89a"].includes(bytes.toString("ascii", 0, 6)) : false;
    if (!signature) throw new Error("Unsupported or mismatched image format; use PNG, JPEG, WebP or GIF");
    return { type: "image", data, mimeType };
  });
  return [...(message ? [{ type: "text" as const, text: message }] : []), ...validated];
}

function isImage(value: unknown): value is ImageContent {
  return !!value && typeof value === "object" && (value as ImageContent).type === "image"
    && typeof (value as ImageContent).data === "string";
}
function id(image: ImageContent): string {
  return createHash("sha256").update(image.mimeType).update("\0").update(image.data).digest("hex");
}

/** Images remain in SQLite/provider input, not duplicated in every streamed frame.
 * A client can retrieve the original through get_image using the returned hash.
 */
export function imageReferences(value: unknown): unknown {
  if (isImage(value)) return { ...value, data: "", imageId: id(value), bytes: Buffer.byteLength(value.data, "base64") };
  if (Array.isArray(value)) return value.map(imageReferences);
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([key, child]) => [key, imageReferences(child)]));
  return value;
}

export function findImage(value: unknown, imageId: string): ImageContent | undefined {
  if (!/^[a-f0-9]{64}$/.test(imageId)) throw new Error("Invalid image ID");
  if (isImage(value)) return id(value) === imageId ? value : undefined;
  if (value && typeof value === "object") {
    for (const child of Object.values(value)) {
      const image = findImage(child, imageId);
      if (image) return image;
    }
  }
}
