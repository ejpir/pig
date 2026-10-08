/**
 * The `subagent` tool: Pi hands tasks to other agents, which work in
 * conversations of their own while Pi carries on. The same parameters as stock
 * Pi's subagent extension (one task, `tasks` side by side, or a `chain` passing
 * `{previous}` along) and the same agent files.
 *
 * The call returns at once. A background crew task runs the subagents, each a
 * durable conversation it owns, and posts their answers back to Pi as one
 * message when the last finishes. Pi's own turns and Esc never reach the crew;
 * stopping the whole session does. After a restart the crew finds its
 * subagents again instead of starting them twice.
 *
 * Progress is a document of the session, one short summary per subagent and
 * never a transcript, since every change is sent to the apps.
 */
import type { Context } from "@earendil-works/chord";
import { Type, type Message } from "@earendil-works/pi-ai";
import type { Models } from "@earendil-works/pi-ai/models";
import {
  configure, defineDoc, defineExtension, defineTask, defineTool,
  type ConversationId, type ConversationView, type Extension, type Harness, type TaskId, type TaskRuntime, type Tx,
} from "@earendil-works/pi-durable";
import type { AgentDefinition, Resources } from "./commands.ts";

const MAX_TASKS = 200;
const CONCURRENCY = 8;
const MAX_OUTPUT = 4000;
/** An answer as the apps see it: they show its first line, and the whole one is the subagent's own. */
const MAX_SHOWN = 500;
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
  /** The crew carried on after a restart. */
  resumed?: boolean;
  /** The subagents work on after the call returned: this, not the call's end, says how each is doing. */
  background: true;
  results: SubagentResult[];
};

/** Which conversation runs each task of a crew, and when it ran: survives a restart. */
const Children = defineDoc<{ started: boolean; children: Record<string, { id: number; startedAt: number; endedAt?: number }> }>({
  kind: "app.subagents", version: 1, scope: "task",
  initial: () => ({ started: false, children: {} }),
});

/** Every call's subagents as they are now, by call ID: what the apps show once a call has returned. */
export const Calls = defineDoc<{ calls: Record<string, SubagentDetails> }>({
  kind: "app.subagent-calls", version: 1, scope: "conversation", history: "latest", fork: "initial",
  initial: () => ({ calls: {} }),
});

/** The marker of the message that brings a crew's answers back to Pi. */
export const REPORT = "subagent_report";

const AGENT = "A listed agent, or a name for a new one you describe in instructions, such as \"architecture\"";
const INSTRUCTIONS = "Who the agent is and how it works: its role, focus and output. Defines a new agent; added to a listed agent's own";
const AGENT_TOOLS = "Tools it may use, from read, write, edit and bash. Give an agent that reviews, researches or plans [\"read\", \"bash\"] so it can't change files. Default: a listed agent's, else all";
const Task = Type.Object({
  agent: Type.String({ description: AGENT }),
  task: Type.String({ description: "Task to delegate to the agent" }),
  instructions: Type.Optional(Type.String({ description: INSTRUCTIONS })),
  tools: Type.Optional(Type.Array(Type.String(), { description: AGENT_TOOLS })),
  cwd: Type.Optional(Type.String({ description: "Working directory for the agent" })),
});
const ChainTask = Type.Object({
  agent: Type.String({ description: AGENT }),
  task: Type.String({ description: "Task with optional {previous} placeholder for prior output" }),
  instructions: Type.Optional(Type.String({ description: INSTRUCTIONS })),
  tools: Type.Optional(Type.Array(Type.String(), { description: AGENT_TOOLS })),
  cwd: Type.Optional(Type.String({ description: "Working directory for the agent" })),
});
const Parameters = Type.Object({
  agent: Type.Optional(Type.String({ description: `${AGENT} (for single mode)` })),
  task: Type.Optional(Type.String({ description: "Task to delegate (for single mode)" })),
  instructions: Type.Optional(Type.String({ description: `${INSTRUCTIONS} (for single mode)` })),
  tools: Type.Optional(Type.Array(Type.String(), { description: `${AGENT_TOOLS} (for single mode)` })),
  tasks: Type.Optional(Type.Array(Task, { description: "Array of {agent, task} for parallel execution" })),
  chain: Type.Optional(Type.Array(ChainTask, { description: "Array of {agent, task} for sequential execution" })),
  agentScope: Type.Optional(Type.String({ description: 'Which agents to use: "user", "project" or "both". Default "both"; a project\'s own agents only once Pi trusts the project.' })),
  cwd: Type.Optional(Type.String({ description: "Working directory for the agent (single mode)" })),
});

type Item = { agent: string; task: string; instructions?: string; tools?: string[]; cwd?: string };

type CrewInput = { callId: string; mode: SubagentDetails["mode"]; items: (Item & { definition: Definition })[] };
type CrewState = { phase: "run" } | { phase: "report"; text: string };

/** What a subagent is: a listed agent, one the call defines, or both. */
type Definition = { name: string; prompt: string; model?: string; tools?: string[] };

/** Stock Pi's search tools are searches through bash in a durable session. */
function durableTools(tools: string[]): string[] {
  const mapped = tools.map((tool) => ["grep", "find", "ls"].includes(tool) ? "bash" : tool);
  return [...new Set(mapped.filter((tool) => TOOLS.includes(tool)))];
}

/** A listed agent named `item.agent`, with what the call adds; or the agent the call defines. */
export function definition(item: Item, agents: readonly AgentDefinition[]): Definition {
  const preset = agents.find((agent) => agent.name === item.agent);
  const tools = item.tools ?? preset?.tools;
  return {
    name: item.agent,
    prompt: [preset?.prompt.trim(), item.instructions?.trim()].filter(Boolean).join("\n\n"),
    ...(preset?.model ? { model: preset.model } : {}),
    ...(tools ? { tools: durableTools(tools) } : {}),
  };
}

/**
 * `harness` is read when a crew runs, after the Harness opened. `resources`
 * gives the agents, read again for each call.
 */
export function subagentExtension(models: Models, harness: () => Harness, resources: () => Resources): Extension {
  // Owned by Pi's conversation, and background: Pi's turns, idle waits and Esc
  // leave it alone; stopping the whole session reaches it.
  const Crew = defineTask<CrewInput, CrewState, null>({
    name: "app.subagent-crew",
    version: 1,
    initial: () => ({ phase: "run" }),
    phases: {
      run: async (crew, runtime, context) => {
        const text = await new CrewRun(runtime, context, harness(), models, extension, crew.input).execute();
        await runtime.commit(() => ({ status: "running", checkpoint: { phase: "report", text } }), context);
      },
      // Comes after Pi's current answer, or starts a turn when Pi is idle; the
      // request ID keeps a restart from reporting twice.
      report: async (crew, runtime, context) => {
        const pi = (await runtime.conversation(runtime.conversationId, context))!;
        await pi.submit({ type: "input", content: crew.state.checkpoint.text, whenBusy: "followUp", requestId: `subagent-report-${crew.id}` }, context);
        await runtime.commit(() => ({ status: "terminal", outcome: { status: "completed", result: null } }), context);
      },
    },
    // Its subagents were stopped first; it says so and reports nothing.
    abort: async (crew, runtime, context) => {
      await runtime.commit(async (tx) => {
        const details = (await tx.doc(Calls, runtime.conversationId)).calls[crew.input.callId];
        for (const result of details?.results ?? []) {
          if (result.status === "running" || result.status === "waiting") {
            result.status = "stopped";
            result.now = "Stopped";
            result.endedAt ??= Date.now();
          }
        }
        return { status: "terminal", outcome: { status: "aborted" } };
      }, context);
    },
  });
  const extension: Extension = defineExtension({
    name: "subagents",
    tasks: [Crew],
    tools: [defineTool({
      name: "subagent",
      description: [
        "Delegate tasks to subagents, each working in a context of its own. They work in the background:",
        "the call returns at once, and their answers come back to you in one message when the last one finishes.",
        "Don't do a task you handed off yourself, or check on it: that fills your context with what their answers will say.",
        "Modes: single (agent + task), parallel (tasks array), chain (sequential, with a {previous} placeholder for the prior step's output).",
        "Use a listed agent, or define one: give it a short name (\"architecture\", \"code-quality\"), instructions for its role and output, and tools.",
      ].join(" "),
      parameters: Parameters,
      // A rerun after a restart finds the crew it started.
      replay: "safe",
      execute: async (args, api, context) => {
        const agents = resources().agents.filter((agent) =>
          args.agentScope === "user" ? agent.scope === "user" : args.agentScope === "project" ? agent.scope === "project" : true);
        const items: Item[] = args.chain?.length ? args.chain : args.tasks?.length ? args.tasks
          : args.agent && args.task ? [{ agent: args.agent, task: args.task, instructions: args.instructions, tools: args.tools, cwd: args.cwd }] : [];
        const modes = Number(!!args.chain?.length) + Number(!!args.tasks?.length) + Number(!!(args.agent && args.task));
        if (modes !== 1) return failure("Provide exactly one mode: agent and task, tasks, or chain.");
        if (items.length > MAX_TASKS) return failure(`At most ${MAX_TASKS} tasks in one call`);
        const unnamed = items.find((item) => !/^[\w.-]{1,64}$/.test(item.agent));
        if (unnamed) return failure(`Name each agent in a word or two, such as "architecture": "${unnamed.agent}" isn't one.`);
        const toolless = items.find((item) => item.tools && !durableTools(item.tools).length);
        if (toolless) return failure(`The ${toolless.agent} agent needs at least one of: ${TOOLS.join(", ")}`);
        const mode: SubagentDetails["mode"] = args.chain?.length ? "chain" : args.tasks?.length ? "parallel" : "single";
        const details: SubagentDetails = { version: 1, mode, background: true, results: items.map((item, index) => ({ index, agent: item.agent, task: item.task, status: "waiting" })) };
        // One commit records the call and starts its crew, once.
        await api.commit(async (tx) => {
          const calls = (await tx.doc(Calls, api.conversationId)).calls;
          if (Object.hasOwn(calls, api.callId)) return;
          calls[api.callId] = JSON.parse(JSON.stringify(details));
          const input: CrewInput = JSON.parse(JSON.stringify({ callId: api.callId, mode, items: items.map((item) => ({ ...item, definition: definition(item, agents) })) }));
          await tx.createTask(Crew, input, { ownership: { kind: "conversation" }, background: true });
        }, context);
        const who = [...new Set(items.map((item) => item.agent))].join(", ");
        const text = `Started ${items.length === 1 ? `the ${who} subagent` : `${items.length} subagents (${who})`} in the background. `
          + "They are doing this work now, and their answers come back to you in one message when the last one finishes. "
          + "Don't do it yourself, check on them or prepare for their answers: end your turn now, unless the user asked for something they aren't doing.";
        return { content: [{ type: "text" as const, text }], details };
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
        "The subagent tool can hand one of these a self-contained task, or an agent you define in the call."].join("\n");
      },
    }],
  });
  return extension;
}

function failure(text: string) {
  return { content: [{ type: "text" as const, text }], isError: true };
}

/** One crew: its subagents, their progress, and what it reports. */
class CrewRun {
  readonly results: SubagentResult[];
  readonly mode: SubagentDetails["mode"];
  readonly items: CrewInput["items"];
  #reported = 0;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #resumed = false;

  constructor(
    readonly runtime: TaskRuntime<CrewInput, CrewState, null, object>, readonly context: Context, readonly harness: Harness,
    readonly models: Models, readonly extension: Extension, readonly input: CrewInput,
  ) {
    this.mode = input.mode;
    this.items = input.items;
    this.results = input.items.map((item, index) => ({ index, agent: item.agent, task: item.task, status: "waiting" }));
  }

  get taskId(): TaskId {
    return this.runtime.taskId;
  }

  /** A commit that changes no task state, returning what `change` does. */
  async write<T>(change: (tx: Tx) => Promise<T>): Promise<T> {
    let result: T;
    await this.runtime.commit(async (tx) => { result = await change(tx); return undefined; }, this.context);
    return result!;
  }

  async execute(): Promise<string> {
    this.#resumed = await this.write(async (tx) => {
      const doc = await tx.doc(Children, this.taskId);
      const resumed = doc.started;
      doc.started = true;
      return resumed;
    });
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
  async run(index: number, item: CrewInput["items"][number]): Promise<SubagentResult> {
    const result = this.results[index];
    result.task = item.task;
    const { id: number, startedAt, endedAt } = await this.child(index, item.definition, item);
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
        const handle = (await this.runtime.conversation(id, this.context))!;
        // The request ID makes a rerun find the same submission instead of sending the task twice.
        const submission = await handle.submit({ type: "input", content: item.task, requestId: `subagent-${index}` }, this.context);
        await submission.wait(this.context);
        await handle.waitForIdle(this.context);
        await this.write(async (tx) => {
          const child = (await tx.doc(Children, this.taskId)).children[index];
          child.endedAt ??= Date.now();
        });
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

  /** The conversation that runs task `index`: created once, configured from its definition. */
  async child(index: number, definition: Definition, item: Item) {
    const parent = await this.runtime.agent(this.context);
    const { model, note } = this.model(definition, parent.model);
    if (note) this.results[index].modelNote = note;
    this.results[index].model = model ? `${model.provider}/${model.modelId}` : undefined;
    const tools = definition.tools
      ? parent.tools.filter((tool) => definition.tools!.includes(tool.name) && TOOLS.includes(tool.name))
      : undefined;
    return this.write(async (tx) => {
      const doc = await tx.doc(Children, this.taskId);
      const existing = doc.children[index];
      if (existing) return { ...existing };
      // A copy of this conversation's agent: cwd, thinking level, extensions.
      const created = await tx.createConversation({ ownership: { kind: "task", taskId: this.taskId } });
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
    });
  }

  /** The agent file's model when this computer has it, else the session's. */
  model(definition: Definition, fallback: { provider: string; modelId: string } | undefined) {
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
      return [call ? describe(call.name, call.arguments, "named") : message.toolName];
    });
    if (!result.interrupted.length) delete result.interrupted;
    const running = live?.tools?.find((slot) => slot.status !== "done");
    if (live?.run) {
      const call = running && calls.get(running.callId);
      // Tools are quick and the card hears once a second: between them, say the last step.
      const previous = [...calls.values()].at(-1);
      result.now = call ? describe(call.name, call.arguments, "now")
        : running ? running.name
        : previous ? describe(previous.name, previous.arguments, "done")
        : live.generation ? "Thinking" : "Working";
      return;
    }
    const last = [...messages].reverse().find((message) => message.role === "assistant");
    if (!last || result.status === "waiting") return;
    const output = last.content.map((block) => block.type === "text" ? block.text : "").join("").trim();
    result.output = output.length > MAX_OUTPUT ? `${output.slice(0, MAX_OUTPUT)}…` : output;
    // A run that ended at its tools, with no answer after them, was stopped.
    const unanswered = messages.at(-1)?.role !== "assistant" || last.stopReason === "toolUse";
    result.status = last.stopReason === "error" ? "failed" : last.stopReason === "aborted" || unanswered ? "stopped" : "done";
    result.now = result.status === "done" ? gist(output) : last.errorMessage ?? (result.status === "stopped" ? "Stopped" : "Failed");
  }

  /** Publishes progress, at most once a second unless `now`. */
  async report(now: boolean) {
    const due = this.#reported + REPORT_MS - Date.now();
    if (!now && due > 0) {
      this.#timer ??= setTimeout(() => { this.#timer = undefined; this.report(true).catch(() => {}); }, due);
      return;
    }
    this.#reported = Date.now();
    const results = this.results.map((result) => result.output && result.output.length > MAX_SHOWN
      ? { ...result, output: `${result.output.slice(0, MAX_SHOWN)}…` } : result);
    const details: SubagentDetails = { version: 1, mode: this.mode, background: true, ...(this.#resumed ? { resumed: true } : {}), results };
    await this.write(async (tx) => {
      (await tx.doc(Calls, this.runtime.conversationId)).calls[this.input.callId] = JSON.parse(JSON.stringify(details));
    });
  }

  /** What Pi hears back. */
  answer(): string {
    const say = (result: SubagentResult) => result.status === "done" ? result.output || "(no answer)"
      : `(${result.status}${result.now ? `: ${result.now}` : ""})`;
    const text = this.mode === "parallel"
      ? this.results.map((result) => `## ${result.agent}: ${firstLine(result.task)}\n\n${say(result)}`).join("\n\n")
      : this.mode === "chain"
        ? [...this.results].reverse().find((result) => result.status !== "waiting")?.status === "done"
          ? say(this.results.at(-1)!)
          : this.results.filter((result) => result.status !== "waiting").map((result, step) => `Step ${step + 1} (${result.agent}): ${say(result)}`).join("\n\n")
        : say(this.results[0]);
    const done = this.results.filter((result) => result.status === "done").length;
    const how = done === this.results.length ? "finished" : `finished, ${done} of ${this.results.length} with an answer`;
    return `<${REPORT} call="${this.input.callId}">\nThe subagents you started have ${how}. This is their report, not a message from the user.\n\n${text}\n</${REPORT}>`;
  }
}

/** A conversation's messages, in order, with the reply still streaming. */
export function transcript(view: ConversationView): Message[] {
  const messages = view.entries.flatMap((entry) => ((entry as { model?: Message[] }).model ?? []))
    .filter((message) => (message.role as string) !== "system");
  const streaming = (view.docs["pi.live"] as { generation?: { message?: Message } } | undefined)?.generation?.message;
  return streaming ? [...messages, streaming] : messages;
}

/**
 * A step: going on ("Reading retry.ts"), done ("Read retry.ts"), or just named
 * ("read retry.ts", or a command as it was run).
 */
export function describe(name: string, args: Record<string, unknown>, tense: "now" | "done" | "named"): string {
  const path = typeof args.path === "string" ? args.path.split("/").at(-1)! : "";
  const command = typeof args.command === "string" ? firstLine(args.command) : "";
  const say = { now: ["Reading", "Editing", "Running"], done: ["Read", "Edited", "Ran"], named: ["read", name, ""] }[tense];
  switch (name) {
    case "read": return `${say[0]} ${path}`;
    case "write": case "edit": return `${say[1]} ${path}`;
    case "bash": return `${say[2]} ${command}`.trim();
    default: return name;
  }
}

/** What an answer says, in a line: its first heading, else its first line ("Perfect! Now I…" heads many). */
export function gist(answer: string): string {
  const heading = answer.split("\n").find((line) => /^#{1,6}\s+\S/.test(line));
  return firstLine(heading ? heading.replace(/^#+\s+/, "").replace(/\*\*/g, "") : answer);
}

function firstLine(text: string): string {
  const line = text.trim().split("\n")[0] ?? "";
  return line.length > 120 ? `${line.slice(0, 120)}…` : line;
}
