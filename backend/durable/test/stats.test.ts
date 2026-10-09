import { expect, test } from "bun:test";
import type { Message } from "@earendil-works/pi-ai";
import type { UsageState } from "@earendil-works/pi-durable";
import { sessionStats } from "../src/run.ts";

const usage = (input: number, output: number, cacheRead: number, cacheWrite: number, cost: number) => ({
  input, output, cacheRead, cacheWrite, totalTokens: input + output + cacheRead + cacheWrite,
  cost: { input: cost, output: 0, cacheRead: 0, cacheWrite: 0, total: cost },
});

test("session stats separate lifetime spend from the current context", () => {
  const lifetime = {
    models: { "provider/model": usage(900_000, 20_000, 400_000, 10_000, 0.4) },
    tools: { subagent: usage(30_000, 2_000, 1_000, 0, 0.03) },
  } as unknown as UsageState;
  const messages = [
    { role: "user", content: "old", timestamp: 1 },
    {
      role: "assistant", content: [{ type: "text", text: "answer" }], api: "anthropic-messages",
      provider: "anthropic", model: "claude", usage: usage(45_000, 5_000, 10_000, 0, 0.02),
      stopReason: "stop", timestamp: 2,
    },
  ] as unknown as Message[];

  const stats = sessionStats(lifetime, messages, 200_000);
  expect(stats.tokens).toEqual({ input: 930_000, output: 22_000, cacheRead: 401_000, cacheWrite: 10_000 });
  expect(stats.cost).toBeCloseTo(0.43);
  expect(stats.contextUsage).toEqual({ tokens: 60_000, contextWindow: 200_000, percent: 30 });
});

test("context usage stays unknown until a provider reports it", () => {
  const messages = [{ role: "user", content: "hello", timestamp: 1 }] as Message[];
  expect(sessionStats(undefined, messages, 128_000).contextUsage).toEqual({
    tokens: null, contextWindow: 128_000, percent: null,
  });
  expect(sessionStats(undefined, messages, undefined).contextUsage).toBeUndefined();
});
