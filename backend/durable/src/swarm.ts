/** Bounded, task-owned peer mailboxes. Calls is a projection, never admission authority. */
import { createHash } from "node:crypto";
import { Type } from "@earendil-works/pi-ai";
import {
  defineDoc, defineExtension, defineTool, LiveDoc,
  type ConversationId, type Tx,
} from "@earendil-works/pi-durable";
import { Calls, type SubagentResult } from "./subagents.ts";

export type SwarmMember = { id: string; agent: string; task: string };
export type SwarmMessage = {
  id: string; sequence: number; from: string; to: string;
  kind: "finding" | "question" | "reply"; text: string; replyTo?: string;
  status: "queued" | "received" | "cancelled"; createdAt: number; receivedAt?: number;
};
export type SwarmDetails = {
  version: 1; id: string; members: SwarmMember[]; messages: SwarmMessage[]; closed: boolean;
};

/** Lifetime caps do not reset on restart. The burst window is also persisted. */
export const SWARM_LIMITS = Object.freeze({
  minPeers: 2, maxPeers: 8, textBytes: 4096, messages: 256, sentPerPeer: 64,
  pendingPerPeer: 32, burst: 8, burstMs: 10_000, readBatch: 16,
  receiveReceiptsPerPeer: 256, projectionEvents: 32, projectionText: 500,
});

type SendReceipt = { hash: string; id: string };
type ReadReceipt = { limit: number; ids: string[] };
export type SwarmState = {
  id: string; callId: string; members: SwarmMember[]; messages: SwarmMessage[]; closed: boolean;
  sends: Record<string, SendReceipt>; reads: Record<string, ReadReceipt>;
  /** At most two events per message (enqueue and retrieval/cancellation): bounded by 512. */
  events: string[];
};
/** One crew's roster, child lifecycle and mailboxes have one durable authority. */
export const Children = defineDoc<{
  started: boolean; children: Record<string, { id: number; startedAt: number; endedAt?: number; result?: SubagentResult }>;
  swarm?: SwarmState;
}>({
  kind: "app.subagents", version: 1, scope: "task",
  initial: () => ({ started: false, children: {} }),
});

const hash = (value: unknown) => createHash("sha256").update(JSON.stringify(value)).digest("hex");
const copy = <T>(value: T): T => JSON.parse(JSON.stringify(value));

/** Never split UTF-16 surrogate pairs in JSON that Rust/native clients read. */
function preview(text: string, characters: number): string {
  let end = 0, count = 0;
  for (const character of text) {
    if (count++ === characters) break;
    end += character.length;
  }
  return text.slice(0, end);
}

export function createSwarm(root: ConversationId, callId: string, items: { agent: string; task: string }[]): SwarmState {
  if (items.length < SWARM_LIMITS.minPeers || items.length > SWARM_LIMITS.maxPeers) {
    throw new Error("Swarms require 2–8 parallel peers");
  }
  return {
    id: `swarm-${hash([String(root), callId])}`, callId,
    members: items.map((item, index) => ({ id: `peer-${index + 1}`, agent: item.agent, task: preview(item.task, 240) })),
    messages: [], closed: false, sends: {}, reads: {}, events: [],
  };
}

/** Only a short tail is projected; full bounded mailboxes remain in the crew task document. */
export function swarmProjection(swarm: SwarmState): SwarmDetails {
  const latestIds = [...new Set([...swarm.events].reverse())].slice(0, SWARM_LIMITS.projectionEvents).reverse();
  const latest = latestIds.map((id) => swarm.messages.find((message) => message.id === id)!);
  return copy({
    version: 1 as const, id: swarm.id,
    members: swarm.members.map((member) => ({ ...member, task: preview(member.task, 240) })), closed: swarm.closed,
    messages: latest.map((message) => ({ ...message, text: preview(message.text, SWARM_LIMITS.projectionText) })),
  });
}

export function cancelPeer(swarm: SwarmState, peerId: string): void {
  for (const message of swarm.messages) {
    // Close this peer's incoming mailbox and questions that can no longer get
    // a reply. Already-published findings/replies remain useful to live peers,
    // even when their author has finished the delegated assignment.
    if (message.status === "queued" && (message.to === peerId || (message.from === peerId && message.kind === "question"))) {
      message.status = "cancelled";
      swarm.events.push(message.id);
    }
  }
}
export function closeSwarm(swarm: SwarmState): void {
  swarm.closed = true;
  for (const message of swarm.messages) if (message.status === "queued") {
    message.status = "cancelled";
    swarm.events.push(message.id);
  }
}

async function publish(tx: Tx, root: ConversationId, swarm: SwarmState) {
  const call = (await tx.doc(Calls, root)).calls[swarm.callId];
  if (call) call.swarm = swarmProjection(swarm);
}

function requestId(id: string) {
  if (!/^[a-zA-Z0-9_-]{1,128}$/.test(id) || ["__proto__", "constructor", "prototype"].includes(id)) {
    throw new Error("Invalid persistent requestId (1–128 letters, digits, _ or -; reserved names forbidden)");
  }
}

/** Resolve identity from the immutable owner edge and exact committed child roster, not model arguments. */
async function bound(tx: Tx, conversationId: ConversationId) {
  const conversation = await tx.conversation(conversationId);
  const owner = conversation?.owner;
  if (!owner) throw new Error("Peer tools are only available to swarm children");
  const crew = await tx.task(owner.taskId);
  if (!crew || crew.kind !== "app.subagent-crew" || crew.conversationId !== owner.conversationId) {
    throw new Error("Not a swarm crew child");
  }
  const doc = await tx.doc(Children, crew.id);
  const swarm = doc.swarm;
  if (!swarm) throw new Error("Not a swarm crew child");
  const index = swarm.members.findIndex((_, index) => doc.children[index]?.id === Number(conversationId));
  if (index < 0) throw new Error("Child is not in this swarm roster");
  return { crew, doc, swarm, self: swarm.members[index] };
}

/** The original delegated submission, not a later follow-up, defines a peer's lifetime. */
async function inactive(tx: Tx, doc: Awaited<ReturnType<typeof bound>>["doc"], index: number) {
  const child = doc.children[index];
  if (!child || child.endedAt !== undefined) return true;
  const id = child.id as unknown as ConversationId;
  const original = await tx.submissionByRequest(id, `subagent-${index}`);
  if (original?.status === "done" || original?.status === "unanswered") return true;
  const live = await tx.doc(LiveDoc, id);
  if (live.run) {
    const run = await tx.task(live.run.taskId);
    if (!run || run.abortRequested || run.state.status === "terminal" || run.state.status === "completing") return true;
  }
  return false;
}

async function reconcile(tx: Tx, binding: Awaited<ReturnType<typeof bound>>) {
  const { crew, doc, swarm } = binding;
  if (crew.abortRequested || crew.state.status === "terminal" || crew.state.status === "completing") closeSwarm(swarm);
  for (let index = 0; index < swarm.members.length; index++) {
    if (await inactive(tx, doc, index)) cancelPeer(swarm, swarm.members[index].id);
  }
}

export type SwarmSend = { to: string; kind: SwarmMessage["kind"]; text: string; requestId: string; replyTo?: string };

/** Called inside one serialized durable commit; the message and its receipt are atomic. */
export async function swarmSend(tx: Tx, conversationId: ConversationId, input: SwarmSend, now = Date.now()) {
  requestId(input.requestId);
  if (!["finding", "question", "reply"].includes(input.kind)) throw new Error("Invalid peer message kind");
  if (!input.text.isWellFormed()) throw new Error("Peer text must be well-formed Unicode");
  if (!input.text.trim() || Buffer.byteLength(input.text, "utf8") > SWARM_LIMITS.textBytes) {
    throw new Error(`Peer text must be nonempty and at most ${SWARM_LIMITS.textBytes} UTF-8 bytes`);
  }
  const binding = await bound(tx, conversationId);
  const { crew, doc, swarm, self } = binding;
  const target = swarm.members.findIndex((member) => member.id === input.to);
  if (target < 0 || input.to === self.id) throw new Error("Send to another exact peer ID in this swarm roster");
  const key = `${self.id}:${input.requestId}`;
  const payloadHash = hash([input.to, input.kind, input.text, input.replyTo ?? null]);
  const previous = swarm.sends[key];
  if (previous && previous.hash !== payloadHash) throw new Error("requestId already belongs to a different peer message");
  await reconcile(tx, binding);
  // A follow-up in the same child must not replay tools from its old delegation.
  // Recovery still reuses receipts while the original submission is active.
  if (await inactive(tx, doc, swarm.members.indexOf(self))) throw new Error("Sender peer has ended");
  if (previous) {
    await publish(tx, crew.conversationId, swarm);
    return { message: copy(swarm.messages.find((message) => message.id === previous.id)!), duplicate: true };
  }
  if (swarm.closed) throw new Error("Swarm is closed");
  if (await inactive(tx, doc, target)) throw new Error("Recipient peer has ended");
  if (input.kind === "reply") {
    const question = swarm.messages.find((message) => message.id === input.replyTo);
    if (!question || question.kind !== "question" || question.from !== input.to || question.to !== self.id
      || question.status !== "received") throw new Error("A reply must reference a question explicitly received from this recipient");
  } else if (input.replyTo !== undefined) throw new Error("replyTo is only valid for replies");
  if (swarm.messages.length >= SWARM_LIMITS.messages) throw new Error("Swarm message cap reached (256)");
  const sent = swarm.messages.filter((message) => message.from === self.id);
  if (sent.length >= SWARM_LIMITS.sentPerPeer) throw new Error("Peer send cap reached (64)");
  if (swarm.messages.filter((message) => message.to === input.to && message.status === "queued").length >= SWARM_LIMITS.pendingPerPeer) {
    throw new Error("Recipient pending mailbox cap reached (32)");
  }
  if (sent.filter((message) => message.createdAt > now - SWARM_LIMITS.burstMs).length >= SWARM_LIMITS.burst) {
    throw new Error("Peer rate limit reached (8 messages per 10 seconds)");
  }
  const message: SwarmMessage = {
    id: `message-${hash([swarm.id, self.id, input.requestId])}`, sequence: swarm.messages.length + 1,
    from: self.id, to: input.to, kind: input.kind, text: input.text,
    ...(input.replyTo === undefined ? {} : { replyTo: input.replyTo }), status: "queued", createdAt: now,
  };
  swarm.messages.push(message);
  swarm.events.push(message.id);
  swarm.sends[key] = { hash: payloadHash, id: message.id };
  await publish(tx, crew.conversationId, swarm);
  return { message: copy(message), duplicate: false };
}

export type SwarmReceive = { requestId: string; limit?: number };
/** Nonblocking retrieval. A received receipt means retrieval ONLY, not agreement or guaranteed effects. */
export async function swarmReceive(tx: Tx, conversationId: ConversationId, input: SwarmReceive, now = Date.now()) {
  requestId(input.requestId);
  const limit = input.limit ?? 8;
  if (!Number.isInteger(limit) || limit < 1 || limit > SWARM_LIMITS.readBatch) throw new Error("Receive limit must be 1–16");
  const binding = await bound(tx, conversationId);
  const { crew, doc, swarm, self } = binding;
  const key = `${self.id}:${input.requestId}`;
  const previous = swarm.reads[key];
  if (previous && previous.limit !== limit) throw new Error("requestId already belongs to a different peer read");
  await reconcile(tx, binding);
  if (swarm.closed || await inactive(tx, doc, swarm.members.indexOf(self))) throw new Error("Swarm or recipient peer has ended");
  if (previous) {
    await publish(tx, crew.conversationId, swarm);
    return { messages: copy(previous.ids.map((id) => swarm.messages.find((message) => message.id === id)!)), duplicate: true };
  }
  if (Object.keys(swarm.reads).filter((key) => key.startsWith(`${self.id}:`)).length >= SWARM_LIMITS.receiveReceiptsPerPeer) {
    throw new Error("Peer receive receipt cap reached (256); empty reads also consume receipts");
  }
  const messages = swarm.messages.filter((message) => message.to === self.id && message.status === "queued").slice(0, limit);
  for (const message of messages) {
    message.status = "received";
    message.receivedAt = now;
    swarm.events.push(message.id);
  }
  swarm.reads[key] = { limit, ids: messages.map((message) => message.id) };
  await publish(tx, crew.conversationId, swarm);
  return { messages: copy(messages), duplicate: false };
}

export async function swarmMembers(tx: Tx, conversationId: ConversationId) {
  const binding = await bound(tx, conversationId);
  await reconcile(tx, binding);
  if (await inactive(tx, binding.doc, binding.swarm.members.indexOf(binding.self))) throw new Error("Peer has ended");
  await publish(tx, binding.crew.conversationId, binding.swarm);
  return copy({ id: binding.swarm.id, self: binding.self.id,
    members: binding.swarm.members.map((member) => ({ ...member, task: preview(member.task, 240) })), closed: binding.swarm.closed });
}

/** Register with the harness, but exclude from default extensions; only swarm children explicitly select it. */
export const SwarmTools = defineExtension({
  name: "subagents.swarm",
  tools: [
    defineTool({
      name: "swarm_members", outputLimits: { maxBytes: 16_384, maxLines: 2000 }, description: "Your exact peer ID and this crew's bounded roster (tasks shortened to 240 characters). No transcripts are shared.",
      parameters: Type.Object({}), replay: "safe",
      execute: async (_, api, context) => ({ output: [{ type: "text", text: JSON.stringify(await api.commit((tx) => swarmMembers(tx, api.conversationId), context)) }] }),
    }),
    defineTool({
      name: "swarm_send", outputLimits: { maxBytes: 32_768, maxLines: 2000 }, description: "Send bounded peer text to an exact roster ID. Use a stable unique requestId; retries must keep the same payload. No new turns are started. queued is not delivery; received means explicit tool retrieval, not agreement or guaranteed effects.",
      parameters: Type.Object({
        to: Type.String(), kind: Type.Union([Type.Literal("finding"), Type.Literal("question"), Type.Literal("reply")]),
        text: Type.String(), requestId: Type.String(), replyTo: Type.Optional(Type.String()),
      }, { additionalProperties: false }), replay: "safe",
      execute: async (args, api, context) => ({ output: [{ type: "text", text: JSON.stringify(await api.commit((tx) => swarmSend(tx, api.conversationId, args), context)) }] }),
    }),
    defineTool({
      name: "swarm_receive", outputLimits: { maxBytes: 512_000, maxLines: 2000 }, description: "Nonblocking read of up to 16 queued messages (default 8). Each fresh requestId reads once, including empty reads; same requestId and limit replay the exact batch after a restart while your original delegated task is active. Later child follow-ups cannot reuse this crew's tools or receipts. A different requestId never redelivers received messages. received means retrieval only, not agreement or guaranteed effects. At most 256 read receipts per peer.",
      parameters: Type.Object({ requestId: Type.String(), limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 16 })) }, { additionalProperties: false }), replay: "safe",
      execute: async (args, api, context) => ({ output: [{ type: "text", text: JSON.stringify(await api.commit((tx) => swarmReceive(tx, api.conversationId, args), context)) }] }),
    }),
  ],
});

export function swarmInstructions(swarm: SwarmState, index: number): string {
  return [
    `You are ${swarm.members[index].id} in swarm ${swarm.id}. Peer IDs, not agent display names, identify recipients.`,
    "Roster (task previews shortened to 240 characters):",
    ...swarm.members.map((member) => `- ${member.id} (${member.agent}): ${preview(member.task, 240)}`),
    "Only read and swarm_members/swarm_send/swarm_receive are available. The parent coordinator is the writer. Current bash/search tool names do not sandbox shell writes, so they are not allowed here.",
    "Use peer tools during this delegated task only. No transcript sharing, spawning, waiting for new turns, or automatic message injection. Receive is nonblocking: do useful work and finish without waiting indefinitely for peers.",
    "Send finding/question/reply to another exact roster ID with a unique persistent requestId. A reply needs replyTo = the ID of a question you retrieved from that recipient. Retry IDs only with identical arguments.",
    "Messages: at most 4096 UTF-8 bytes; 8 sends/10 seconds, 64 sends per peer, 256 per crew, 32 queued per recipient. Receive: fresh requestId, 1–16 messages (default 8), at most 256 read receipts including empty reads. received means explicit retrieval, NOT agreement or guaranteed effects. Ending a peer cancels its incoming mail and unanswered outgoing questions; its published findings/replies remain retrievable by live peers. Stopping the crew cancels all pending mail.",
  ].join("\n");
}
