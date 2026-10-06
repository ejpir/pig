export type PromptCommand = {
  name: string;
  description: string;
  source: "prompt";
  sourceInfo: null;
};

export const promptCommands: readonly PromptCommand[] = [
  {
    name: "review",
    description: "Review the local changes and report concrete findings",
    source: "prompt",
    sourceInfo: null,
  },
  {
    name: "explain",
    description: "Explain this project and how its main pieces fit together",
    source: "prompt",
    sourceInfo: null,
  },
  {
    name: "fix-tests",
    description: "Run the relevant tests, diagnose failures, and fix them",
    source: "prompt",
    sourceInfo: null,
  },
];

const templates = new Map([
  ["review", "Review the local changes. Inspect the diff, identify concrete bugs and risks, and report findings. Do not modify files unless the request after the command asks you to."],
  ["explain", "Explain this project. Inspect the repository, then describe its purpose, architecture, important entry points, and how to work on it."],
  ["fix-tests", "Run the relevant tests, diagnose the failures, fix their root causes, and verify the result."],
]);

/** Expands only commands this backend advertised. Unknown slash text remains ordinary user input. */
export function expandPromptCommand(message: string): string {
  const match = /^\/([^\s]+)(?:\s+([\s\S]*))?$/.exec(message.trim());
  if (!match) return message;
  const template = templates.get(match[1]);
  if (!template) return message;
  const detail = match[2]?.trim();
  return detail ? `${template}\n\nAdditional request: ${detail}` : template;
}
