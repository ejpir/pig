import { setSwarmEnabled } from "../src/session-settings.ts";
import { expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { Type, type Message, type TranscriptContext, type ToolCall } from "@earendil-works/pi-ai";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import {
  createRegistry, defineExtension, defineTool, Harness, MemoryStorage,
  type ConversationId, type Storage, type TaskId,
} from "@earendil-works/pi-durable";
import { JsonlStorage } from "@earendil-works/pi-durable/storage/jsonl";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { noResources } from "../src/commands.ts";
import { Calls, subagentExtension, submitSubagentInput } from "../src/subagents.ts";
import { Children, SwarmTools } from "../src/swarm.ts";

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const text = (message: Message) => typeof message.content === "string" ? message.content
  : message.content.filter((block) => block.type === "text").map((block) => block.text).join("\n");
const lastUser = (messages: readonly Message[]) => text([...messages].reverse().find((message) => message.role === "user")!);
const toolResult = (messages: readonly Message[], id: string) => messages
  .slice(messages.findLastIndex((message) => message.role === "user") + 1)
  .find((message) => message.role === "toolResult" && message.toolCallId === id);
const call = (name: string, args: ToolCall["arguments"], id: string) =>
  fauxAssistantMessage(fauxToolCall(name, args, { id }), { stopReason: "toolUse" });
const gate = () => {
  let open!: () => void;
  const promise = new Promise<void>((resolve) => { open = resolve; });
  return { promise, open };
};
async function eventually<T>(read: () => Promise<T | undefined>): Promise<T> {
  for (let attempt = 0; attempt < 400; attempt++) {
    const value = await read();
    if (value !== undefined) return value;
    await Bun.sleep(5);
  }
  throw new Error("Timed out waiting for original delegated result");
}

async function fixture(
  respond: (request: TranscriptContext) => ReturnType<typeof fauxAssistantMessage> | Promise<ReturnType<typeof fauxAssistantMessage>>,
  read: () => Promise<void> = async () => {},
  storage: Storage = new MemoryStorage(),
  throwBeforeOutput?: string,
) {
  const faux = fauxProvider(), models = createModels(), registry = createRegistry();
  models.setProvider(faux.provider);
  // Throw at the harness's stream entry point: pi-ai otherwise converts a
  // provider exception to an assistant error message, missing the no-entry case.
  const streamSimple = models.streamSimple.bind(models);
  models.streamSimple = (model, request, options) => {
    if (lastUser(request.messages) === throwBeforeOutput) throw new Error("provider failed before streaming");
    return streamSimple(model, request, options);
  };
  faux.setResponses(Array.from({ length: 100 }, () => respond));
  const coding = defineExtension({ name: "lifetime-test-coding", tools: [defineTool({
    name: "read", description: "Gated read", replay: "safe", parameters: Type.Object({ path: Type.String() }),
    execute: async () => { await read(); return { output: [{ type: "text" as const, text: "read complete" }] }; },
  })] });
  registry.install(coding);
  let harness!: Harness;
  const subagents = subagentExtension(models, () => harness, () => noResources);
  registry.install(subagents);
  registry.install(SwarmTools);
  const options = { models, registry, settings: { extensions: [coding, subagents] } };
  harness = await Harness.open(storage, options, context);
  const readState = (id: TaskId) => harness.snapshot(Children, id, context);
  let lastState: Awaited<ReturnType<typeof readState>>;
  // Task documents retire once the crew terminates. Keep its last committed
  // revision so final assertions still inspect the frozen result, not Calls' preview.
  const observeState = () => harness.subscribeCommits((publication) => {
    for (const change of publication.changes) {
      if (change.type === "document" && change.record.kind === "app.subagents" && change.value !== null) {
        lastState = clone(change.value) as NonNullable<typeof lastState>;
      }
    }
  });
  observeState();
  const model = faux.getModel();
  let root = await harness.root(context, { agent: { model: { provider: model.provider, modelId: model.id } } });
  await setSwarmEnabled(root, true, context); // Explicit test admission, never the default.
  const details = () => root.commit(async (tx) => clone((await tx.doc(Calls, root.id)).calls["lifetime-call"]), context);
  const crew = () => root.commit(async (tx) => {
    const tasks = await tx.scanTasks({ kind: "app.subagent-crew" }, 1);
    return tasks.items[0].id;
  }, context);
  const state = async () => {
    const id = await crew();
    return clone((await readState(id)) ?? lastState!);
  };
  return {
    get harness() { return harness; }, get root() { return root; }, details, crew, state,
    close: () => harness.close(context),
    reopen: async (storage: Storage) => {
      harness = await Harness.open(storage, options, context);
      root = await harness.root(context);
      observeState();
      harness.resume();
    },
  };
}

for (const legacy of [false, true]) test(
  `original swarm answers survive a different direct follow-up and JSONL restart${legacy ? " (legacy endedAt without result)" : ""}`,
  async () => {
    const directory = await mkdtemp(join(tmpdir(), "pi-swarm-lifetime-"));
    const peer2Blocked = gate(), releaseOldRead = gate(), restartedRead = gate(), finishPeer2 = gate();
    const followupBlocked = gate(), finishFollowup = gate();
    const original = "Original delegated answer\n" + "x".repeat(4200) + "\nFinal finding.";
    const followup = "A completely different follow-up answer.";
    const reports: string[] = [];
    let restarting = false;
    let replayedError: Message | undefined;
    const send = { to: "peer-2", kind: "finding", text: "original finding", requestId: "original-send" };
    const run = await fixture(async ({ messages }) => {
      const latest = lastUser(messages);
      if (latest === "delegate") return toolResult(messages, "lifetime-call") ? fauxAssistantMessage("Delegated.")
        : call("subagent", { tasks: [{ agent: "same", task: "peer one" }, { agent: "same", task: "peer two" }], swarm: true }, "lifetime-call");
      if (latest.startsWith("<subagent_report")) { reports.push(latest); return fauxAssistantMessage("Report handled."); }
      if (latest === "peer one") {
        if (!toolResult(messages, "original-send-tool")) return call("swarm_send", send, "original-send-tool");
        await peer2Blocked.promise;
        return fauxAssistantMessage(original);
      }
      if (latest === "peer two") {
        if (!toolResult(messages, "inbound")) return call("swarm_send", {
          to: "peer-1", kind: "question", text: "cancel this when peer one ends", requestId: "inbound",
        }, "inbound");
        if (!toolResult(messages, "hold")) return call("read", { path: "hold" }, "hold");
        return fauxAssistantMessage("Peer two original answer.");
      }
      if (latest === "direct follow-up") {
        const replay = toolResult(messages, "replayed-send-tool");
        if (!replay) return call("swarm_send", send, "replayed-send-tool");
        replayedError = replay;
        followupBlocked.open();
        await finishFollowup.promise;
        return fauxAssistantMessage(followup);
      }
      throw new Error(`Unexpected request: ${latest}`);
    }, async () => {
      if (restarting) { restartedRead.open(); await finishPeer2.promise; }
      else { peer2Blocked.open(); await releaseOldRead.promise; }
    }, await JsonlStorage.open(directory, new NodeExecutionEnv({ cwd: directory }), context));
    try {
      await (await run.root.submit({ type: "input", content: "delegate" }, context)).wait(context);
      const intermediate = await eventually(async () => {
        const details = await run.details();
        return details?.results[0].status === "done" ? details : undefined;
      });
      expect(intermediate.swarm!.closed).toBe(false);
      expect(intermediate.results[1].status).toBe("running");
      expect(intermediate.results[0].output).toBe(`${original.slice(0, 500)}…`);
      const before = await run.state();
      const frozen = before.children[0].result!;
      expect(frozen).toMatchObject({ status: "done", output: original, endedAt: before.children[0].endedAt });
      expect(before.swarm!.messages.find((message) => message.to === "peer-1")!.status).toBe("cancelled");
      expect(before.swarm!.messages.find((message) => message.to === "peer-2")!.status).toBe("queued");

      const child = (await run.harness.conversation(Number(frozen.conversationId) as unknown as ConversationId, context))!;
      await child.waitForIdle(context);
      await submitSubagentInput(child, intermediate.results[0], {
        message: "direct follow-up", requestId: "direct-followup", mode: "followUp",
      }, context);
      await followupBlocked.promise;
      expect(replayedError?.role === "toolResult" && replayedError.isError).toBe(true);
      expect(text(replayedError!)).toContain("has ended");
      expect((await run.state()).children[0].result).toEqual(frozen);
      expect((await run.details()).results[0]).toEqual(intermediate.results[0]);
      finishFollowup.open();
      await child.waitForIdle(context);
      expect((await child.context(context)).messages.filter((message) => message.role === "assistant").map(text).at(-1)).toBe(followup);
      expect((await run.state()).children[0].result).toEqual(frozen);
      expect((await run.details()).results[0]).toEqual(intermediate.results[0]);

      if (legacy) {
        const crew = await run.crew();
        await run.root.commit(async (tx) => { delete (await tx.doc(Children, crew)).children[0].result; }, context);
      }
      // Seal writes before releasing the safe read; its original generation is recovered, not answered on close.
      const closing = run.close();
      releaseOldRead.open();
      await closing;
      restarting = true;
      await run.reopen(await JsonlStorage.open(directory, new NodeExecutionEnv({ cwd: directory }), context));
      await restartedRead.promise;
      const restored = await eventually(async () => {
        const details = await run.details();
        return details?.resumed && details.results[0].status === "done"
          && details.results.every((result) => result.conversationId !== undefined) ? details : undefined;
      });
      expect(restored.results[0]).toEqual(intermediate.results[0]);
      expect((await run.state()).children[0].result).toEqual(frozen);
      expect(restored.results.map((result) => result.conversationId)).toEqual(intermediate.results.map((result) => result.conversationId));
      expect(restored.results[1].status).toBe("running");
      expect(restored.swarm!.closed).toBe(false);
      finishPeer2.open();
      const completed = await eventually(async () => {
        const details = await run.details();
        return details?.swarm?.closed && details.results.every((result) => result.status === "done") ? details : undefined;
      });
      await eventually(async () => reports.length ? reports : undefined);
      expect(completed.results.map((result) => result.status)).toEqual(["done", "done"]);
      expect(completed.results[0]).toEqual(intermediate.results[0]);
      expect((await run.state()).children[0].result).toEqual(frozen);
      expect(reports).toHaveLength(1);
      expect(reports[0]).toContain(original);
      expect(reports[0]).toContain("Peer two original answer.");
      expect(reports[0]).not.toContain(followup);
    } finally {
      releaseOldRead.open(); finishPeer2.open(); finishFollowup.open();
      await run.close();
      await rm(directory, { recursive: true, force: true });
    }
  }, 15_000,
);

for (const mode of ["single", "chain"] as const) test(`${mode} forwards full answers while keeping app previews short`, async () => {
  const original = "Original answer\n" + "x".repeat(12_000) + "\nFinal finding.";
  const final = mode === "single" ? original : "Chain answer\n" + "y".repeat(12_000) + "\nFinal recommendation.";
  const reports: string[] = [];
  const run = await fixture(({ messages }) => {
    const latest = lastUser(messages);
    if (latest === "delegate") return toolResult(messages, "lifetime-call") ? fauxAssistantMessage("Delegated.")
      : call("subagent", mode === "single"
        ? { agent: "scout", task: "original" }
        : { chain: [{ agent: "scout", task: "original" }, { agent: "reviewer", task: "Review: {previous}" }] }, "lifetime-call");
    if (latest === "original") return fauxAssistantMessage(original);
    if (latest.startsWith("Review: ")) {
      expect(latest).toBe(`Review: ${original}`);
      return fauxAssistantMessage(final);
    }
    if (latest.startsWith("<subagent_report")) { reports.push(latest); return fauxAssistantMessage("Handled."); }
    throw new Error(`Unexpected request: ${latest}`);
  });
  try {
    await (await run.root.submit({ type: "input", content: "delegate" }, context)).wait(context);
    await eventually(async () => reports.length ? reports : undefined);
    expect(reports).toHaveLength(1);
    expect(reports[0]).toContain(final);
    const details = await run.details();
    expect(details.results.every((result) => result.status === "done")).toBe(true);
    expect(details.results[0].output).toBe(`${original.slice(0, 500)}…`);
    expect(details.results.at(-1)!.output).toBe(`${final.slice(0, 500)}…`);
  } finally {
    await run.close();
  }
});

test("the crew freezes original settlement without waiting for a queued later child turn", async () => {
  const originalBlocked = gate(), finishOriginal = gate(), followupBlocked = gate(), finishFollowup = gate();
  const peer2Blocked = gate(), finishPeer2 = gate();
  const reports: string[] = [];
  const run = await fixture(async ({ messages }) => {
    const latest = lastUser(messages);
    if (latest === "delegate") return toolResult(messages, "lifetime-call") ? fauxAssistantMessage("Delegated.")
      : call("subagent", { tasks: [{ agent: "a", task: "original" }, { agent: "b", task: "blocked peer" }], swarm: true }, "lifetime-call");
    if (latest === "original") { originalBlocked.open(); await finishOriginal.promise; return fauxAssistantMessage("Original only."); }
    if (latest === "queued later turn") { followupBlocked.open(); await finishFollowup.promise; return fauxAssistantMessage("Later only."); }
    if (latest === "blocked peer") { peer2Blocked.open(); await finishPeer2.promise; return fauxAssistantMessage("Peer complete."); }
    if (latest.startsWith("<subagent_report")) { reports.push(latest); return fauxAssistantMessage("Handled."); }
    throw new Error(`Unexpected request: ${latest}`);
  });
  try {
    await (await run.root.submit({ type: "input", content: "delegate" }, context)).wait(context);
    await originalBlocked.promise;
    await peer2Blocked.promise;
    const state = await run.state();
    const child = (await run.harness.conversation(state.children[0].id as unknown as ConversationId, context))!;
    // Queue through the SDK while the original run is busy. It starts at the
    // original answer boundary, so conversation-wide waitForIdle would hang.
    const later = await child.submit({ type: "input", content: "queued later turn", whenBusy: "followUp" }, context);
    finishOriginal.open();
    await followupBlocked.promise;
    const frozen = await eventually(async () => (await run.state()).children[0].result);
    expect(frozen).toMatchObject({ status: "done", output: "Original only." });
    expect((await later.status(context)).status).toBe("placed");
    finishPeer2.open();
    await eventually(async () => reports.length ? reports : undefined);
    expect(reports[0]).toContain("Original only.");
    expect(reports[0]).not.toContain("Later only.");
    expect((await later.status(context)).status).toBe("placed");
    finishFollowup.open();
    await later.wait(context);
    expect((await run.state()).children[0].result).toEqual(frozen);
  } finally {
    finishOriginal.open(); finishPeer2.open(); finishFollowup.open();
    await run.close();
  }
});

test("a provider thrown before any assistant output freezes failed, not running", async () => {
  const reports: string[] = [];
  const run = await fixture(({ messages }) => {
    const latest = lastUser(messages);
    if (latest === "delegate") return toolResult(messages, "lifetime-call") ? fauxAssistantMessage("Delegated.")
      : call("subagent", { agent: "scout", task: "early failure" }, "lifetime-call");
    if (latest.startsWith("<subagent_report")) { reports.push(latest); return fauxAssistantMessage("Failure reported."); }
    throw new Error(`Unexpected request: ${latest}`);
  }, undefined, undefined, "early failure");
  try {
    await (await run.root.submit({ type: "input", content: "delegate" }, context)).wait(context);
    const failed = await eventually(async () => {
      const details = await run.details();
      return details?.results[0].status === "failed" ? details.results[0] : undefined;
    });
    expect(failed.now).toContain("provider failed before streaming");
    expect(failed.output).toBeUndefined();
    expect(failed.endedAt).toBeNumber();
    const childId = Number(failed.conversationId) as unknown as ConversationId;
    const child = (await run.harness.conversation(childId, context))!;
    expect((await child.context(context)).entries.flatMap((entry) => entry.model ?? []).filter((message) => message.role === "assistant")).toEqual([]);
    const receipt = await run.root.commit((tx) => tx.submissionByRequest(childId, "subagent-0"), context);
    expect(receipt).toMatchObject({ status: "unanswered", reason: "faulted" });
    expect((await run.state()).children[0]).toMatchObject({ endedAt: failed.endedAt, result: failed });
    await eventually(async () => reports.length ? reports : undefined);
    expect(reports[0]).toContain("(failed: provider failed before streaming)");
  } finally { await run.close(); }
});
