import { Type } from "@earendil-works/pi-ai";
import { defineExtension, defineTool } from "@earendil-works/pi-durable";
import { getOrThrow } from "@earendil-works/pi-durable/env";
import { executeJavaScript } from "./javascript-execution.ts";

const ResultSchema = Type.Object({
  file: Type.String(),
  cwd: Type.String(),
  stdout: Type.String(),
  stderr: Type.String(),
  exitCode: Type.Union([Type.Integer(), Type.Null()]),
  reason: Type.Union(
    ["exit", "error", "cancelled", "timeout", "output_limit", "busy"].map(
      (value) => Type.Literal(value),
    ),
  ),
  terminationConfirmed: Type.Boolean(),
});

export const JavascriptTools = defineExtension({
  name: "javascript-files",
  tools: [
    defineTool({
      name: "javascript",
      description: [
        "Execute an existing .js/.mjs/.cjs file with Bun's Node-compatible APIs in a fresh worker, not a Node executable, shell, or eval in the SDK/auth context.",
        "file is relative to the conversation workspace; args are literal process.argv.slice(2). Relative imports use the script's directory.",
        "Workers cannot process.chdir(). For workspace-relative filesystem operations use process.env.PI_JAVASCRIPT_CWD or require('node:worker_threads').workerData.cwd; process.cwd() is the runtime process cwd. Bun import.meta.main is false in a worker.",
        "Noninteractive. stdout/stderr stream together, with separated bounded values in structured output. Combined capture is limited to 64 KiB; overflow stops the worker. timeout is 1–300 seconds, default 30. One JavaScript worker runs at a time.",
        "Unsandboxed: scripts have the app/host's filesystem and network permissions. Node/Bun compatibility on Android is not complete; native desktop modules and installed npm packages are not supplied automatically.",
        "Cancellation/timeout requests worker termination, not rollback or guaranteed termination of spawned processes. Unconfirmed termination blocks further scripts. Interrupted invocations are not replayed automatically; inspect effects before any explicit retry.",
      ].join(" "),
      parameters: Type.Object(
        {
          file: Type.String({
            minLength: 1,
            maxLength: 4096,
            description:
              "Path to the JavaScript file, relative to the conversation workspace or absolute.",
          }),
          args: Type.Optional(
            Type.Array(Type.String(), {
              maxItems: 128,
              description:
                "Literal script argv values (no shell interpolation), at most 32 KiB total.",
            }),
          ),
          timeout: Type.Optional(
            Type.Integer({
              minimum: 1,
              maximum: 300,
              description: "Execution timeout in seconds; default 30.",
            }),
          ),
        },
        { additionalProperties: false },
      ),
      replay: "unsafe",
      outputLimits: { maxBytes: 96 * 1024, maxLines: 2000, retain: "tail" },
      structuredOutputSchema: ResultSchema,
      execute: async (args, api, context) => {
        if (!api.env)
          throw new Error(
            "JavaScript requires a host filesystem execution environment.",
          );
        const file = getOrThrow(await api.env.absolutePath(args.file, context));
        const start = Date.now();
        const result = await executeJavaScript(
          { ...args, file },
          api.env.cwd,
          context.abortSignal,
          (_stream, chunk) => api.output(chunk),
        );
        context.abortSignal?.throwIfAborted();
        const failed = result.reason !== "exit" || result.exitCode !== 0;
        const status =
          result.reason === "exit" ? `exit ${result.exitCode}` : result.reason;
        api.output(
          `\n[JavaScript: ${status}${result.terminationConfirmed ? "" : "; worker termination unconfirmed—do not retry"}]\n`,
        );
        if (
          result.reason === "busy" ||
          (result.reason === "error" && !result.stdout && result.stderr)
        ) {
          // Validation/spawn failures have no running stream to report them.
          if (
            result.reason === "busy" ||
            !api.retainedOutput().text.includes(result.stderr)
          )
            api.output(result.stderr);
        }
        return {
          isError: failed,
          structuredOutput: { ...result },
          details: {
            version: 1,
            file: result.file,
            cwd: result.cwd,
            args: args.args ?? [],
            exitCode: result.exitCode,
            reason: result.reason,
            terminationConfirmed: result.terminationConfirmed,
            stdoutBytes: Buffer.byteLength(result.stdout),
            stderrBytes: Buffer.byteLength(result.stderr),
            durationMs: Date.now() - start,
          },
          diagnostics: failed
            ? [
                {
                  severity: "error" as const,
                  code: `javascript_${result.reason}`,
                  message:
                    result.reason === "exit"
                      ? `JavaScript exited with code ${result.exitCode}`
                      : `JavaScript ${result.reason}${result.terminationConfirmed ? "" : ": termination is unconfirmed; inspect effects before retrying"}`,
                },
              ]
            : [],
        };
      },
    }),
  ],
});
