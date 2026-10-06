import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { imageType, readImage } from "../src/read.ts";

const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");

describe("reading images", () => {
  const cwd = mkdtempSync(join(tmpdir(), "pi-read-"));
  const api = { env: new NodeExecutionEnv({ cwd }) };

  test("recognises what providers accept by signature", () => {
    expect(imageType(PNG)).toBe("image/png");
    expect(imageType(Buffer.from([0xff, 0xd8, 0xff, 0xe0]))).toBe("image/jpeg");
    expect(imageType(Buffer.from("GIF89a..."))).toBe("image/gif");
    expect(imageType(Buffer.from("hello"))).toBeUndefined();
  });

  test("an image comes back as image content, named by its path", async () => {
    writeFileSync(join(cwd, "shot.png"), PNG);
    const result = await readImage("shot.png", api, BACKGROUND_CONTEXT);
    expect(result?.content[0]).toEqual({ type: "text", text: "shot.png [image/png]" });
    expect(result?.content[1]).toEqual({ type: "image", data: PNG.toString("base64"), mimeType: "image/png" });
  });

  test("text is left to the stock tool, and huge images are refused", async () => {
    writeFileSync(join(cwd, "notes.txt"), "hi");
    expect(await readImage("notes.txt", api, BACKGROUND_CONTEXT)).toBeUndefined();
    writeFileSync(join(cwd, "big.png"), Buffer.concat([PNG, Buffer.alloc(1024 * 1024)]));
    expect(readImage("big.png", api, BACKGROUND_CONTEXT)).rejects.toThrow("at most 1 MB");
  });
});
