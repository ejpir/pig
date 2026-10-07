/** Separate test executable: no real providers, credentials or network imports. */
import { appendFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { Type } from "@earendil-works/pi-ai";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxProvider, fauxAssistantMessage, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { defineExtension, defineTool } from "@earendil-works/pi-durable";
import type { JsonValue } from "@earendil-works/chord";
import { run } from "../src/run.ts";
import { noResources } from "../src/commands.ts";

const faux = fauxProvider({ tokensPerSecond: 120 });
const models = createModels();
models.setProvider(faux.provider);
const work = (name: string, replay: "safe" | "unsafe") => defineTool({
  name, replay, description: "Test checkpoint recovery", parameters: Type.Object({}),
  execute: async (_, api, context) => {
    const cwd = api.env!.cwd!;
    appendFileSync(join(cwd, `${name}.log`), "execution\n");
    api.output("started; waiting for release\n");
    while (!existsSync(join(cwd, "release"))) {
      if (context.abortSignal?.aborted) throw context.abortSignal.reason;
      await Bun.sleep(20);
    }
    return { content: [{ type: "text" as const, text: "work complete" }] };
  },
});
faux.setResponses(Array.from({ length: 100 }, () => (transcript) => {
  const lastUser = transcript.messages.findLastIndex((message) => message.role === "user");
  const message = transcript.messages[lastUser];
  const text = message?.role === "user" ? message.content : "";
  if (Array.isArray(text)) {
    const images = text.filter((part) => part.type === "image");
    return fauxAssistantMessage(`Received ${images.length} image(s): ${images.map((part) => `${part.mimeType}:${Buffer.from(part.data, "base64").length}`).join(", ")}`);
  }
  const after = transcript.messages.slice(lastUser + 1);
  // Hands work to scouts: two tasks side by side, or one that runs the crash-test tool.
  if (text === "delegate" || text === "delegate unsafe") {
    const result = after.find((message) => message.role === "toolResult");
    if (!result) {
      const args: Record<string, JsonValue> = text === "delegate"
        ? { tasks: [{ agent: "scout", task: "find alpha" }, { agent: "scout", task: "find beta" }] }
        : { agent: "scout", task: "unsafe" };
      return fauxAssistantMessage(fauxToolCall("subagent", args, { id: text === "delegate" ? "delegate-call" : "delegate-unsafe-call" }), { stopReason: "toolUse" });
    }
    const said = result.role === "toolResult" ? result.content.map((block) => block.type === "text" ? block.text : "").join("") : "";
    return fauxAssistantMessage(`Delegated: ${said}`);
  }
  if (typeof text === "string" && text.startsWith("find ")) return fauxAssistantMessage(`found ${text.slice(5)}`);
  if (text === "safe" || text === "unsafe") {
    if (!after.some((message) => message.role === "toolResult")) {
      return fauxAssistantMessage(fauxToolCall(`${text}_work`, {}, { id: `${text}-call` }), { stopReason: "toolUse" });
    }
    return fauxAssistantMessage(`Finished ${text} after recovery.`);
  }
  return fauxAssistantMessage(`Finished: ${text}`);
}));
const scout = { name: "scout", description: "Finds things", prompt: "Answer in two words.", scope: "user" as const, filePath: "/agents/scout.md" };
await run(models, [defineExtension({ name: "fixture", tools: [work("safe_work", "safe"), work("unsafe_work", "unsafe")] })],
  () => ({ ...noResources, agents: [scout] }));
