import { describe, expect, test } from "bun:test";
import { definition, describe as step, gist } from "../src/subagents.ts";
import type { AgentDefinition } from "../src/commands.ts";

const scout: AgentDefinition = {
  name: "scout", description: "Finds things", prompt: "Be brief.\n", model: "claude-haiku-4-5",
  tools: ["read", "grep", "find", "ls", "bash"], scope: "user", filePath: "/agents/scout.md",
};

describe("subagent definitions", () => {
  test("a name no file has is an agent the call defines", () => {
    expect(definition({ agent: "architecture", task: "Review src", instructions: " Review module boundaries. ", tools: ["read", "bash"] }, [scout]))
      .toEqual({ name: "architecture", prompt: "Review module boundaries.", tools: ["read", "bash"] });
    // Without tools it gets the session's, and without instructions just its name.
    expect(definition({ agent: "helper", task: "Help" }, [])).toEqual({ name: "helper", prompt: "" });
  });

  test("a listed agent keeps its file, and the call can add to it", () => {
    expect(definition({ agent: "scout", task: "Look" }, [scout]))
      .toEqual({ name: "scout", prompt: "Be brief.", model: "claude-haiku-4-5", tools: ["read", "bash"] });
    expect(definition({ agent: "scout", task: "Look", instructions: "Only tests.", tools: ["read"] }, [scout]))
      .toEqual({ name: "scout", prompt: "Be brief.\n\nOnly tests.", model: "claude-haiku-4-5", tools: ["read"] });
  });

  test("stock Pi's search tools are bash, and unknown ones are left out", () => {
    expect(definition({ agent: "x", task: "t", tools: ["grep", "find", "ls", "web"] }, []).tools).toEqual(["bash"]);
  });

  test("a step reads as going on, done or named", () => {
    expect(step("read", { path: "/repo/src/retry.ts" }, "now")).toBe("Reading retry.ts");
    expect(step("read", { path: "/repo/src/retry.ts" }, "done")).toBe("Read retry.ts");
    expect(step("bash", { command: "pnpm test\n--watch" }, "done")).toBe("Ran pnpm test");
    expect(step("bash", { command: "pnpm test" }, "named")).toBe("pnpm test");
    expect(step("edit", { path: "a/b.ts" }, "named")).toBe("edit b.ts");
  });

  test("an answer's gist is its first heading, past a preamble", () => {
    expect(gist("Perfect! Now I have a complete picture.\n\n## Architecture Review: **Runner**\n\nBody")).toBe("Architecture Review: Runner");
    expect(gist("found alpha")).toBe("found alpha");
  });
});
