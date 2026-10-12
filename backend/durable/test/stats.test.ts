import { expect, test } from "bun:test";
import type { Message } from "@earendil-works/pi-ai";
import { estimateMessageTokens } from "@earendil-works/pi-ai/utils/estimate";
import { DEFAULT_COMPACTION_POLICY, type ContextView, type UsageState } from "@earendil-works/pi-durable";
import { contextTokens, sessionStats } from "../src/run.ts";

const usage = (input: number, output: number, cacheRead: number, cacheWrite: number, cost: number) => ({
  input, output, cacheRead, cacheWrite, totalTokens: input + output + cacheRead + cacheWrite,
  cost: { input: cost, output: 0, cacheRead: 0, cacheWrite: 0, total: cost },
});
const user = (content: string, timestamp = 1): Message => ({ role: "user", content, timestamp });
const assistant = (tokens: number, timestamp = 2): Message => ({
  role: "assistant", content: [{ type: "text", text: "answer" }], api: "anthropic-messages",
  provider: "anthropic", model: "claude", usage: usage(tokens, 0, 0, 0, 0.02), stopReason: "stop", timestamp,
} as Message);
function view(messages: Message[], ids = messages.map((_, index) => index + 1), head?: number): ContextView {
  const entries = messages.map((message, index) => ({
    id: ids[index], kind: message.role === "assistant" ? "pi.assistant" : "pi.user", model: [message],
  }));
  return { entries, contributions: messages.map((message) => [message]), messages,
    head: head === undefined ? undefined : { id: head, head: 2, kind: "pi.compaction" },
  } as unknown as ContextView;
}

test("session stats separate lifetime spend from the current context", () => {
  const lifetime = {
    models: { "provider/model": usage(900_000, 20_000, 400_000, 10_000, 0.4) },
    tools: { subagent: usage(30_000, 2_000, 1_000, 0, 0.03) },
  } as unknown as UsageState;
  const stats = sessionStats(lifetime, view([user("old"), assistant(60_000)]), 200_000);
  expect(stats.tokens).toEqual({ input: 930_000, output: 22_000, cacheRead: 401_000, cacheWrite: 10_000 });
  expect(stats.cost).toBeCloseTo(0.43);
  expect(stats.contextUsage).toEqual({ tokens: 60_000, contextWindow: 200_000, percent: 30 });
  expect(stats.compaction).toEqual(DEFAULT_COMPACTION_POLICY);
});

test("unmeasured active context is estimated; an unknown/zero window has no percentage", () => {
  const current = view([user("hello")]);
  const tokens = estimateMessageTokens(current.messages[0]);
  expect(sessionStats(undefined, current, 128_000).contextUsage).toEqual({
    tokens, contextWindow: 128_000, percent: tokens / 128_000 * 100,
  });
  expect(sessionStats(undefined, current, undefined).contextUsage).toBeUndefined();
  expect(sessionStats(undefined, current, 0).contextUsage).toBeUndefined();
});

test("a placed summary invalidates retained assistant usage, regardless of timestamps", () => {
  for (const timestamp of [0, 100, 1000]) {
    const messages = [user("short summary", 100), assistant(237_000, timestamp), user("next prompt", 101)];
    const current = view(messages, [3, 2, 4], 3);
    const expected = messages.reduce((tokens, message) => tokens + estimateMessageTokens(message), 0);
    expect(contextTokens(current)).toBe(expected);
    expect(sessionStats(undefined, current, 272_000).contextUsage?.tokens).toBe(expected);
  }
});

test("only usage from entries newer than the marker anchors the active estimate", () => {
  const summary = user("summary", 100);
  for (const id of [2, 3, 4]) {
    const answer = assistant(40_000, 0); // Clock order deliberately disagrees with entry order.
    const trailing = user("pending prompt", 1000);
    const current = view([summary, answer, trailing], [3, id, 5], 3);
    expect(contextTokens(current)).toBe(id > 3
      ? 40_000 + estimateMessageTokens(trailing)
      : current.messages.reduce((tokens, message) => tokens + estimateMessageTokens(message), 0));
  }
});

test("stats use effective policy including disabled and custom background settings", () => {
  const policy = { ...DEFAULT_COMPACTION_POLICY, enabled: false, reserveTokens: 10_000, backgroundTokens: 0 };
  const stats = sessionStats(undefined, view([assistant(50_000)]), 272_000, policy);
  expect(stats.compaction).toEqual(policy);
});

test("real durable context drops retained usage immediately after a summary is placed", async () => {
  const { BACKGROUND_CONTEXT: context } = await import("@earendil-works/chord/context");
  const { createModels } = await import("@earendil-works/pi-ai/models");
  const { Harness, MemoryStorage, createRegistry, UserEntry, AssistantEntry, CompactionEntry } = await import("@earendil-works/pi-durable");
  const harness = await Harness.open(new MemoryStorage(), { models: createModels(), registry: createRegistry() }, context);
  try {
    const root = await harness.root(context);
    await root.commit(async (tx) => {
      await tx.appendEntry(UserEntry, root.id, { model: [user("old prefix".repeat(10000))] });
      const kept = await tx.appendEntry(AssistantEntry, root.id, { model: [assistant(237_000)] });
      await tx.appendEntry(CompactionEntry, root.id, { head: kept.id, model: [user("short summary")], data: { reason: "threshold" } });
    }, context);
    const compacted = await root.context(context);
    const expected = compacted.messages.reduce((sum, message) => sum + estimateMessageTokens(message), 0);
    expect(contextTokens(compacted)).toBe(expected);
    expect(sessionStats(undefined, compacted, 272_000).contextUsage?.tokens).toBeLessThan(100);
    await root.commit(async (tx) => {
      await tx.appendEntry(AssistantEntry, root.id, { model: [assistant(46_000)] });
    }, context);
    expect(contextTokens(await root.context(context))).toBe(46_000);
  } finally {
    await harness.close(context);
  }
});

test("a delayed pre-summary context read carries its obsolete head ID", async () => {
  const { BACKGROUND_CONTEXT: context } = await import("@earendil-works/chord/context");
  const { createModels } = await import("@earendil-works/pi-ai/models");
  const { Harness, MemoryStorage, createRegistry, UserEntry, AssistantEntry, CompactionEntry } = await import("@earendil-works/pi-durable");
  let started!: () => void, release!: () => void;
  const captured = new Promise<void>((resolve) => { started = resolve; });
  const proceed = new Promise<void>((resolve) => { release = resolve; });
  let armed = false;
  const storage = new Proxy(new MemoryStorage(), {
    get(target, key) {
      const value = Reflect.get(target, key, target);
      if (key === "scanEntries") return async (...args: unknown[]) => {
        if (armed && args[1] === 256) {
          armed = false;
          started();
          await proceed;
        }
        return Reflect.apply(value, target, args);
      };
      return typeof value === "function" ? value.bind(target) : value;
    },
  });
  const harness = await Harness.open(storage, { models: createModels(), registry: createRegistry() }, context);
  try {
    const root = await harness.root(context);
    const kept = await root.commit(async (tx) => {
      await tx.appendEntry(UserEntry, root.id, { model: [user("old prefix")] });
      return tx.appendEntry(AssistantEntry, root.id, { model: [assistant(237_000)] });
    }, context);
    armed = true;
    const pending = root.context(context);
    await captured;
    await root.commit(async (tx) => {
      await tx.appendEntry(CompactionEntry, root.id, { head: kept.id, model: [user("short summary")], data: { reason: "threshold" } });
    }, context);
    const fresh = sessionStats(undefined, await root.context(context), 272_000);
    release();
    const old = sessionStats(undefined, await pending, 272_000);
    expect(old.contextHeadId).toBe(0);
    expect(old.contextUsage?.tokens).toBe(237_000);
    expect(fresh.contextHeadId).toBeGreaterThan(old.contextHeadId);
    expect(fresh.contextUsage?.tokens).toBeLessThan(100);
  } finally {
    release();
    await harness.close(context);
  }
});
