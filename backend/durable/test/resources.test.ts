import { afterEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { ProjectTrustStore } from "@earendil-works/pi-coding-agent";
import { expandPromptCommand, promptCommands } from "../src/commands.ts";
import { loadResources } from "../src/resources.ts";

let root: string;
afterEach(() => rmSync(root, { recursive: true, force: true }));

function write(path: string, text: string) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text);
}

/** A home with stock Pi's folders, and a git project with its own. */
function host() {
  root = mkdtempSync(join(tmpdir(), "durable-resources-"));
  const home = join(root, "home"), agent = join(home, ".pi", "agent"), project = join(root, "repo");
  write(join(agent, "prompts", "deploy.md"), "---\ndescription: Deploy it\nargument-hint: <env>\n---\nDeploy to $1.\n");
  write(join(agent, "prompts", "plain.md"), "First line describes it.\nMore.\n");
  write(join(agent, "prompts", "notes.txt"), "not a template");
  write(join(agent, "skills", "lint", "SKILL.md"), "---\nname: lint\ndescription: Lint the code\n---\nRun the linter.\n");
  write(join(home, ".agents", "skills", "hidden", "SKILL.md"), "---\nname: hidden\ndescription: Only on request\ndisable-model-invocation: true\n---\nSecret steps.\n");
  mkdirSync(join(project, ".git"), { recursive: true });
  write(join(project, ".pi", "prompts", "deploy.md"), "---\ndescription: Project deploy\n---\nProject deploy to $1.\n");
  write(join(project, ".agents", "skills", "release", "SKILL.md"), "---\nname: release\ndescription: Cut a release\n---\nTag it.\n");
  write(join(agent, "agents", "scout.md"), "---\nname: scout\ndescription: Fast recon\nmodel: claude-haiku-4-5\ntools: read, grep, bash\n---\nBe brief.\n");
  write(join(agent, "agents", "worker.md"), "---\nname: worker\ndescription: Does the work\ntools: [read, write, edit, bash]\n---\nFinish it.\n");
  write(join(agent, "agents", "broken.md"), "---\nname: broken\n---\nNo description.\n");
  write(join(project, ".pi", "agents", "scout.md"), "---\nname: scout\ndescription: The project's scout\n---\nLook here.\n");
  return { home, agent, project };
}

const names = (commands: readonly { name: string }[]) => commands.map((command) => command.name).sort();

describe("the host's templates and skills", () => {
  test("without a project, the user's own", () => {
    const { home, agent } = host();
    const found = loadResources(undefined, agent, home);
    expect(names(promptCommands(found))).toEqual(
      ["deploy", "explain", "fix-tests", "plain", "review", "skill:hidden", "skill:lint"],
    );
    const deploy = found.templates.find((template) => template.name === "deploy")!;
    expect([deploy.description, deploy.argumentHint, deploy.sourceInfo?.scope]).toEqual(["Deploy it", "<env>", "user"]);
    expect(found.templates.find((template) => template.name === "plain")!.description).toBe("First line describes it.");
    expect(expandPromptCommand("/deploy staging", found)).toBe("Deploy to staging.");
    expect(expandPromptCommand("/skill:lint", found)).toContain("\n\nRun the linter.\n</skill>");
  });

  test("the model hears of skills it may use, not those kept for the command", () => {
    const { home, agent } = host();
    const found = loadResources(undefined, agent, home);
    expect(found.skillsPrompt).toContain("lint");
    expect(found.skillsPrompt).not.toContain("hidden");
  });

  test("an untrusted project's own folders are left alone", () => {
    const { home, agent, project } = host();
    const found = loadResources(project, agent, home);
    expect(names(promptCommands(found))).not.toContain("skill:release");
    expect(expandPromptCommand("/deploy prod", found)).toBe("Deploy to prod.");
  });

  test("a trusted project's win over the user's", () => {
    const { home, agent, project } = host();
    new ProjectTrustStore(agent).set(project, true);
    const found = loadResources(project, agent, home);
    expect(names(promptCommands(found))).toContain("skill:release");
    expect(expandPromptCommand("/deploy prod", found)).toBe("Project deploy to prod.");
    expect(found.skills.find((skill) => skill.name === "release")!.sourceInfo.scope).toBe("project");
  });

  test("agents as stock Pi's subagent extension reads them, the project's once trusted", () => {
    const { home, agent, project } = host();
    const user = loadResources(project, agent, home).agents;
    expect(user.map((found) => found.name)).toEqual(["scout", "worker"]);
    expect(user[0]).toMatchObject({ description: "Fast recon", model: "claude-haiku-4-5", tools: ["read", "grep", "bash"], prompt: "Be brief.", scope: "user" });
    expect(user[1].tools).toEqual(["read", "write", "edit", "bash"]);
    new ProjectTrustStore(agent).set(project, true);
    const trusted = loadResources(project, agent, home).agents;
    expect(trusted.find((found) => found.name === "scout")).toMatchObject({ description: "The project's scout", scope: "project" });
  });

  test("a project with nothing to trust needs no trust", () => {
    root = mkdtempSync(join(tmpdir(), "durable-resources-"));
    const agent = join(root, "agent");
    write(join(agent, "prompts", "hi.md"), "Hi");
    mkdirSync(join(root, "repo"));
    expect(names(loadResources(join(root, "repo"), agent, root).templates)).toEqual(["hi"]);
  });
});
