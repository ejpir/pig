/** Committed-state adapter, not a stock Pi extension host. Stdout is JSONL only. */
import { createHash } from "node:crypto";
import { fstatSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { once } from "node:events";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import type { ModelThinkingLevel } from "@earendil-works/pi-ai";
import { getSupportedThinkingLevels, type Models } from "@earendil-works/pi-ai/models";
import {
  Harness, createRegistry, defineDoc, defineExtension, section,
  type Extension, type ConversationView, type SubmissionId,
} from "@earendil-works/pi-durable";
import { CodingTools } from "@earendil-works/pi-durable/tools";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { SqliteStorage } from "@earendil-works/pi-durable/storage/sqlite";
import { openNodeSqliteDatabase } from "@earendil-works/pi-durable/storage/sqlite/node";
import { authProviders, publicModel } from "./catalog.ts";
import { findImage, imageReferences, promptContent } from "./images.ts";

const context = BACKGROUND_CONTEXT;
const MAX_RECORD = 16 * 1024 * 1024;
const Metadata = defineDoc<{ name: string; autoCompaction: boolean; initialized: boolean }>({
  kind: "app.desktop", version: 1, scope: "conversation", history: "latest", fork: "initial",
  initial: () => ({ name: "", autoCompaction: true, initialized: false }),
});
// Bound receipt retention explicitly rather than silently expiring deduplication keys.
const Receipts = defineDoc<{ hashes: Record<string, string> }>({
  kind: "app.desktop-receipts", version: 1, scope: "conversation", history: "latest", fork: "initial",
  initial: () => ({ hashes: {} }),
});
const commands = [
  "prompt", "abort", "clear_queue", "get_submission", "get_state", "get_messages",
  "get_session_stats", "get_backend_info", "get_active_tools", "get_available_models",
  "get_available_thinking_levels", "get_auth_providers", "get_commands", "get_settings",
  "set_model", "set_thinking_level", "set_session_name", "set_auto_compaction",
  "get_image",
  "cancel_submission",
];
const info = {
  backend: "pi-durable", version: "1", piVersion: "1.0.2", protocolVersion: 1,
  bunVersion: Bun.version, workerPid: process.pid, commands,
  features: ["durable_execution", "committed_snapshots", "request_deduplication", "remote_pi_credentials", "builtin_providers", "image_prompts", "image_references"],
  experimental: true,
  limitations: ["Login on the SSH host; no interactive durable login flow yet", "No stock Pi extensions, skills or session-tree migration"],
};

/** Strict LF framing; Unicode line separators inside JSON strings are ordinary data. */
export async function* records(input: AsyncIterable<Buffer>): AsyncGenerator<Record<string, unknown>> {
  let pending = Buffer.alloc(0);
  const decoder = new TextDecoder("utf-8", { fatal: true });
  for await (const chunk of input) {
    pending = Buffer.concat([pending, chunk]);
    let end: number;
    while ((end = pending.indexOf(10)) !== -1) {
      if (end + 1 > MAX_RECORD) throw new Error("Durable request exceeds 16 MiB");
      const record: unknown = JSON.parse(decoder.decode(pending.subarray(0, end)));
      pending = pending.subarray(end + 1);
      if (!record || typeof record !== "object" || Array.isArray(record) || typeof (record as { type?: unknown }).type !== "string") {
        throw new Error("Durable request must be an object with a string type");
      }
      yield record as Record<string, unknown>;
    }
    if (pending.length >= MAX_RECORD) throw new Error("Durable request exceeds 16 MiB");
  }
  if (pending.length) throw new Error("Unterminated durable request");
}

function string(record: Record<string, unknown>, key: string): string {
  if (typeof record[key] !== "string") throw new Error(`${key} must be a string`);
  return record[key] as string;
}

export async function run(models: Models, extensions: readonly Extension[] = []): Promise<void> {
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === "--list-models") {
    // Discovery is independent of session admission/storage. Only public
    // metadata is returned; no provider call or conversation is created.
    await models.refresh({ allowNetwork: false });
    console.log(JSON.stringify({ version: 1, models: (await models.getAvailable()).map(publicModel) }));
    return;
  }
  const option = (name: string) => {
    const index = args.indexOf(name);
    if (index < 0 || !args[index + 1]) throw new Error(`Missing ${name}`);
    return args[index + 1];
  };
  const directory = option("--state");
  const cwd = option("--cwd");
  const key = option("--key");
  if (process.env.PI_DESKTOP_DURABLE_OWNED !== "1") throw new Error("Launch through the Rust durable-worker storage owner");
  if (process.platform !== "win32") {
    // This inherited descriptor keeps the OS writer lock held even if the Rust owner is killed.
    const fd = Number(process.env.PI_DESKTOP_DURABLE_LOCK_FD);
    if (!Number.isInteger(fd) || fd < 3 || !fstatSync(fd).isFile()) throw new Error("Missing inherited durable writer lock");
  }
  const mask = process.umask(0o077);
  let storage: SqliteStorage;
  try {
    mkdirSync(directory, { recursive: true, mode: 0o700 });
    const database = await openNodeSqliteDatabase(join(directory, "session.sqlite"));
    try {
      // The upstream default is NORMAL. FULL is required before acknowledging host-failure-durable admissions.
      await database.exec("PRAGMA synchronous = FULL");
      storage = await SqliteStorage.open(database);
    } catch (error) {
      await database.close();
      throw error;
    }
  } finally {
    process.umask(mask);
  }
  const registry = createRegistry();
  registry.install(CodingTools);
  registry.install(defineExtension({ name: "desktop-coding", sections: [
    section("preamble", () => "You are a coding assistant. Inspect the project with your tools. Do not repeat an interrupted unsafe operation without checking its effects first.", { tag: false }),
    section("cwd", (input) => input.env?.cwd),
  ] }));
  for (const extension of extensions) registry.install(extension);
  let autoCompaction = true;
  const harness = await Harness.open(storage, {
    models, registry, settings: { get compaction() { return { enabled: autoCompaction }; } },
    env: ({ cwd: selected = cwd }) => new NodeExecutionEnv({ cwd: selected }),
    onReport: (error) => console.error("Durable report:", error),
  }, context);
  const provider = process.env.PI_DESKTOP_DURABLE_PROVIDER;
  const modelId = process.env.PI_DESKTOP_DURABLE_MODEL;
  const root = await harness.root(context, { agent: {
    cwd, ...(provider && modelId ? { model: { provider, modelId } } : {}),
  } });
  await root.commit(async (tx) => { (await tx.doc(Metadata, root.id)).initialized = true; }, context);
  const state = await root.viewState(context);
  const metadata = await harness.documentState(Metadata, root.id, context);
  if (!metadata?.value) throw new Error("Durable desktop metadata was not committed");
  autoCompaction = metadata.value.autoCompaction;
  const watch = await root.watch(context);
  // One output line at a time, with backpressure. The durable watch itself bounds pending frames.
  let output = Promise.resolve();
  const send = (record: unknown): Promise<void> => {
    const line = JSON.stringify(record) + "\n";
    if (Buffer.byteLength(line) > MAX_RECORD) throw new Error("Durable response exceeds 16 MiB");
    const next = output.then(async () => {
      if (!process.stdout.write(line)) await once(process.stdout, "drain");
    });
    output = next;
    return next;
  };
  const snapshot = (value: ConversationView) => send({ type: "durable_state", key,
    data: imageReferences({ ...value, docs: { ...value.docs, "app.desktop": metadata.value } }), backend: info });
  await snapshot(watch.value);
  watch.start(async (value) => { await snapshot(value); });
  // Missing definitions, corrupt/newer storage and startup errors must fail closed, not select stock Pi.
  harness.resume();

  const chosenModel = () => {
    const selected = state.value.docs["pi.agent"].model as { provider: string; modelId: string } | undefined;
    return selected ? models.getModel(selected.provider, selected.modelId) : undefined;
  };
  const respond = async (record: Record<string, unknown>): Promise<unknown> => {
    const value = state.value;
    switch (record.type) {
      case "get_backend_info": return info;
      case "get_state": return {}; // Rust derives state from the committed view, never these read acknowledgements.
      case "get_messages": return { remoteSnapshot: true };
      case "get_image": {
        const image = findImage(value, string(record, "imageId"));
        if (!image) throw new Error("Image is not in this session");
        return { image };
      }
      case "get_session_stats": return {};
      case "get_commands": return { commands: [] };
      case "get_settings": return { effective: { compaction: { enabled: autoCompaction } }, durable: true };
      case "get_active_tools": return { activeTools: (await root.agent(context)).tools.map((tool) => ({ name: tool.name, description: tool.description })) };
      case "get_available_models": {
        await models.refresh({ allowNetwork: false }); // Reload host-side auth/models files and cached catalogs.
        return { models: (await models.getAvailable()).map(publicModel) };
      }
      case "get_auth_providers": return { providers: await authProviders(models) };
      case "get_available_thinking_levels": return { levels: chosenModel() ? getSupportedThinkingLevels(chosenModel()!) : ["off"] };
      case "set_model": {
        if (record.persist === true) throw new Error("Durable global model defaults are not implemented");
        const provider = string(record, "provider"), modelId = string(record, "modelId");
        const model = models.getModel(provider, modelId);
        if (!model) throw new Error(`Model not found: ${provider}/${modelId}`);
        await root.configure({ model: { provider, modelId } }, context);
        return publicModel(model);
      }
      case "set_thinking_level": {
        const level = string(record, "level");
        if (!chosenModel() || !getSupportedThinkingLevels(chosenModel()!).includes(level as ModelThinkingLevel)) throw new Error("Unsupported thinking level");
        await root.configure({ thinkingLevel: level as ModelThinkingLevel }, context);
        return { level };
      }
      case "set_auto_compaction": {
        if (typeof record.enabled !== "boolean") throw new Error("enabled must be a boolean");
        await root.commit(async (tx) => { (await tx.doc(Metadata, root.id)).autoCompaction = record.enabled as boolean; }, context);
        autoCompaction = record.enabled;
        await snapshot(state.value);
        return {};
      }
      case "set_session_name": {
        if (record.sessionPath !== undefined) throw new Error("Cannot rename another durable session");
        const name = string(record, "name");
        await root.commit(async (tx) => { (await tx.doc(Metadata, root.id)).name = name; }, context);
        await snapshot(state.value);
        return {};
      }
      case "prompt": {
        const message = string(record, "message"), requestId = string(record, "requestId");
        if (!/^[a-zA-Z0-9_-]{1,128}$/.test(requestId)) throw new Error("Invalid persistent requestId");
        if (!chosenModel()) throw new Error("Choose a configured model before submitting a durable prompt");
        const content = promptContent(message, record.images, chosenModel()!.input.includes("image"));
        const mode = record.streamingBehavior;
        if (mode !== undefined && mode !== "steer" && mode !== "followUp") throw new Error("Invalid streamingBehavior");
        const hash = createHash("sha256").update(JSON.stringify([content, mode ?? "reject"])).digest("hex");
        const duplicate = await root.commit(async (tx) => {
          const existing = await tx.submissionByRequest(root.id, requestId);
          const hashes = (await tx.doc(Receipts, root.id)).hashes;
          const previous = Object.hasOwn(hashes, requestId) ? hashes[requestId] : undefined;
          if (previous && previous !== hash) throw new Error("requestId already belongs to a different prompt");
          if (!previous && Object.keys(hashes).length >= 10_000) throw new Error("Durable prototype reached its 10,000-prompt receipt limit");
          if (["__proto__", "constructor", "prototype"].includes(requestId)) throw new Error("Reserved requestId");
          hashes[requestId] = hash;
          return existing !== undefined;
        }, context);
        const submission = await root.submit({ type: "input", content, requestId, whenBusy: mode ?? "reject" }, context);
        const receipt = await submission.status(context);
        return { disposition: duplicate ? "handled" : receipt.status === "queued" ? "queued" : "started", submissionId: submission.id, requestId, duplicate };
      }
      case "get_submission": return { submission: await root.commit((tx) => tx.submissionByRequest(root.id, string(record, "requestId")), context) ?? null };
      case "clear_queue": {
        const inbox = value.docs["pi.inbox"].items as { id: SubmissionId }[];
        for (const item of inbox) await harness.abortSubmission(item.id, context, root.id);
        return {};
      }
      case "cancel_submission": {
        const wanted = string(record, "submissionId");
        const inbox = value.docs["pi.inbox"].items as { id: SubmissionId }[];
        const item = inbox.find((item) => String(item.id) === wanted);
        if (!item) throw new Error("This prompt is no longer queued; refresh the session");
        await harness.abortSubmission(item.id, context, root.id);
        return {};
      }
      case "abort": await root.abort(context); return {};
      default: throw new Error(`${record.type} is not supported by the experimental durable backend`);
    }
  };
  try {
    for await (const record of records(process.stdin)) {
      if (typeof record.id !== "string" || !record.id.length || record.id.length > 256) throw new Error("Missing request correlation ID");
      try {
        const data = await respond(record);
        await send({ type: "response", id: record.id, command: record.type, success: true, data });
      } catch (error) {
        await send({ type: "response", id: record.id, command: record.type, success: false, error: String(error) });
      }
    }
  } finally {
    // Owner-pipe EOF stops this process, NOT the durable run. close preserves pending checkpoints.
    await watch.stop();
    state.dispose();
    metadata.dispose();
    await harness.close(context);
    await output;
  }
}
