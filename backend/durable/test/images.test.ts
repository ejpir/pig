import { expect, test } from "bun:test";
import { findImage, imageReferences, MAX_IMAGE_BYTES, promptContent } from "../src/images.ts";

const image = { type: "image" as const, mimeType: "image/png", data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==" };
test("text and image-only prompts retain real image bytes", () => {
  expect(promptContent("hi", undefined, false)).toBe("hi");
  expect(promptContent("hi", [], false)).toBe("hi");
  expect(promptContent("hi", [image], true)).toEqual([{ type: "text", text: "hi" }, image]);
  expect(promptContent("", [image], true)).toEqual([image]);
});
test("invalid, mismatched, oversized and non-vision images fail before admission", () => {
  for (const images of [null, {}, [null], [{...image, mimeType:"image/jpeg"}], [{...image, data:"not base64"}], [{...image, data:""}], Array(5).fill(image), [{...image, data:Buffer.alloc(MAX_IMAGE_BYTES + 1).toString("base64")}]]) {
    expect(() => promptContent("", images, true)).toThrow();
  }
  expect(() => promptContent("", [image], false)).toThrow("does not accept images");
});
test("streamed references stay small and retrieval returns the persisted original", () => {
  const view = { entries: [{ model: [{ role: "user", content: [image] }] }] };
  const wire = imageReferences(view) as any;
  const reference = wire.entries[0].model[0].content[0];
  expect(reference.data).toBe("");
  expect(reference.bytes).toBeGreaterThan(0);
  expect(findImage(view, reference.imageId)).toEqual(image);
  expect(view.entries[0].model[0].content[0].data).toBe(image.data);
  expect(findImage(view, "0".repeat(64))).toBeUndefined();
  expect(() => findImage(view, "../../file")).toThrow();
});
