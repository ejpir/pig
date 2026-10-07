/**
 * The `subagent` tool: Pi hands a task to another agent, which works in a
 * conversation of its own and answers back. The same parameters as stock Pi's
 * subagent extension (one task, `tasks` side by side, or a `chain` passing
 * `{previous}` along) and the same agent files, but each subagent is a durable
 * conversation owned by the call: stopping the call stops it, and after a
 * restart the call finds it again instead of starting it twice.
 *
 * Progress is the call's `details`: one short summary per subagent, never a
 * transcript, since every commit sends the parent's view.
 */
import type { Context } from "@earendil-works/chord";
import { Type, type Message } from "@earendil-works/pi-ai";
import type { Models } from "@earendil-works/pi-ai/models";
import {
  configure, defineDoc, defineExtension, defineTool,
  type ConversationId, type ConversationView, type Extension, type Harness, type ToolExecutionApi,
} from "@earendil-works/pi-durable";
import type { AgentDefinition, Resources } from "./commands.ts";

const MAX_PARALLEL = 8;
const CONCURRENCY = 4;
const MAX_OUTPUT = 4000;
/** How often running subagents report, at most: each report is a commit. */
const REPORT_MS = 1000;
/** Tools a subagent may be given; agent files may name stock Pi's others, which durable doesn't have. */
const TOOLS = ["read", "write", "edit", "bash"];

export type SubagentStatus = "waiting" | "running" | "done" | "failed" | "stopped";

export type SubagentResult = {
  index: number;
  agent: string;
  task: string;
  status: SubagentStatus;
  conversationId?: string;
  /** "provider/modelId" it runs on. */
  model?: string;
  /** Why it doesn't run on the model its file names. */
  modelNote?: string;
  /** What it is doing now, or how it ended. */
  now?: string;
  startedAt?: number;
  endedAt?: number;
  cost?: number;
  /** Its final answer, shortened. */
  output?: string;
  /** Commands a restart cut off, which weren't repeated by themselves. */
  interrupted?: string[];
};

export type SubagentDetails = {
  version: 1;
  mode: "single" | "parallel" | "chain";
  /** This call carried on after a restart. */
  resumed?: boolean;
  results: SubagentResult[];
};

/** Which conversation runs each task of a call, and when it ran: survives a restart of the call. */
const Children = defineDoc<{ started: boolean; children: Record<string, { id: number; startedAt: number; endedAt?: number }> }>({
  kind: "app.subagents", version: 1, scope: "task",
  initial: () => ({ started: false, children: {} }),
});

const Task = Type.Object({
  agent: Type.String({ description: "Name of the agent to invoke" }),
  task: Type.String({ description: "Task to delegate to the agent" }),
  cwd: Type.Optional(Type.String({ description: "Working directory for the agent" })),
});
const ChainTask = Type.Object({
  agent: Type.String({ description: "Name of the agent to invoke" }),
  task: Type.String({ description: "Task with optional {previous} placeholder for prior output" }),
  cwd: Type.Optional(Type.String({ description: "Working directory for the agent" })),
});
const Parameters = Type.Object({
  agent: Type.Optional(Type.String({ description: "Name of the agent to invoke (for single mode)" })),
  task: Type.Optional(Type.String({ description: "Task to delegate (for single mode)" })),
  tasks: Type.Optional(Type.Array(Task, { description: "Array of {agent, task} for parallel execution" })),
  chain: Type.Optional(Type.Array(ChainTask, { description: "Array of {agent, task} for sequential execution" })),
  agentScope: Type.Optional(Type.String({ description: 'Which agents to use: "user", "project" or "both". Default "both"; a project\'s own agents only once Pi trusts the project.' })),
  cwd: Type.Optional(Type.String({ description: "Working directory for the agent (single mode)" })),
});

type Item = { agent: string; task: string; cwd?: string };

/**
 * `harness` is read when a call runs, after the Harness opened. `resources`
 * gives the agents, read again for each call.
 */
export function subagentExtension(models: Models, harness: () => Harness, resources: () => Resources): Extension {
  const extension: Extension = defineExtension({
    name: "subagents",
    tools: [defineTool({
      name: "subagent",
      description: [
        "Delegate tasks to specialized subagents, each with a context of its own.",
        "Modes: single (agent + task), parallel (tasks array), chain (sequential, with a {previous} placeholder for the prior step's output).",
        "Available agents are listed in the system prompt.",
      ].join(" "),
      parameters: Parameters,
      // A rerun after a restart finds the same children and their submissions.
      replay: "safe",
      execute: async (args, api, context) => {
        const agents = resources().agents.filter((agent) =>
          args.agentScope === "user" ? agent.scope === "user" : args.agentScope === "project" ? agent.scope === "project" : true);
        const items: Item[] = args.chain?.length ? args.chain : args.tasks?.length ? args.tasks
          : args.agent && args.task ? [{ agent: args.agent, task: args.task, cwd: args.cwd }] : [];
        const modes = Number(!!args.chain?.length) + Number(!!args.tasks?.length) + Number(!!(args.agent && args.task));
        const available = agents.map((agent) => agent.name).join(", ") || "none";
        if (modes !== 1) return failure(`Provide exactly one mode: agent and task, tasks, or chain. Available agents: ${available}`);
        if (items.length > MAX_PARALLEL) return failure(`At most ${MAX_PARALLEL} tasks at once`);
        const unknown = items.find((item) => !agents.some((agent) => agent.name === item.agent));
        if (unknown) return failure(`Unknown agent "${unknown.agent}". Available agents: ${available}`);
        const mode: SubagentDetails["mode"] = args.chain?.length ? "chain" : args.tasks?.length ? "parallel" : "single";
        const run = new Call(api, context, harness(), models, extension, mode, items, agents);
        return run.execute();
      },
    })],
    sections: [{
      key: "subagents",
      tag: false,
      render: () => {
        const agents = resources().agents;
        if (!agents.length) return undefined;
        return ["<available_agents>", ...agents.map((agent) =>
          `- ${agent.name}: ${agent.description}${agent.model ? ` (model: ${agent.model})` : ""}`), "</available_agents>",
        "Use the subagent tool to hand one of these agents a self-contained task."].join("\n");
      },
    }],
  });
  return extension;
}

function failure(text: string) {
  return { content: [{ type: "text" as const, text }], isError: true };
}

/** One call of the tool: its subagents, their progress, and the answer. */
class Call {
  readonly results: SubagentResult[];
  #reported = 0;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #resumed = false;

  constructor(
    readonly api: ToolExecutionApi, readonly context: Context, readonly harness: Harness, readonly models: Models,
    readonly extension: Extension, readonly mode: SubagentDetails["mode"], readonly items: Item[],
    readonly agents: readonly AgentDefinition[],
  ) {
    this.results = items.map((item, index) => ({ index, agent: item.agent, task: item.task, status: "waiting" }));
  }

  async execute() {
    this.#resumed = await this.api.commit(async (tx) => {
      const doc = await tx.doc(Children, this.api.taskId);
      const resumed = doc.started;
      doc.started = true;
      return resumed;
    }, this.context);
    await this.report(true);
    try {
      if (this.mode === "chain") {
        let previous = "";
        for (const [index, item] of this.items.entries()) {
          const result = await this.run(index, { ...item, task: item.task.replaceAll("{previous}", previous) });
          if (result.status !== "done") break;
          previous = result.output ?? "";
        }
      } else {
        let next = 0;
        const worker = async () => {
          while (next < this.items.length) {
            const index = next++;
            await this.run(index, this.items[index]);
          }
        };
        await Promise.all(Array.from({ length: Math.min(CONCURRENCY, this.items.length) }, worker));
      }
    } finally {
      clearTimeout(this.#timer);
    }
    await this.report(true);
    return this.answer();
  }

  /** Runs one task to its end in its own conversation; found again after a restart. */
  async run(index: number, item: Item): Promise<SubagentResult> {
    const result = this.results[index];
    result.task = item.task;
    const definition = this.agents.find((agent) => agent.name === item.agent)!;
    const { id: number, startedAt, endedAt } = await this.child(index, definition, item);
    const id = number as unknown as ConversationId;
    result.conversationId = String(number);
    result.startedAt = startedAt;
    result.status = "running";
    const conversation = (await this.harness.conversation(id, this.context))!;
    const view = await conversation.viewState(this.context);
    const follow = () => { this.observe(result, view.value); this.report(false).catch(() => {}); };
    const unsubscribe = view.subscribe(follow);
    try {
      follow();
      if (endedAt === undefined) {
        const handle = (await this.api.conversation(id, this.context))!;
        // The request ID makes a rerun find the same submission instead of sending the task twice.
        const submission = await handle.submit({ type: "input", content: item.task, requestId: `subagent-${index}` }, this.context);
        await submission.wait(this.context);
        await handle.waitForIdle(this.context);
        await this.api.commit(async (tx) => {
          const child = (await tx.doc(Children, this.api.taskId)).children[index];
          child.endedAt ??= Date.now();
        }, this.context);
      }
      this.observe(result, view.value);
      result.endedAt = endedAt ?? Date.now();
      await this.report(true);
      return result;
    } finally {
      unsubscribe();
      view.dispose();
    }
  }

  /** The conversation that runs task `index`: created once, configured from its agent file. */
  async child(index: number, definition: AgentDefinition, item: Item) {
    const parent = await this.api.agent(this.context);
    const { model, note } = this.model(definition, parent.model);
    if (note) this.results[index].modelNote = note;
    this.results[index].model = model ? `${model.provider}/${model.modelId}` : undefined;
    const tools = definition.tools
      ? parent.tools.filter((tool) => definition.tools!.includes(tool.name) && TOOLS.includes(tool.name))
      : undefined;
    return this.api.commit(async (tx) => {
      const doc = await tx.doc(Children, this.api.taskId);
      const existing = doc.children[index];
      if (existing) return { ...existing };
      // A copy of this conversation's agent: cwd, thinking level, extensions.
      const created = await tx.createConversation({ ownership: { kind: "task", taskId: this.api.taskId } });
      await configure(tx, created.id, {
        ...(model ? { model } : {}),
        ...(tools ? { tools } : {}),
        ...(item.cwd ? { cwd: item.cwd } : {}),
        // No subagents of its own.
        extensions: { remove: [this.extension] },
        instructions: [
          `You are the ${definition.name} subagent. Another agent handed you this task; your final answer goes back to it, so make it complete and concise.`,
          definition.prompt.trim(),
        ].filter(Boolean).join("\n\n"),
      });
      const child: { id: number; startedAt: number; endedAt?: number } = { id: Number(created.id), startedAt: Date.now() };
      doc.children[index] = child;
      return child;
    }, this.context);
  }

  /** The agent file's model when this computer has it, else the session's. */
  model(definition: AgentDefinition, fallback: { provider: string; modelId: string } | undefined) {
    if (!definition.model) return { model: fallback };
    const [provider, id] = definition.model.includes("/") ? definition.model.split("/", 2) : [undefined, definition.model];
    const found = provider
      ? this.models.getModel(provider, id)
      : (fallback && this.models.getModel(fallback.provider, id)) ?? this.models.getProviders()
        .map((candidate) => this.models.getModel(candidate.id, id)).find(Boolean);
    if (found) return { model: { provider: found.provider, modelId: found.id } };
    const instead = fallback ? `${fallback.provider}/${fallback.modelId}` : "the session's model";
    return { model: fallback, note: `${definition.model} isn't set up on this computer; using ${instead}` };
  }

  /** Reads a subagent's state, latest step, cost and answer from its view. */
  observe(result: SubagentResult, view: ConversationView) {
    const messages = transcript(view);
    const live = view.docs["pi.live"] as { run?: unknown; generation?: unknown; tools?: { callId: string; name: string; status: string }[] } | undefined;
    result.cost = Object.values((view.docs["pi.usage"] ?? {}) as Record<string, Record<string, { cost?: { total?: number } }>>)
      .flatMap((bucket) => Object.values(bucket)).reduce((sum, usage) => sum + (usage.cost?.total ?? 0), 0);
    const calls = new Map<string, { name: string; arguments: Record<string, unknown> }>();
    for (const message of messages) {
      if (message.role !== "assistant") continue;
      for (const block of message.content) if (block.type === "toolCall") calls.set(block.id, block);
    }
    // A restart cut these off; pi-durable marks each result instead of running it again.
    result.interrupted = view.entries.flatMap((entry) => {
      const marked = (entry.data as { diagnostics?: { code?: string }[] } | undefined)?.diagnostics
        ?.some((diagnostic) => diagnostic.code === "interrupted");
      const message = entry.model?.[0];
      if (!marked || message?.role !== "toolResult") return [];
      const call = calls.get(message.toolCallId);
      return [call ? describe(call.name, call.arguments, false) : message.toolName];
    });
    if (!result.interrupted.length) delete result.interrupted;
    const running = live?.tools?.find((slot) => slot.status !== "done");
    if (live?.run) {
      const call = running && calls.get(running.callId);
      result.now = call ? describe(call.name, call.arguments, true) : running ? running.name : live.generation ? "Thinking" : "Working";
      return;
    }
    const last = [...messages].reverse().find((message) => message.role === "assistant");
    if (!last || result.status === "waiting") return;
    const output = last.content.map((block) => block.type === "text" ? block.text : "").join("").trim();
    result.output = output.length > MAX_OUTPUT ? `${output.slice(0, MAX_OUTPUT)}…` : output;
    // A run that ended at its tools, with no answer after them, was stopped.
    const unanswered = messages.at(-1)?.role !== "assistant" || last.stopReason === "toolUse";
    result.status = last.stopReason === "error" ? "failed" : last.stopReason === "aborted" || unanswered ? "stopped" : "done";
    result.now = result.status === "done" ? firstLine(output) : last.errorMessage ?? (result.status === "stopped" ? "Stopped" : "Failed");
  }

  /** Publishes progress, at most once a second unless `now`. */
  async report(now: boolean) {
    const due = this.#reported + REPORT_MS - Date.now();
    if (!now && due > 0) {
      this.#timer ??= setTimeout(() => { this.#timer = undefined; this.report(true).catch(() => {}); }, due);
      return;
    }
    this.#reported = Date.now();
    const details: SubagentDetails = { version: 1, mode: this.mode, ...(this.#resumed ? { resumed: true } : {}), results: this.results };
    await this.api.details(JSON.parse(JSON.stringify(details)), this.context);
  }

  answer() {
    const done = this.results.filter((result) => result.status === "done");
    const say = (result: SubagentResult) => result.status === "done" ? result.output || "(no answer)"
      : `(${result.status}${result.now ? `: ${result.now}` : ""})`;
    const text = this.mode === "parallel"
      ? this.results.map((result) => `## ${result.agent}: ${firstLine(result.task)}\n\n${say(result)}`).join("\n\n")
      : this.mode === "chain"
        ? [...this.results].reverse().find((result) => result.status !== "waiting")?.status === "done"
          ? say(this.results.at(-1)!)
          : this.results.filter((result) => result.status !== "waiting").map((result, step) => `Step ${step + 1} (${result.agent}): ${say(result)}`).join("\n\n")
        : say(this.results[0]);
    const failed = this.mode === "parallel" ? done.length === 0 : done.length !== this.results.length;
    return { content: [{ type: "text" as const, text }], isError: failed };
  }
}

/** A conversation's messages, in order, with the reply still streaming. */
export function transcript(view: ConversationView): Message[] {
  const messages = view.entries.flatMap((entry) => ((entry as { model?: Message[] }).model ?? []))
    .filter((message) => (message.role as string) !== "system");
  const streaming = (view.docs["pi.live"] as { generation?: { message?: Message } } | undefined)?.generation?.message;
  return streaming ? [...messages, streaming] : messages;
}

/** "Reading google-gemini.ts", "Running pnpm test". */
function describe(name: string, args: Record<string, unknown>, live: boolean): string {
  const path = typeof args.path === "string" ? args.path.split("/").at(-1)! : "";
  const command = typeof args.command === "string" ? firstLine(args.command) : "";
  switch (name) {
    case "read": return live ? `Reading ${path}` : `read ${path}`;
    case "write": case "edit": return live ? `Editing ${path}` : `${name} ${path}`;
    case "bash": return live ? `Running ${command}` : command;
    default: return name;
  }
}

function firstLine(text: string): string {
  const line = text.trim().split("\n")[0] ?? "";
  return line.length > 120 ? `${line.slice(0, 120)}…` : line;
}
