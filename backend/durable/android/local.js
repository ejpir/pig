// Android in-process transport for the shared desktop Durable runner.
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { run } from "../src/run.ts";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { phoneModels, installNativeFetch } from "./native-models.js";

const sessions = new Map();
const pending = new Map();
const http = new Map();
const terminal = new Map();
const outbox = [];
let bytes = 0;
let next = 0;
function emit(value) {
  const text = JSON.stringify(value);
  const size = Buffer.byteLength(text);
  if (size > 16 * 1024 * 1024 || bytes + size > 64 * 1024 * 1024)
    throw new Error("Native delivery queue limit");
  outbox.push(text);
  bytes += size;
}
function call(engine, type, data) {
  const id = ++next;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    try {
      emit({ channel: "host", engine, type, id, ...data });
    } catch (error) {
      pending.delete(id);
      reject(error);
    }
  });
}
const bridge = {
  call,
  fetch(operation, request, signal) {
    signal?.throwIfAborted();
    const id = ++next;
    return new Promise((resolve, reject) => {
      const state = {
        resolve,
        controller: null,
        terminal: false,
        fail(message) {
          if (state.terminal) return;
          state.terminal = true;
          const error = new Error(message);
          // reader.cancel() already closed the stream before notifying us.
          try {
            state.controller?.error(error);
          } catch {
            /* Already closed. */
          }
          reject(error);
        },
        abort() {
          if (state.terminal) return;
          state.fail("Request cancelled");
          // Local settlement must not depend on control-queue admission.
          try {
            emit({ channel: "host", type: "http_cancel", id });
          } catch {
            /* Native release/lease supervision still owns socket teardown. */
          }
        },
        cleanup() {
          signal?.removeEventListener("abort", state.abort);
        },
      };
      http.set(id, state);
      signal?.addEventListener("abort", state.abort, { once: true });
      try {
        emit({ channel: "host", type: "http", id, operation, request });
      } catch (error) {
        http.delete(id);
        state.cleanup();
        state.fail("Native request could not be admitted");
      }
    });
  },
};

class Input {
  values = [];
  bytes = 0;
  waiter = null;
  closed = false;
  put(value) {
    if (this.closed) throw new Error("Session transport closed");
    const item = Buffer.from(JSON.stringify(value) + "\n");
    if (
      this.values.length >= 128 ||
      this.bytes + item.length > 64 * 1024 * 1024
    )
      throw new Error("Session input queue limit");
    if (this.waiter) {
      const resolve = this.waiter;
      this.waiter = null;
      resolve({ value: item, done: false });
    } else {
      this.values.push(item);
      this.bytes += item.length;
    }
  }
  close() {
    this.closed = true;
    this.waiter?.({ done: true });
    this.waiter = null;
  }
  [Symbol.asyncIterator]() {
    return this;
  }
  next() {
    if (this.values.length) {
      const value = this.values.shift();
      this.bytes -= value.length;
      return Promise.resolve({ value, done: false });
    }
    return this.closed
      ? Promise.resolve({ done: true })
      : new Promise((resolve) => {
          this.waiter = resolve;
        });
  }
}

const phone = {
  init({ home, temp }) {
    process.env.HOME = home;
    process.env.TMPDIR = temp;
  },
  open(engine, options) {
    if (sessions.has(engine)) throw new Error("Duplicate embedded session");
    const input = new Input();
    const control = { mode: options.mode ?? "Reply" };
    const environments = new Set();
    const session = {
      input,
      control,
      synthetic: options.synthetic,
      failed: false,
    };
    sessions.set(engine, session);
    const features = [
      "durable_execution",
      "committed_snapshots",
      "incremental_events",
      "request_deduplication",
      "native_subscription_accounts",
      "image_prompts",
      "image_references",
      "manual_compaction",
      "subagents",
    ];
    const uiCapabilities = [
      "models",
      "thinking",
      "attachments",
      "streaming",
      "stop",
      "visualize",
      "manual_compaction",
      "auto_compaction",
    ];
    session.running = run(
      phoneModels(engine, bridge, options.synthetic, control),
      [],
      undefined,
      {
        state: options.state,
        cwd: options.cwd,
        key: options.key,
        input,
        assertOwner: () => call(engine, "assert_owner", {}),
        stopInput: () => input.close(),
        initialModel: { provider: options.provider, modelId: options.model },
        initialThinking: options.thinking,
        output: (line) =>
          emit({ channel: "protocol", engine, record: JSON.parse(line) }),
        env: ({ cwd }) => {
          const env = new NodeExecutionEnv({
            cwd: cwd ?? options.cwd,
            shellPath: options.shellPath ?? "/system/bin/sh",
            shellEnv: {
              PATH: "/system/bin",
              HOME: options.home,
              TMPDIR: options.temp,
            },
          });
          environments.add(env);
          return env;
        },
        info: {
          features,
          uiCapabilities,
          limitations: [
            "Android sh, not GNU Bash/Linux toolchains",
            "In-process experimental Local backend",
          ],
          local: true,
        },
        // Never forward raw provider/account diagnostics across the presentation boundary.
        onReport: () => {},
      },
    )
      .catch(() => {
        session.failed = true;
      })
      .finally(async () => {
        const cleanup = await Promise.allSettled(
          [...environments].map((env) => env.cleanup(BACKGROUND_CONTEXT)),
        );
        session.failed ||= cleanup.some(
          (result) => result.status === "rejected",
        );
        sessions.delete(engine);
        // A separate bounded-by-session control lane cannot be starved by output.
        // Failed close keeps the native writer lock quarantined until process exit.
        terminal.set(engine, session.failed);
      });
  },
  send(engine, record) {
    const session = sessions.get(engine);
    try {
      if (!session) throw new Error("Local session is closed");
      if (record.type === "phone_fixture_mode" && session.synthetic)
        session.control.mode = record.mode;
      else session.input.put(record);
    } catch {
      emit({
        channel: "protocol",
        engine,
        record: {
          type: "response",
          id: record.id,
          command: record.type,
          success: false,
          error:
            "Local command could not be admitted; saved state was retained",
        },
      });
    }
  },
  close(engine) {
    sessions.get(engine)?.input.close();
  },
  deliver(id, data, error) {
    const item = pending.get(id);
    if (item) {
      pending.delete(id);
      error ? item.reject(new Error(error)) : item.resolve(data);
    }
  },
  http(event) {
    const item = http.get(event.id);
    if (!item) return;
    if (event.kind === "released") {
      // Cancellation may have no logical done/error event. Never strand fetch or
      // its body while native code has already torn down the HTTP runtime.
      item.fail("Native request ended before completion");
      item.cleanup();
      http.delete(event.id);
      return;
    }
    if (item.terminal) return;
    if (event.kind === "headers")
      item.resolve(
        new Response(
          new ReadableStream({
            start(controller) {
              item.controller = controller;
            },
            cancel() {
              item.abort();
            },
          }),
          { status: event.status, headers: event.headers },
        ),
      );
    else if (event.kind === "chunk")
      item.controller?.enqueue(new Uint8Array(event.bytes));
    else if (event.kind === "done") {
      item.terminal = true;
      item.controller?.close();
    } else if (event.kind === "error")
      item.fail("Native provider transport failed");
  },
  drain() {
    const selected = [];
    for (const [engine, failed] of terminal) {
      if (failed)
        selected.push(
          JSON.stringify({
            channel: "protocol",
            engine,
            record: {
              type: "local_error",
              error:
                "Local durable session could not close safely. State was retained; restart the app to reopen it.",
            },
          }),
        );
      selected.push(
        JSON.stringify({ channel: "closed", engine, success: !failed }),
      );
    }
    terminal.clear();
    let size = selected.reduce(
      (total, value) => total + Buffer.byteLength(value) + 1,
      2,
    );
    while (
      outbox.length &&
      size + Buffer.byteLength(outbox[0]) + 1 < 22 * 1024 * 1024
    ) {
      const value = outbox.shift();
      bytes -= Buffer.byteLength(value);
      size += Buffer.byteLength(value) + 1;
      selected.push(value);
    }
    return "[" + selected.join(",") + "]";
  },
};

export function install() {
  installNativeFetch(bridge);
  globalThis.__piPhone = phone;
}
