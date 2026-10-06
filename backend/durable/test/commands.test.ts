import { describe, expect, test } from "bun:test";
import { expandPromptCommand, promptCommands } from "../src/commands.ts";

describe("durable prompt commands", () => {
  test("advertises only commands the backend expands", () => {
    expect(promptCommands.map((command) => command.name)).toEqual([
      "review",
      "explain",
      "fix-tests",
    ]);
    expect(promptCommands.every((command) => command.source === "prompt")).toBe(true);
  });

  test("expands an advertised command and preserves its arguments", () => {
    expect(expandPromptCommand("/review focus on races")).toContain(
      "Additional request: focus on races",
    );
    expect(expandPromptCommand("/fix-tests")).toContain("Run the relevant tests");
  });

  test("leaves unknown slash text and ordinary prompts alone", () => {
    expect(expandPromptCommand("/unknown keep this")).toBe("/unknown keep this");
    expect(expandPromptCommand("hello")).toBe("hello");
  });
});
