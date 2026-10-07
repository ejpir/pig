import { describe, expect, test } from "bun:test";
import {
  expandPromptCommand, noResources, parseCommandArgs, promptCommands, substituteArgs,
  type Resources, type Skill,
} from "../src/commands.ts";

const source = (path: string) => ({ path, source: "local", scope: "user" as const, origin: "top-level" as const });
const skill: Skill = {
  name: "check", description: "Check the build", filePath: "/home/me/.pi/agent/skills/check/SKILL.md",
  baseDir: "/home/me/.pi/agent/skills/check", sourceInfo: source("/home/me/.pi/agent/skills/check/SKILL.md"),
  disableModelInvocation: false,
};
const resources: Resources = {
  templates: [
    { name: "greet", description: "Greet", content: "Say hello to $1 and ${2:-everyone}. All: $@", sourceInfo: source("/p/greet.md") },
    { name: "review", description: "Mine", content: "My review of $ARGUMENTS", sourceInfo: source("/p/review.md") },
  ],
  skills: [skill],
  agents: [],
  skillsPrompt: "",
  readSkill: () => "\nRun the build.\n",
};

describe("durable prompt commands", () => {
  test("lists the built-ins without any files", () => {
    expect(promptCommands(noResources).map((command) => command.name)).toEqual(["review", "explain", "fix-tests"]);
    expect(promptCommands(noResources).every((command) => command.source === "prompt" && command.sourceInfo === null)).toBe(true);
  });

  test("lists the host's templates, then built-ins they don't replace, then skills", () => {
    const commands = promptCommands(resources);
    expect(commands.map((command) => [command.name, command.source])).toEqual([
      ["greet", "prompt"], ["review", "prompt"], ["explain", "prompt"], ["fix-tests", "prompt"], ["skill:check", "skill"],
    ]);
    expect(commands[1].description).toBe("Mine");
    expect(commands[4].sourceInfo?.path).toBe(skill.filePath);
  });

  test("a built-in keeps what follows it as typed", () => {
    expect(expandPromptCommand("/fix-tests", noResources)).toStartWith("Run the relevant tests");
    expect(expandPromptCommand("/explain don't skip the CLI", noResources)).toEndWith(
      "\n\nAdditional request: don't skip the CLI",
    );
  });

  test("a template takes its arguments as stock Pi does", () => {
    expect(expandPromptCommand('/greet "Ada Lovelace"', resources)).toBe(
      "Say hello to Ada Lovelace and everyone. All: Ada Lovelace",
    );
    expect(expandPromptCommand("/review the parser", resources)).toBe("My review of the parser");
  });

  test("a skill becomes the block both apps show as a skill", () => {
    expect(expandPromptCommand("/skill:check", resources)).toBe(
      `<skill name="check" location="${skill.filePath}">\nReferences are relative to ${skill.baseDir}.\n\nRun the build.\n</skill>`,
    );
    expect(expandPromptCommand("/skill:check only the tests", resources)).toEndWith("</skill>\n\nonly the tests");
    const broken = { ...resources, readSkill: () => { throw new Error("gone"); } };
    expect(() => expandPromptCommand("/skill:check", broken)).toThrow("Could not read the check skill");
  });

  test("leaves unknown slash text and ordinary prompts alone", () => {
    expect(expandPromptCommand("/unknown keep this", resources)).toBe("/unknown keep this");
    expect(expandPromptCommand("/skill:missing", resources)).toBe("/skill:missing");
    expect(expandPromptCommand("hello", resources)).toBe("hello");
  });
});

describe("template arguments", () => {
  test("quotes group words", () => {
    expect(parseCommandArgs(`one "two three" 'four'  five`)).toEqual(["one", "two three", "four", "five"]);
  });

  test("every placeholder", () => {
    const args = ["a", "b", "c"];
    expect(substituteArgs("$1|$3|$4|$@|$ARGUMENTS", args)).toBe("a|c||a b c|a b c");
    expect(substituteArgs("${4:-none}|${@:2}|${@:1:2}", args)).toBe("none|b c|a b");
    expect(substituteArgs("${@:-nothing}", [])).toBe("nothing");
    expect(substituteArgs("$1", ["$2"])).toBe("$2");
  });
});
