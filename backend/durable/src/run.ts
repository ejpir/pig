/** Committed-state adapter, not a stock Pi extension host. Stdout is JSONL only. */
import { createHash } from "node:crypto";
import { fstatSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { once } from "node:events";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import type { Message, ModelThinkingLevel, Usage } from "@earendil-works/pi-ai";
import {
  calculateContextTokens,
  estimateMessageTokens,
} from "@earendil-works/pi-ai/utils/estimate";
import {
  getSupportedThinkingLevels,
  type Models,
} from "@earendil-works/pi-ai/models";
import {
  CompactionEntry,
  DEFAULT_COMPACTION_POLICY,
  GenerationTask,
  Harness,
  InboxDoc,
  LiveDoc,
  UserEntry,
  createRegistry,
  defineDoc,
  defineExtension,
  section,
  watchEvents,
  type AgentEvent,
  type Conversation,
  type ConversationId,
  type ConversationView,
  type ContextView,
  type CompactionPolicy,
  type EntryDraft,
  type Extension,
  type SnapshotEvent,
  type SubmissionId,
  type UsageState,
  type WatchEnd,
} from "@earendil-works/pi-durable";
import { JavascriptTools } from "./javascript.ts";
import { CodingTools } from "@earendil-works/pi-durable/tools";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { SqliteStorage } from "@earendil-works/pi-durable/storage/sqlite";
import { openNodeSqliteDatabase } from "@earendil-works/pi-durable/storage/sqlite/node";
import { authProviders, publicModel } from "./catalog.ts";
import { findImage, imageReferences, promptContent } from "./images.ts";
import {
  expandPromptCommand,
  noResources,
  promptCommands,
  type Resources,
} from "./commands.ts";
import { imageRead } from "./read.ts";
import { History, shown } from "./history.ts";
import {
  Calls,
  subagentExtension,
  subagentResult,
  submitSubagentInput,
  transcript,
} from "./subagents.ts";
import { SwarmTools } from "./swarm.ts";
import {
  Metadata,
  compactionPolicy as resolveCompactionPolicy,
  setCompactionThreshold,
  setSwarmEnabled,
  swarmEnabled,
} from "./session-settings.ts";
import { compactionRegistry } from "./compaction-policy.ts";

const context = BACKGROUND_CONTEXT;
export const MAX_RECORD = 64 * 1024 * 1024;
// Bound receipt retention explicitly rather than silently expiring deduplication keys.
const Receipts = defineDoc<{ hashes: Record<string, string> }>({
  kind: "app.desktop-receipts",
  version: 1,
  scope: "conversation",
  history: "latest",
  fork: "initial",
  initial: () => ({ hashes: {} }),
});
const commands = [
  "prompt",
  "abort",
  "clear_queue",
  "get_submission",
  "get_state",
  "get_messages",
  "get_session_stats",
  "get_backend_info",
  "get_active_tools",
  "get_available_models",
  "get_available_thinking_levels",
  "get_auth_providers",
  "get_commands",
  "get_settings",
  "set_model",
  "set_thinking_level",
  "set_session_name",
  "set_auto_compaction",
  "set_swarm_enabled",
  "compact",
  "set_compaction_threshold",
  "get_image",
  "cancel_submission",
  "get_subagent",
  "stop_subagent",
  "prompt_subagent",
];
const defaultInfo = {
  backend: "pi-durable",
  version: "1",
  piVersion: "1.2.0",
  protocolVersion: 2,
  bunVersion: Bun.version,
  workerPid: process.pid,
  commands,
  features: [
    "durable_execution",
    "committed_snapshots",
    "incremental_events",
    "request_deduplication",
    "remote_pi_credentials",
    "builtin_providers",
    "image_prompts",
    "image_references",
    "prompt_templates",
    "skills",
    "subagents",
    "subagent_input",
    "swarm_messaging",
    "manual_compaction",
  ],
  experimental: true,
  limitations: [
    "Login on the SSH host; no interactive durable login flow yet",
    "No stock Pi extensions, Pi packages or session-tree migration",
  ],
};

/** Strict LF framing; Unicode line separators inside JSON strings are ordinary data. */
export async function* records(
  input: AsyncIterable<Buffer>,
): AsyncGenerator<Record<string, unknown>> {
  let pending = Buffer.alloc(0);
  const decoder = new TextDecoder("utf-8", { fatal: true });
  for await (const chunk of input) {
    pending = Buffer.concat([pending, chunk]);
    let end: number;
    while ((end = pending.indexOf(10)) !== -1) {
      if (end + 1 > MAX_RECORD)
        throw new Error("Durable request exceeds 64 MiB");
      const record: unknown = JSON.parse(
        decoder.decode(pending.subarray(0, end)),
      );
      pending = pending.subarray(end + 1);
      if (
        !record ||
        typeof record !== "object" ||
        Array.isArray(record) ||
        typeof (record as { type?: unknown }).type !== "string"
      ) {
        throw new Error("Durable request must be an object with a string type");
      }
      yield record as Record<string, unknown>;
    }
    if (pending.length >= MAX_RECORD)
      throw new Error("Durable request exceeds 64 MiB");
  }
  if (pending.length) throw new Error("Unterminated durable request");
}

/** A live runner cannot continue once its sole exact event stream has ended. */
export async function eventStreamFailure(
  closed: Promise<WatchEnd>,
): Promise<never> {
  const end = await closed;
  if (end.reason === "listener_error") {
    throw new Error(
      `Durable event stream listener failed: ${end.error.message}`,
      { cause: end.error },
    );
  }
  if (end.reason === "session_failed") {
    throw new Error(`Durable session failed: ${String(end.error)}`, {
      cause: end.error,
    });
  }
  throw new Error(`Durable event stream closed unexpectedly: ${end.reason}`);
}

function string(record: Record<string, unknown>, key: string): string {
  if (typeof record[key] !== "string")
    throw new Error(`${key} must be a string`);
  return record[key] as string;
}

function submissionId(value: string): SubmissionId {
  if (!/^[1-9][0-9]*$/.test(value))
    throw new Error("submissionId must be a positive integer string");
  const id = Number(value);
  if (!Number.isSafeInteger(id))
    throw new Error("submissionId is out of range");
  return id as SubmissionId;
}

/**
 * A targeted task abort leaves the inbox intact, unlike Conversation.abort().
 * The harness normally advances that inbox from a run's final boundary; an
 * aborted generation has no such boundary, so place the next batch here using
 * the pinned harness's one-at-a-time queue policy.
 */
async function resumeInboxAfterTargetedAbort(
  root: Conversation,
): Promise<void> {
  await root.commit(async (tx) => {
    let head = (await tx.latestHeadMarker(root.id))?.head;
    const inbox = await tx.doc(InboxDoc, root.id);
    const live = await tx.doc(LiveDoc, root.id);
    if (live.run !== undefined) return;

    const items = inbox.items;
    const writes = items.flatMap((item, index) =>
      item.mode === "write" ? [index] : [],
    );
    const first = (mode: "steer" | "followUp") => {
      const index = items.findIndex((item) => item.mode === mode);
      return index < 0 ? [] : [index];
    };
    const users = [...first("steer"), ...first("followUp")].sort(
      (a, b) => a - b,
    );

    for (const index of writes) {
      const item = items[index];
      if (item?.mode !== "write") continue;
      const draft = item.entry as unknown as EntryDraft;
      if (
        typeof draft.head === "number" &&
        head !== undefined &&
        draft.head < head
      ) {
        tx.settleSubmission(item.id, { status: "unanswered", reason: "stale" });
        continue;
      }
      const entry = await tx.appendEntry(root.id, draft);
      if (draft.head !== undefined)
        head = draft.head === "self" ? entry.id : draft.head;
      tx.placeSubmission(item.id, entry.id);
    }

    const placed: SubmissionId[] = [];
    for (const index of users) {
      const item = items[index];
      if (item === undefined || item.mode === "write") continue;
      const message = {
        role: "user" as const,
        content: item.content,
        timestamp: Date.now(),
      };
      const entry = await tx.appendEntry(UserEntry, root.id, {
        model: [message],
      });
      tx.placeSubmission(item.id, entry.id);
      placed.push(item.id);
    }
    for (const index of [...writes, ...users].sort((a, b) => b - a))
      items.splice(index, 1);

    if (placed.length > 0) {
      live.run = {
        taskId: await tx.createTask(
          GenerationTask,
          {},
          {
            ownership: { kind: "conversation" },
            conversationId: root.id,
          },
        ),
        inputs: placed,
      };
    }
  }, context);
}

/** Stop the run that owns an exact placed submission without withdrawing peers in the inbox. */
async function abortPlacedSubmission(
  harness: Harness,
  root: Conversation,
  submissionId: SubmissionId,
): Promise<void> {
  for (;;) {
    const submission = await harness.submission(submissionId, context);
    if (submission === undefined)
      throw new Error("This prompt is no longer available to cancel");
    const status = await submission.status(context);
    if (status.status === "done" || status.status === "unanswered") break;
    if (status.status !== "placed")
      throw new Error("This prompt is no longer available to cancel");
    const taskId = await root.commit(async (tx) => {
      const live = await tx.doc(LiveDoc, root.id);
      return live.run?.inputs.includes(submissionId)
        ? live.run.taskId
        : undefined;
    }, context);
    if (taskId === undefined) {
      const refreshed = await submission.status(context);
      if (refreshed.status === "done" || refreshed.status === "unanswered")
        break;
      throw new Error("This prompt is no longer available to cancel");
    }
    await harness.abortTask(taskId, context);
    await harness.waitForTask(taskId, context);
  }
  await resumeInboxAfterTargetedAbort(root);
}

/** Match durable's head-aware estimate: retained usage describes the removed prefix. */
export function contextTokens(view: ContextView): number {
  const after = view.head?.id ?? Number.NEGATIVE_INFINITY;
  for (let index = view.entries.length - 1; index >= 0; index--) {
    if (view.entries[index].id <= after) continue;
    const message = view.contributions[index].findLast(
      (message) =>
        message.role === "assistant" &&
        message.stopReason !== "aborted" &&
        message.stopReason !== "error" &&
        calculateContextTokens(message.usage) > 0,
    );
    if (message?.role !== "assistant") continue;
    const position = view.messages.lastIndexOf(message);
    if (position < 0) continue;
    return (
      calculateContextTokens(message.usage) +
      view.messages
        .slice(position + 1)
        .reduce((tokens, next) => tokens + estimateMessageTokens(next), 0)
    );
  }
  return view.messages.reduce(
    (tokens, message) => tokens + estimateMessageTokens(message),
    0,
  );
}

const SUMMARY_PREFIX =
  "The conversation history before this point was compacted into the following summary:\n\n<summary>\n";
const SUMMARY_SUFFIX = "\n</summary>";

function compactionSummary(messages: readonly Message[] | undefined): string {
  const message = messages?.find((candidate) => candidate.role === "user");
  if (!message || message.role !== "user")
    throw new Error("Compaction summary entry has no user message");
  const text =
    typeof message.content === "string"
      ? message.content
      : message.content
          .filter((block) => block.type === "text")
          .map((block) => block.text)
          .join("\n");
  if (!text.startsWith(SUMMARY_PREFIX) || !text.endsWith(SUMMARY_SUFFIX)) {
    throw new Error("Compaction summary entry has an invalid wrapper");
  }
  return text.slice(SUMMARY_PREFIX.length, -SUMMARY_SUFFIX.length);
}

export function sessionStats(
  state: UsageState | undefined,
  view: ContextView,
  contextWindow: number | undefined,
  compaction: CompactionPolicy = DEFAULT_COMPACTION_POLICY,
) {
  const tokens = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 };
  let cost = 0;
  for (const bucket of [state?.models ?? {}, state?.tools ?? {}]) {
    for (const value of Object.values(bucket) as unknown as Usage[]) {
      tokens.input += value.input;
      tokens.output += value.output;
      tokens.cacheRead += value.cacheRead;
      tokens.cacheWrite += value.cacheWrite;
      cost += value.cost.total;
    }
  }
  const current = contextTokens(view);
  return {
    tokens,
    cost,
    compaction,
    contextHeadId: view.head?.id ?? 0,
    ...(contextWindow === undefined || contextWindow <= 0
      ? {}
      : {
          contextUsage: {
            tokens: current,
            contextWindow,
            percent: (current / contextWindow) * 100,
          },
        }),
  };
}

/**
 * `resources` finds the host's templates and skills: the project's too when
 * given its folder. Read again for each prompt and command list, so a new file
 * needs no restart.
 */
/** In-process host boundary. Execution, commands, projection and recovery stay shared. */
export interface RunHost {
  state: string;
  cwd: string;
  key: string;
  input: AsyncIterable<Buffer>;
  output: (line: string) => void | Promise<void>;
  assertOwner: () => void | Promise<void>;
  stopInput: () => void;
  env?: (target: { cwd?: string }) => NodeExecutionEnv;
  initialModel?: { provider: string; modelId: string };
  initialThinking?: string;
  info?: Record<string, unknown>;
  onReport?: (error: unknown) => void;
}
export async function run(
  models: Models,
  extensions: readonly Extension[] = [],
  resources: (cwd?: string) => Resources = () => noResources,
  host?: RunHost,
): Promise<void> {
  const info = { ...defaultInfo, ...host?.info };
  const report =
    host?.onReport ??
    ((error: unknown) => console.error("Durable report:", error));
  const args = host ? [] : process.argv.slice(2);
  if (args.length === 1 && args[0] === "--list-commands") {
    // Before a session has a folder: the user's own commands and the built-ins.
    console.log(
      JSON.stringify({ version: 1, commands: promptCommands(resources()) }),
    );
    return;
  }
  if (args.length === 1 && args[0] === "--list-models") {
    // Discovery is independent of session admission/storage. Only public
    // metadata is returned; no provider call or conversation is created.
    await models.refresh({ allowNetwork: false });
    console.log(
      JSON.stringify({
        version: 1,
        models: (await models.getAvailable()).map(publicModel),
      }),
    );
    return;
  }
  const option = (name: string) => {
    const index = args.indexOf(name);
    if (index < 0 || !args[index + 1]) throw new Error(`Missing ${name}`);
    return args[index + 1];
  };
  const directory = host?.state ?? option("--state");
  const cwd = host?.cwd ?? option("--cwd");
  const key = host?.key ?? option("--key");
  if (host) await host.assertOwner();
  else {
    if (process.env.PI_DESKTOP_DURABLE_OWNED !== "1")
      throw new Error("Launch through the Rust durable-worker storage owner");
    if (process.platform !== "win32") {
      // This inherited descriptor keeps the OS writer lock held even if the Rust owner is killed.
      const fd = Number(process.env.PI_DESKTOP_DURABLE_LOCK_FD);
      if (!Number.isInteger(fd) || fd < 3 || !fstatSync(fd).isFile())
        throw new Error("Missing inherited durable writer lock");
    }
  }
  const mask = host ? undefined : process.umask(0o077);
  let storage: SqliteStorage;
  try {
    mkdirSync(directory, { recursive: true, mode: 0o700 });
    const database = await openNodeSqliteDatabase(
      join(directory, "session.sqlite"),
    );
    try {
      // The upstream default is NORMAL. FULL is required before acknowledging host-failure-durable admissions.
      await database.exec("PRAGMA synchronous = FULL");
    } catch (error) {
      await database.close();
      throw error;
    }
    storage = await SqliteStorage.open(database);
  } finally {
    if (mask !== undefined) process.umask(mask);
  }
  let ownedHarness: Harness | undefined;
  let stopEvents: (() => Promise<unknown>) | undefined;
  const dispose: (() => void)[] = [];
  const deliveries = new Set<Promise<unknown>>();
  let compacting: Promise<void> | undefined;
  let commands: Promise<void> | undefined;
  let flushing = Promise.resolve();
  let stopping = false;
  let runFailed = false;
  try {
    let found = resources(cwd);
    const registry = createRegistry();
    registry.install(CodingTools);
    registry.install(JavascriptTools);
    registry.install(
      defineExtension({
        name: "desktop-coding",
        wraps: [imageRead],
        sections: [
          section(
            "preamble",
            () =>
              "You are a coding assistant. Inspect the project with your tools. Do not repeat an interrupted unsafe operation without checking its effects first.",
            { tag: false },
          ),
          section("cwd", (input) => input.env?.cwd),
          // Unchanged between requests unless a skill changes, so the provider's cache holds.
          section("skills", () => found.skillsPrompt || undefined, {
            tag: false,
          }),
        ],
      }),
    );
    // The tool runs only once the Harness below is open.
    let opened: Harness | undefined;
    registry.install(
      subagentExtension(
        models,
        () => opened!,
        () => found,
      ),
    );
    // Definitions are recoverable in the registry, but only opted-in children
    // explicitly select these tools; parent and ordinary crew defaults exclude them.
    registry.install(SwarmTools);
    for (const extension of extensions) registry.install(extension);
    // Live resolvers are bound to committed views before resume starts any work.
    let compactionMetadata: () => {
      autoCompaction: boolean;
      compactionThresholdPercent?: number;
    } = () => ({ autoCompaction: true });
    let contextWindow: () => number | undefined = () => undefined;
    const compactionPolicy = (): CompactionPolicy =>
      resolveCompactionPolicy(compactionMetadata(), contextWindow());
    const harness =
      (opened =
      ownedHarness =
        await Harness.open(
          storage,
          {
            models,
            registry: compactionRegistry(registry, () => compactionMetadata()),
            settings: {
              extensions: registry
                .snapshot()
                .installed()
                .filter((extension) => extension.name !== SwarmTools.name),
              // Percentage offsets are resolved per generation by the registry adapter.
              // Other tasks need only the unchanged reserve/retention and enabled flag.
              get compaction() {
                return resolveCompactionPolicy(compactionMetadata(), undefined);
              },
            },
            env:
              host?.env ??
              (({ cwd: selected = cwd }) =>
                new NodeExecutionEnv({ cwd: selected })),
            onReport: report,
          },
          context,
        ));
    const provider =
      host?.initialModel?.provider ??
      (host ? undefined : process.env.PI_DESKTOP_DURABLE_PROVIDER);
    const modelId =
      host?.initialModel?.modelId ??
      (host ? undefined : process.env.PI_DESKTOP_DURABLE_MODEL);
    const root = await harness.root(context, {
      agent: {
        cwd,
        ...(provider && modelId ? { model: { provider, modelId } } : {}),
        ...(host?.initialThinking
          ? { thinkingLevel: host.initialThinking as ModelThinkingLevel }
          : {}),
      },
    });
    await root.commit(async (tx) => {
      const metadata = await tx.doc(Metadata, root.id);
      metadata.initialized = true;
      metadata.swarmEnabled ??= false;
      // Made here, so the apps can follow it before any subagent starts.
      (await tx.doc(Calls, root.id)).calls ??= {};
    }, context);
    const state = await root.viewState(context);
    dispose.push(() => state.dispose());
    const metadata = await harness.documentState(Metadata, root.id, context);
    if (metadata) dispose.push(() => metadata.dispose());
    if (!metadata?.value)
      throw new Error("Durable desktop metadata was not committed");
    // Subagents working in the background, which the view's own documents don't carry.
    const calls = await harness.documentState(Calls, root.id, context);
    if (calls) dispose.push(() => calls.dispose());
    if (!calls) throw new Error("Durable subagent calls were not committed");
    compactionMetadata = () => metadata.value!;
    const chosenModel = () => {
      const selected = state.value.docs["pi.agent"].model as
        | { provider: string; modelId: string }
        | undefined;
      return selected
        ? models.getModel(selected.provider, selected.modelId)
        : undefined;
    };
    contextWindow = () => chosenModel()?.contextWindow;
    const events = await watchEvents(harness, root.id, context);
    stopEvents = () => events.stop();
    // One output line at a time, with backpressure. The durable event watch itself
    // bounds pending frames and replaces an overflow with one resync snapshot.
    let failOutput!: (error: unknown) => void;
    const outputFailure = new Promise<never>((_, reject) => {
      failOutput = reject;
    });
    // A delivery can fail in a Chord subscription before the command loop installs
    // its race; keep that brief interval from becoming an unhandled rejection.
    void outputFailure.catch(() => {});
    let output = Promise.resolve();
    const send = (record: unknown): Promise<void> => {
      let line: string;
      try {
        line = JSON.stringify(record) + "\n";
        if (Buffer.byteLength(line) > MAX_RECORD)
          throw new Error("Durable response exceeds 64 MiB");
      } catch (error) {
        failOutput(error);
        return Promise.reject(error);
      }
      const next = output.then(async () => {
        if (host) await host.output(line);
        else if (!process.stdout.write(line))
          await once(process.stdout, "drain");
      });
      void next.catch(failOutput);
      flushing = output = next;
      return next;
    };
    const history = new History(root, context);
    const inboxAt = (snapshot: SnapshotEvent): unknown[] => {
      const current =
        (
          state.value.docs["pi.inbox"] as
            | { items?: { id: number }[] }
            | undefined
        )?.items ?? [];
      const wanted = new Set(snapshot.inbox.map((item) => Number(item.id)));
      return current.filter((item) => wanted.has(Number(item.id)));
    };
    const viewAt = (snapshot: SnapshotEvent): ConversationView =>
      ({
        conversation: state.value.conversation,
        entries: snapshot.entries,
        docs: {
          "pi.live": {
            ...(snapshot.run === undefined ? {} : { run: snapshot.run }),
            ...(snapshot.generation === undefined
              ? {}
              : { generation: snapshot.generation }),
            tools: snapshot.tools,
            nestedTools: snapshot.nestedTools,
            compactionFailure: snapshot.compactionFailure,
            compactions: snapshot.compactions,
          },
          "pi.inbox": { items: inboxAt(snapshot) },
          "pi.agent": snapshot.agent,
          "pi.usage": snapshot.usage,
        },
      }) as unknown as ConversationView;
    // Full lifetime history crosses the pipe only at attachment or when the
    // bounded event stream explicitly requests a resynchronization.
    const snapshot = async (value: SnapshotEvent): Promise<void> => {
      const view = viewAt(value);
      const entries = await history.entries(view);
      await send({
        type: "durable_state",
        key,
        data: imageReferences({
          ...view,
          entries,
          docs: {
            ...view.docs,
            "app.desktop": metadata.value,
            "app.subagent-calls": calls.value ?? { calls: {} },
          },
        }),
        backend: info,
      });
    };
    const wireEvent = (event: AgentEvent): unknown => {
      if (event.type === "message_end" || event.type === "entry_appended") {
        return { ...event, entry: shown(event.entry) };
      }
      if (event.type === "inbox_update") {
        const wanted = new Set(event.items.map((item) => Number(item.id)));
        const inbox =
          (
            state.value.docs["pi.inbox"] as
              | { items?: { id: number }[] }
              | undefined
          )?.items ?? [];
        return {
          ...event,
          inbox: inbox.filter((item) => wanted.has(Number(item.id))),
        };
      }
      return event;
    };
    await snapshot(events.snapshot);
    events.start((batch) => {
      const delivery = (async () => {
        const resync = batch.find(
          (event): event is SnapshotEvent => event.type === "snapshot",
        );
        if (resync !== undefined) await snapshot(resync);
        else
          await send({
            type: "durable_events",
            key,
            events: imageReferences(batch.map(wireEvent)),
            backend: info,
          });
      })();
      deliveries.add(delivery);
      void delivery.finally(() => deliveries.delete(delivery)).catch(() => {});
      return delivery;
    });
    const eventFailure = eventStreamFailure(events.closed);
    void eventFailure.catch(() => {});
    const unsubscribeMetadata = metadata.subscribe(() => {
      send({ type: "durable_metadata", key, data: metadata.value }).catch(
        report,
      );
    });
    const unsubscribeCalls = calls.subscribe(() => {
      send({
        type: "durable_subagents",
        key,
        data: calls.value ?? { calls: {} },
      }).catch(report);
    });
    dispose.push(unsubscribeMetadata, unsubscribeCalls);
    // Missing definitions, corrupt/newer storage and startup errors must fail closed, not select stock Pi.
    harness.resume();

    /** One of this session's subagents: a conversation a call of its own created. */
    const subagent = async (record: Record<string, unknown>) => {
      const conversationId = string(record, "conversationId");
      const id = Number(conversationId);
      const result = subagentResult(
        calls.value ?? { calls: {} },
        conversationId,
      );
      const conversation =
        Number.isSafeInteger(id) && result
          ? await harness.conversation(id as unknown as ConversationId, context)
          : undefined;
      const view = conversation && (await conversation.viewState(context));
      if (
        !conversation ||
        !view ||
        view.value.conversation.owner?.conversationId !== root.id
      ) {
        view?.dispose();
        throw new Error("This subagent is not part of this session");
      }
      return { conversation, view, result: result! };
    };
    const respond = async (
      record: Record<string, unknown>,
    ): Promise<unknown> => {
      const value = state.value;
      switch (record.type) {
        case "get_backend_info":
          return info;
        case "get_state":
          return {}; // Rust derives state from the committed view, never these read acknowledgements.
        case "get_messages":
          return { remoteSnapshot: true };
        case "get_image": {
          const imageId = string(record, "imageId");
          if (record.conversationId === undefined) {
            const image = findImage(
              { ...value, entries: await history.entries(value) },
              imageId,
            );
            if (!image) throw new Error("Image is not in this session");
            return { image };
          }
          // One a subagent's tool returned.
          const { conversation, view } = await subagent(record);
          try {
            const image = findImage(
              {
                ...view.value,
                entries: await new History(conversation, context).entries(
                  view.value,
                ),
              },
              imageId,
            );
            if (!image)
              throw new Error("Image is not in this subagent's conversation");
            return { image };
          } finally {
            view.dispose();
          }
        }
        case "get_subagent": {
          // Its transcript only when asked for: the parent's snapshots carry a summary.
          const { conversation, view, result } = await subagent(record);
          try {
            const entries = await new History(conversation, context).entries(
              view.value,
            );
            const live = view.value.docs["pi.live"] as
              | {
                  run?: { taskId: unknown };
                  tools?: {
                    callId: string;
                    name: string;
                    status: string;
                    output?: string;
                  }[];
                }
              | undefined;
            return imageReferences({
              conversationId: string(record, "conversationId"),
              busy: !!live?.run,
              runId: live?.run === undefined ? null : String(live.run.taskId),
              resumable:
                result.status === "done" ||
                result.status === "failed" ||
                result.status === "stopped",
              messages: transcript({ ...view.value, entries }),
              tools: (live?.tools ?? [])
                .filter((slot) => slot.status !== "done")
                .map((slot) => ({
                  callId: slot.callId,
                  name: slot.name,
                  output: slot.output ?? "",
                })),
            });
          } finally {
            view.dispose();
          }
        }
        case "stop_subagent": {
          // The call that started it carries on, and tells the model it was stopped.
          const { conversation, view } = await subagent(record);
          view.dispose();
          await conversation.abort(context);
          return {};
        }
        case "prompt_subagent": {
          const { conversation, view, result } = await subagent(record);
          view.dispose();
          const requestId = string(record, "requestId");
          const message = string(record, "message");
          const mode = string(record, "streamingBehavior");
          if (mode !== "steer" && mode !== "followUp")
            throw new Error("Invalid streamingBehavior");
          const expectedRunId =
            record.expectedRunId === undefined
              ? undefined
              : string(record, "expectedRunId");
          return submitSubagentInput(
            conversation,
            result,
            { message, requestId, mode, expectedRunId },
            context,
          );
        }
        case "get_session_stats": {
          const current = await root.context(context);
          return sessionStats(
            value.docs["pi.usage"] as unknown as UsageState | undefined,
            current,
            chosenModel()?.contextWindow,
            compactionPolicy(),
          );
        }
        case "get_commands":
          return { commands: promptCommands((found = resources(cwd))) };
        case "get_settings":
          return {
            effective: {
              compaction: compactionPolicy(),
              subagents: { swarmEnabled: swarmEnabled(metadata.value) },
            },
            compactionThresholdPercent:
              metadata.value?.compactionThresholdPercent ?? null,
            durable: true,
          };
        case "get_active_tools":
          return {
            activeTools: (await root.agent(context)).tools.map((tool) => ({
              name: tool.name,
              description: tool.description,
            })),
          };
        case "get_available_models": {
          await models.refresh({ allowNetwork: false }); // Reload host-side auth/models files and cached catalogs.
          return { models: (await models.getAvailable()).map(publicModel) };
        }
        case "get_auth_providers":
          return { providers: await authProviders(models) };
        case "get_available_thinking_levels":
          return {
            levels: chosenModel()
              ? getSupportedThinkingLevels(chosenModel()!)
              : ["off"],
          };
        case "set_model": {
          if (record.persist === true)
            throw new Error(
              "Durable global model defaults are not implemented",
            );
          const provider = string(record, "provider"),
            modelId = string(record, "modelId");
          const model = models.getModel(provider, modelId);
          if (!model)
            throw new Error(`Model not found: ${provider}/${modelId}`);
          await root.configure({ model: { provider, modelId } }, context);
          return publicModel(model);
        }
        case "set_thinking_level": {
          const level = string(record, "level");
          if (
            !chosenModel() ||
            !getSupportedThinkingLevels(chosenModel()!).includes(
              level as ModelThinkingLevel,
            )
          )
            throw new Error("Unsupported thinking level");
          await root.configure(
            { thinkingLevel: level as ModelThinkingLevel },
            context,
          );
          return { level };
        }
        case "set_swarm_enabled":
          return setSwarmEnabled(root, record.enabled, context);
        case "set_compaction_threshold": {
          const result = await setCompactionThreshold(
            root,
            record.percent,
            context,
          );
          return { ...result, compaction: compactionPolicy() };
        }
        case "set_auto_compaction": {
          if (typeof record.enabled !== "boolean")
            throw new Error("enabled must be a boolean");
          await root.commit(async (tx) => {
            (await tx.doc(Metadata, root.id)).autoCompaction =
              record.enabled as boolean;
          }, context);
          return {};
        }
        case "set_session_name": {
          if (record.sessionPath !== undefined)
            throw new Error("Cannot rename another durable session");
          const name = string(record, "name");
          await root.commit(async (tx) => {
            (await tx.doc(Metadata, root.id)).name = name;
          }, context);
          return {};
        }
        case "compact": {
          const instructions =
            record.customInstructions === undefined
              ? undefined
              : string(record, "customInstructions");
          // The durable task can compact concurrently, but its response would
          // then wait for a busy run's next boundary and block this JSONL pipe.
          // Refuse instead of aborting (which would also discard queued inputs).
          const live = state.value.docs["pi.live"] as
            | { run?: unknown }
            | undefined;
          const inbox = state.value.docs["pi.inbox"] as
            | { items?: unknown[] }
            | undefined;
          if (live?.run || (inbox?.items?.length ?? 0) > 0) {
            throw new Error(
              "Stop the session and clear queued prompts before compacting",
            );
          }
          const before = await root.context(context);
          const tokensBefore = contextTokens(before);
          const task = await harness.waitForTask(
            await root.compact(instructions, context),
            context,
          );
          const outcome = task.state.outcome;
          if (outcome.status === "failed" || outcome.status === "faulted")
            throw new Error(outcome.error.message);
          if (outcome.status === "aborted")
            throw new Error(outcome.reason ?? "Compaction cancelled");
          if (outcome.status === "orphaned") throw new Error(outcome.reason);
          const result = outcome.result;
          let entryId = result.entryId;
          if (result.submissionId !== undefined) {
            const submission = await harness.submission(
              result.submissionId,
              context,
            );
            if (!submission)
              throw new Error("Compaction summary submission disappeared");
            const settled = await submission.wait(context);
            if (settled.status === "unanswered")
              throw new Error(settled.reason);
            if (settled.type !== "write")
              throw new Error("Compaction produced an invalid submission");
            entryId = settled.entry;
          }
          if (entryId === undefined)
            throw new Error("Nothing to compact (session too small)");
          const after = await root.context(context);
          const entry = after.entries.find(
            (candidate) => candidate.id === entryId,
          );
          if (!CompactionEntry.is(entry) || entry.head === undefined) {
            throw new Error(
              "Compaction summary was not placed in the active context",
            );
          }
          return {
            summary: compactionSummary(entry.model),
            firstKeptEntryId: String(entry.head),
            tokensBefore,
            estimatedTokensAfter: contextTokens(after),
          };
        }
        case "prompt": {
          const typed = string(record, "message"),
            requestId = string(record, "requestId");
          if (!/^[a-zA-Z0-9_-]{1,128}$/.test(requestId))
            throw new Error("Invalid persistent requestId");
          if (!chosenModel())
            throw new Error(
              "Choose a configured model before submitting a durable prompt",
            );
          const acceptsImages = chosenModel()!.input.includes("image");
          const mode = record.streamingBehavior;
          if (mode !== undefined && mode !== "steer" && mode !== "followUp")
            throw new Error("Invalid streamingBehavior");
          // A retry is recognised by what was typed: an edited template or skill
          // doesn't make it a different prompt.
          const hash = createHash("sha256")
            .update(
              JSON.stringify([
                promptContent(typed, record.images, acceptsImages),
                mode ?? "reject",
              ]),
            )
            .digest("hex");
          const content = promptContent(
            expandPromptCommand(typed, (found = resources(cwd))),
            record.images,
            acceptsImages,
          );
          const duplicate = await root.commit(async (tx) => {
            const existing = await tx.submissionByRequest(root.id, requestId);
            const hashes = (await tx.doc(Receipts, root.id)).hashes;
            const previous = Object.hasOwn(hashes, requestId)
              ? hashes[requestId]
              : undefined;
            if (previous && previous !== hash)
              throw new Error(
                "requestId already belongs to a different prompt",
              );
            if (!previous && Object.keys(hashes).length >= 10_000)
              throw new Error(
                "Durable prototype reached its 10,000-prompt receipt limit",
              );
            if (["__proto__", "constructor", "prototype"].includes(requestId))
              throw new Error("Reserved requestId");
            hashes[requestId] = hash;
            return existing !== undefined;
          }, context);
          const submission = await root.submit(
            { type: "input", content, requestId, whenBusy: mode ?? "reject" },
            context,
          );
          const receipt = await submission.status(context);
          return {
            disposition: duplicate
              ? "handled"
              : receipt.status === "queued"
                ? "queued"
                : "started",
            submissionId: String(submission.id),
            requestId,
            duplicate,
          };
        }
        case "get_submission": {
          const submission = await root.commit(
            (tx) =>
              tx.submissionByRequest(root.id, string(record, "requestId")),
            context,
          );
          return {
            submission:
              submission === undefined
                ? null
                : { ...submission, id: String(submission.id) },
          };
        }
        case "clear_queue": {
          const inbox = value.docs["pi.inbox"].items as { id: SubmissionId }[];
          for (const item of inbox)
            await harness.abortSubmission(item.id, context, root.id);
          return {};
        }
        case "cancel_submission": {
          const id = submissionId(string(record, "submissionId"));
          const requestId =
            typeof record.requestId === "string" ? record.requestId : undefined;
          const handle = await harness.submission(id, context);
          if (handle === undefined)
            throw new Error("This prompt is no longer available to cancel");
          const submission = await handle.status(context);
          if (
            submission.conversationId !== root.id ||
            (requestId !== undefined && submission.requestId !== requestId)
          ) {
            throw new Error("This prompt is no longer available to cancel");
          }
          // Submission IDs are durable session-wide identities. requestId adds a
          // second check for locally admitted rows, but authoritative queue rows
          // can safely use the ID alone even if the scheduler just placed them.
          const outcome = await harness.abortSubmission(id, context, root.id);
          if (outcome === "already_placed") {
            // Abort only the task owning this placed submission;
            // Conversation.abort() would discard unrelated queued prompts.
            await abortPlacedSubmission(harness, root, id);
          }
          if (outcome === "not_found")
            throw new Error("This prompt is no longer available to cancel");
          return { outcome };
        }
        // Stopping the session stops the subagents working in the background too.
        case "abort":
          await root.abort(context, { background: true });
          return {};
        default:
          throw new Error(
            `${record.type} is not supported by the experimental durable backend`,
          );
      }
    };
    const answer = async (record: Record<string, unknown>): Promise<void> => {
      try {
        const data = await respond(record);
        await send({
          type: "response",
          id: record.id,
          command: record.type,
          success: true,
          data,
        });
      } catch (error) {
        await send({
          type: "response",
          id: record.id,
          command: record.type,
          success: false,
          error: String(error),
        });
      }
    };
    const commandsLoop = async () => {
      for await (const record of records(host?.input ?? process.stdin)) {
        if (stopping) break;
        if (
          typeof record.id !== "string" ||
          !record.id.length ||
          record.id.length > 256
        )
          throw new Error("Missing request correlation ID");
        if (record.type === "compact") {
          if (compacting !== undefined) {
            await send({
              type: "response",
              id: record.id,
              command: record.type,
              success: false,
              error: "A compaction is already running",
            });
            continue;
          }
          const operation = answer(record);
          const tracked = operation.finally(() => {
            if (compacting === tracked) compacting = undefined;
          });
          compacting = tracked;
          tracked.catch(report);
          continue;
        }
        if (
          compacting !== undefined &&
          record.type !== "abort" &&
          record.type !== "get_session_stats"
        ) {
          await send({
            type: "response",
            id: record.id,
            command: record.type,
            success: false,
            error: "Wait for compaction to finish",
          });
          continue;
        }
        await answer(record);
      }
    };
    // Listener rejection terminates watchEvents by contract. Chord document
    // subscribers isolate failures, so the shared output failure signal covers
    // their callbacks too. Either condition is fatal: staying attached without
    // incremental delivery would present a stale running session indefinitely.
    commands = commandsLoop();
    await Promise.race([commands, eventFailure, outputFailure]);
  } catch (error) {
    runFailed = true;
    throw error;
  } finally {
    stopping = true;
    host?.stopInput();
    const failures: unknown[] = [];
    const cleanup = async (operation: () => void | Promise<unknown>) => {
      try {
        await operation();
      } catch (error) {
        failures.push(error);
      }
    };
    await cleanup(() => stopEvents?.());
    // Suspension preserves checkpoints; user Stop remains the explicit abort command.
    await cleanup(() =>
      ownedHarness ? ownedHarness.close(context) : storage.close(context),
    );
    const settled = await Promise.allSettled([
      ...deliveries,
      ...(host && commands ? [commands] : []),
      ...(compacting ? [compacting] : []),
      flushing,
    ]);
    for (const outcome of settled)
      if (outcome.status === "rejected") failures.push(outcome.reason);
    for (const operation of dispose.reverse()) await cleanup(operation);
    if (failures.length) {
      if (!runFailed)
        throw new AggregateError(failures, "Durable cleanup failed");
      for (const error of failures) report(error);
    }
  }
}
