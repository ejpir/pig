import { expect, test } from "bun:test";
import { MAX_RECORD, records } from "../src/run.ts";

async function collect(chunks: Buffer[]) {
  const result = [];
  async function* input() { for (const chunk of chunks) yield chunk; }
  for await (const record of records(input())) result.push(record);
  return result;
}

test("LF framing preserves Unicode separators and split UTF-8", async () => {
  const message = "hello\u2028🌱\u2029world";
  const bytes = Buffer.from(JSON.stringify({ type: "prompt", message }) + "\n");
  expect(await collect(Array.from(bytes, (byte) => Buffer.from([byte])))).toEqual([{ type: "prompt", message }]);
});
test("multiple records and CRLF", async () => {
  expect(await collect([Buffer.from('{"type":"one"}\r\n{"type":"two"}\n')])).toEqual([{ type: "one" }, { type: "two" }]);
});
test("unterminated, invalid, non-object and malformed UTF-8 records fail closed", async () => {
  for (const bytes of [Buffer.from('{"type":"one"}'), Buffer.from('null\n'), Buffer.from('{}\n'), Buffer.from('[]\n'), Buffer.from([255, 10])]) {
    await expect(collect([bytes])).rejects.toThrow();
  }
});
test("record bounds", async () => {
  await expect(collect([Buffer.alloc(MAX_RECORD, 32)])).rejects.toThrow("64 MiB");
});
