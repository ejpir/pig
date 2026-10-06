/** The durable `read` tool, plus images: a screenshot Pi took can be looked at
 * and shown in the apps. The stock tool refuses images; text stays its job. */
import type { Context } from "@earendil-works/chord";
import { wrapTool } from "@earendil-works/pi-durable";
import { getOrThrow } from "@earendil-works/pi-durable/env";
import { createReadTool } from "@earendil-works/pi-durable/tools";
import { MAX_IMAGE_BYTES } from "./images.ts";

/** What providers accept, by the bytes' signature. */
export function imageType(bytes: Uint8Array): string | undefined {
  const starts = (offset: number, ascii: string) =>
    [...ascii].every((char, index) => bytes[offset + index] === char.charCodeAt(0));
  if (bytes[0] === 0x89 && starts(1, "PNG\r\n\x1a\n")) return "image/png";
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return "image/jpeg";
  if (starts(0, "GIF87a") || starts(0, "GIF89a")) return "image/gif";
  if (starts(0, "RIFF") && starts(8, "WEBP")) return "image/webp";
  return undefined;
}

type ReadApi = { env?: { absolutePath(path: string, context: Context): Promise<unknown>; readBinaryFile(path: string, context: Context): Promise<unknown> } };

/** An image file as the model and the apps see it, or undefined for anything else. */
export async function readImage(path: string, api: ReadApi, context: Context) {
  const env = api.env;
  if (!env) return undefined;
  const absolute = getOrThrow(await env.absolutePath(path.replace(/^@/, ""), context) as never) as string;
  const bytes = getOrThrow(await env.readBinaryFile(absolute, context) as never) as Uint8Array;
  const mimeType = imageType(bytes);
  if (!mimeType) return undefined;
  if (bytes.byteLength > MAX_IMAGE_BYTES) {
    throw new Error(`${path} is ${(bytes.byteLength / 1e6).toFixed(1)} MB; images can be at most 1 MB. Make it smaller first, for example a screenshot with a smaller --window-size.`);
  }
  return {
    content: [
      { type: "text" as const, text: `${path} [${mimeType}]` },
      { type: "image" as const, data: Buffer.from(bytes).toString("base64"), mimeType },
    ],
  };
}

const read = createReadTool();

export const imageRead = wrapTool(read, (tool) => ({
  ...tool,
  description: `${tool.description} PNG, JPEG, GIF and WebP images up to 1 MB are shown to you, and to the person in their apps.`,
  async execute(args, api, context) {
    return (await readImage(args.path, api as ReadApi, context)) ?? tool.execute(args, api, context);
  },
}));
