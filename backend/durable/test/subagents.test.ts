import { describe, expect, test } from "bun:test";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { Type, type Message } from "@earendil-works/pi-ai";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import {
  createRegistry, defineExtension, defineTool, Harness, InboxDoc, LiveDoc, MemoryStorage,
} from "@earendil-works/pi-durable";
import {
  definition, describe as step, gist, subagentExtension, submitSubagentInput,
} from "../src/subagents.ts";
import { noResources, type AgentDefinition } from "../src/commands.ts";

const scout: AgentDefinition = {
  name: "scout", description: "Finds things", prompt: "Be brief.\n", model: "claude-haiku-4-5",
  tools: ["read", "grep", "find", "ls", "bash"], scope: "user", filePath: "/agents/scout.md",
};

const gate = () => {
  let open!: () => void;
  const promise = new Promise<void>((resolve) => { open = resolve; });
  return { promise, open };
};

async function eventually<T>(read: () => Promise<T | undefined>): Promise<T> {
  for (let attempt = 0; attempt < 200; attempt++) {
    const value = await read();
    if (value !== undefined) return value;
    await Bun.sleep(5);
  }
  throw new Error("Timed out waiting for durable state");
}

const messageText = (message: Message): string => typeof message.content === "string"
  ? message.content
  : message.content.filter((block) => block.type === "text").map((block) => block.text).join("\n");

describe("subagent definitions", () => {
  test("a name no file has is an agent the call defines", () => {
    expect(definition({ agent: "architecture", task: "Review src", instructions: " Review module boundaries. ", tools: ["read", "bash"] }, [scout]))
      .toEqual({ name: "architecture", prompt: "Review module boundaries.", tools: ["read", "bash"] });
    // Without tools it gets the session's, and without instructions just its name.
    expect(definition({ agent: "helper", task: "Help" }, [])).toEqual({ name: "helper", prompt: "" });
  });

  test("a listed agent keeps its file, and the call can add to it", () => {
    expect(definition({ agent: "scout", task: "Look" }, [scout]))
      .toEqual({ name: "scout", prompt: "Be brief.", model: "claude-haiku-4-5", tools: ["read", "bash"] });
    expect(definition({ agent: "scout", task: "Look", instructions: "Only tests.", tools: ["read"] }, [scout]))
      .toEqual({ name: "scout", prompt: "Be brief.\n\nOnly tests.", model: "claude-haiku-4-5", tools: ["read"] });
  });

  test("stock Pi's search tools are bash, and unknown ones are left out", () => {
    expect(definition({ agent: "x", task: "t", tools: ["grep", "find", "ls", "web"] }, []).tools).toEqual(["bash"]);
  });

  test("JavaScript is opt-in for explicit child policies, never added to read-only lists", () => {
    expect(definition({agent:"x",task:"t",tools:["read","javascript"]},[]).tools).toEqual(["read","javascript"]);
    expect(definition({agent:"x",task:"t",tools:["read"]},[]).tools).toEqual(["read"]);
  });

  test("a step reads as going on, done or named", () => {
    expect(step("read", { path: "/repo/src/retry.ts" }, "now")).toBe("Reading retry.ts");
    expect(step("read", { path: "/repo/src/retry.ts" }, "done")).toBe("Read retry.ts");
    expect(step("bash", { command: "pnpm test\n--watch" }, "done")).toBe("Ran pnpm test");
    expect(step("bash", { command: "pnpm test" }, "named")).toBe("pnpm test");
    expect(step("edit", { path: "a/b.ts" }, "named")).toBe("edit b.ts");
  });

  test("an answer's gist is its first heading, past a preamble", () => {
    expect(gist("Perfect! Now I have a complete picture.\n\n## Architecture Review: **Runner**\n\nBody")).toBe("Architecture Review: Runner");
    expect(gist("found alpha")).toBe("found alpha");
  });
});

test("direct child input steers only an exact run and starts follow-ups only from idle", async () => {
  const context = BACKGROUND_CONTEXT;
  const started = gate(), release = gate();
  const faux = fauxProvider();
  const models = createModels();
  models.setProvider(faux.provider);
  const text = (messages: readonly Message[]) => {
    const user = [...messages].reverse().find((message) => message.role === "user");
    return user?.role === "user" ? messageText(user) : "";
  };
  faux.setResponses(Array.from({ length: 12 }, () => ({ messages }: { messages: readonly Message[] }) => {
    const latest = text(messages);
    if (latest === "initial child") {
      const hasResult = messages.some((message) => message.role === "toolResult" && message.toolCallId === "hold");
      return hasResult
        ? fauxAssistantMessage("initial child complete")
        : fauxAssistantMessage(fauxToolCall("bash", { command: "hold" }, { id: "hold" }), { stopReason: "toolUse" });
    }
    if (latest === "change course") return fauxAssistantMessage("steered child complete");
    if (latest === "another child task") return fauxAssistantMessage("follow-up child complete");
    throw new Error(`Unexpected faux request: ${latest}`);
  }));

  const registry = createRegistry();
  registry.install(defineExtension({ name: "direct-child-test", tools: [defineTool({
    name: "bash", description: "Hold one child run", replay: "safe",
    parameters: Type.Object({ command: Type.String() }),
    execute: async ({ command }) => {
      if (command !== "hold") throw new Error(`Unexpected command: ${command}`);
      started.open();
      await release.promise;
      return { output: [{ type: "text" as const, text: "released" }] };
    },
  })] }));
  const harness = await Harness.open(new MemoryStorage(), { models, registry }, context);
  try {
    const model = faux.getModel();
    const agent = { model: { provider: model.provider, modelId: model.id } };
    const parent = await harness.root(context, { agent });
    const child = await harness.createConversation({ ownership: { kind: "ownerless" }, agent }, context);
    const initial = await child.submit({ type: "input", content: "initial child" }, context);
    await started.promise;
    const view = await child.viewState(context);
    const runId = String((view.value.docs["pi.live"] as { run: { taskId: unknown } }).run.taskId);
    view.dispose();
    const result = { index: 0, agent: "scout", task: "initial child", status: "running" as const };

    await expect(submitSubagentInput(child, result, {
      message: "wrong run", requestId: "wrong-run", mode: "steer", expectedRunId: `${runId}-later`,
    }, context)).rejects.toThrow("ended or changed");
    await expect(submitSubagentInput(child, { ...result, status: "done" }, {
      message: "racing follow-up", requestId: "busy-follow-up", mode: "followUp",
    }, context)).rejects.toThrow("started other work");
    const receipt = await submitSubagentInput(child, result, {
      message: "change course", requestId: "exact-steer", mode: "steer", expectedRunId: runId,
    }, context);
    expect(receipt).toMatchObject({ mode: "steer", disposition: "queued", duplicate: false });
    expect(await submitSubagentInput(child, result, {
      message: "change course", requestId: "exact-steer", mode: "steer", expectedRunId: runId,
    }, context)).toMatchObject({ duplicate: true, submissionId: receipt.submissionId });
    await expect(submitSubagentInput(child, result, {
      message: "different payload", requestId: "exact-steer", mode: "steer", expectedRunId: runId,
    }, context)).rejects.toThrow("belongs to different subagent input");
    const routed = await child.commit(async (tx) => JSON.parse(JSON.stringify(await tx.doc(InboxDoc, child.id))), context);
    const parentInbox = await parent.commit(async (tx) => JSON.parse(JSON.stringify(await tx.doc(InboxDoc, parent.id))), context);
    expect(routed.items).toEqual([{ id: expect.anything(), mode: "steer", content: "change course" }]);
    expect(parentInbox.items).toEqual([]);

    release.open();
    await initial.wait(context);
    await child.waitForIdle(context);
    await expect(submitSubagentInput(child, result, {
      message: "stale steer", requestId: "stale-steer", mode: "steer", expectedRunId: runId,
    }, context)).rejects.toThrow("ended or changed");

    const follow = await submitSubagentInput(child, { ...result, status: "done" }, {
      message: "another child task", requestId: "child-follow-up", mode: "followUp",
    }, context);
    expect(follow).toMatchObject({ mode: "followUp", disposition: "started", duplicate: false });
    await child.waitForIdle(context);
    const childContext = await child.context(context);
    expect(childContext.messages.filter((message) => message.role === "user").map(messageText))
      .toEqual(["initial child", "change course", "another child task"]);
    expect((await parent.context(context)).messages).toEqual([]);
  } finally {
    release.open();
    await harness.close(context);
  }
});

test("background reports steer only their delegating run and wait behind a later user turn", async () => {
  const context = BACKGROUND_CONTEXT;
  const sameRunStarted = gate(), releaseSameRun = gate();
  const subagentStarted = gate(), releaseSubagent = gate();
  const laterStarted = gate(), releaseLater = gate();
  const laterContinuations: Message[][] = [];
  const sameRunReports: Message[][] = [];
  const laterReports: Message[][] = [];
  const faux = fauxProvider();
  const models = createModels();
  models.setProvider(faux.provider);
  const respond = (request: { messages: readonly Message[] }) => {
    const messages = structuredClone(request.messages) as Message[];
    const lastUser = messages.findLastIndex((message) => message.role === "user");
    const user = messages[lastUser];
    const text = user?.role === "user" ? messageText(user) : "";
    const after = messages.slice(lastUser + 1);
    const hasResult = (callId: string) => after.some((message) => message.role === "toolResult" && message.toolCallId === callId);
    if (text === "delegate same run") {
      return fauxAssistantMessage([
        fauxToolCall("subagent", { agent: "scout", task: "quick task" }, { id: "same-call" }),
        fauxToolCall("bash", { command: "hold same run" }, { id: "same-hold" }),
      ], { stopReason: "toolUse" });
    }
    if (text === "quick task") return fauxAssistantMessage("quick subagent complete");
    if (text === "delegate slow") {
      return hasResult("delegate-call")
        ? fauxAssistantMessage("Delegation started.")
        : fauxAssistantMessage(fauxToolCall("subagent", { agent: "scout", task: "slow task" }, { id: "delegate-call" }), { stopReason: "toolUse" });
    }
    if (text === "slow task") {
      return hasResult("subagent-hold")
        ? fauxAssistantMessage("subagent complete")
        : fauxAssistantMessage(fauxToolCall("bash", { command: "hold subagent" }, { id: "subagent-hold" }), { stopReason: "toolUse" });
    }
    if (text === "later user turn") {
      if (!hasResult("later-hold")) {
        return fauxAssistantMessage(fauxToolCall("bash", { command: "hold later" }, { id: "later-hold" }), { stopReason: "toolUse" });
      }
      laterContinuations.push(messages);
      return fauxAssistantMessage("later turn complete");
    }
    if (text.startsWith("<subagent_report")) {
      if (text.includes('call="same-call"')) sameRunReports.push(messages);
      else laterReports.push(messages);
      return fauxAssistantMessage("report handled separately");
    }
    throw new Error(`Unexpected faux request: ${text}`);
  };
  faux.setResponses(Array.from({ length: 20 }, () => respond));

  const registry = createRegistry();
  registry.install(defineExtension({ name: "subagent-routing-test", tools: [defineTool({
    name: "bash", description: "A deterministic gated command", replay: "safe",
    parameters: Type.Object({ command: Type.String() }),
    execute: async ({ command }) => {
      if (command === "hold same run") {
        sameRunStarted.open();
        await releaseSameRun.promise;
      } else if (command === "hold subagent") {
        subagentStarted.open();
        await releaseSubagent.promise;
      } else if (command === "hold later") {
        laterStarted.open();
        await releaseLater.promise;
      } else {
        throw new Error(`Unexpected command: ${command}`);
      }
      return { output: [{ type: "text" as const, text: `${command} complete` }] };
    },
  })] }));
  let harness!: Harness;
  registry.install(subagentExtension(models, () => harness, () => ({ ...noResources, agents: [scout] })));
  harness = await Harness.open(new MemoryStorage(), { models, registry }, context);
  try {
    const model = faux.getModel();
    const root = await harness.root(context, { agent: { model: { provider: model.provider, modelId: model.id } } });

    const sameRun = await root.submit({ type: "input", content: "delegate same run" }, context);
    await sameRunStarted.promise;
    const tied = await eventually(async () => root.commit(async (tx) => {
      const inbox = JSON.parse(JSON.stringify(await tx.doc(InboxDoc, root.id))) as { items: { mode: string; content?: unknown }[] };
      const live = JSON.parse(JSON.stringify(await tx.doc(LiveDoc, root.id))) as { run?: { inputs: number[] } };
      const report = inbox.items.find((item) => typeof item.content === "string" && item.content.includes('call="same-call"'));
      return report === undefined ? undefined : { report, live };
    }, context));
    expect(tied.report.mode).toBe("steer");
    expect(tied.live.run?.inputs.map(Number)).toEqual([Number(sameRun.id)]);
    releaseSameRun.open();
    await sameRun.wait(context);
    await root.waitForIdle(context);
    expect(sameRunReports).toHaveLength(1);

    const delegated = await root.submit({ type: "input", content: "delegate slow" }, context);
    await subagentStarted.promise;
    await delegated.wait(context);
    await root.waitForIdle(context);

    const later = await root.submit({ type: "input", content: "later user turn" }, context);
    await laterStarted.promise;
    releaseSubagent.open();

    const queued = await eventually(async () => root.commit(async (tx) => {
      const inbox = JSON.parse(JSON.stringify(await tx.doc(InboxDoc, root.id))) as { items: { mode: string; content?: unknown }[] };
      const live = JSON.parse(JSON.stringify(await tx.doc(LiveDoc, root.id))) as { run?: { inputs: number[] } };
      const report = inbox.items.find((item) => typeof item.content === "string" && item.content.startsWith("<subagent_report"));
      return report === undefined ? undefined : { report, live };
    }, context));
    expect(queued.report.mode).toBe("followUp");
    expect(queued.live.run?.inputs.map(Number)).toEqual([Number(later.id)]);

    releaseLater.open();
    await later.wait(context);
    await root.waitForIdle(context);
    expect(laterContinuations).toHaveLength(1);
    expect(laterContinuations[0].filter((message) => message.role === "user").map(messageText).at(-1)).toBe("later user turn");
    expect(laterReports).toHaveLength(1);
    const reportUsers = laterReports[0].filter((message) => message.role === "user").map(messageText);
    expect(reportUsers.at(-1)).toStartWith("<subagent_report");
    expect(laterReports[0].some((message) => message.role === "assistant" && messageText(message) === "later turn complete")).toBe(true);
  } finally {
    releaseSameRun.open();
    releaseSubagent.open();
    releaseLater.open();
    await harness.close(context);
  }
});
