/**
 * The host's prompt templates and skills, from the folders stock Pi reads by
 * default. Packages and extra paths in settings aren't read: resolving them can
 * install packages. A project's own folders are read only once stock Pi has
 * been told to trust the project, as it would without a prompt to ask.
 */
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import {
  CONFIG_DIR_NAME, ProjectTrustStore, SettingsManager, formatSkillsForPrompt, getAgentDir,
  hasTrustRequiringProjectResources, loadSkillsFromDir, parseFrontmatter, stripFrontmatter,
} from "@earendil-works/pi-coding-agent";
import type { AgentDefinition, PromptTemplate, Resources, Skill, SourceInfo } from "./commands.ts";

/** Without `cwd`, only the user's own; with it, the project's first when trusted. */
export function loadResources(cwd?: string, agentDir = getAgentDir(), home = homedir()): Resources {
  const project = cwd && trusted(cwd, agentDir) ? resolve(cwd) : undefined;
  const userAgents = join(home, ".agents");
  // Earlier folders win a name, as in stock Pi: the project's, then the user's.
  const skillFolders: (readonly [string, SourceInfo["scope"]])[] = [
    ...(project ? [[join(project, CONFIG_DIR_NAME), "project"] as const] : []),
    ...(project ? ancestors(project).map((dir) => join(dir, ".agents")).filter((dir) => dir !== userAgents)
      .map((dir) => [dir, "project"] as const) : []),
    [agentDir, "user"] as const,
    [userAgents, "user"] as const,
  ];
  const skills = new Map<string, Skill>();
  for (const [baseDir, scope] of skillFolders) {
    for (const skill of loadSkillsFromDir({ dir: join(baseDir, "skills"), source: "local" }).skills) {
      if (skills.has(skill.name)) continue;
      skills.set(skill.name, {
        name: skill.name, description: skill.description, filePath: skill.filePath, baseDir: skill.baseDir,
        disableModelInvocation: skill.disableModelInvocation,
        sourceInfo: { path: skill.filePath, source: "local", scope, origin: "top-level", baseDir },
      });
    }
  }
  const templates = new Map<string, PromptTemplate>();
  for (const [dir, scope] of [
    ...(project ? [[join(project, CONFIG_DIR_NAME, "prompts"), "project"] as const] : []),
    [join(agentDir, "prompts"), "user"] as const,
  ]) {
    for (const template of templatesIn(dir, scope)) {
      if (!templates.has(template.name)) templates.set(template.name, template);
    }
  }
  const agents = new Map<string, AgentDefinition>();
  for (const [dir, scope] of [
    ...(project ? [[join(project, CONFIG_DIR_NAME, "agents"), "project"] as const] : []),
    [join(agentDir, "agents"), "user"] as const,
  ]) {
    for (const agent of agentsIn(dir, scope)) {
      if (!agents.has(agent.name)) agents.set(agent.name, agent);
    }
  }
  const found = [...skills.values()];
  return {
    templates: [...templates.values()],
    skills: found,
    agents: [...agents.values()],
    skillsPrompt: formatSkillsForPrompt(found, "read").trim(),
    readSkill: (skill) => stripFrontmatter(readFileSync(skill.filePath, "utf8")),
  };
}

/** Stock Pi's decision when it can't ask: a remembered answer, else the default, unless there is nothing to trust. */
function trusted(cwd: string, agentDir: string): boolean {
  if (!hasTrustRequiringProjectResources(cwd)) return true;
  const remembered = new ProjectTrustStore(agentDir).get(cwd);
  if (remembered !== null) return remembered;
  return SettingsManager.create(cwd, agentDir, { projectTrusted: false }).getDefaultProjectTrust() === "always";
}

/** `dir` and its parents, up to the repository's root, or the disk's without one. */
function ancestors(dir: string): string[] {
  const found = [dir];
  while (!existsSync(join(found.at(-1)!, ".git")) && dirname(found.at(-1)!) !== found.at(-1)) {
    found.push(dirname(found.at(-1)!));
  }
  return found;
}

/** Each `.md` file directly in `dir`, named by its file. */
function templatesIn(dir: string, scope: SourceInfo["scope"]): PromptTemplate[] {
  let entries: string[];
  try {
    entries = readdirSync(dir).filter((name) => name.endsWith(".md")).sort();
  } catch {
    return [];
  }
  return entries.flatMap((name) => {
    const path = join(dir, name);
    try {
      if (!statSync(path).isFile()) return [];
      const { frontmatter, body } = parseFrontmatter(readFileSync(path, "utf8"));
      const firstLine = body.split("\n").find((line) => line.trim()) ?? "";
      const description = typeof frontmatter.description === "string" && frontmatter.description
        ? frontmatter.description
        : firstLine.length > 60 ? `${firstLine.slice(0, 60)}...` : firstLine;
      const hint = frontmatter["argument-hint"];
      return [{
        name: basename(name, ".md"), description, content: body,
        ...(typeof hint === "string" ? { argumentHint: hint } : {}),
        sourceInfo: { path, source: "local", scope, origin: "top-level" as const, baseDir: dir },
      }];
    } catch (error) {
      console.error(`Skipping prompt template ${path}: ${error}`);
      return [];
    }
  });
}

/** Each `.md` file directly in `dir` with a `name` and `description`, as stock Pi's subagent extension reads them. */
function agentsIn(dir: string, scope: AgentDefinition["scope"]): AgentDefinition[] {
  let entries: string[];
  try {
    entries = readdirSync(dir).filter((name) => name.endsWith(".md")).sort();
  } catch {
    return [];
  }
  return entries.flatMap((name) => {
    const path = join(dir, name);
    try {
      if (!statSync(path).isFile()) return [];
      const { frontmatter, body } = parseFrontmatter(readFileSync(path, "utf8"));
      if (typeof frontmatter.name !== "string" || typeof frontmatter.description !== "string") return [];
      // `tools: read, bash` or `tools: [read, bash]`.
      const tools = (Array.isArray(frontmatter.tools) ? frontmatter.tools : typeof frontmatter.tools === "string" ? frontmatter.tools.split(",") : [])
        .filter((tool): tool is string => typeof tool === "string").map((tool) => tool.trim()).filter(Boolean);
      return [{
        name: frontmatter.name, description: frontmatter.description, prompt: body, scope, filePath: path,
        ...(typeof frontmatter.model === "string" && frontmatter.model ? { model: frontmatter.model } : {}),
        ...(tools.length ? { tools } : {}),
      }];
    } catch (error) {
      console.error(`Skipping agent ${path}: ${error}`);
      return [];
    }
  });
}
