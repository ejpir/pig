import { expect, test } from "bun:test";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { Type, type AssistantMessage, type AssistantMessageEvent, type Model, type ToolCall } from "@earendil-works/pi-ai";
import { createModels, createProvider, type Models } from "@earendil-works/pi-ai/models";
import { AssistantMessageEventStream } from "@earendil-works/pi-ai/utils/event-stream";
import {
  createRegistry, defineExtension, defineTool, Harness, LiveDoc, MemoryStorage, watchEvents,
  type AgentEvent, type MessageChange,
} from "@earendil-works/pi-durable";
import { durableModels, publicAssistantMessage } from "../src/model-stream.ts";

const context = BACKGROUND_CONTEXT;
const clone = <T>(value: T): T => structuredClone(value);
const scratchKeys = ["partialJson", "partialArgs", "index", "customInput", "streamIndex"] as const;
const model: Model<"openai-responses"> = {
  id: "offline-stream", name: "Offline stream", api: "openai-responses", provider: "model-stream-test",
  baseUrl: "https://must-not-be-called.invalid", reasoning: true, input: ["text"],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 100_000, maxTokens: 4096,
};
function toolCall(args: ToolCall["arguments"] = {
  payload: "", nested: { list: [{ value: "" }] },
  partialJson: "argument, not scratch", partialArgs: { index: 7, customInput: "keep me" },
  index: 9, customInput: "public argument",
}) {
  return {
    type: "toolCall" as const, id: "call-1", name: "capture", arguments: args,
    thoughtSignature: "opaque signature", namespace: "public.namespace",
    public: { visible: true, index: 6 }, unknownMetadata: { partialJson: "also public", customInput: "metadata" },
    partialJson: "{", partialArgs: { parser: "private" }, index: 0, customInput: "private input",
  };
}
function message(content: AssistantMessage["content"] = [toolCall()]): AssistantMessage {
  return {
    role: "assistant", content, api: model.api, provider: model.provider, model: model.id,
    responseId: "response-1", responseModel: "provider-model", providerThinkingLevel: "native-high", thinkingLevel: "high",
    usage: { input: 2, output: 0, cacheRead: 1, cacheWrite: 0, cacheWrite1h: 0, reasoning: 0, totalTokens: 3,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } },
    stopReason: "pending", rawStopReason: "provider-stop", endTurn: false, timestamp: 1, durationMs: 17,
    diagnostics: [],
  };
}
// An independent expected projection, not the implementation under test.
function expectedPublic(input: AssistantMessage): AssistantMessage {
  const result = clone(input);
  for (const block of result.content) {
    const omitted = block.type === "toolCall" ? scratchKeys : block.type === "thinking" ? ["index", "redactedChunks"] : ["index"];
    for (const key of omitted) delete (block as unknown as Record<string, unknown>)[key];
  }
  return result;
}
function expectNoScratch(input: AssistantMessage) {
  for (const block of input.content) {
    if (block.type === "toolCall") for (const key of scratchKeys) expect(Object.hasOwn(block, key)).toBe(false);
  }
}
function freeze<T>(value: T): T {
  if (value && typeof value === "object") {
    for (const child of Object.values(value)) freeze(child);
    Object.freeze(value);
  }
  return value;
}
function gate<T = void>() {
  let open!: (value: T) => void;
  const promise = new Promise<T>((resolve) => { open = resolve; });
  return { promise, open };
}
// Deadlines only diagnose a missing event; provider chunks are ordered by commits, never sleeps.
async function bounded<T>(promise: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([promise, new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error("Expected gated stream event was not committed")), 2000);
    })]);
  } finally { clearTimeout(timer); }
}

test("publicAssistantMessage removes only known direct provider bookkeeping without mutation or deep copying", () => {
  const args = toolCall().arguments;
  const first = Object.assign(toolCall(args), { streamIndex: 42, extraProviderField: "keep unknown metadata" });
  // Undefined scratch still must be absent as an own property in the public result.
  const second = { ...toolCall({ index: 10, nested: [{ partialArgs: "argument" }] }), id: "call-2", partialJson: undefined };
  const text = { type: "text" as const, text: "hello 🌍", textSignature: "text signature", index: 33, partialJson: "text metadata" };
  const thinking = { type: "thinking" as const, thinking: "reasoning", thinkingSignature: "thinking signature", redacted: true,
    index: 34, partialArgs: { index: 35 }, customInput: "thinking metadata" };
  const input = Object.assign(message([text, thinking, first, second]), {
    partialJson: "message metadata", partialArgs: { index: 5 }, index: 88, customInput: "message metadata",
    providerMetadata: { partialJson: "keep", flags: ["unknown"] },
  });
  const before = clone(input);
  freeze(input);
  const output = publicAssistantMessage(input);
  expect(output).toEqual(expectedPublic(before));
  expect(input).toEqual(before);
  expectNoScratch(output);
  expect(output.content[2]).not.toBe(first);
  expect((output.content[2] as ToolCall).arguments).toBe(args);
  expect((output.content[2] as typeof first).public).toBe(first.public);
  expect((output.content[2] as typeof first).unknownMetadata).toBe(first.unknownMetadata);
  expect(output.content[0]).toEqual(expectedPublic(message([text])).content[0]);
  expect(output.content[1]).toEqual(expectedPublic(message([thinking])).content[0]);
  expect(output.usage).toBe(input.usage);
  expect((output as typeof input).providerMetadata).toBe(input.providerMetadata);
  expect(publicAssistantMessage(output)).toEqual(output);
});

class ObservedStream extends AssistantMessageEventStream {
  pulls = 0;
  override async *[Symbol.asyncIterator](): AsyncGenerator<AssistantMessageEvent> {
    const iterator = super[Symbol.asyncIterator]();
    try {
      while (true) {
        const next = await iterator.next();
        if (next.done) return;
        this.pulls++;
        yield next.value;
      }
    } finally { await iterator.return?.(); }
  }
}
// Private state catches a proxy/object wrapper accidentally using itself as `this`.
class PrivateRuntime {
  #calls: { method: string; args: unknown[] }[] = [];
  #value = { untouched: "runtime result" };
  streamSource = new ObservedStream();
  response = message();
  get privateValue() { return this.#value; }
  record(method: string, args: unknown[]) { this.#calls.push({ method, args }); return this.#value; }
  calls() { return this.#calls; }
  stream(...args: unknown[]) { this.record("stream", args); return this.streamSource; }
  streamSimple(...args: unknown[]) { this.record("streamSimple", args); return this.streamSource; }
  streamDeferred(...args: unknown[]) { this.record("streamDeferred", args); return this.streamSource; }
  async complete(...args: unknown[]) { this.record("complete", args); return this.response; }
  async completeSimple(...args: unknown[]) { this.record("completeSimple", args); return this.response; }
  async fetchDeferred(...args: unknown[]) { this.record("fetchDeferred", args); return this.response; }
}
const ordinaryMethods = [
  "getProviders", "getProvider", "getModels", "getModel", "getModelsOfType", "getModelOfType", "getAllModels",
  "refresh", "checkAuth", "getAvailable", "getAvailableOfType", "getAllAvailable", "getAuth", "login", "logout",
  "cancelDeferred", "generateImages", "classify", "setProvider", "deleteProvider", "clearProviders", "registerNativeProvider",
] as const;
for (const name of ordinaryMethods) {
  Object.defineProperty(PrivateRuntime.prototype, name, {
    value: function (this: PrivateRuntime, ...args: unknown[]) { return this.record(name, args); },
  });
}
function privateRuntime() {
  return new PrivateRuntime() as PrivateRuntime & Models & Record<typeof ordinaryMethods[number], (...args: unknown[]) => unknown>;
}

test("durableModels preserves the runtime subtype and delegates every other operation with original this and inputs", async () => {
  const original = privateRuntime();
  const wrapped: typeof original = durableModels(original);
  expect(wrapped.privateValue).toBe(original.privateValue);
  const args = [model, { arbitrary: "input" }, { signal: new AbortController().signal }];
  for (const name of ordinaryMethods) {
    const result = await (wrapped[name] as (...args: unknown[]) => unknown)(...args);
    expect(result).toBe(original.privateValue);
    const call = original.calls().at(-1)!;
    expect(call.method).toBe(name);
    expect(call.args).toHaveLength(args.length);
    call.args.forEach((value, index) => expect(value).toBe(args[index]));
  }
  expect(wrapped.calls()).toBe(original.calls());
});

const streamMethods = ["stream", "streamSimple", "streamDeferred"] as const;
for (const method of streamMethods) {
  for (const terminal of ["done", "error"] as const) {
    test(`${method} lazily normalizes every event and terminal ${terminal} result without a second queue`, async () => {
      const original = privateRuntime();
      const raw = original.streamSource;
      const partial = Object.assign(message([
        { type: "text", text: "text", textSignature: "text-sig" },
        { type: "thinking", thinking: "thought", thinkingSignature: "thinking-sig" }, toolCall(),
      ]), { providerExtension: { customInput: "preserved" } });
      const call = partial.content[2] as ReturnType<typeof toolCall>;
      const events: AssistantMessageEvent[] = [
        { type: "start", partial },
        { type: "text_start", contentIndex: 0, partial },
        { type: "text_delta", contentIndex: 0, delta: "text", partial },
        { type: "text_end", contentIndex: 0, content: "text", partial },
        { type: "thinking_start", contentIndex: 1, partial },
        { type: "thinking_delta", contentIndex: 1, delta: "thought", partial },
        { type: "thinking_end", contentIndex: 1, content: "thought", partial },
        { type: "toolcall_start", contentIndex: 2, partial },
        { type: "toolcall_delta", contentIndex: 2, delta: "{\"payload\":\"🌍\"}", partial },
        { type: "toolcall_end", contentIndex: 2, toolCall: call, partial },
      ];
      partial.stopReason = terminal === "done" ? "toolUse" : "error";
      if (terminal === "error") partial.errorMessage = "offline failure";
      events.push(terminal === "done"
        ? { type: "done", reason: "toolUse", message: partial }
        : { type: "error", reason: "error", error: partial });
      // Queue in the *provider's* stream, before calling the adapter.
      for (const event of events) raw.push(event);
      const before = clone(partial);
      const wrapped = durableModels(original);
      const args = [model, { messages: [] }, { signal: new AbortController().signal }];
      const stream = (wrapped[method] as (...args: unknown[]) => AssistantMessageEventStream)(...args);
      expect(original.calls().at(-1)?.method).toBe(method);
      original.calls().at(-1)!.args.forEach((arg, index) => expect(arg).toBe(args[index]));
      // result() must not drain/enqueue events, even when the provider already finished.
      expect(await stream.result()).toEqual(expectedPublic(partial));
      expect(raw.pulls).toBe(0);
      const iterator = stream[Symbol.asyncIterator]();
      for (const source of events) {
        const next = await iterator.next();
        expect(next.done).toBe(false);
        const event = next.value!;
        const expected = clone(source);
        if ("partial" in expected && "partial" in source) expected.partial = expectedPublic(source.partial);
        else if (expected.type === "done") expected.message = expectedPublic(partial);
        else if (expected.type === "error") expected.error = expectedPublic(partial);
        if (expected.type === "toolcall_end") expected.toolCall = expectedPublic(message([call])).content[0] as ToolCall;
        expect(event).toEqual(expected);
        if ("partial" in event) {
          expectNoScratch(event.partial);
          expect((event.partial.content[2] as ToolCall).arguments).toBe(call.arguments);
          expect(event.partial.content[0]).toEqual(partial.content[0]);
        }
        expect(raw.pulls).toBe(events.indexOf(source) + 1);
      }
      expect((await iterator.next()).done).toBe(true);
      expect(partial).toEqual(before);
    });
  }
  test(`${method} normalizes result-only streams without consuming their iterator`, async () => {
    const original = privateRuntime();
    const stream = durableModels(original)[method](model, {} as never);
    const result = stream.result();
    original.response.stopReason = "stop";
    original.streamSource.end(original.response);
    expect(await result).toEqual(expectedPublic(original.response));
    expect(original.streamSource.pulls).toBe(0);
    expect(Object.hasOwn(original.response.content[0]!, "partialJson")).toBe(true);
  });
}

for (const method of ["complete", "completeSimple", "fetchDeferred"] as const) {
  test(`${method} normalizes the original runtime result with original this and without mutation`, async () => {
    const original = privateRuntime();
    freeze(original.response);
    const args = [model, { id: "deferred-id" }, { signal: new AbortController().signal }];
    const output = await (durableModels(original)[method] as (...args: unknown[]) => Promise<AssistantMessage>)(...args);
    expect(output).toEqual(expectedPublic(original.response));
    expectNoScratch(output);
    expect((output.content[0] as ToolCall).arguments).toBe((original.response.content[0] as ToolCall).arguments);
    expect(original.calls().at(-1)?.method).toBe(method);
    original.calls().at(-1)!.args.forEach((arg, index) => expect(arg).toBe(args[index]));
    expect(Object.hasOwn(original.response.content[0]!, "customInput")).toBe(true);
  });
}

// Reconstruct the public wire protocol instead of relying on final whole messages to hide broken deltas.
function applyChanges(current: AssistantMessage, changes: readonly MessageChange[]): AssistantMessage {
  for (const change of changes) {
    if (change.type === "message") { current = clone(change.message); continue; }
    const block = current.content[change.contentIndex];
    if (change.type === "text_delta") {
      if (block?.type !== "text") throw new Error("text delta without text block");
      block.text += change.delta;
    } else if (change.type === "thinking_delta") {
      if (block?.type !== "thinking") throw new Error("thinking delta without thinking block");
      block.thinking += change.delta;
    } else if (change.type === "toolcall_delta") {
      if (block?.type !== "toolCall" || change.path.length === 0) throw new Error("invalid tool argument delta");
      let target: any = block.arguments;
      for (const segment of change.path.slice(0, -1)) target = target[segment];
      const key = change.path.at(-1)!;
      if (typeof target[key] !== "string") throw new Error("tool delta does not address a string");
      target[key] += change.delta;
    } else if ("block" in change) {
      current.content[change.contentIndex] = clone(change.block);
    } else { throw new Error("Unknown message change"); }
  }
  return current;
}
function eventLog() {
  const events: AgentEvent[] = [];
  let changed = gate();
  return {
    events,
    listener: async (batch: readonly AgentEvent[]) => {
      events.push(...batch);
      const previous = changed;
      changed = gate();
      previous.open();
    },
    async next<T extends AgentEvent["type"]>(type: T, after = 0,
      matches?: (event: Extract<AgentEvent, { type: T }>) => boolean,
    ): Promise<Extract<AgentEvent, { type: T }>> {
      while (true) {
        const found = events.slice(after).find((event) => event.type === type &&
          (matches === undefined || matches(event as Extract<AgentEvent, { type: T }>)));
        if (found) return found as Extract<AgentEvent, { type: T }>;
        await bounded(changed.promise);
      }
    },
  };
}
async function streamingHarness(args?: ToolCall["arguments"]) {
  const raw = new AssistantMessageEventStream();
  const ready = gate();
  const call = toolCall(args);
  const text = { type: "text" as const, text: "", textSignature: "keep text signature" };
  const thinking = { type: "thinking" as const, thinking: "", thinkingSignature: "keep thinking signature" };
  const partial = message([text, thinking, call]);
  const received: ToolCall["arguments"][] = [];
  let requests = 0;
  const provider = createProvider({
    id: model.provider, models: [model],
    auth: { apiKey: { name: "Offline fake", resolve: async () => ({ auth: {} }) } },
    api: {
      stream: () => { throw new Error("Harness should call streamSimple, not a network API"); },
      streamSimple: () => {
        requests++;
        if (requests === 1) { ready.open(); return raw; }
        const result = message([{ type: "text", text: "finished offline" }]);
        result.stopReason = "stop";
        const final = new AssistantMessageEventStream();
        final.push({ type: "start", partial: message([]) });
        final.push({ type: "done", reason: "stop", message: result });
        return final;
      },
    },
  });
  const models = createModels();
  models.setProvider(provider);
  const registry = createRegistry();
  registry.install(defineExtension({ name: "model-stream-regression", tools: [defineTool({
    name: "capture", description: "Capture exact offline arguments", replay: "safe",
    parameters: Type.Object({ payload: Type.String(), nested: Type.Any() }, { additionalProperties: true }),
    execute: async (args) => {
      received.push(clone(args));
      return { output: [{ type: "text", text: "captured" }] };
    },
  })] }));
  const harness = await Harness.open(new MemoryStorage(), {
    models: durableModels(models), registry, settings: { progress: { partialIntervalMs: 2 }, compaction: { enabled: false } },
  }, context);
  const root = await harness.root(context, { agent: {
    model: { provider: model.provider, modelId: model.id },
  } });
  const watch = await watchEvents(harness, root.id, context);
  const log = eventLog();
  watch.start(log.listener);
  const submission = await root.submit({ type: "input", content: "Generate a tool call offline" }, context);
  await bounded(ready.promise);
  const committed = () => root.commit(async (tx): Promise<AssistantMessage | undefined> => {
    const current = (await tx.doc(LiveDoc, root.id)).generation?.message;
    return current === undefined ? undefined : JSON.parse(JSON.stringify(current));
  }, context);
  const scratch = (round: number) => {
    call.partialJson = JSON.stringify(call.arguments) + " ".repeat(round);
    call.partialArgs = { parser: `private-${round}` };
    call.index = round;
    call.customInput = `private input ${round}`;
  };
  const finish = async () => {
    partial.stopReason = "toolUse";
    raw.push({ type: "toolcall_end", contentIndex: 2, toolCall: call, partial });
    raw.push({ type: "done", reason: "toolUse", message: partial });
    expect(await submission.wait(context)).toMatchObject({ type: "input", status: "done" });
    await root.waitForIdle(context);
    expect(received).toEqual([call.arguments]);
    expect(requests).toBe(2);
    const reconnect = await watchEvents(harness, root.id, context);
    try {
      const assistants = reconnect.snapshot.entries.flatMap((entry) => entry.model ?? []).filter((entry) => entry.role === "assistant");
      expect(assistants).toHaveLength(2);
      for (const assistant of assistants) expectNoScratch(assistant);
      expect(assistants[0]).toEqual(expectedPublic(partial));
    } finally { await reconnect.stop(); }
  };
  const close = async () => {
    // end() releases a failed test's provider wait without creating any new request.
    const aborted = { ...partial, stopReason: "aborted" as const };
    raw.end(aborted);
    await watch.stop();
    await harness.close(context);
  };
  return { harness, root, raw, call, text, thinking, partial, received, log, committed, scratch, finish, close };
}

test("provider scratch no longer turns growing argument strings into whole blocks; committed/reconnected snapshots are deep and clean", async () => {
  const fixture = await streamingHarness();
  const { raw, call, text, thinking, partial, log } = fixture;
  let reconnect: Awaited<ReturnType<typeof watchEvents>> | undefined;
  try {
    raw.push({ type: "start", partial });
    raw.push({ type: "toolcall_start", contentIndex: 2, partial });
    const started = await log.next("message_start", 0, (event) => event.message.role === "assistant");
    expect(started.message.role).toBe("assistant");
    let reconstructed = clone(started.message as AssistantMessage);
    const first = expectedPublic(partial);
    expect(started.message).toEqual(first);
    expect(await fixture.committed()).toEqual(first);
    reconnect = await watchEvents(fixture.harness, fixture.root.id, context);
    expect(reconnect.snapshot.generation?.message).toEqual(first);
    const heldReconnectMessage = reconnect.snapshot.generation!.message!;
    expectNoScratch(heldReconnectMessage);
    await reconnect.stop();
    reconnect = undefined;

    const args = call.arguments as { payload: string; nested: { list: { value: string }[] }; partialJson: string };
    const chunks = ["const greeting = '", "你好 🌍';\n", "// café é\n", "return greeting;\n" + "x".repeat(256)];
    for (const [index, chunk] of chunks.entries()) {
      const after = log.events.length;
      args.payload += chunk;
      args.nested.list[0].value += `nested ${chunk}`;
      args.partialJson += ` public-${index}`; // Same-named *argument* must not be stripped.
      text.text += `text ${chunk}`;
      thinking.thinking += `thought ${chunk}`;
      fixture.scratch(index + 1);
      // Mutating the provider's nested objects cannot revise an already committed snapshot.
      expect(started.message).toEqual(first);
      expect(heldReconnectMessage).toEqual(first);
      if (index === 0) expect(await fixture.committed()).toEqual(first);
      raw.push({ type: "toolcall_delta", contentIndex: 2, delta: chunk, partial });
      const update = await log.next("message_update", after);
      expect(update.changes.map((change) => change.type).sort()).toEqual(([
        "text_delta", "thinking_delta", "toolcall_delta", "toolcall_delta", "toolcall_delta",
      ] as MessageChange["type"][]).sort());
      expect(update.changes.filter((change) => change.type === "toolcall_delta")
        .map((change) => JSON.stringify(change.path)).sort()).toEqual([
        ["payload"], ["nested", "list", 0, "value"], ["partialJson"],
      ].map((path) => JSON.stringify(path)).sort());
      reconstructed = applyChanges(reconstructed, update.changes);
      reconstructed.usage = clone(update.usage);
      expect(reconstructed).toEqual(expectedPublic(partial));
      expect(await fixture.committed()).toEqual(expectedPublic(partial));
      expectNoScratch(reconstructed);
      expect(fixture.received).toEqual([]); // Partials never execute tools.
    }
    const midstream = await watchEvents(fixture.harness, fixture.root.id, context);
    try {
      expect(midstream.snapshot.generation?.message).toEqual(reconstructed);
      expectNoScratch(midstream.snapshot.generation!.message!);
    } finally { await midstream.stop(); }
    await fixture.finish();
    expect(started.message).toEqual(first);
    expect(heldReconnectMessage).toEqual(first);
  } finally {
    await reconnect?.stop();
    await fixture.close();
  }
});

test("an 80-byte argument append does not resend a 20 KB partial", async () => {
  const fixture = await streamingHarness({ payload: "x".repeat(20_000), nested: { list: [{ value: "" }] } });
  const { raw, partial, call, log } = fixture;
  try {
    fixture.scratch(0);
    raw.push({ type: "start", partial });
    await log.next("message_start", 0, (event) => event.message.role === "assistant");
    const after = log.events.length;
    const delta = "y".repeat(80);
    (call.arguments as { payload: string }).payload += delta;
    fixture.scratch(1);
    raw.push({ type: "toolcall_delta", contentIndex: 2, delta, partial });
    const update = await log.next("message_update", after);
    expect(update.changes).toEqual([{ type: "toolcall_delta", contentIndex: 2, path: ["payload"], delta }]);
    // This is the record shape the Rust adapter forwards to the phone.
    const wire = JSON.stringify({ type: "message_update", assistantMessageEvent: update.changes[0] });
    expect(Buffer.byteLength(wire)).toBeLessThan(300);
    expect(wire).not.toContain("x".repeat(100));
    await fixture.finish();
  } finally { await fixture.close(); }
});

test("structural tool argument and public metadata changes retain block fallback, then resume valid string deltas", async () => {
  const fixture = await streamingHarness({ payload: "before", nested: { list: [{ value: "a" }, { value: "b" }] }, number: 1, removable: true });
  const { raw, partial, call, log } = fixture;
  try {
    raw.push({ type: "start", partial });
    let reconstructed = clone((await log.next("message_start", 0, (event) => event.message.role === "assistant")).message as AssistantMessage);
    const args = call.arguments as Record<string, any>;
    const changes = [
      () => { args.payload = "replacement"; }, // Non-prefix replacement, not an append.
      () => { args.number = 2; },
      () => { args.added = { enabled: true }; },
      () => { delete args.removable; },
      () => { args.nested.list.pop(); }, // Array shrinking is structural.
      () => { call.namespace = "changed.public.namespace"; call.thoughtSignature = "new signature"; call.public.visible = false; },
    ];
    for (const [index, change] of changes.entries()) {
      const after = log.events.length;
      change();
      fixture.scratch(index + 1);
      raw.push({ type: "toolcall_delta", contentIndex: 2, delta: "provider JSON change", partial });
      const update = await log.next("message_update", after);
      expect(update.changes).toEqual([{ type: "block", contentIndex: 2, block: expectedPublic(partial).content[2] }]);
      reconstructed = applyChanges(reconstructed, update.changes);
      expect(reconstructed).toEqual(expectedPublic(partial));
      expect(await fixture.committed()).toEqual(reconstructed);
      expect(fixture.received).toEqual([]);
    }
    const after = log.events.length;
    args.payload += " 🌍";
    args.nested.list[0].value += " 你好";
    fixture.scratch(99);
    raw.push({ type: "toolcall_delta", contentIndex: 2, delta: "unicode suffix", partial });
    const update = await log.next("message_update", after);
    expect(update.changes).toEqual([
      { type: "toolcall_delta", contentIndex: 2, path: ["payload"], delta: " 🌍" },
      { type: "toolcall_delta", contentIndex: 2, path: ["nested", "list", 0, "value"], delta: " 你好" },
    ]);
    reconstructed = applyChanges(reconstructed, update.changes);
    expect(reconstructed).toEqual(expectedPublic(partial));
    await fixture.finish();
  } finally { await fixture.close(); }
});
