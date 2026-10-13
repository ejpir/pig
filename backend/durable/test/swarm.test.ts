import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { Type, type Message, type TranscriptContext, type ToolCall } from "@earendil-works/pi-ai";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import {
  createRegistry, defineExtension, defineTask, defineTool, Harness, MemoryStorage,
  type ConversationId, type Storage,
} from "@earendil-works/pi-durable";
import { JsonlStorage } from "@earendil-works/pi-durable/storage/jsonl";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { Calls, subagentExtension } from "../src/subagents.ts";
import { Metadata, setSwarmEnabled, swarmEnabled } from "../src/session-settings.ts";
import {
  cancelPeer, Children, closeSwarm, createSwarm, SWARM_LIMITS, SwarmTools, swarmMembers,
  swarmInstructions, swarmProjection, swarmReceive, swarmSend, type SwarmSend,
} from "../src/swarm.ts";
import { noResources, type AgentDefinition } from "../src/commands.ts";

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const text = (message: Message) => typeof message.content === "string" ? message.content
  : message.content.filter((block) => block.type === "text").map((block) => block.text).join("\n");
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
  throw new Error("Timed out waiting for durable swarm state");
}

// A live but unscheduled crew makes admission tests deterministic. Real providers are exercised below.
const Crew = defineTask<{}, { phase: "run" }, null>({
  name: "app.subagent-crew", version: 1, initial: () => ({ phase: "run" }),
  phases: { run: async (_, runtime, ctx) => { await runtime.sleep(Date.now() + 60_000, ctx); } },
  abort: async (_, runtime, ctx) => { await runtime.commit(() => ({ status: "terminal", outcome: { status: "aborted" } }), ctx); },
});

async function mailbox(peers = 2, storage: Storage = new MemoryStorage()) {
  const models = createModels(), registry = createRegistry();
  registry.install(defineExtension({ name: "test-crew", tasks: [Crew] }));
  let harness = await Harness.open(storage, { models, registry }, context);
  const root = await harness.root(context);
  const items = Array.from({ length: peers }, (_, index) => ({ agent: "same-name", task: `task ${index + 1}` }));
  const crew = await root.commit(async (tx) => {
    const crew = await tx.createTask(Crew, {}, { ownership: { kind: "conversation" }, background: true });
    const swarm = createSwarm(root.id, "origin", items);
    (await tx.doc(Children, crew)).swarm = swarm;
    (await tx.doc(Calls, root.id)).calls.origin = {
      version: 1, background: true, mode: "parallel", results: [], swarm: swarmProjection(swarm),
    };
    return crew;
  }, context);
  const children: ConversationId[] = [];
  for (let index = 0; index < peers; index++) {
    const child = await harness.createConversation({ ownership: { kind: "task", taskId: crew } }, context);
    children.push(child.id);
    await root.commit(async (tx) => { (await tx.doc(Children, crew)).children[index] = { id: Number(child.id), startedAt: 1 }; }, context);
  }
  const commit = <T>(callback: Parameters<typeof root.commit<T>>[0]) => harness.commit(callback, context);
  return {
    root, crew, children, models, registry,
    send: (peer: number, input: SwarmSend, now = 100_000) => commit((tx) => swarmSend(tx, children[peer], input, now)),
    receive: (peer: number, requestId: string, limit = 8, now = 100_001) => commit((tx) => swarmReceive(tx, children[peer], { requestId, limit }, now)),
    state: () => commit(async (tx) => clone(await tx.doc(Children, crew))),
    projection: () => commit(async (tx) => clone((await tx.doc(Calls, root.id)).calls.origin.swarm!)),
    commit,
    reopen: async (storage: Storage) => { harness = await Harness.open(storage, { models, registry }, context); },
    close: () => harness.close(context),
  };
}
const finding = (requestId: string, to = "peer-2", message = "found a boundary"): SwarmSend => ({ to, kind: "finding", text: message, requestId });

describe("durable swarm mailboxes", () => {
  test("stable root-owned roster, exact routing, dedup and collision checks; Calls is not authority", async () => {
    const box = await mailbox();
    try {
      const roster = await box.commit((tx) => swarmMembers(tx, box.children[0]));
      expect(roster).toMatchObject({ self: "peer-1", closed: false, members: [
        { id: "peer-1", agent: "same-name", task: "task 1" }, { id: "peer-2", agent: "same-name", task: "task 2" },
      ] });
      expect(createSwarm(box.root.id, "origin", roster.members).id).toBe(roster.id);
      expect(createSwarm(box.root.id, "other", roster.members).id).not.toBe(roster.id);
      // A stale/forged app projection cannot authorize messages, or prevent an active peer from using the broker.
      await box.commit(async (tx) => { (await tx.doc(Calls, box.root.id)).calls.origin.swarm!.closed = true; });
      const first = await box.send(0, finding("persistent"));
      expect(first).toMatchObject({ duplicate: false, message: { from: "peer-1", to: "peer-2", status: "queued", sequence: 1 } });
      expect(await box.send(0, finding("persistent"))).toEqual({ ...first, duplicate: true });
      await expect(box.send(0, finding("persistent", "peer-2", "changed"))).rejects.toThrow("different peer message");
      await expect(box.send(0, { ...finding("persistent"), kind: "question" })).rejects.toThrow("different peer message");
      expect((await box.state()).swarm!.messages.length).toBe(1);
      expect((await box.projection()).closed).toBe(false);
      expect((await box.receive(0, "sender-read")).messages).toEqual([]);
      const read = await box.receive(1, "recipient-read");
      expect(read.messages).toHaveLength(1);
      expect(read.messages[0]).toMatchObject({ id: first.message.id, status: "received", receivedAt: 100_001 });
      expect((await box.projection()).messages[0].status).toBe("received");
    } finally { await box.close(); }
  });

  test("invalid, foreign, self, non-roster and root senders cannot route; sender is child-bound", async () => {
    const box = await mailbox();
    try {
      for (const to of ["peer-1", "peer-3", "same-name", "other-swarm/peer-2", "__proto__"]) {
        await expect(box.send(0, finding(`invalid-${to.replace(/\W/g, "_")}`, to))).rejects.toThrow("exact peer ID");
      }
      await expect(box.commit((tx) => swarmSend(tx, box.root.id, finding("root")))).rejects.toThrow("only available to swarm children");
      const stranger = await box.root.commit(async (tx) => {
        return (await tx.createConversation({ ownership: { kind: "task", taskId: box.crew } })).id;
      }, context);
      await expect(box.commit((tx) => swarmSend(tx, stranger, finding("stranger")))).rejects.toThrow("not in this swarm roster");
      for (const requestId of ["", "a".repeat(129), "__proto__", "constructor", "prototype", "a:b"]) {
        await expect(box.send(0, finding(requestId))).rejects.toThrow("Invalid persistent requestId");
      }
      const forged = await box.send(0, { ...finding("bound"), from: "peer-2" } as SwarmSend);
      expect(forged.message.from).toBe("peer-1");
    } finally { await box.close(); }
  });

  test("replies correlate only to retrieved questions from that exact peer", async () => {
    const box = await mailbox(3);
    try {
      const question = await box.send(0, { ...finding("question"), kind: "question" });
      await expect(box.send(0, { ...finding("question", "peer-3"), kind: "question" })).rejects.toThrow("different peer message");
      const reply: SwarmSend = { to: "peer-1", kind: "reply", text: "yes, the boundary is here", requestId: "answer", replyTo: question.message.id };
      await expect(box.send(1, reply)).rejects.toThrow("explicitly received");
      await box.receive(1, "retrieve");
      await expect(box.send(2, reply)).rejects.toThrow("explicitly received");
      await expect(box.send(1, { ...reply, replyTo: "foreign-id" })).rejects.toThrow("explicitly received");
      await expect(box.send(1, { ...reply, kind: "finding" })).rejects.toThrow("only valid for replies");
      const answer = await box.send(1, reply);
      expect(answer.message).toMatchObject({ from: "peer-2", to: "peer-1", kind: "reply", replyTo: question.message.id });
      expect(await box.send(1, reply)).toMatchObject({ duplicate: true, message: { id: answer.message.id } });
      expect((await box.receive(0, "answer-read")).messages[0].replyTo).toBe(question.message.id);
    } finally { await box.close(); }
  });

  test("bounded FIFO reads replay the exact batch, dedup across fresh reads, and retain empty-read receipts", async () => {
    const box = await mailbox();
    try {
      const empty = await box.receive(1, "empty");
      for (let index = 0; index < 3; index++) await box.send(0, finding(`m-${index}`));
      expect(await box.receive(1, "empty")).toEqual({ ...empty, duplicate: true });
      const first = await box.receive(1, "batch", 2);
      expect(first.messages.map((message) => message.sequence)).toEqual([1, 2]);
      expect(await box.receive(1, "batch", 2, 999_999)).toEqual({ ...first, duplicate: true });
      await expect(box.receive(1, "batch", 1)).rejects.toThrow("different peer read");
      expect((await box.receive(1, "next", 16)).messages.map((message) => message.sequence)).toEqual([3]);
      expect((await box.receive(1, "last")).messages).toEqual([]);
      for (const limit of [0, 17, 1.5]) await expect(box.receive(1, `bad-${String(limit).replace(".", "_")}`, limit)).rejects.toThrow("1–16");
    } finally { await box.close(); }
  });

  test("UTF-8 text, burst, pending and lifetime send caps reject without consuming IDs", async () => {
    const box = await mailbox();
    try {
      await expect(box.send(0, finding("size", "peer-2", "é".repeat(2049)))).rejects.toThrow("4096 UTF-8 bytes");
      await expect(box.send(0, finding("blank", "peer-2", "  \n"))).rejects.toThrow("nonempty");
      await box.send(0, finding("size", "peer-2", "é".repeat(2048)));
      expect((await box.projection()).messages[0].text.length).toBe(500);
      for (let index = 1; index < 8; index++) await box.send(0, finding(`burst-${index}`));
      await expect(box.send(0, finding("rate"))).rejects.toThrow("rate limit");
      expect(await box.send(0, finding("burst-1"))).toMatchObject({ duplicate: true });
      for (let index = 8; index < 32; index++) await box.send(0, finding(`cap-${index}`), 100_000 + index * 11_000);
      await expect(box.send(0, finding("pending"), 1_000_000)).rejects.toThrow("pending mailbox cap");
      await box.receive(1, "drain-1", 16);
      await box.receive(1, "drain-2", 16);
      for (let index = 32; index < 64; index++) await box.send(0, finding(`cap-${index}`), 1_000_000 + index * 11_000);
      await expect(box.send(0, finding("lifetime"), 2_000_000)).rejects.toThrow("Peer send cap");
      expect((await box.state()).swarm!.messages).toHaveLength(64);
      const projection = await box.projection();
      expect(projection.messages.length).toBe(SWARM_LIMITS.projectionEvents);
      expect(projection.messages.every((message) => message.text.length <= SWARM_LIMITS.projectionText)).toBe(true);
    } finally { await box.close(); }
  });

  test("Unicode previews never split surrogate pairs at message or roster limits", async () => {
    const box = await mailbox();
    try {
      const body = `${"a".repeat(499)}🚀${"b".repeat(100)}`;
      const task = `${"t".repeat(239)}🌱${"more".repeat(100)}`;
      await box.commit(async (tx) => {
        (await tx.doc(Children, box.crew)).swarm!.members[0].task = task;
      });
      for (const text of ["\ud83d", "\ude80"]) {
        await expect(box.send(0, finding("invalid-unicode", "peer-2", text))).rejects.toThrow("well-formed Unicode");
      }
      await box.send(0, finding("unicode", "peer-2", body));
      const projection = await box.projection();
      expect(projection.messages[0].text).toBe(`${"a".repeat(499)}🚀`);
      expect(projection.messages[0].text.isWellFormed()).toBe(true);
      const roster = await box.commit((tx) => swarmMembers(tx, box.children[0]));
      expect(roster.members[0].task).toBe(`${"t".repeat(239)}🌱`);
      expect(roster.members[0].task.isWellFormed()).toBe(true);
      expect(swarmInstructions((await box.state()).swarm!, 0)).toContain(`${"t".repeat(239)}🌱`);
    } finally { await box.close(); }
  });

  test("whole-crew message cap and per-peer receive receipt cap are hard and replay remains available", async () => {
    const box = await mailbox(8);
    try {
      for (let round = 0; round < 32; round++) {
        for (let peer = 0; peer < 8; peer++) await box.send(peer, finding(`round-${round}`, `peer-${(peer + 1) % 8 + 1}`), 100_000 + round * 11_000);
        for (let peer = 0; peer < 8; peer++) await box.receive(peer, `drain-${round}`);
      }
      await expect(box.send(0, finding("crew-cap"), 999_999)).rejects.toThrow("Swarm message cap");
      for (let index = 32; index < 256; index++) await box.receive(0, `drain-${index}`);
      await expect(box.receive(0, "overflow")).rejects.toThrow("receipt cap");
      expect((await box.receive(0, "drain-0")).duplicate).toBe(true);
      const state = (await box.state()).swarm!;
      expect(state.messages).toHaveLength(256);
      expect(state.events).toHaveLength(512);
    } finally { await box.close(); }
  }, 20_000);

  test("terminal original submissions and durable peer lifecycle reject sends; peer/root closure cancels pending only", async () => {
    const box = await mailbox(3);
    try {
      const received = await box.send(0, finding("retrieved"));
      await box.receive(1, "retrieval");
      const queued = await box.send(0, finding("queued"));
      const outgoing = await box.send(1, finding("outgoing", "peer-3"));
      const question = await box.send(1, { ...finding("outgoing-question", "peer-3"), kind: "question" });
      await box.commit(async (tx) => {
        const doc = await tx.doc(Children, box.crew);
        doc.children[1].endedAt = 100_002;
        cancelPeer(doc.swarm!, "peer-2");
      });
      await expect(box.send(0, finding("terminal"))).rejects.toThrow("Recipient peer has ended");
      await expect(box.send(1, finding("ended-sender", "peer-3"))).rejects.toThrow("Sender peer has ended");
      await expect(box.send(1, finding("outgoing", "peer-3"))).rejects.toThrow("Sender peer has ended");
      await expect(box.receive(1, "retrieval")).rejects.toThrow("has ended");
      await expect(box.commit((tx) => swarmMembers(tx, box.children[1]))).rejects.toThrow("Peer has ended");
      expect(await box.send(0, finding("queued"))).toMatchObject({ duplicate: true, message: { id: queued.message.id, status: "cancelled" } });
      const state = (await box.state()).swarm!;
      expect(state.messages.find((message) => message.id === received.message.id)!.status).toBe("received");
      expect(state.messages.find((message) => message.id === outgoing.message.id)!.status).toBe("queued");
      expect(state.messages.find((message) => message.id === question.message.id)!.status).toBe("cancelled");
      expect((await box.receive(2, "read-finished-author")).messages.map((message) => message.id)).toEqual([outgoing.message.id]);
      // Prove a completion receipt rejects even before the crew loop stamps endedAt.
      await box.commit(async (tx) => {
        const submission = await tx.createSubmission({ conversationId: box.children[2], requestId: "subagent-2", type: "input", status: "queued" });
        tx.settleSubmission(submission.id, { status: "unanswered", reason: "aborted" });
      });
      await expect(box.send(0, finding("original-ended", "peer-3"))).rejects.toThrow("Recipient peer has ended");
      await box.commit(async (tx) => closeSwarm((await tx.doc(Children, box.crew)).swarm!));
      await expect(box.send(0, finding("closed", "peer-3"))).rejects.toThrow("Swarm is closed");
      expect(await box.send(0, finding("retrieved"))).toMatchObject({ duplicate: true, message: { status: "received" } });
      await expect(box.receive(1, "retrieval")).rejects.toThrow("has ended");
      await expect(box.receive(0, "after-close")).rejects.toThrow("has ended");
    } finally { await box.close(); }
  });

  test("a published reply remains retrievable after its author finishes", async () => {
    const box = await mailbox();
    try {
      const question = await box.send(0, { ...finding("question"), kind: "question" });
      await box.receive(1, "question-read");
      const reply = await box.send(1, { to: "peer-1", kind: "reply", text: "Keep v1 snapshots", requestId: "reply", replyTo: question.message.id });
      await box.commit(async (tx) => {
        const doc = await tx.doc(Children, box.crew);
        doc.children[1].endedAt = 100_002;
        cancelPeer(doc.swarm!, "peer-2");
      });
      const read = await box.receive(0, "finished-author-reply");
      expect(read.messages).toMatchObject([{ id: reply.message.id, kind: "reply", status: "received", replyTo: question.message.id }]);
      await expect(box.send(0, finding("no-new-mail"))).rejects.toThrow("Recipient peer has ended");
    } finally { await box.close(); }
  });

  test("mailboxes, stable identity and send/read receipts survive JSONL close/reopen", async () => {
    const directory = await mkdtemp(join(tmpdir(), "pi-swarm-"));
    const box = await mailbox(2, await JsonlStorage.open(directory, new NodeExecutionEnv({ cwd: directory }), context));
    try {
      const first = await box.send(0, finding("durable"));
      const batch = await box.receive(1, "durable-read");
      const queued = await box.send(0, finding("pending"));
      const before = await box.state();
      await box.close();
      await box.reopen(await JsonlStorage.open(directory, new NodeExecutionEnv({ cwd: directory }), context));
      expect(await box.state()).toEqual(before);
      expect(await box.send(0, finding("durable"))).toEqual({ message: batch.messages[0], duplicate: true });
      expect(await box.receive(1, "durable-read")).toEqual({ ...batch, duplicate: true });
      expect((await box.receive(1, "new-read")).messages.map((message) => message.id)).toEqual([queued.message.id]);
      expect(batch.messages[0].id).toBe(first.message.id);
      await expect(box.send(0, finding("durable", "peer-2", "changed after restart"))).rejects.toThrow("different peer message");
    } finally { await box.close(); await rm(directory, { recursive: true, force: true }); }
  });
});

async function runtimeFixture(respond: (request: TranscriptContext) => ReturnType<typeof fauxAssistantMessage> | Promise<ReturnType<typeof fauxAssistantMessage>>, presets: AgentDefinition[] = [], read?: () => Promise<void>, storage: Storage = new MemoryStorage(), swarmAllowed = true) {
  const faux = fauxProvider(), models = createModels(), registry = createRegistry();
  models.setProvider(faux.provider);
  faux.setResponses(Array.from({ length: 100 }, () => respond));
  const coding = defineExtension({ name: "swarm-test-coding", tools: ["read", "write", "edit", "bash"].map((name) => defineTool({
    name, description: `Test ${name}`, replay: name === "read" ? "safe" : "unsafe", parameters: Type.Object({ path: Type.Optional(Type.String()) }),
    execute: async () => {
      if (name !== "read") throw new Error("Swarm must never run a writer or shell");
      await read?.();
      return { output: [{ type: "text" as const, text: "read complete" }] };
    },
  })) });
  registry.install(coding);
  let harness!: Harness;
  const subagents = subagentExtension(models, () => harness, () => ({ ...noResources, agents: presets }));
  registry.install(subagents);
  registry.install(SwarmTools);
  // Peer tools are registered, but not part of any ordinary/root agent's defaults.
  const options = { models, registry, settings: { extensions: [coding, subagents] } };
  harness = await Harness.open(storage, options, context);
  const model = faux.getModel();
  let root = await harness.root(context, { agent: { model: { provider: model.provider, modelId: model.id } } });
  if (swarmAllowed) await setSwarmEnabled(root, true, context);
  const details = () => root.commit(async (tx) => clone((await tx.doc(Calls, root.id)).calls["swarm-call"]), context);
  return {
    get harness() { return harness; }, get root() { return root; }, details, close: () => harness.close(context),
    reopen: async (storage: Storage) => { harness = await Harness.open(storage, options, context); root = await harness.root(context); },
  };
}
const lastUser = (messages: readonly Message[]) => text([...messages].reverse().find((message) => message.role === "user")!);
const result = (messages: readonly Message[], id: string) => {
  const message = messages.find((message) => message.role === "toolResult" && message.toolCallId === id);
  return message?.role === "toolResult" ? JSON.parse(text(message)) : undefined;
};
const hasResult = (messages: readonly Message[], id: string) => messages.some((message) => message.role === "toolResult" && message.toolCallId === id);
const call = (name: string, args: ToolCall["arguments"], id: string) => fauxAssistantMessage(fauxToolCall(name, args, { id }), { stopReason: "toolUse" });

test("session policy rejects new swarms without silently downgrading ordinary delegation", async () => {
  const fixture = await runtimeFixture(async ({ messages }) => {
    const latest = lastUser(messages);
    if (["blocked", "ordinary", "enabled"].includes(latest)) {
      const id = `${latest}-call`;
      return hasResult(messages, id) ? fauxAssistantMessage("Handled.")
        : call("subagent", { tasks: [{ agent: "a", task: "read a" }, { agent: "b", task: "read b" }], swarm: latest !== "ordinary" }, id);
    }
    return fauxAssistantMessage("Read-only result.");
  }, [], undefined, new MemoryStorage(), false);
  try {
    expect(swarmEnabled(undefined)).toBe(false);
    expect(swarmEnabled({})).toBe(false);
    expect(swarmEnabled({ swarmEnabled: true })).toBe(true);
    expect(await fixture.root.commit(async (tx) => swarmEnabled(await tx.doc(Metadata, fixture.root.id)), context)).toBe(false);
    for (const invalid of ["false", 0, null, undefined]) {
      await expect(setSwarmEnabled(fixture.root, invalid, context)).rejects.toThrow("boolean");
    }
    await setSwarmEnabled(fixture.root, false, context);
    await (await fixture.root.submit({ type: "input", content: "blocked" }, context)).wait(context);
    const transcript = await fixture.root.context(context);
    const rejected = transcript.messages.find((message) => message.role === "toolResult" && message.toolCallId === "blocked-call")!;
    expect(rejected.role === "toolResult" && rejected.isError).toBe(true);
    expect(text(rejected)).toContain("Swarm is disabled");
    const calls = () => fixture.root.commit(async (tx) => clone((await tx.doc(Calls, fixture.root.id)).calls), context);
    expect(await calls()).toEqual({});
    await (await fixture.root.submit({ type: "input", content: "ordinary" }, context)).wait(context);
    await eventually(async () => (await calls())["ordinary-call"]?.results.every((peer) => peer.status === "done") || undefined);
    expect((await calls())["ordinary-call"].swarm).toBeUndefined();
    await setSwarmEnabled(fixture.root, true, context);
    await (await fixture.root.submit({ type: "input", content: "enabled" }, context)).wait(context);
    await eventually(async () => (await calls())["enabled-call"]?.swarm?.closed || undefined);
    expect((await calls())["enabled-call"].swarm?.members).toHaveLength(2);
    expect((await calls())["blocked-call"]).toBeUndefined();
  } finally { await fixture.close(); }
});

// Gates are provider request gates, not tools: they prove messages do not start/steer recipient turns.
test("faux providers exchange routed questions/replies using child-only tools and existing Calls projection", async () => {
  const questionSent = gate(), answerSent = gate(), finish = gate();
  const observations: { self: string; tools: string[]; system: string; users: string[] }[] = [];
  let questionId = "";
  const fixture = await runtimeFixture(async ({ messages }) => {
    const tools = messages.flatMap((message) => message.role === "system" ? message.toolsAdded ?? [] : []);
    const systemPrompt = messages.filter((message) => message.role === "system")
      .map((message) => [text(message), ...Object.values(message.sections ?? {})].join("\n")).join("\n");
    const latest = lastUser(messages);
    if (latest === "delegate") {
      if (hasResult(messages, "swarm-call")) return fauxAssistantMessage("Delegated.");
      expect(tools!.map((tool) => tool.name)).not.toContain("swarm_send");
      return call("subagent", { tasks: [{ agent: "same", task: "ask" }, { agent: "same", task: "answer" }], swarm: true }, "swarm-call");
    }
    if (latest.startsWith("<subagent_report")) return fauxAssistantMessage("Coordinated.");
    const roster = result(messages, "roster");
    if (!roster) {
      // Before either child's first tool, the whole durable membership exists.
      await fixture.root.commit(async (tx) => {
        const crews = await tx.scanTasks({ kind: "app.subagent-crew" }, 1);
        const children = await tx.doc(Children, crews.items[0].id);
        expect(Object.keys(children.children).length).toBe(2);
        expect(children.swarm!.members.map((member) => member.id)).toEqual(["peer-1", "peer-2"]);
      }, context);
      return call("swarm_members", {}, "roster");
    }
    if (!observations.some((entry) => entry.self === roster.self)) observations.push({
      self: roster.self, tools: tools!.map((tool) => tool.name), system: systemPrompt ?? "",
      users: messages.filter((message) => message.role === "user").map(text),
    });
    if (latest === "ask") {
      const sent = result(messages, "question");
      if (!sent) return call("swarm_send", { to: "peer-2", kind: "question", text: "which boundary?", requestId: "q" }, "question");
      questionId = sent.message.id;
      questionSent.open();
      await answerSent.promise;
      const read = result(messages, "reply-read");
      if (!read) return call("swarm_receive", { requestId: "r" }, "reply-read");
      expect(read.messages[0]).toMatchObject({ from: "peer-2", kind: "reply", replyTo: questionId, status: "received" });
      finish.open();
      return fauxAssistantMessage("Asked and explicitly retrieved a reply.");
    }
    if (latest === "answer") {
      await questionSent.promise;
      const read = result(messages, "question-read");
      if (!read) return call("swarm_receive", { requestId: "q-read", limit: 1 }, "question-read");
      expect(read.messages[0]).toMatchObject({ from: "peer-1", id: questionId, kind: "question", status: "received" });
      if (!result(messages, "reply")) return call("swarm_send", { to: "peer-1", kind: "reply", text: "the mailbox boundary", requestId: "a", replyTo: questionId }, "reply");
      answerSent.open();
      await finish.promise;
      return fauxAssistantMessage("Answered the retrieved question.");
    }
    throw new Error(`Unexpected request: ${latest}`);
  });
  try {
    const submission = await fixture.root.submit({ type: "input", content: "delegate" }, context);
    await submission.wait(context);
    const final = await eventually(async () => { const details = await fixture.details(); return details?.swarm?.closed ? details : undefined; });
    expect(final.results.map((entry) => [entry.agent, entry.peerId, entry.status])).toEqual([["same", "peer-1", "done"], ["same", "peer-2", "done"]]);
    expect(final.swarm!.messages.map((message) => message.status)).toEqual(["received", "received"]);
    for (const observation of observations) {
      expect(observation.tools.sort()).toEqual(["read", "swarm_members", "swarm_receive", "swarm_send"]);
      expect(observation.system).toContain(`You are ${observation.self}`);
      expect(observation.system).toContain("peer-1 (same): ask");
      expect(observation.system).toContain("peer-2 (same): answer");
      expect(observation.users).toHaveLength(1);
    }
    expect(observations).toHaveLength(2);
    const rootAgent = await fixture.root.agent(context);
    expect(rootAgent.tools.map((tool) => tool.name)).toContain("write");
    expect(rootAgent.tools.map((tool) => tool.name)).not.toContain("swarm_receive");
  } finally { questionSent.open(); answerSent.open(); finish.open(); await fixture.close(); }
});

test("opt-in validation rejects single/chain, peer count and explicit/preset unsafe policies before creating a crew", async () => {
  const preset: AgentDefinition = { name: "unsafe", description: "unsafe preset", prompt: "test", tools: ["read", "bash"], scope: "user", filePath: "/agents/unsafe.md" };
  const inputs: ToolCall["arguments"][] = [
    { agent: "a", task: "t", swarm: true },
    { chain: [{ agent: "a", task: "t" }, { agent: "b", task: "t" }], swarm: true },
    { tasks: [{ agent: "a", task: "t" }], swarm: true },
    { tasks: Array.from({ length: 9 }, () => ({ agent: "a", task: "t" })), swarm: true },
    ...["write", "edit", "bash", "grep", "find", "ls"].map((tool) => ({ tasks: [{ agent: "a", task: "t", tools: ["read", tool] }, { agent: "b", task: "t", tools: ["read"] }], swarm: true })),
    { tasks: [{ agent: "unsafe", task: "t" }, { agent: "b", task: "t" }], swarm: true },
  ];
  let input = inputs[0];
  const fixture = await runtimeFixture(({ messages }) => {
    if (messages.some((message) => message.role === "toolResult")) return fauxAssistantMessage("Rejected.");
    return call("subagent", input, "invalid");
  }, [preset]);
  try {
    for (input of inputs) {
      const submission = await fixture.root.submit({ type: "input", content: "validate" }, context);
      await submission.wait(context);
      await fixture.root.waitForIdle(context);
      const transcript = await fixture.root.context(context);
      const rejection = transcript.messages.filter((message) => message.role === "toolResult").at(-1)!;
      expect(rejection.role === "toolResult" && rejection.isError).toBe(true);
      expect(text(rejection)).toMatch(/2–8 parallel|only read/);
      if (text(rejection).includes("only read")) expect(text(rejection)).toContain("do not sandbox shell writes");
      // Reset so the scripted provider doesn't mistake a previous turn's results for this turn.
      await fixture.root.reset(undefined, context);
    }
    const calls = await fixture.root.commit(async (tx) => clone(await tx.doc(Calls, fixture.root.id)), context);
    expect(calls.calls).toEqual({});
  } finally { await fixture.close(); }
});

test("root stop closes the durable swarm and cancels pending mail without replaying unsafe tools", async () => {
  const sent = gate(), reading = gate(), release = gate();
  const fixture = await runtimeFixture(async ({ messages }) => {
    const latest = lastUser(messages);
    if (latest === "delegate") return hasResult(messages, "swarm-call") ? fauxAssistantMessage("Delegated.")
      : call("subagent", { tasks: [{ agent: "a", task: "sender" }, { agent: "b", task: "recipient" }], swarm: true }, "swarm-call");
    if (latest === "sender") {
      if (!result(messages, "send")) return call("swarm_send", { to: "peer-2", kind: "finding", text: "pending", requestId: "pending" }, "send");
      sent.open();
      return call("read", { path: "hold" }, "hold");
    }
    if (latest === "recipient") { await sent.promise; return call("read", { path: "hold" }, "hold"); }
    throw new Error(`Unexpected request: ${latest}`);
  }, [], async () => { reading.open(); await release.promise; });
  try {
    const submission = await fixture.root.submit({ type: "input", content: "delegate" }, context);
    await submission.wait(context);
    await reading.promise;
    const before = await fixture.details();
    expect(before.swarm!.messages[0].status).toBe("queued");
    const abort = fixture.root.abort(context, { background: true });
    release.open();
    await abort;
    const after = await fixture.details();
    expect(after.swarm!.closed).toBe(true);
    expect(after.swarm!.messages[0].status).toBe("cancelled");
    expect(after.results.every((entry) => entry.status === "stopped")).toBe(true);
  } finally { sent.open(); release.open(); await fixture.close(); }
});

test("a live swarm recovers the same crew/children/submissions and mailbox receipts after JSONL restart", async () => {
  const directory = await mkdtemp(join(tmpdir(), "pi-live-swarm-"));
  const blocked = gate(), releaseRead = gate(), retrieved = gate();
  let reads = 0, restarting = false;
  const fixture = await runtimeFixture(async ({ messages }) => {
    const latest = lastUser(messages);
    if (latest === "delegate") return hasResult(messages, "swarm-call") ? fauxAssistantMessage("Delegated.")
      : call("subagent", { tasks: [{ agent: "same", task: "sender" }, { agent: "same", task: "recipient" }], swarm: true }, "swarm-call");
    if (latest.startsWith("<subagent_report")) return fauxAssistantMessage("Handled recovered report.");
    if (latest === "sender") {
      if (!hasResult(messages, "send")) return call("swarm_send", { to: "peer-2", kind: "finding", text: "survive restart", requestId: "stable-send" }, "send");
      if (!hasResult(messages, "read")) return call("read", { path: "pause" }, "read");
      if (!hasResult(messages, "retry")) return call("swarm_send", { to: "peer-2", kind: "finding", text: "survive restart", requestId: "stable-send" }, "retry");
      expect(result(messages, "retry").duplicate).toBe(true);
      await retrieved.promise;
      return fauxAssistantMessage("Recovered sender complete.");
    }
    if (latest === "recipient") {
      if (!hasResult(messages, "read")) return call("read", { path: "pause" }, "read");
      if (!hasResult(messages, "receive")) return call("swarm_receive", { requestId: "stable-read" }, "receive");
      expect(result(messages, "receive").messages[0]).toMatchObject({ from: "peer-1", text: "survive restart", status: "received" });
      retrieved.open();
      return fauxAssistantMessage("Recovered recipient complete.");
    }
    throw new Error(`Unexpected request: ${latest}`);
  }, [], async () => {
    if (restarting) return;
    if (++reads === 2) blocked.open();
    await releaseRead.promise;
  }, await JsonlStorage.open(directory, new NodeExecutionEnv({ cwd: directory }), context));
  try {
    await (await fixture.root.submit({ type: "input", content: "delegate" }, context)).wait(context);
    await blocked.promise;
    const before = await eventually(async () => {
      const details = await fixture.details();
      return details?.results.every((peer) => peer.conversationId !== undefined) ? details : undefined;
    });
    expect(before.swarm!.messages[0].status).toBe("queued");
    const ids = before.results.map((peer) => peer.conversationId);
    await setSwarmEnabled(fixture.root, false, context);
    expect(await fixture.details()).toEqual(before);
    // Close first seals scheduling and tool writes; only then release the safe read gates.
    const close = fixture.close();
    releaseRead.open();
    await close;
    restarting = true;
    await fixture.reopen(await JsonlStorage.open(directory, new NodeExecutionEnv({ cwd: directory }), context));
    expect(await fixture.root.commit(async (tx) => swarmEnabled(await tx.doc(Metadata, fixture.root.id)), context)).toBe(false);
    fixture.harness.resume();
    const after = await eventually(async () => { const details = await fixture.details(); return details?.swarm?.closed ? details : undefined; });
    expect(after.resumed).toBe(true);
    expect(after.results.map((peer) => peer.conversationId)).toEqual(ids);
    expect(after.results.map((peer) => peer.peerId)).toEqual(["peer-1", "peer-2"]);
    expect(after.results.map((peer) => peer.status)).toEqual(["done", "done"]);
    expect(after.swarm!.id).toBe(before.swarm!.id);
    expect(after.swarm!.messages).toHaveLength(1);
    expect(after.swarm!.messages[0]).toMatchObject({ id: before.swarm!.messages[0].id, status: "received" });
    for (const id of ids) {
      const child = (await fixture.harness.conversation(Number(id) as unknown as ConversationId, context))!;
      const transcript = await child.context(context);
      expect(transcript.messages.filter((message) => message.role === "user")).toHaveLength(1);
    }
  } finally { releaseRead.open(); retrieved.open(); await fixture.close(); await rm(directory, { recursive: true, force: true }); }
});

test("a finished peer's findings remain retrievable while other peers continue", async () => {
  const allowRecipient = gate();
  const fixture = await runtimeFixture(async ({ messages }) => {
    const latest = lastUser(messages);
    if (latest === "delegate") return hasResult(messages, "swarm-call") ? fauxAssistantMessage("Delegated.")
      : call("subagent", { tasks: [{ agent: "a", task: "sender" }, { agent: "b", task: "recipient" }], swarm: true }, "swarm-call");
    if (latest.startsWith("<subagent_report")) return fauxAssistantMessage("Handled.");
    if (latest === "sender") return hasResult(messages, "send") ? fauxAssistantMessage("Sender ended.")
      : call("swarm_send", { to: "peer-2", kind: "finding", text: "unretrieved", requestId: "queued" }, "send");
    if (latest === "recipient") {
      await allowRecipient.promise;
      if (!hasResult(messages, "receive")) return call("swarm_receive", { requestId: "after-sender-ended" }, "receive");
      expect(result(messages, "receive").messages).toMatchObject([{ from: "peer-1", text: "unretrieved", status: "received" }]);
      return fauxAssistantMessage("Recipient retrieved the finished author's finding without a new turn.");
    }
    throw new Error(`Unexpected request: ${latest}`);
  });
  try {
    await (await fixture.root.submit({ type: "input", content: "delegate" }, context)).wait(context);
    const intermediate = await eventually(async () => { const details = await fixture.details(); return details?.results[0].status === "done" ? details : undefined; });
    expect(intermediate.swarm!.closed).toBe(false);
    expect(intermediate.swarm!.messages[0].status).toBe("queued");
    allowRecipient.open();
    const completed = await eventually(async () => { const details = await fixture.details(); return details?.swarm?.closed ? details : undefined; });
    expect(completed.results.every((peer) => peer.status === "done")).toBe(true);
    expect(completed.swarm!.messages[0].status).toBe("received");
  } finally { allowRecipient.open(); await fixture.close(); }
});
