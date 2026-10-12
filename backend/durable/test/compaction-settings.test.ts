import { expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import { DEFAULT_COMPACTION_POLICY, Harness, MemoryStorage, UserEntry, createRegistry } from "@earendil-works/pi-durable";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { JsonlStorage } from "@earendil-works/pi-durable/storage/jsonl";
import { Metadata, compactionPolicy, setCompactionThreshold } from "../src/session-settings.ts";
import { compactionRegistry } from "../src/compaction-policy.ts";

const effectivePercent = (window: number, policy: ReturnType<typeof compactionPolicy>) =>
  (window - policy.reserveTokens - policy.backgroundTokens) / window * 100;

test("old sessions retain token defaults; a custom trigger preserves reserve and retention", () => {
  expect(compactionPolicy({ autoCompaction: true }, 272_000)).toEqual(DEFAULT_COMPACTION_POLICY);
  const custom = compactionPolicy({ autoCompaction: true, compactionThresholdPercent: 70 }, 272_000);
  expect(effectivePercent(272_000, custom)).toBe(70);
  expect(custom.reserveTokens).toBe(DEFAULT_COMPACTION_POLICY.reserveTokens);
  expect(custom.keepRecentTokens).toBe(DEFAULT_COMPACTION_POLICY.keepRecentTokens);
  expect(compactionPolicy({ autoCompaction: false, compactionThresholdPercent: 70 }, 272_000)).toEqual({ ...custom, enabled: false });
});

test("saved percentage follows model size and is clamped at the blocking safety limit", () => {
  const settings = { autoCompaction: true, compactionThresholdPercent: 90 };
  expect(effectivePercent(272_000, compactionPolicy(settings, 272_000))).toBe(90);
  const smaller = compactionPolicy(settings, 128_000);
  expect(smaller.backgroundTokens).toBe(0);
  expect(effectivePercent(128_000, smaller)).toBeCloseTo(87.2);
  for (const window of [undefined, 0, -1, NaN, Infinity]) {
    expect(compactionPolicy(settings, window)).toEqual(DEFAULT_COMPACTION_POLICY);
  }
  expect(compactionPolicy(settings, 8192).backgroundTokens).toBe(0);
});

test("threshold validates strictly, commits without enabling compaction, and resets to defaults", async () => {
  const harness = await Harness.open(new MemoryStorage(), { models: createModels(), registry: createRegistry() }, context);
  try {
    const root = await harness.root(context);
    await root.commit(async (tx) => { (await tx.doc(Metadata, root.id)).autoCompaction = false; }, context);
    for (const percent of [undefined, "80", true, {}, NaN, Infinity, -Infinity, 9.9, 95.1]) {
      await expect(setCompactionThreshold(root, percent, context)).rejects.toThrow("percent must be");
    }
    expect(await setCompactionThreshold(root, 81.9, context)).toEqual({ percent: 81.9 });
    const read = () => root.commit(async (tx) => ({ ...(await tx.doc(Metadata, root.id)) }), context);
    expect(await read()).toMatchObject({ autoCompaction: false, compactionThresholdPercent: 81.9 });
    await expect(setCompactionThreshold(root, "60", context)).rejects.toThrow();
    expect((await read()).compactionThresholdPercent).toBe(81.9);
    expect(await setCompactionThreshold(root, null, context)).toEqual({ percent: null });
    expect((await read()).compactionThresholdPercent).toBeUndefined();
    for (const percent of [10, 95]) await setCompactionThreshold(root, percent, context);
  } finally {
    await harness.close(context);
  }
});

test("the live policy getter actually starts compaction below the default trigger", async () => {
  const faux = fauxProvider({ models: [{ id: "small", contextWindow: 100_000 }] });
  const models = createModels();
  models.setProvider(faux.provider);
  faux.setResponses(Array.from({ length: 10 }, () => fauxAssistantMessage("short result")));
  const settings = { autoCompaction: true, compactionThresholdPercent: 95 };
  const harness = await Harness.open(new MemoryStorage(), {
    models, registry: compactionRegistry(createRegistry(), () => settings),
    settings: { get compaction() { return compactionPolicy(settings, undefined); } },
  }, context);
  try {
    const model = faux.getModel();
    const root = await harness.root(context, { agent: { model: { provider: model.provider, modelId: model.id } } });
    await root.commit(async (tx) => {
      // Around 45k tokens: above custom 35%, below the default 50.8%.
      for (let i = 0; i < 3; i++) await tx.appendEntry(UserEntry, root.id, {
        model: [{ role: "user", content: "history ".repeat(7500), timestamp: i }],
      });
    }, context);
    settings.compactionThresholdPercent = 35;
    harness.resume();
    await (await root.submit({ type: "input", content: "continue" }, context)).wait(context);
    let compacted = false;
    for (let i = 0; i < 200; i++) {
      if ((await root.context(context)).head !== undefined) { compacted = true; break; }
      await Bun.sleep(5);
    }
    expect(compacted).toBe(true);
  } finally {
    await harness.close(context);
  }
});

test("different-model conversations use their own window, not the root window", async () => {
  const faux = fauxProvider({ models: [{ id: "root", contextWindow: 272_000 }, { id: "child", contextWindow: 128_000 }] });
  const models = createModels();
  models.setProvider(faux.provider);
  faux.setResponses(Array.from({ length: 10 }, () => fauxAssistantMessage("short result")));
  const settings = { autoCompaction: true, compactionThresholdPercent: 70 };
  const harness = await Harness.open(new MemoryStorage(), {
    models, registry: compactionRegistry(createRegistry(), () => settings),
    settings: { get compaction() { return compactionPolicy(settings, undefined); } },
  }, context);
  try {
    const model = faux.getModel();
    await harness.root(context, { agent: { model: { provider: model.provider, modelId: "root" } } });
    const child = await harness.createConversation({ ownership: { kind: "ownerless" }, agent: { model: { provider: model.provider, modelId: "child" } } }, context);
    await child.commit(async (tx) => {
      for (let i = 0; i < 4; i++) await tx.appendEntry(UserEntry, child.id, {
        model: [{ role: "user", content: "x".repeat(50_000), timestamp: i }],
      });
    }, context);
    await (await child.submit({ type: "input", content: "continue" }, context)).wait(context);
    await child.waitForIdle(context);
    await Bun.sleep(20);
    // 50k is below the child's 89.6k trigger. Root-sized offsets would trigger at 46.4k.
    expect((await child.context(context)).head).toBeUndefined();
    await (await child.submit({ type: "input", content: "x".repeat(180_000) }, context)).wait(context);
    let compacted = false;
    for (let i = 0; i < 200; i++) {
      if ((await child.context(context)).head !== undefined) { compacted = true; break; }
      await Bun.sleep(5);
    }
    expect(compacted).toBe(true);
  } finally {
    await harness.close(context);
  }
});

test("committed threshold survives storage reopen and is available before resume", async () => {
  const directory = await mkdtemp(join(tmpdir(), "pi-compaction-settings-"));
  const options = { models: createModels(), registry: createRegistry() };
  const open = async () => Harness.open(await JsonlStorage.open(directory, new NodeExecutionEnv({ cwd: directory }), context), options, context);
  let harness = await open();
  try {
    const root = await harness.root(context);
    await setCompactionThreshold(root, 65, context);
    await harness.close(context);
    harness = await open();
    const reopened = await harness.root(context);
    const metadata = await harness.documentState(Metadata, reopened.id, context);
    expect(metadata!.value!.compactionThresholdPercent).toBe(65);
    expect(effectivePercent(200_000, compactionPolicy(metadata!.value!, 200_000))).toBe(65);
    metadata!.dispose();
  } finally {
    await harness.close(context);
    await rm(directory, { recursive: true, force: true });
  }
});
