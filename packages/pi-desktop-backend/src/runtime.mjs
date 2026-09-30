import { realpathSync } from 'node:fs';
import { isAbsolute, join, resolve } from 'node:path';
import { resolveSessionFile } from './commands.mjs';

function path(value, cwd) {
  if (value.startsWith('~/')) value = join(process.env.HOME ?? '', value.slice(2));
  return isAbsolute(value) ? value : resolve(cwd, value);
}

export function parseOptions(pi, args) {
  const options = pi.parseArgs(args);
  const errors = options.diagnostics.filter(item => item.type === 'error');
  if (errors.length) throw new Error(errors.map(item => item.message).join('; '));
  const unsupported = ['apiKey', 'continue', 'resume', 'fork', 'sessionId', 'export', 'print',
    'listModels', 'tuiMode', 'useTheme', 'verbose'];
  for (const key of unsupported) {
    if (options[key] !== undefined) throw new Error(`The desktop backend does not support CLI option ${key}`);
  }
  if (options.mode !== undefined && options.mode !== 'rpc') throw new Error('The desktop backend only supports --mode rpc');
  if (options.messages.length || options.fileArgs.length) throw new Error('Send prompts over JSONL, not CLI arguments');
  if (options.session && options.noSession) throw new Error('--session and --no-session are mutually exclusive');
  if (options.session && !isAbsolute(options.session)) throw new Error('--session requires an absolute saved session path');
  return options;
}

export async function createRuntime(pi, options, cwd = process.cwd()) {
  cwd = realpathSync(cwd);
  const agentDir = pi.getAgentDir();
  const bootstrap = pi.SettingsManager.create(cwd, agentDir, { projectTrusted: false });
  pi.applyHttpProxySettings(bootstrap.getGlobalSettings().httpProxy);
  pi.configureHttpDispatcher();
  const sessionDir = options.sessionDir ? path(options.sessionDir, cwd)
    : process.env.PI_SESSION_DIR ?? bootstrap.getSessionDir();
  const manager = options.session ? pi.SessionManager.open(await resolveSessionFile(options.session), sessionDir)
    : options.noSession ? pi.SessionManager.inMemory(cwd)
    : pi.SessionManager.create(cwd, sessionDir);
  if (options.name !== undefined) {
    const name = options.name.trim();
    if (!name) throw new Error('--name requires nonempty text');
    manager.appendSessionInfo(name);
  }
  const resources = {
    extensionFactories: pi.builtInExtensions,
    additionalExtensionPaths: (options.extensions ?? []).map(value => path(value, cwd)),
    additionalSkillPaths: (options.skills ?? []).map(value => path(value, cwd)),
    additionalPromptTemplatePaths: (options.promptTemplates ?? []).map(value => path(value, cwd)),
    additionalThemePaths: (options.themes ?? []).map(value => path(value, cwd)),
    noExtensions: options.noExtensions, noSkills: options.noSkills,
    noPromptTemplates: options.noPromptTemplates, noThemes: options.noThemes,
    noContextFiles: options.noContextFiles, systemPrompt: options.systemPrompt,
    appendSystemPrompt: options.appendSystemPrompt,
  };
  const trustByCwd = new Map();
  const factory = async ({ cwd, agentDir, sessionManager, sessionStartEvent, projectTrustContext }) => {
    const settingsManager = pi.SettingsManager.create(cwd, agentDir, { projectTrusted: false });
    const trustStore = new pi.ProjectTrustStore(agentDir);
    const services = await pi.createAgentSessionServices({
      cwd, agentDir, settingsManager, modelRuntimeSignal: AbortSignal.timeout(15_000),
      extensionFlagValues: options.unknownFlags, resourceLoaderOptions: resources,
      resourceLoaderReloadOptions: {
        resolveProjectTrust: async ({ extensionsResult }) => {
          if (trustByCwd.has(cwd)) return trustByCwd.get(cwd);
          const trusted = await pi.resolveProjectTrusted({
            cwd, trustStore, trustOverride: options.projectTrustOverride,
            defaultProjectTrust: settingsManager.getDefaultProjectTrust(), extensionsResult,
            projectTrustContext: projectTrustContext ?? pi.createProjectTrustContext({
              cwd, mode: 'rpc', settingsManager, hasUI: false,
            }),
            onExtensionError: message => console.error(message),
          });
          trustByCwd.set(cwd, trusted);
          return trusted;
        },
      },
    });
    const extensionErrors = services.resourceLoader.getExtensions().errors;
    const diagnostics = [...services.diagnostics, ...extensionErrors.map(({ path, error }) => ({
      type: 'error', message: `Failed to load extension ${path}: ${error}`,
    }))];
    // A failed resource stays an SDK-reported load error; it must not prevent
    // inspecting or using the remaining session. Invalid flags/services still fail.
    if (services.diagnostics.some(item => item.type === 'error')) {
      throw new Error(services.diagnostics.filter(item => item.type === 'error').map(item => item.message).join('; '));
    }
    const patterns = options.models ?? settingsManager.getEnabledModels();
    const scoped = patterns?.length ? pi.resolveModelScopeFromModels(patterns,
      services.modelRuntime.getAvailableSnapshot()) : { scopedModels: [], diagnostics: [] };
    for (const diagnostic of scoped.diagnostics) console.error(diagnostic.message);
    const selected = pi.resolveCliModel({ cliProvider: options.provider, cliModel: options.model,
      cliThinking: options.thinking, modelRuntime: services.modelRuntime });
    if (selected.error) throw new Error(selected.error);
    if (options.provider && !options.model) throw new Error('--provider requires --model');
    const created = await pi.createAgentSessionFromServices({
      services, sessionManager, sessionStartEvent, model: selected.model,
      thinkingLevel: options.thinking ?? selected.thinkingLevel,
      scopedModels: scoped.scopedModels, tools: options.tools, excludeTools: options.excludeTools,
      noTools: options.noTools ? 'all' : options.noBuiltinTools ? 'builtin' : undefined,
    });
    return { ...created, services, diagnostics };
  };
  return pi.createAgentSessionRuntime(factory, { cwd: manager.getCwd(), agentDir, sessionManager: manager });
}
