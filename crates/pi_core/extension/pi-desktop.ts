/**
 * Pi Desktop's extension. pi-desktop runs every session as stock `pi --mode rpc -e <this file>`.
 *
 * Desktop channel (PI_DESKTOP_CHANNEL, a socket path or `tcp:<port>` on 127.0.0.1 with
 * PI_DESKTOP_CHANNEL_TOKEN): what pi's RPC mode lacks, through pi's public extension API only.
 * The extension connects when a session starts and says `hello`; pi recreates extensions when it
 * replaces the session (new, resume, fork, reload), and the new instance connects again. The
 * desktop sends `{id, type, ...}`; each reply is shaped like a pi RPC response,
 * `{type: "response", id, command, success, data | error}`, so the desktop handles both alike.
 * Requests that need session control (tree navigation, reload) run as this extension's
 * `/pi-desktop` command, which pi executes with a command context and never sends to the model.
 *
 * Bridge (PI_DESKTOP_LSP, the same address forms with PI_DESKTOP_LSP_TOKEN): the desktop runs the
 * language servers and jj, so pi needs neither:
 * - after `edit` and `write`, the desktop's errors for that file, and new errors in other files,
 *   are appended to the tool result;
 * - when a run ends, errors that appeared from other changes (for example files that `bash`
 *   changed, or a slow `cargo check`) are added as a message;
 * - before and after each `bash` call the desktop takes a jj snapshot, so what the command changed
 *   can be put back (it answers at once when that is off);
 * - with PI_DESKTOP_JJ_TOOLS=1, pi gets `jj_log`, `jj_diff` and `jj_show`.
 * Each bridge request is one JSON line on its own connection; the reply is `{"text"}` (empty when
 * there is nothing to report) or `{"error"}`. Bridge failures are silent: all of it is optional.
 */
import { randomUUID } from "node:crypto";
import { open, realpath, stat } from "node:fs/promises";
import { connect, type Socket } from "node:net";
import { homedir } from "node:os";
import { dirname, isAbsolute, resolve } from "node:path";
import { realpathSync } from "node:fs";
import { getSupportedThinkingLevels } from "@earendil-works/pi-ai";
import type { ExtensionAPI, ExtensionCommandContext, ExtensionContext } from "@earendil-works/pi-coding-agent";
import {
	CURRENT_SESSION_VERSION,
	DefaultPackageManager,
	getAgentDir,
	hasTrustRequiringProjectResources,
	ProjectTrustStore,
	SessionManager,
	SettingsManager,
	VERSION,
} from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

/** The pi release this extension is written against; the desktop refuses others. */
const EXTENSION_VERSION = "0.99.1-1";
const PROTOCOL_VERSION = 2;
const COMMAND = "pi-desktop";
const COMMAND_DESCRIPTION = "Pi Desktop's session requests (used by the desktop app)";
// pi runs an extension command at once unless a run is settling; this covers a stuck pi.
const COMMAND_START_LIMIT_MS = 15_000;

const channelAddress = process.env.PI_DESKTOP_CHANNEL;
const channelToken = process.env.PI_DESKTOP_CHANNEL_TOKEN;
const bridgeAddress = process.env.PI_DESKTOP_LSP;
const bridgeToken = process.env.PI_DESKTOP_LSP_TOKEN;
// The desktop waits at most 4 s per file and 20 s per run; this covers a stuck desktop.
const FILE_LIMIT_MS = 8_000;
const RUN_LIMIT_MS = 30_000;
// A snapshot of a large repository can take seconds; the command waits for it.
const SNAPSHOT_LIMIT_MS = 30_000;
const JJ_LIMIT_MS = 30_000;
const SHARE_LIMIT_MS = 25_000;
const jjTools = process.env.PI_DESKTOP_JJ_TOOLS === "1";
// pi's Radius gateway (`DEFAULT_RADIUS_GATEWAY`), which pi does not export.
const RADIUS_GATEWAY = "https://radius.pi.dev";
const LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"];
const RESOURCE_SETTINGS = [
	"packages",
	"extensions",
	"skills",
	"prompts",
	"defaultProvider",
	"defaultModel",
	"defaultThinkingLevel",
	"enabledModels",
];

type Request = Record<string, unknown> & { id: string; type: string };
type Handler = (request: Request, ctx: ExtensionContext) => Promise<unknown> | unknown;
type CommandHandler = (request: Request, ctx: ExtensionCommandContext) => Promise<unknown>;

/** Resolves a tool's path argument the way pi's file tools do. */
function absolute(path: string, cwd: string): string {
	const plain = path.startsWith("@") ? path.slice(1) : path;
	const home = plain === "~" || plain.startsWith("~/") ? homedir() + plain.slice(1) : plain;
	return resolve(cwd, home);
}

function connectTo(address: string): Socket {
	const port = /^tcp:(\d+)$/.exec(address)?.[1];
	return port ? connect({ host: "127.0.0.1", port: Number(port) }) : connect(address);
}

function text(value: unknown): string {
	if (typeof value === "string") return value;
	if (!Array.isArray(value)) return "";
	return value
		.filter((block) => block?.type === "text" && typeof block.text === "string")
		.map((block) => block.text)
		.join("");
}

function string(request: Request, key: string): string {
	const value = request[key];
	if (typeof value !== "string") throw new Error(`${key} must be a string`);
	return value;
}

function optionalString(request: Request, key: string): string | undefined {
	return request[key] === undefined ? undefined : string(request, key);
}

/**
 * Checks a bounded header before opening another session file: SessionManager.open can otherwise
 * create new data when given a nonexistent or malformed file.
 */
async function sessionFile(value: string): Promise<string> {
	if (!isAbsolute(value)) throw new Error("sessionPath must be absolute");
	const file = await realpath(value);
	if (!(await stat(file)).isFile()) throw new Error("Not a saved session file");
	const handle = await open(file, "r");
	try {
		const bytes = Buffer.alloc(64 * 1024);
		const { bytesRead } = await handle.read(bytes);
		const end = bytes.subarray(0, bytesRead).indexOf(10);
		if (end < 0) throw new Error("Missing or oversized session header");
		const header = JSON.parse(bytes.subarray(0, end).toString("utf8").replace(/^﻿/, ""));
		if (header.type !== "session" || typeof header.id !== "string" || typeof header.cwd !== "string" || !isAbsolute(header.cwd)) {
			throw new Error("Not a saved session file");
		}
		return file;
	} finally {
		await handle.close();
	}
}

/** pi's trust store keys: canonical absolute folders. */
function trustPath(cwd: string): string {
	const path = resolve(cwd);
	try {
		return realpathSync(path);
	} catch {
		return path;
	}
}

/** A settings writer of its own; pi rereads the file and writes back only fields it changed. */
function settingsFor(ctx: ExtensionContext): SettingsManager {
	return SettingsManager.create(ctx.cwd, getAgentDir(), { projectTrusted: ctx.isProjectTrusted() });
}

function pickSettings(settings: object): Record<string, unknown> {
	return Object.fromEntries(Object.entries(settings).filter(([key]) => RESOURCE_SETTINGS.includes(key)));
}

/** One request per connection to the desktop's bridge; "" on any failure. */
function ask(request: object, limitMs: number, signal: AbortSignal | undefined): Promise<string> {
	return new Promise((done) => {
		if (!bridgeAddress) {
			done("");
			return;
		}
		let reply = "";
		const client = connectTo(bridgeAddress);
		const timer = setTimeout(() => finish(""), limitMs);
		const onAbort = () => finish("");
		function finish(answer: string) {
			clearTimeout(timer);
			signal?.removeEventListener("abort", onAbort);
			client.destroy();
			done(answer);
		}
		signal?.addEventListener("abort", onAbort, { once: true });
		client.setEncoding("utf8");
		client.on("connect", () =>
			client.write(`${JSON.stringify(bridgeToken ? { ...request, token: bridgeToken } : request)}\n`),
		);
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

export default function (pi: ExtensionAPI) {
	if (channelAddress) desktopChannel(pi, channelAddress);
	if (!bridgeAddress) return;

	pi.on("tool_call", async (event, ctx) => {
		if (event.toolName !== "bash") return;
		await ask({ op: "snapshot_before", toolCallId: event.toolCallId }, SNAPSHOT_LIMIT_MS, ctx.signal);
	});

	pi.on("tool_result", async (event, ctx) => {
		if (event.toolName === "bash") {
			await ask({ op: "snapshot_after", toolCallId: event.toolCallId }, SNAPSHOT_LIMIT_MS, ctx.signal);
			return;
		}
		if ((event.toolName !== "edit" && event.toolName !== "write") || event.isError) return;
		const path = event.input.path;
		if (typeof path !== "string") return;
		const answer = await ask({ op: "file", path: absolute(path, ctx.cwd) }, FILE_LIMIT_MS, ctx.signal);
		if (!answer) return;
		return { content: [...event.content, { type: "text", text: answer.trimEnd() }] };
	});

	if (jjTools) registerJjTools(pi);

	pi.on("agent_before_settle", async (event, ctx) => {
		if (event.outcome === "aborted") return;
		const answer = await ask({ op: "run_end" }, RUN_LIMIT_MS, ctx.signal);
		if (!answer) return;
		return {
			entries: [
				...event.entries,
				{ type: "custom_message", customType: "pi-desktop-lsp", content: answer.trimEnd(), display: true },
			],
		};
	});
}

/** Read-only jj history, answered by the desktop's built-in jj. */
function registerJjTools(pi: ExtensionAPI) {
	const answer = async (request: object, signal: AbortSignal | undefined) => {
		const reply = await ask(request, JJ_LIMIT_MS, signal);
		return {
			content: [{ type: "text" as const, text: reply || "jj is not available for this project right now." }],
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

function desktopChannel(pi: ExtensionAPI, address: string) {
	let ctx: ExtensionContext | undefined;
	let socket: Socket | undefined;
	let closing = false;
	let inFlight = 0;
	// Mutations run one at a time, in arrival order; reads answer at once.
	let queue: Promise<unknown> = Promise.resolve();
	const commands = new Map<string, (ctx: ExtensionCommandContext) => Promise<void>>();

	const send = (record: object) => {
		if (socket && !socket.destroyed) socket.write(`${JSON.stringify(record)}\n`);
	};
	const closeWhenDone = () => {
		if (closing && inFlight === 0) socket?.end();
	};
	// This extension's own command, as pi lists it. Its file is not a user's extension.
	const ownCommand = () =>
		pi.getCommands().find((command) => command.source === "extension" && command.description === COMMAND_DESCRIPTION);

	/**
	 * Runs `handler` as this extension's command, which gets pi's session-control context. pi runs
	 * an extension command at once, even during a run, and never sends it to the model.
	 */
	const asCommand =
		(handler: CommandHandler): Handler =>
		(request) =>
			new Promise((resolveResult, reject) => {
				// Without the command, pi would send the text to the model as a prompt.
				const name = ownCommand()?.name;
				if (!name) {
					reject(new Error("Pi Desktop's session command is not loaded"));
					return;
				}
				const unstarted = setTimeout(() => {
					commands.delete(request.id);
					reject(new Error("Pi did not run Pi Desktop's session command"));
				}, COMMAND_START_LIMIT_MS);
				commands.set(request.id, (commandCtx) => {
					clearTimeout(unstarted);
					return handler(request, commandCtx).then(resolveResult, reject);
				});
				pi.sendUserMessage(`/${name} ${request.id}`, { expandPromptTemplates: true });
			});

	const idle = (current: ExtensionContext) => {
		if (!current.isIdle() || current.hasPendingMessages()) throw new Error("Wait for the current run and queued work to finish");
	};

	const packages = (request: Request, current: ExtensionContext) => {
		const manager = new DefaultPackageManager({ cwd: current.cwd, agentDir: getAgentDir(), settingsManager: settingsFor(current) });
		manager.setProgressCallback((progress: unknown) => send({ type: "package_progress", id: request.id, progress }));
		return manager;
	};

	const reads: Record<string, Handler> = {
		get_backend_info: () => ({
			backend: "pi-desktop-extension",
			version: EXTENSION_VERSION,
			protocolVersion: PROTOCOL_VERSION,
			piVersion: VERSION,
			nodeVersion: process.versions.node,
			bunVersion: process.versions.bun,
			commands: [...Object.keys(reads), ...Object.keys(mutations)],
			features: ["fork_cwd"],
			limitations: ["native auth mutations use terminal handoff"],
		}),
		get_active_tools: () => {
			const tools = new Map(pi.getAllTools().map((tool) => [tool.name, tool]));
			return {
				activeTools: pi.getActiveTools().map((name) => ({
					name,
					description: tools.get(name)?.description,
					sourceInfo: tools.get(name)?.sourceInfo,
				})),
			};
		},
		// Data-only entries pi keeps out of the model's context, such as the jj change each desktop
		// turn made. Written on the current branch; read from all branches.
		get_custom_entries: (request, current) => ({
			entries: current.sessionManager
				.getEntries()
				.filter((entry) => entry.type === "custom" && entry.customType === string(request, "customType")),
		}),
		get_settings: (_request, current) => {
			// pi's own getters, so unset values report pi's defaults.
			const s = SettingsManager.inMemory(pi.getSettings());
			return {
				defaultProvider: s.getDefaultProvider(),
				defaultModel: s.getDefaultModel(),
				defaultThinkingLevel: s.getDefaultThinkingLevel(),
				modelThinkingLevels: s.getAllModelThinkingLevels(),
				enabledModels: s.getEnabledModels(),
				scopedModels: current.scopedModels.map(({ model }) => `${model.provider}/${model.id}`),
				steeringMode: s.getSteeringMode(),
				followUpMode: s.getFollowUpMode(),
				autoCompaction: s.getCompactionEnabled(),
				autoRetry: s.getRetryEnabled(),
				cacheWarming: s.getCacheWarmingMode(),
				defaultProjectTrust: s.getDefaultProjectTrust(),
				hideThinkingBlock: s.getHideThinkingBlock(),
				showCacheMissNotices: s.getShowCacheMissNotices(),
			};
		},
		list_sessions: async (request, current) => {
			const dir = current.sessionManager.getSessionDir();
			// pi's default keeps one folder per project under the agent folder; a session folder
			// of one's own (`--session-dir`, PI_SESSION_DIR or `sessionDir`) holds every project's.
			const own = resolve(dirname(dir)) !== resolve(getAgentDir(), "sessions");
			const sessions =
				request.scope === "all"
					? await (own ? SessionManager.listAll(dir) : SessionManager.listAll())
					: await SessionManager.list(current.cwd, dir);
			return {
				sessions: sessions.map(({ allMessagesText: _text, created, modified, ...info }) => ({
					...info,
					...(Number.isNaN(created.getTime()) ? {} : { created: created.toISOString() }),
					modified: modified.toISOString(),
				})),
			};
		},
		get_project_trust: asCommand(async (_request, current) => {
			// pi reports no list of loaded extensions; these are the ones that added a tool or command.
			const own = ownCommand()?.sourceInfo.path;
			const extensions = new Map<string, { path: string; sourceInfo: object; status: string; commands: string[] }>();
			const add = (info: { path: string } | undefined, command?: string) => {
				if (!info || info.path === own || info.path.startsWith("builtin:")) return;
				const extension = extensions.get(info.path) ?? { path: info.path, sourceInfo: info, status: "loaded", commands: [] };
				if (command) extension.commands.push(`/${command}`);
				extensions.set(info.path, extension);
			};
			for (const tool of pi.getAllTools()) add(tool.sourceInfo);
			for (const command of pi.getCommands()) if (command.source === "extension") add(command.sourceInfo, command.name);
			const settings = settingsFor(current);
			return {
				cwd: current.cwd,
				trusted: current.isProjectTrusted(),
				hasProjectResources: hasTrustRequiringProjectResources(current.cwd),
				loadedExtensions: [...extensions.values()],
				savedDecision: new ProjectTrustStore(getAgentDir()).getEntry(current.cwd),
				contextFiles: (current.getSystemPromptOptions().contextFiles ?? []).map((file) => file.path),
				userSettings: pickSettings(settings.getGlobalSettings()),
				projectSettings: pickSettings(settings.getProjectSettings()),
			};
		}),
		get_auth_providers: (_request, current) => {
			const registry = current.modelRegistry;
			const providers: object[] = [];
			const ids = [...new Set(registry.getAll().map((model) => model.provider))];
			for (const id of ids) {
				const provider = registry.getProvider(id);
				if (!provider) continue;
				const auth = registry.getProviderAuthStatus(id);
				const model = registry.getAll().find((candidate) => candidate.provider === id);
				const status = auth.configured
					? { type: model && registry.isUsingOAuth(model) ? "oauth" : "api_key", source: auth.label ?? auth.source }
					: undefined;
				const name = registry.getProviderDisplayName(id);
				if (provider.auth?.oauth) {
					providers.push({ id, name, authType: "oauth", loginLabel: provider.auth.oauth.loginLabel, canLogin: true, status });
				}
				if (provider.auth?.apiKey) {
					providers.push({ id, name, authType: "api_key", canLogin: provider.auth.apiKey.login !== undefined, status });
				}
			}
			return { providers: providers.sort((a, b) => String((a as { name: string }).name).localeCompare(String((b as { name: string }).name))) };
		},
		list_packages: (request, current) => ({ packages: packages(request, current).listConfiguredPackages() }),
		check_package_updates: async (request, current) => ({ updates: await packages(request, current).checkForAvailableUpdates() }),
	};

	const mutations: Record<string, Handler> = {
		append_custom_entry: (request, current) => {
			const customType = string(request, "customType");
			// The desktop's own records only: extensions keep their state in custom entries too.
			if (!customType.startsWith("pi-desktop-")) throw new Error("customType must start with pi-desktop-");
			const data = request.data;
			if (!data || typeof data !== "object" || Array.isArray(data)) throw new Error("data must be an object");
			idle(current);
			pi.appendEntry(customType, data);
			return { entryId: current.sessionManager.getLeafId() };
		},
		set_label: (request, current) => {
			const target = string(request, "targetId");
			if (!current.sessionManager.getEntry(target)) throw new Error("Label target entry does not exist");
			pi.setLabel(target, optionalString(request, "label")?.trim() || undefined);
		},
		// A fork that works in another folder, such as a jj workspace with the files as they were at
		// this entry: a new session file whose header names that folder. This process keeps its
		// session; the desktop opens the new one.
		fork: async (request, current) => {
			idle(current);
			const cwd = await realpath(string(request, "cwd"));
			if (!(await stat(cwd)).isDirectory()) throw new Error("cwd must be a folder");
			const manager = current.sessionManager;
			const file = manager.getSessionFile();
			if (!file) throw new Error("This session is not saved yet");
			const entry = manager.getEntry(string(request, "entryId"));
			if (entry?.type !== "message" || entry.message.role !== "user") throw new Error("Fork needs a user message");
			const copy = SessionManager.open(file, manager.getSessionDir(), cwd);
			const sessionPath = entry.parentId ? copy.createBranchedSession(entry.parentId) : copy.newSession();
			return { text: text(entry.message.content), cancelled: false, sessionPath, sessionId: copy.getSessionId() };
		},
		set_session_name: async (request, current) => {
			const name = string(request, "name").trim();
			if (!name) throw new Error("Session name cannot be empty");
			const file = await sessionFile(string(request, "sessionPath"));
			const active = current.sessionManager.getSessionFile();
			if (active && file === (await realpath(active).catch(() => active))) pi.setSessionName(name);
			else SessionManager.open(file).appendSessionInfo(name);
		},
		set_model: async (request, current) => {
			const provider = string(request, "provider");
			const modelId = string(request, "modelId");
			const model = current.modelRegistry.getAvailable().find((m) => m.provider === provider && m.id === modelId);
			if (!model) throw new Error(`Model not found: ${provider}/${modelId}`);
			if (!(await pi.setModel(model))) throw new Error(`No API key for ${provider}/${modelId}`);
			if (request.persist === true) {
				// As pi's own persisting model switch: the default, and the model joins a non-empty cycle.
				const settings = settingsFor(current);
				settings.setDefaultModelAndProvider(provider, modelId);
				const enabled = settings.getEnabledModels();
				const reference = `${provider}/${modelId}`;
				if (current.scopedModels.length > 0 && enabled?.length && !enabled.some((p) => p.toLowerCase() === reference.toLowerCase())) {
					settings.setEnabledModels([...enabled, reference]);
				}
				await settings.flush();
			}
			return model;
		},
		set_model_thinking_level: async (request, current) => {
			const provider = string(request, "provider");
			const modelId = string(request, "modelId");
			const model = current.modelRegistry.getAvailable().find((m) => m.provider === provider && m.id === modelId);
			if (!model) throw new Error(`Model not found: ${provider}/${modelId}`);
			const level = optionalString(request, "level");
			const settings = settingsFor(current);
			const isCurrent = current.model?.provider === provider && current.model.id === modelId;
			if (level === undefined || level === "") {
				settings.removeModelThinkingLevel(provider, modelId);
				if (isCurrent) pi.setThinkingLevel(settings.getDefaultThinkingLevel() ?? "medium");
			} else {
				if (!LEVELS.includes(level) || !getSupportedThinkingLevels(model).includes(level)) {
					throw new Error("Thinking level not supported by this model");
				}
				settings.setModelThinkingLevel(provider, modelId, level as never);
				if (isCurrent) pi.setThinkingLevel(level as never);
			}
			await settings.flush();
		},
		// The saved cycle applies to new sessions; this one keeps its scope.
		set_scoped_models: async (request, current) => {
			const patterns = request.patterns;
			if (patterns !== null && (!Array.isArray(patterns) || !patterns.every((p) => typeof p === "string"))) {
				throw new Error("patterns must be null or an array of strings");
			}
			const available = current.modelRegistry.getAvailable();
			const all =
				patterns !== null &&
				available.every((model) => patterns.some((p) => String(p).toLowerCase() === `${model.provider}/${model.id}`.toLowerCase()));
			const settings = settingsFor(current);
			settings.setEnabledModels(patterns === null || all ? undefined : [...(patterns as string[])]);
			await settings.flush();
			return { scopedModels: current.scopedModels.map(({ model }) => `${model.provider}/${model.id}`), enabledModels: settings.getEnabledModels() };
		},
		set_project_trust: (request, current) => {
			const path = trustPath(current.cwd);
			const parent = dirname(path) === path ? undefined : dirname(path);
			const choice = request.choice;
			// As pi's trust options: this folder, the parent folder (clearing this folder's own
			// decision), or distrust.
			const option =
				choice === "trust"
					? { trusted: true, savedPath: path, updates: [{ path, decision: true }] }
					: choice === "trust-parent" && parent
						? { trusted: true, savedPath: parent, updates: [{ path: parent, decision: true }, { path, decision: null }] }
						: choice === "distrust"
							? { trusted: false, savedPath: path, updates: [{ path, decision: false }] }
							: undefined;
			if (!option) throw new Error(choice === "trust-parent" ? "No parent folder to trust" : "Invalid choice");
			new ProjectTrustStore(getAgentDir()).setMany(option.updates);
			return { trusted: option.trusted, savedPath: option.savedPath };
		},
		install_package: async (request, current) => {
			const local = request.local === true;
			if (local && !current.isProjectTrusted()) {
				throw new Error("Project is not trusted. Save a trust decision and restart to change project packages.");
			}
			await packages(request, current).installAndPersist(string(request, "source"), { local });
		},
		remove_package: async (request, current) => {
			const local = request.local === true;
			if (local && !current.isProjectTrusted()) {
				throw new Error("Project is not trusted. Save a trust decision and restart to change project packages.");
			}
			if (!(await packages(request, current).removeAndPersist(string(request, "source"), { local }))) {
				throw new Error("No matching package found");
			}
		},
		update_packages: async (request, current) => {
			await packages(request, current).update(optionalString(request, "source"));
		},
		share: (_request, current) => shareToRadius(pi, current),
		navigate_tree: asCommand(async (request, current) => {
			idle(current);
			const targetId = string(request, "targetId");
			const target = current.sessionManager.getEntry(targetId);
			const result = await current.navigateTree(targetId, { summarize: request.summarize === true });
			// pi puts a user or custom message's text back in the editor: the leaf moves to its parent.
			const editorText =
				!result.cancelled && target?.type === "message" && target.message.role === "user"
					? text(target.message.content)
					: !result.cancelled && target?.type === "custom_message"
						? text(target.content)
						: undefined;
			return { cancelled: result.cancelled, editorText };
		}),
		reload: asCommand(async (_request, current) => {
			idle(current);
			await current.reload();
		}),
	};

	const handle = async (request: Request) => {
		const current = ctx;
		const reply = (body: object) => send({ type: "response", id: request.id, command: request.type, ...body });
		const handler = reads[request.type] ?? mutations[request.type];
		inFlight += 1;
		try {
			if (!current) throw new Error("The session is not ready");
			if (!handler) throw new Error(`Unknown command: ${request.type}`);
			const run = () => Promise.resolve(handler(request, current));
			const result = reads[request.type] ? run() : (queue = queue.then(run, run));
			const data = await result;
			reply(data === undefined ? { success: true } : { success: true, data });
		} catch (error) {
			reply({ success: false, error: error instanceof Error ? error.message : String(error) });
		} finally {
			inFlight -= 1;
			closeWhenDone();
		}
	};

	pi.registerCommand(COMMAND, {
		description: COMMAND_DESCRIPTION,
		handler: async (args, commandCtx) => {
			const run = commands.get(args.trim());
			commands.delete(args.trim());
			await run?.(commandCtx);
		},
	});

	pi.on("session_start", async (_event, current) => {
		ctx = current;
		closing = false;
		if (socket) return;
		const client = connectTo(address);
		socket = client;
		client.setEncoding("utf8");
		client.on("connect", () =>
			send({ type: "hello", token: channelToken, protocolVersion: PROTOCOL_VERSION, extension: EXTENSION_VERSION, piVersion: VERSION }),
		);
		let buffer = "";
		client.on("data", (chunk: string) => {
			buffer += chunk;
			for (let end = buffer.indexOf("\n"); end >= 0; end = buffer.indexOf("\n")) {
				const line = buffer.slice(0, end);
				buffer = buffer.slice(end + 1);
				let request: Request;
				try {
					request = JSON.parse(line);
				} catch {
					continue;
				}
				if (typeof request?.id === "string" && typeof request.type === "string") void handle(request);
			}
		});
		client.on("error", () => client.destroy());
		client.on("close", () => {
			if (socket === client) socket = undefined;
		});
	});

	// pi replaces this instance after the session ends. Requests still running reply first.
	pi.on("session_shutdown", async () => {
		closing = true;
		closeWhenDone();
	});
}

/**
 * Uploads the current branch to Radius as pi's `/share` does, when Radius is set up; otherwise
 * `{destination: null}`, and the desktop falls back to a private gist of pi's HTML export.
 */
async function shareToRadius(pi: ExtensionAPI, ctx: ExtensionContext): Promise<object> {
	const registry = ctx.modelRegistry;
	const token = registry.getProvider("radius") ? await registry.getApiKeyForProvider("radius") : undefined;
	if (!token) return { destination: null };
	const manager = ctx.sessionManager;
	const timestamp = new Date().toISOString();
	const records: object[] = [
		{ type: "session", version: CURRENT_SESSION_VERSION, id: manager.getSessionId(), timestamp, cwd: manager.getCwd() },
	];
	let parentId: string | null = null;
	for (const entry of manager.getBranch()) {
		records.push({ ...entry, parentId });
		parentId = entry.id;
	}
	// The viewer shows the system prompt and the tools the model had.
	const active = new Set(pi.getActiveTools());
	records.push({
		type: "custom",
		customType: "pi.share",
		id: randomUUID().slice(0, 8),
		parentId,
		timestamp,
		data: {
			systemPrompt: ctx.getSystemPrompt(),
			tools: pi
				.getAllTools()
				.filter((tool) => active.has(tool.name))
				.map((tool) => ({ name: tool.name, description: tool.description, parameters: tool.parameters })),
		},
	});
	const body = `${records.map((record) => JSON.stringify(record)).join("\n")}\n`;
	const endpoint = new URL("/v1/artifacts", RADIUS_GATEWAY);
	endpoint.searchParams.set("visibility", "organization");
	endpoint.searchParams.set("title", "Pi session");
	const response = await fetch(endpoint, {
		method: "POST",
		signal: AbortSignal.timeout(SHARE_LIMIT_MS),
		headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/x-ndjson" },
		body,
	});
	const data = (await response.json().catch(() => null)) as { artifact?: { canonical_url?: unknown } } | null;
	// Never echo a remote response body; it might contain credential material.
	if (!response.ok || !data?.artifact) throw new Error(`Radius share upload failed (HTTP ${response.status})`);
	const url = data.artifact.canonical_url;
	if (typeof url !== "string" || !/^https?:\/\//.test(url)) throw new Error("Share service did not return a URL");
	return { destination: "radius", url };
}
