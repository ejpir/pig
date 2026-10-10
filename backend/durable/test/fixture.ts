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
const work = (name: string, replay: "safe" | "unsafe", release = "release") => defineTool({
  name, replay, description: "Test checkpoint recovery", parameters: Type.Object({}),
  execute: async (_, api, context) => {
    const cwd = api.env!.cwd!;
    appendFileSync(join(cwd, `${name}.log`), "execution\n");
    api.output("started; waiting for release\n");
    while (!existsSync(join(cwd, release))) {
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
    const written = text.filter((part) => part.type === "text").map((part) => part.text).join("\n");
    if (written.includes("<conversation>") && written.includes("Use this EXACT format:")) {
      return fauxAssistantMessage(written.includes("Additional focus: keep API names")
        ? "Compacted fixture history with API names."
        : "Compacted fixture history.");
    }
    const images = text.filter((part) => part.type === "image");
    return fauxAssistantMessage(`Received ${images.length} image(s): ${images.map((part) => `${part.mimeType}:${Buffer.from(part.data, "base64").length}`).join(", ")}`);
  }
  const after = transcript.messages.slice(lastUser + 1);
  // Hands work to scouts: two tasks side by side, or one that runs the crash-test tool.
  if (text === "delegate" || text === "delegate unsafe" || text === "delegate custom") {
    const result = after.find((message) => message.role === "toolResult");
    if (!result) {
      // "delegate custom" names agents no file defines, as a model splitting work up would.
      const args: Record<string, JsonValue> = text === "delegate"
        ? { tasks: [{ agent: "scout", task: "find alpha" }, { agent: "scout", task: "find beta" }] }
        : text === "delegate custom"
          ? { tasks: [
            { agent: "architecture", task: "find layers", instructions: "Review module boundaries.", tools: ["read", "grep"] },
            { agent: "code-quality", task: "find smells", instructions: "Review code quality." }] }
          : { agent: "scout", task: "unsafe" };
      return fauxAssistantMessage(fauxToolCall("subagent", args, { id: `${text.replace(" ", "-")}-call` }), { stopReason: "toolUse" });
    }
    const said = result.role === "toolResult" ? result.content.map((block) => block.type === "text" ? block.text : "").join("") : "";
    return fauxAssistantMessage(`Delegated: ${said}`);
  }
  if (typeof text === "string" && text.startsWith("find ")) return fauxAssistantMessage(`found ${text.slice(5)}`);
  // The subagents' answers, once they all finished.
  if (typeof text === "string" && text.startsWith("<subagent_report")) {
    return fauxAssistantMessage(`Heard back: ${text.split("\n").slice(3, -1).join(" ").trim()}`);
  }
  if (typeof text === "string" && text.startsWith("large fixture turn:")) {
    return fauxAssistantMessage("Finished the large fixture turn.");
  }
  if (typeof text === "string" && text.includes("<conversation>") && text.includes("Use this EXACT format:")) {
    return fauxAssistantMessage(text.includes("Additional focus: keep API names")
      ? "Compacted fixture history with API names."
      : "Compacted fixture history.");
  }
  if (text === "queued hold") {
    if (!after.some((message) => message.role === "toolResult")) {
      return fauxAssistantMessage(fauxToolCall("queued_work", {}, { id: "queued-work-call" }), { stopReason: "toolUse" });
    }
    return fauxAssistantMessage("Finished queued hold after cancellation race.");
  }
  if (text === "safe" || text === "unsafe") {
    if (!after.some((message) => message.role === "toolResult")) {
      return fauxAssistantMessage(fauxToolCall(`${text}_work`, {}, { id: `${text}-call` }), { stopReason: "toolUse" });
    }
    return fauxAssistantMessage(`Finished ${text} after recovery.`);
  }
  return fauxAssistantMessage(`Finished: ${text}`);
}));
const scout = { name: "scout", description: "Finds things", prompt: "Answer in two words.", scope: "user" as const, filePath: "/agents/scout.md" };
await run(models, [defineExtension({ name: "fixture", tools: [
  work("safe_work", "safe"), work("unsafe_work", "unsafe"), work("queued_work", "safe", "release-queued"),
] })],
  () => ({ ...noResources, agents: [scout] }));
