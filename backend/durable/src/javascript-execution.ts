// File-based JavaScript execution, not eval in the SDK/auth realm or a Bun CLI.
import { realpath, stat } from "node:fs/promises";
import { extname, resolve } from "node:path";
import { homedir, tmpdir } from "node:os";
import { Worker } from "node:worker_threads";

export const JAVASCRIPT_OUTPUT_BYTES = 64 * 1024;
const MAX_SOURCE_BYTES = 2 * 1024 * 1024;
const TERMINATE_GRACE_MS = 2000;
// One worker at a time bounds ordinary use; it is NOT a memory/security sandbox.
const active = new Set<Worker>();
export interface JavaScriptRequest {
  file: string;
  args?: string[];
  timeout?: number;
}
export interface JavaScriptResult {
  file: string;
  cwd: string;
  stdout: string;
  stderr: string;
  exitCode: number | null;
  reason: "exit" | "error" | "cancelled" | "timeout" | "output_limit" | "busy";
  terminationConfirmed: boolean;
}

export async function executeJavaScript(
  request: JavaScriptRequest,
  cwd: string,
  signal?: AbortSignal,
  onOutput?: (stream: "stdout" | "stderr", chunk: string) => void,
): Promise<JavaScriptResult> {
  const base: JavaScriptResult = {
    file: request.file,
    cwd,
    stdout: "",
    stderr: "",
    exitCode: null,
    reason: "error",
    terminationConfirmed: true,
  };
  if (signal?.aborted) return { ...base, reason: "cancelled" };
  try {
    if (
      !request.file ||
      request.file.includes("\0") ||
      request.file.length > 4096
    )
      throw new Error("Provide a JavaScript file path, not inline source.");
    const args = request.args ?? [];
    if (
      args.length > 128 ||
      args.some((arg) => typeof arg !== "string" || arg.includes("\0")) ||
      Buffer.byteLength(JSON.stringify(args)) > 32768
    )
      throw new Error("Script arguments exceed the supported limit.");
    const timeout = request.timeout ?? 30;
    if (!Number.isInteger(timeout) || timeout < 1 || timeout > 300)
      throw new Error("timeout must be an integer from 1 to 300 seconds.");
    const file = await realpath(resolve(cwd, request.file));
    const info = await stat(file);
    if (!info.isFile() || ![".js", ".mjs", ".cjs"].includes(extname(file)))
      throw new Error(
        "file must be an existing .js, .mjs or .cjs regular file.",
      );
    if (info.size > MAX_SOURCE_BYTES)
      throw new Error("Script entry file exceeds 2 MiB.");
    base.file = file;
    if (signal?.aborted) return { ...base, reason: "cancelled" };
    if (active.size)
      return {
        ...base,
        reason: "busy",
        stderr:
          "A JavaScript worker is still running or stopping. Nothing was started.",
      };
    return await new Promise<JavaScriptResult>((resolveResult) => {
      const worker = new Worker(file, {
        argv: args,
        // Never copy the parent environment or install any native account bridge.
        env: {
          PATH: process.env.PATH ?? "/system/bin",
          HOME: process.env.HOME ?? homedir(),
          TMPDIR: process.env.TMPDIR ?? tmpdir(),
          PI_JAVASCRIPT_CWD: cwd,
        },
        workerData: { cwd },
        stdout: true,
        stderr: true,
      });
      active.add(worker);
      let reason: JavaScriptResult["reason"] = "exit";
      let settled = false;
      let captured = 0;
      const stdout: string[] = [],
        stderr: string[] = [];
      const decoders = { stdout: new TextDecoder(), stderr: new TextDecoder() };
      let grace: ReturnType<typeof setTimeout> | undefined;
      const finish = (
        exitCode: number | null,
        terminationConfirmed: boolean,
      ) => {
        if (settled) return;
        captureText(stdout, decoders.stdout.decode());
        captureText(stderr, decoders.stderr.decode());
        settled = true;
        clearTimeout(timer);
        if (grace) clearTimeout(grace);
        signal?.removeEventListener("abort", abort);
        resolveResult({
          ...base,
          stdout: stdout.join(""),
          stderr: stderr.join(""),
          exitCode,
          reason,
          terminationConfirmed,
        });
      };
      const stop = (why: JavaScriptResult["reason"]) => {
        if (reason !== "exit" || settled) return;
        reason = why;
        // Do not free the slot until exit. A native operation may not stop at
        // once; after grace, report uncertainty and quarantine that worker.
        grace = setTimeout(() => finish(null, false), TERMINATE_GRACE_MS);
        void worker.terminate().catch(() => {
          /* Exit remains the authority. */
        });
      };
      const captureText = (target: string[], text: string) => {
        if (settled || !text) return;
        const bytes = Buffer.from(text);
        let keep = Math.min(bytes.length, JAVASCRIPT_OUTPUT_BYTES - captured);
        // UTF-8 decoding can expand invalid bytes into U+FFFD. Bound encoded
        // result bytes, and never split a code point at the capture boundary.
        while (keep > 0 && keep < bytes.length && (bytes[keep] & 0xc0) === 0x80)
          keep--;
        if (keep > 0) {
          const part = bytes.subarray(0, keep).toString("utf8");
          target.push(part);
          captured += keep;
          try {
            onOutput?.(target === stdout ? "stdout" : "stderr", part);
          } catch {
            stop("cancelled");
          }
        }
        if (keep < bytes.length) stop("output_limit");
      };
      const capture = (stream: "stdout" | "stderr", chunk: Buffer | string) => {
        const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
        captureText(
          stream === "stdout" ? stdout : stderr,
          decoders[stream].decode(bytes, { stream: true }),
        );
      };
      const abort = () => stop("cancelled");
      const timer = setTimeout(() => stop("timeout"), timeout * 1000);
      worker.stdout!.on("data", (chunk) => capture("stdout", chunk));
      worker.stderr!.on("data", (chunk) => capture("stderr", chunk));
      worker.on("error", (error) => {
        captureText(
          stderr,
          error instanceof Error
            ? (error.stack ?? error.message)
            : String(error),
        );
        stop("error");
      });
      worker.on("exit", (code) => {
        active.delete(worker);
        finish(code, true);
      });
      signal?.addEventListener("abort", abort, { once: true });
      if (signal?.aborted) abort();
    });
  } catch (error) {
    return {
      ...base,
      stderr: String(error instanceof Error ? error.message : error).slice(
        0,
        4096,
      ),
    };
  }
}
