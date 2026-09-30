/**
 * pi-desktop's link to its session: language server feedback, jj snapshots around
 * commands, and read-only jj tools.
 *
 * pi-desktop loads this with `pi -e` and sets PI_DESKTOP_LSP to a socket path, or to
 * `tcp:<port>` on 127.0.0.1 with PI_DESKTOP_LSP_TOKEN (Windows); elsewhere it does
 * nothing. The desktop runs the language servers and jj, so pi needs neither:
 * - after `edit` and `write`, the desktop's errors for that file, and new errors in
 *   other files, are appended to the tool result;
 * - when a run ends, errors that appeared from other changes (for example files that
 *   `bash` changed, or a slow `cargo check`) are added as a message;
 * - before and after each `bash` call the desktop takes a jj snapshot, so what the
 *   command changed can be put back (it answers at once when that is off);
 * - with PI_DESKTOP_JJ_TOOLS=1, pi gets `jj_log`, `jj_diff` and `jj_show`.
 *
 * Each request is one JSON line; the reply is `{"text"}` (empty when there is nothing
 * to report) or `{"error"}`. Failures are silent: all of this is optional.
 */
import { connect } from "node:net";
import { homedir } from "node:os";
import { resolve } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const address = process.env.PI_DESKTOP_LSP;
const token = process.env.PI_DESKTOP_LSP_TOKEN;
// The desktop waits at most 4 s per file and 20 s per run; this covers a stuck desktop.
const FILE_LIMIT_MS = 8_000;
const RUN_LIMIT_MS = 30_000;
// A snapshot of a large repository can take seconds; the command waits for it.
const SNAPSHOT_LIMIT_MS = 30_000;
const JJ_LIMIT_MS = 30_000;
const jjTools = process.env.PI_DESKTOP_JJ_TOOLS === "1";

function ask(to: string, request: object, limitMs: number, signal: AbortSignal | undefined): Promise<string> {
	return new Promise((done) => {
		let reply = "";
		const port = /^tcp:(\d+)$/.exec(to)?.[1];
		const client = port ? connect({ host: "127.0.0.1", port: Number(port) }) : connect(to);
		const timer = setTimeout(() => finish(""), limitMs);
		const onAbort = () => finish("");
		function finish(text: string) {
			clearTimeout(timer);
			signal?.removeEventListener("abort", onAbort);
			client.destroy();
			done(text);
		}
		signal?.addEventListener("abort", onAbort, { once: true });
		client.setEncoding("utf8");
		client.on("connect", () => client.write(`${JSON.stringify(token ? { ...request, token } : request)}\n`));
		client.on("data", (chunk: string) => {
			reply += chunk;
		});
		client.on("end", () => {
			try {
				const answer = JSON.parse(reply) as { text?: unknown };
				finish(typeof answer.text === "string" ? answer.text : "");
			} catch {
				finish("");
			}
		});
		client.on("error", () => finish(""));
	});
}

/** Resolves a tool's path argument the way pi's file tools do. */
function absolute(path: string, cwd: string): string {
	const plain = path.startsWith("@") ? path.slice(1) : path;
	const home = plain === "~" || plain.startsWith("~/") ? homedir() + plain.slice(1) : plain;
	return resolve(cwd, home);
}

export default function (pi: ExtensionAPI) {
	if (!address) return;

	pi.on("tool_call", async (event, ctx) => {
		if (event.toolName !== "bash") return;
		await ask(address, { op: "snapshot_before", toolCallId: event.toolCallId }, SNAPSHOT_LIMIT_MS, ctx.signal);
	});

	pi.on("tool_result", async (event, ctx) => {
		if (event.toolName === "bash") {
			await ask(address, { op: "snapshot_after", toolCallId: event.toolCallId }, SNAPSHOT_LIMIT_MS, ctx.signal);
			return;
		}
		if ((event.toolName !== "edit" && event.toolName !== "write") || event.isError) return;
		const path = event.input.path;
		if (typeof path !== "string") return;
		const text = await ask(address, { op: "file", path: absolute(path, ctx.cwd) }, FILE_LIMIT_MS, ctx.signal);
		if (!text) return;
		return { content: [...event.content, { type: "text", text: text.trimEnd() }] };
	});

	if (jjTools) registerJjTools(pi, address);

	pi.on("agent_before_settle", async (event, ctx) => {
		if (event.outcome === "aborted") return;
		const text = await ask(address, { op: "run_end" }, RUN_LIMIT_MS, ctx.signal);
		if (!text) return;
		return {
			entries: [
				...event.entries,
				{ type: "custom_message", customType: "pi-desktop-lsp", content: text.trimEnd(), display: true },
			],
		};
	});
}

/** Read-only jj history, answered by the desktop's built-in jj. */
function registerJjTools(pi: ExtensionAPI, to: string) {
	const answer = async (request: object, signal: AbortSignal | undefined) => {
		const text = await ask(to, request, JJ_LIMIT_MS, signal);
		return {
			content: [{ type: "text" as const, text: text || "jj is not available for this project right now." }],
			details: {},
		};
	};
	const revision = Type.String({
		description: "A change ID or its prefix as jj_log shows it (such as kkmpptxz), a commit ID, or @ for the working copy",
	});
	pi.registerTool({
		name: "jj_log",
		label: "jj log",
		description:
			"List this project's recent jj changes, newest first: change ID, files and lines changed, description. Each pi turn that changed files is its own change, described with its prompt.",
		promptSnippet: "List recent jj changes (turns) with their files and descriptions",
		parameters: Type.Object({
			limit: Type.Optional(Type.Number({ description: "At most this many changes (default 10)" })),
		}),
		async execute(_toolCallId, params, signal) {
			return answer({ op: "jj_log", limit: params.limit ?? 10 }, signal);
		},
	});
	pi.registerTool({
		name: "jj_diff",
		label: "jj diff",
		description: "Show a jj change's diff in unified format, for every file or one file.",
		promptSnippet: "Show a jj change's diff",
		parameters: Type.Object({
			revision,
			path: Type.Optional(Type.String({ description: "One file, relative to the project root" })),
		}),
		async execute(_toolCallId, params, signal) {
			return answer({ op: "jj_diff", revision: params.revision, path: params.path }, signal);
		},
	});
	pi.registerTool({
		name: "jj_show",
		label: "jj show",
		description: "Show a jj change's full description, author, time and the files it changed.",
		promptSnippet: "Show a jj change's description and files",
		parameters: Type.Object({ revision }),
		async execute(_toolCallId, params, signal) {
			return answer({ op: "jj_show", revision: params.revision }, signal);
		},
	});
}
