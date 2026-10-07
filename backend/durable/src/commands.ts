/**
 * Slash commands, expanded as stock Pi expands them before a prompt is admitted:
 * `/skill:name args` and `/template args`. The expanded text is what gets
 * committed, so recovery never reads these files again. No stock Pi imports, so
 * the faux fixture stays separate; `resources.ts` finds the files.
 */

/** Where a command's file came from, as stock Pi reports it. */
export type SourceInfo = {
  path: string;
  source: string;
  scope: "user" | "project" | "temporary";
  origin: "package" | "top-level";
  baseDir?: string;
};

export type PromptTemplate = {
  name: string;
  description: string;
  argumentHint?: string;
  content: string;
  sourceInfo: SourceInfo | null;
};

export type Skill = {
  name: string;
  description: string;
  filePath: string;
  baseDir: string;
  sourceInfo: SourceInfo;
  /** Only `/skill:name` runs it; the model isn't told about it. */
  disableModelInvocation: boolean;
};

/** An agent Pi can hand work to: a Markdown file, as stock Pi's subagent extension reads them. */
export type AgentDefinition = {
  name: string;
  description: string;
  /** "claude-sonnet-4-5" or "provider/model"; absent: the session's. */
  model?: string;
  /** Absent: every tool the session has. */
  tools?: string[];
  /** The file's body: its instructions. */
  prompt: string;
  scope: "user" | "project";
  filePath: string;
};

export type Resources = {
  templates: readonly PromptTemplate[];
  skills: readonly Skill[];
  agents: readonly AgentDefinition[];
  /** The skills the model may use, for the system prompt; empty without any. */
  skillsPrompt: string;
  /** A skill's instructions, without frontmatter. */
  readSkill(skill: Skill): string;
};

export type PromptCommand = {
  name: string;
  description: string;
  source: "prompt" | "skill";
  sourceInfo: SourceInfo | null;
};

/** The backend's own templates, for the starters both apps offer. A user's template of the same name wins. */
export const builtinTemplates: readonly PromptTemplate[] = [
  {
    name: "review",
    description: "Review the local changes and report concrete findings",
    content: "Review the local changes. Inspect the diff, identify concrete bugs and risks, and report findings. Do not modify files unless the request after the command asks you to.",
    sourceInfo: null,
  },
  {
    name: "explain",
    description: "Explain this project and how its main pieces fit together",
    content: "Explain this project. Inspect the repository, then describe its purpose, architecture, important entry points, and how to work on it.",
    sourceInfo: null,
  },
  {
    name: "fix-tests",
    description: "Run the relevant tests, diagnose failures, and fix them",
    content: "Run the relevant tests, diagnose the failures, fix their root causes, and verify the result.",
    sourceInfo: null,
  },
];

export const noResources: Resources = { templates: [], skills: [], agents: [], skillsPrompt: "", readSkill: () => "" };

/** The user's templates, then the built-ins they don't replace. */
function templates(resources: Resources): PromptTemplate[] {
  const names = new Set(resources.templates.map((template) => template.name));
  return [...resources.templates, ...builtinTemplates.filter((template) => !names.has(template.name))];
}

export function promptCommands(resources: Resources): PromptCommand[] {
  return [
    ...templates(resources).map((template) => ({
      name: template.name, description: template.description, source: "prompt" as const, sourceInfo: template.sourceInfo,
    })),
    ...resources.skills.map((skill) => ({
      name: `skill:${skill.name}`, description: skill.description, source: "skill" as const, sourceInfo: skill.sourceInfo,
    })),
  ];
}

/** Expands only commands this backend lists. Other slash text stays ordinary user input. */
export function expandPromptCommand(message: string, resources: Resources): string {
  if (message.startsWith("/skill:")) {
    const space = message.indexOf(" ");
    const name = space === -1 ? message.slice(7) : message.slice(7, space);
    const args = space === -1 ? "" : message.slice(space + 1).trim();
    const skill = resources.skills.find((skill) => skill.name === name);
    if (!skill) return message;
    let body: string;
    try {
      body = resources.readSkill(skill).trim();
    } catch (error) {
      throw new Error(`Could not read the ${skill.name} skill: ${error}`);
    }
    // The same block stock Pi writes, which both apps show as a skill.
    const block = `<skill name="${skill.name}" location="${skill.filePath}">\nReferences are relative to ${skill.baseDir}.\n\n${body}\n</skill>`;
    return args ? `${block}\n\n${args}` : block;
  }
  const match = /^\/([^\s]+)(?:\s+([\s\S]*))?$/.exec(message);
  if (!match) return message;
  const template = templates(resources).find((template) => template.name === match[1]);
  if (!template) return message;
  if (!template.sourceInfo) {
    // A built-in takes what follows it as it was typed.
    const detail = match[2]?.trim();
    return detail ? `${template.content}\n\nAdditional request: ${detail}` : template.content;
  }
  return substituteArgs(template.content, parseCommandArgs(match[2] ?? ""));
}

/** Arguments split on whitespace, with quotes kept together; as stock Pi's `parseCommandArgs`. */
export function parseCommandArgs(text: string): string[] {
  const args: string[] = [];
  let current = "";
  let quote: string | null = null;
  for (const char of text) {
    if (quote) {
      if (char === quote) quote = null;
      else current += char;
    } else if (char === '"' || char === "'") {
      quote = char;
    } else if (/\s/.test(char)) {
      if (current) args.push(current);
      current = "";
    } else {
      current += char;
    }
  }
  if (current) args.push(current);
  return args;
}

/**
 * Stock Pi's placeholders: `$1`, `$@`, `$ARGUMENTS`, `${N:-default}`,
 * `${@:-default}`, `${@:N}` and `${@:N:L}`. Values aren't substituted again.
 */
export function substituteArgs(content: string, args: string[]): string {
  const all = args.join(" ");
  return content.replace(
    /\$\{(\d+|ARGUMENTS|@):-([^}]*)\}|\$\{@:(\d+)(?::(\d+))?\}|\$(ARGUMENTS|@|\d+)/g,
    (_, target, fallback, start, length, simple) => {
      if (target) {
        const value = target === "@" || target === "ARGUMENTS" ? all : args[Number(target) - 1];
        return value ? value : fallback;
      }
      if (start) {
        const from = Math.max(Number(start) - 1, 0);
        return (length ? args.slice(from, from + Number(length)) : args.slice(from)).join(" ");
      }
      if (simple === "ARGUMENTS" || simple === "@") return all;
      return args[Number(simple) - 1] ?? "";
    },
  );
}
