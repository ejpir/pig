/** Separate test executable: no real providers, credentials or network imports. */
import { appendFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { Type } from "@earendil-works/pi-ai";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxProvider, fauxAssistantMessage, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { defineExtension, defineTool } from "@earendil-works/pi-durable";
import { run } from "../src/run.ts";

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
  const after = transcript.messages.slice(lastUser + 1);
  if (text === "safe" || text === "unsafe") {
    if (!after.some((message) => message.role === "toolResult")) {
      return fauxAssistantMessage(fauxToolCall(`${text}_work`, {}, { id: `${text}-call` }), { stopReason: "toolUse" });
    }
    return fauxAssistantMessage(`Finished ${text} after recovery.`);
  }
  return fauxAssistantMessage(`Finished: ${text}`);
}));
await run(models, [defineExtension({ name: "fixture", tools: [work("safe_work", "safe"), work("unsafe_work", "unsafe")] })]);
