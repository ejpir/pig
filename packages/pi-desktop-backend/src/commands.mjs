import { spawn } from 'node:child_process';
import { open, realpath, stat } from 'node:fs/promises';
import { isAbsolute } from 'node:path';
import { promptWithDisposition, PI_VERSION } from './compat.mjs';
import { shareSession } from './share.mjs';

export const COMMANDS = [
  'get_backend_info', 'prompt', 'steer', 'follow_up', 'abort', 'clear_queue', 'new_session', 'bash', 'abort_bash',
  'get_state', 'get_messages', 'get_entries', 'get_custom_entries', 'append_custom_entry', 'get_tree', 'get_settings', 'get_session_stats',
  'set_model', 'cycle_model', 'get_available_models', 'set_thinking_level', 'cycle_thinking_level',
  'get_available_thinking_levels', 'set_model_thinking_level', 'set_scoped_models',
  'set_steering_mode', 'set_follow_up_mode', 'set_auto_compaction', 'compact',
  'set_auto_retry', 'abort_retry', 'set_cache_warming', 'set_default_project_trust',
  'export_html', 'switch_session', 'fork', 'clone', 'get_fork_messages', 'navigate_tree',
  'set_label', 'set_session_name', 'list_sessions', 'delete_session', 'share', 'get_last_assistant_text',
  'get_project_trust', 'set_project_trust', 'get_auth_providers', 'get_commands',
  'reload', 'list_packages', 'check_package_updates', 'install_package', 'remove_package', 'update_packages',
];
export const EXCLUSIVE = new Set(['prompt', 'steer', 'follow_up', 'compact', 'new_session', 'switch_session', 'fork', 'clone',
  'navigate_tree', 'set_label', 'append_custom_entry', 'set_session_name', 'delete_session', 'share', 'reload', 'bash',
  'set_project_trust', 'install_package', 'remove_package', 'update_packages']);
const IDLE = new Set([...EXCLUSIVE].filter(type => !['prompt', 'steer', 'follow_up'].includes(type)));
const LEVELS = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh'];
const resourceSettings = settings => Object.fromEntries(Object.entries(settings)
  .filter(([key]) => ['packages', 'extensions', 'skills', 'prompts', 'defaultProvider',
    'defaultModel', 'defaultThinkingLevel', 'enabledModels'].includes(key)));
const required = (command, key, kind = 'string') => {
  if (typeof command[key] !== kind) throw new Error(`${key} must be a ${kind}`);
  return command[key];
};
const optional = (command, key, kind) => {
  if (command[key] !== undefined) required(command, key, kind);
};
const oneOf = (command, key, values) => {
  if (!values.includes(command[key])) throw new Error(`Invalid ${key}; expected ${values.join(', ')}`);
  return command[key];
};
export function validate(command) {
  if (!command || typeof command !== 'object' || Array.isArray(command)) throw new Error('Command must be an object');
  required(command, 'type'); optional(command, 'id', 'string');
  const strings = {
    prompt: ['message'], steer: ['message'], follow_up: ['message'], bash: ['command'],
    set_model: ['provider', 'modelId'], set_model_thinking_level: ['provider', 'modelId'],
    switch_session: ['sessionPath'], fork: ['entryId'], navigate_tree: ['targetId'], set_label: ['targetId'],
    set_session_name: ['name'], delete_session: ['sessionPath'],
    append_custom_entry: ['customType'], get_custom_entries: ['customType'],
    install_package: ['source'], remove_package: ['source'],
  };
  for (const key of strings[command.type] ?? []) required(command, key);
  for (const key of ['local', 'persist', 'summarize', 'replaceInstructions', 'excludeFromContext']) optional(command, key, 'boolean');
  for (const key of ['sessionPath', 'outputPath', 'label', 'since', 'source', 'customInstructions', 'parentSession', 'cwd']) optional(command, key, 'string');
  if (['set_auto_compaction', 'set_auto_retry'].includes(command.type)) required(command, 'enabled', 'boolean');
  if (command.type === 'set_thinking_level') oneOf(command, 'level', LEVELS);
  if (command.type === 'set_model_thinking_level' && command.level !== undefined) oneOf(command, 'level', LEVELS);
  if (['set_steering_mode', 'set_follow_up_mode'].includes(command.type)) oneOf(command, 'mode', ['one-at-a-time', 'all']);
  if (command.type === 'set_cache_warming') oneOf(command, 'mode', ['off', 'streaming', 'idle']);
  if (command.type === 'set_default_project_trust') oneOf(command, 'value', ['ask', 'always', 'never']);
  if (command.type === 'set_project_trust') oneOf(command, 'choice', ['trust', 'trust-parent', 'distrust']);
  if (command.type === 'list_sessions' && command.scope !== undefined) oneOf(command, 'scope', ['all', 'project']);
  if (command.streamingBehavior !== undefined) oneOf(command, 'streamingBehavior', ['steer', 'followUp']);
  if (command.type === 'set_scoped_models' && command.patterns !== null &&
      (!Array.isArray(command.patterns) || !command.patterns.every(pattern => typeof pattern === 'string'))) {
    throw new Error('patterns must be null or an array of strings');
  }
  // The desktop's own records only: extensions keep their state in custom entries too.
  if (command.type === 'append_custom_entry') {
    if (!command.customType.startsWith('pi-desktop-')) throw new Error('customType must start with pi-desktop-');
    if (!command.data || typeof command.data !== 'object' || Array.isArray(command.data)) throw new Error('data must be an object');
  }
  if (command.images !== undefined && (!Array.isArray(command.images) || !command.images.every(image =>
    image?.type === 'image' && typeof image.data === 'string' && typeof image.mimeType === 'string'))) {
    throw new Error('images must be an array of image content');
  }
  return command;
}

// Check a bounded header before opening a different session: SessionManager.open
// can otherwise create new data when given a nonexistent or malformed file.
export async function resolveSessionFile(value) {
  if (!isAbsolute(value)) throw new Error('sessionPath must be absolute');
  const file = await realpath(value);
  if (!(await stat(file)).isFile()) throw new Error('Not a saved session file');
  const handle = await open(file, 'r');
  try {
    const bytes = Buffer.alloc(64 * 1024);
    const { bytesRead } = await handle.read(bytes);
    const lineEnd = bytes.subarray(0, bytesRead).indexOf(10);
    if (lineEnd < 0) throw new Error('Missing or oversized session header');
    const header = JSON.parse(bytes.subarray(0, lineEnd).toString('utf8').replace(/^\uFEFF/, ''));
    if (header.type !== 'session' || typeof header.id !== 'string' || typeof header.cwd !== 'string' || !isAbsolute(header.cwd)) {
      throw new Error('Not a saved session file');
    }
    return file;
  } finally { await handle.close(); }
}

export function createCommands(pi, host, output, options = {}) {
  let activeShare;
  const packageManager = id => {
    const manager = new pi.DefaultPackageManager({ cwd: host.session.sessionManager.getCwd(),
      agentDir: host.services.agentDir, settingsManager: host.session.settingsManager });
    manager.setProgressCallback(progress => output({ type: 'package_progress', id, progress }));
    return manager;
  };
  const active = async file => host.session.sessionFile && file === await realpath(host.session.sessionFile).catch(() => host.session.sessionFile);
  const dispatch = async command => {
    validate(command);
    const { type, id } = command;
    const session = host.session;
    if (IDLE.has(type) && (session.isStreaming || session.isCompacting || session.pendingMessageCount || session.isBashRunning)) {
      throw new Error('Wait for the current run and queued work to finish');
    }
    if (type === 'prompt') {
      // The server keeps the preflight gate until the correlated acknowledgement.
      return new Promise((resolve, reject) => promptWithDisposition(session, command,
        disposition => resolve({ disposition }), reject));
    }
    switch (type) {
      case 'get_backend_info': return { backend: 'pi-desktop-backend', version: '0.1.0',
        protocolVersion: 1, piVersion: PI_VERSION, nodeVersion: process.versions.node,
        // Set in the standalone build, which runs on Bun's Node.js compatibility.
        bunVersion: process.versions.bun, commands: COMMANDS,
        // Optional parameters the desktop checks for before using them.
        features: ['fork_cwd'],
        limitations: ['native auth mutations use terminal handoff', 'delete_session requires a working trash executable'] };
      case 'steer': case 'follow_up': {
        let queued = false;
        const unsubscribe = session.subscribe(event => { if (event.type === 'queue_update') queued = true; });
        try {
          const disposition = await (type === 'steer' ? session.steer(command.message, command.images, { source: 'rpc' })
            : session.followUp(command.message, command.images, { source: 'rpc' }));
          return { disposition: disposition ?? (queued ? 'queued' : 'handled') };
        } finally { unsubscribe(); }
      }
      case 'abort': activeShare?.abort(); session.abortBash(); await session.abort(); return;
      case 'abort_bash': session.abortBash(); return;
      case 'bash': {
        if (!command.command.trim()) throw new Error('Shell command must not be empty');
        const event = await session.extensionRunner.emitUserBash({ type: 'user_bash',
          command: command.command, excludeFromContext: command.excludeFromContext ?? false,
          cwd: session.sessionManager.getCwd() });
        if (event?.result) {
          session.recordBashResult(command.command, event.result, { excludeFromContext: command.excludeFromContext });
          return event.result;
        }
        return session.executeBash(command.command, undefined, {
          excludeFromContext: command.excludeFromContext ?? false, id, operations: event?.operations });
      }
      case 'clear_queue': return session.clearQueue();
      case 'new_session': return host.newSession(command.parentSession ? { parentSession: command.parentSession } : undefined);
      case 'get_state': {
        const tools = new Map(session.getAllTools().map(tool => [tool.name, tool]));
        return {
          activeTools: session.getActiveToolNames().map(name => ({ name,
            description: tools.get(name)?.description, sourceInfo: tools.get(name)?.sourceInfo })),
          model: session.model, thinkingLevel: session.thinkingLevel, isStreaming: session.isStreaming,
          isCompacting: session.isCompacting, isBashRunning: session.isBashRunning, steeringMode: session.steeringMode, followUpMode: session.followUpMode,
          sessionFile: session.sessionFile, sessionId: session.sessionId, sessionName: session.sessionName,
          autoCompactionEnabled: session.autoCompactionEnabled, messageCount: session.messages.length,
          pendingMessageCount: session.pendingMessageCount,
        };
      }
      case 'get_messages': return { messages: session.messages };
      case 'get_entries': {
        let entries = session.sessionManager.getEntries();
        if (command.since !== undefined) {
          const index = entries.findIndex(entry => entry.id === command.since);
          if (index < 0) throw new Error(`Entry not found: ${command.since}`);
          entries = entries.slice(index + 1);
        }
        return { entries, leafId: session.sessionManager.getLeafId() };
      }
      // Data-only entries pi keeps out of the model's context, such as the jj change
      // each desktop turn made. Written on the current branch; read from all branches.
      case 'append_custom_entry':
        return { entryId: session.sessionManager.appendCustomEntry(command.customType, command.data) };
      case 'get_custom_entries': return { entries: session.sessionManager.getEntries()
        .filter(entry => entry.type === 'custom' && entry.customType === command.customType) };
      case 'get_tree': return { tree: session.sessionManager.getTree(), leafId: session.sessionManager.getLeafId() };
      case 'get_session_stats': return session.getSessionStats();
      case 'get_settings': {
        const s = session.settingsManager;
        return { defaultProvider: s.getDefaultProvider(), defaultModel: s.getDefaultModel(),
          defaultThinkingLevel: s.getDefaultThinkingLevel(), modelThinkingLevels: s.getAllModelThinkingLevels(),
          enabledModels: s.getEnabledModels(), scopedModels: session.scopedModels.map(({ model }) => `${model.provider}/${model.id}`),
          steeringMode: s.getSteeringMode(), followUpMode: s.getFollowUpMode(), autoCompaction: s.getCompactionEnabled(),
          autoRetry: s.getRetryEnabled(), cacheWarming: s.getCacheWarmingMode(), defaultProjectTrust: s.getDefaultProjectTrust(),
          hideThinkingBlock: s.getHideThinkingBlock(), showCacheMissNotices: s.getShowCacheMissNotices() };
      }
      case 'get_available_models': return { models: session.modelRuntime.getAvailableSnapshot() };
      case 'get_available_thinking_levels': return { levels: session.getAvailableThinkingLevels() };
      case 'set_model': {
        const model = session.modelRuntime.getAvailableSnapshot().find(model => model.provider === command.provider && model.id === command.modelId);
        if (!model) throw new Error(`Model not found: ${command.provider}/${command.modelId}`);
        await session.setModel(model, { persist: command.persist ?? false }); return model;
      }
      case 'cycle_model': return await session.cycleModel() ?? null;
      case 'set_thinking_level': session.setThinkingLevel(command.level, { persist: command.persist ?? false }); return;
      case 'cycle_thinking_level': { const level = session.cycleThinkingLevel(); return level ? { level } : null; }
      case 'set_model_thinking_level': {
        const model = session.modelRuntime.getAvailableSnapshot().find(model => model.provider === command.provider && model.id === command.modelId);
        if (!model) throw new Error(`Model not found: ${command.provider}/${command.modelId}`);
        const current = session.model?.provider === model.provider && session.model.id === model.id;
        if (command.level === undefined) {
          session.settingsManager.removeModelThinkingLevel(model.provider, model.id);
          if (current) session.setThinkingLevel(session.settingsManager.getDefaultThinkingLevel() ?? 'medium');
        } else {
          if (!pi.getSupportedThinkingLevels(model).includes(command.level)) throw new Error('Thinking level not supported by this model');
          session.settingsManager.setModelThinkingLevel(model.provider, model.id, command.level);
          if (current) session.setThinkingLevel(command.level);
        }
        return;
      }
      case 'set_scoped_models': {
        const available = session.modelRuntime.getAvailableSnapshot();
        const scoped = command.patterns === null ? [] : pi.resolveModelScopeFromModels(command.patterns, available).scopedModels;
        const all = scoped.length && available.every(model => scoped.some(item => item.model.provider === model.provider && item.model.id === model.id));
        session.setScopedModels(all ? [] : scoped);
        if (command.persist) session.settingsManager.setEnabledModels(command.patterns === null || all ? undefined : [...command.patterns]);
        return { scopedModels: session.scopedModels.map(({ model }) => `${model.provider}/${model.id}`) };
      }
      case 'set_steering_mode': session.setSteeringMode(command.mode); return;
      case 'set_follow_up_mode': session.setFollowUpMode(command.mode); return;
      case 'set_auto_compaction': session.setAutoCompactionEnabled(command.enabled); return;
      case 'compact': return session.compact(command.customInstructions);
      case 'set_auto_retry': session.setAutoRetryEnabled(command.enabled); return;
      case 'abort_retry': session.abortRetry(); return;
      case 'set_cache_warming': session.setCacheWarmingMode(command.mode); return;
      case 'set_default_project_trust': session.settingsManager.setDefaultProjectTrust(command.value); return;
      case 'switch_session': return host.switchSession(await resolveSessionFile(command.sessionPath));
      case 'fork': {
        if (command.cwd === undefined) {
          const result = await host.fork(command.entryId); return { text: result.selectedText, cancelled: result.cancelled };
        }
        // A fork that works in another folder, such as a jj workspace with the files as
        // they were at this entry: a new session file whose header names that folder.
        // This process keeps its session; the desktop opens the new one.
        const cwd = await realpath(command.cwd);
        if (!(await stat(cwd)).isDirectory()) throw new Error('cwd must be a folder');
        const manager = session.sessionManager;
        const file = manager.getSessionFile();
        if (!file) throw new Error('This session is not saved yet');
        const entry = manager.getEntry(command.entryId);
        if (entry?.type !== 'message' || entry.message.role !== 'user') throw new Error('Fork needs a user message');
        const content = entry.message.content;
        const text = typeof content === 'string' ? content
          : content.filter(block => block.type === 'text').map(block => block.text).join('');
        const copy = pi.SessionManager.open(file, manager.getSessionDir(), cwd);
        const sessionPath = entry.parentId ? copy.createBranchedSession(entry.parentId) : copy.newSession();
        return { text, cancelled: false, sessionPath, sessionId: copy.getSessionId() };
      }
      case 'clone': {
        const leaf = session.sessionManager.getLeafId();
        if (!leaf) throw new Error('Cannot clone session: no current entry selected');
        const result = await host.fork(leaf, { position: 'at' }); return { cancelled: result.cancelled };
      }
      case 'get_fork_messages': return { messages: session.getUserMessagesForForking() };
      case 'navigate_tree': return session.navigateTree(command.targetId, { summarize: command.summarize,
        customInstructions: command.customInstructions, replaceInstructions: command.replaceInstructions, label: command.label });
      case 'set_label':
        if (!session.sessionManager.getEntry(command.targetId)) throw new Error('Label target entry does not exist');
        session.sessionManager.appendLabelChange(command.targetId, command.label?.trim() || undefined); return;
      case 'get_last_assistant_text': return { text: session.getLastAssistantText() };
      case 'set_session_name': {
        const name = command.name.trim(); if (!name) throw new Error('Session name cannot be empty');
        const file = command.sessionPath === undefined ? undefined : await resolveSessionFile(command.sessionPath);
        if (file === undefined || await active(file)) session.setSessionName(name);
        else pi.SessionManager.open(file).appendSessionInfo(name);
        return;
      }
      case 'list_sessions': {
        const manager = session.sessionManager;
        const sessions = command.scope === 'all' ? await pi.SessionManager.listAll(manager.usesDefaultSessionDir() ? undefined : manager.getSessionDir())
          : await pi.SessionManager.list(manager.getCwd(), manager.getSessionDir());
        return { sessions: sessions.map(({ allMessagesText, created, modified, ...info }) => ({ ...info,
          ...(Number.isNaN(created.getTime()) ? {} : { created: created.toISOString() }), modified: modified.toISOString() })) };
      }
      case 'delete_session': {
        const file = await resolveSessionFile(command.sessionPath);
        if (await active(file)) throw new Error('Cannot delete the currently active session');
        // Never silently fall back to permanent unlink when Trash fails.
        await new Promise((resolve, reject) => {
          const child = spawn('trash', ['--', file], { stdio: 'ignore', windowsHide: true, timeout: 15_000 });
          child.on('error', reject); child.on('close', code => code === 0 ? resolve()
            : reject(new Error('Could not move session to Trash; no permanent deletion was attempted')));
        });
        return { method: 'trash' };
      }
      case 'export_html': return { path: await session.exportToHtml(command.outputPath) };
      case 'get_project_trust': {
        const cwd = session.sessionManager.getCwd();
        return { cwd, trusted: session.settingsManager.isProjectTrusted(),
          hasProjectResources: pi.hasTrustRequiringProjectResources(cwd),
          loadedExtensions: [
            ...session.resourceLoader.getExtensions().extensions.map(extension => ({
              path: extension.resolvedPath, sourceInfo: extension.sourceInfo, status: 'loaded',
              commands: session.extensionRunner.getRegisteredCommands()
                .filter(command => command.sourceInfo?.path === extension.sourceInfo.path)
                .map(command => `/${command.invocationName}`),
            })),
            ...session.resourceLoader.getExtensions().errors.map(error => ({
              path: error.path, status: 'load-error', error: error.error, commands: [],
            })),
          ],
          savedDecision: new pi.ProjectTrustStore(host.services.agentDir).getEntry(cwd),
          contextFiles: session.resourceLoader.getAgentsFiles().agentsFiles.map(file => file.path),
          userSettings: resourceSettings(session.settingsManager.getGlobalSettings()),
          projectSettings: resourceSettings(session.settingsManager.getProjectSettings()) };
      }
      case 'set_project_trust': {
        const cwd = session.sessionManager.getCwd(); const parent = pi.getProjectTrustParentPath(cwd);
        const choice = pi.getProjectTrustOptions(cwd).find(option => command.choice === 'distrust' ? !option.trusted
          : option.trusted && (option.savedPath === parent) === (command.choice === 'trust-parent'));
        if (!choice?.savedPath) throw new Error('No parent folder to trust');
        new pi.ProjectTrustStore(host.services.agentDir).setMany(choice.updates);
        return { trusted: choice.trusted, savedPath: choice.savedPath };
      }
      case 'get_auth_providers': {
        const runtime = session.modelRuntime; const providers = [];
        for (const provider of runtime.getProviders()) {
          const auth = runtime.getProviderAuthStatus(provider.id);
          const status = auth.configured ? { type: runtime.isUsingOAuth(provider.id) ? 'oauth' : 'api_key', source: auth.label ?? auth.source } : undefined;
          if (provider.auth.oauth) providers.push({ id: provider.id, name: provider.name, authType: 'oauth',
            loginLabel: provider.auth.oauth.loginLabel, canLogin: true, status });
          if (provider.auth.apiKey) providers.push({ id: provider.id, name: provider.name, authType: 'api_key',
            canLogin: provider.auth.apiKey.login !== undefined, status });
        }
        return { providers: providers.sort((a, b) => a.name.localeCompare(b.name)) };
      }
      case 'get_commands': return { commands: [
        ...session.extensionRunner.getRegisteredCommands().map(command => ({ name: command.invocationName,
          description: command.description, source: 'extension', sourceInfo: command.sourceInfo })),
        ...session.promptTemplates.map(template => ({ name: template.name, description: template.description,
          source: 'prompt', sourceInfo: template.sourceInfo })),
        ...session.resourceLoader.getSkills().skills.map(skill => ({ name: `skill:${skill.name}`,
          description: skill.description, source: 'skill', sourceInfo: skill.sourceInfo })),
      ] };
      case 'reload': await session.reload(); return;
      case 'list_packages': return { packages: packageManager(id).listConfiguredPackages() };
      case 'check_package_updates': return { updates: await packageManager(id).checkForAvailableUpdates() };
      case 'install_package': case 'remove_package': {
        if (command.local && !session.settingsManager.isProjectTrusted()) throw new Error('Project is not trusted. Save a trust decision and restart to change project packages.');
        const manager = packageManager(id);
        if (type === 'install_package') await manager.installAndPersist(command.source, { local: command.local });
        else if (!await manager.removeAndPersist(command.source, { local: command.local })) throw new Error('No matching package found');
        return;
      }
      case 'update_packages': await packageManager(id).update(command.source); return;
      case 'share': {
        const controller = new AbortController(); activeShare = controller;
        try {
          return await shareSession(pi, session, { signal: AbortSignal.any([
            controller.signal, ...(options.signal ? [options.signal] : []),
          ]) });
        } finally { activeShare = undefined; }
      }
      case 'login': case 'logout': case 'abort_login': throw new Error('Native auth mutations are not implemented; use Pi /login or /logout in the terminal, then restart the desktop session');
      default: throw new Error(`Unknown command: ${type}`);
    }
  };
  return async command => {
    const settings = host.session.settingsManager;
    const data = await dispatch(command);
    if (command.persist || ['set_model_thinking_level', 'set_cache_warming', 'set_default_project_trust',
      'set_auto_compaction', 'set_auto_retry', 'set_steering_mode', 'set_follow_up_mode',
      'install_package', 'remove_package', 'update_packages'].includes(command.type)) {
      await settings.flush();
    }
    return data;
  };
}
