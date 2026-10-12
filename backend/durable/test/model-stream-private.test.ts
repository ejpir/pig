import { expect, test } from "bun:test";
import { copyJson } from "@earendil-works/chord";
import { fauxAssistantMessage } from "@earendil-works/pi-ai/providers/faux";
import type { AssistantMessage } from "@earendil-works/pi-ai";
import { publicAssistantMessage } from "../src/model-stream.ts";

test("Bedrock binary thinking buffers are omitted before durable's JSON capture", () => {
  const chunks = [new Uint8Array([1, 2, 3]), new Uint8Array([4, 5])];
  const thinking = {
    type: "thinking" as const, thinking: "", redacted: true,
    thinkingSignature: "opaque-public-signature", index: 2, redactedChunks: chunks,
  };
  const text = { type: "text" as const, text: "answer", textSignature: "public-text-signature", index: 3 };
  const message: AssistantMessage = { ...fauxAssistantMessage(""), content: [thinking, text] };
  expect(() => copyJson(message, { omitUndefinedProperties: true })).toThrow();

  const projected = publicAssistantMessage(message);
  expect<unknown>(copyJson(projected, { omitUndefinedProperties: true })).toEqual(projected);
  expect(projected.content).toEqual([
    { type: "thinking", thinking: "", redacted: true, thinkingSignature: "opaque-public-signature" },
    { type: "text", text: "answer", textSignature: "public-text-signature" },
  ]);
  // The provider still needs these buffers/indexes to assemble the final signature.
  expect(thinking.redactedChunks).toBe(chunks);
  expect(thinking.index).toBe(2);
  expect(text.index).toBe(3);
});

test("OpenAI's streamIndex is omitted only from the tool block", () => {
  const argumentsValue = { index: 1, streamIndex: 2, partialArgs: "valid user argument", customInput: { redactedChunks: [3] } };
  const tool = {
    type: "toolCall" as const, id: "call", name: "write", arguments: argumentsValue,
    streamIndex: 0, partialArgs: "provider buffer", namespace: "tools", thoughtSignature: "public-signature",
  };
  const projected = publicAssistantMessage({ ...fauxAssistantMessage(""), content: [tool] });
  expect(projected.content[0]).toEqual({
    type: "toolCall", id: "call", name: "write", arguments: argumentsValue,
    namespace: "tools", thoughtSignature: "public-signature",
  });
  expect((projected.content[0] as typeof tool).arguments).toBe(argumentsValue);
  expect(tool.streamIndex).toBe(0);
  expect(tool.partialArgs).toBe("provider buffer");
});
