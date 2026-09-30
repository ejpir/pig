import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile, access } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { JsonlReader } from '../src/protocol.mjs';

const cli = fileURLToPath(new URL('../src/cli.mjs', import.meta.url));
// PI_DESKTOP_BACKEND_BINARY runs the suite against the standalone build instead.
const binary = process.env.PI_DESKTOP_BACKEND_BINARY;
const [command, entry] = binary ? [binary, []] : [process.execPath, [cli]];
const extension = fileURLToPath(new URL('./fixtures/extension.ts', import.meta.url));
async function environment(t) {
  const root = await mkdtemp(join(tmpdir(), 'pi-backend-test-'));
  const project = join(root, 'project'); await mkdir(project);
  t.after(() => rm(root, { recursive: true, force: true }));
  return { root, project };
}
function peer(t, env, args = ['--no-session']) {
  // Allow only process/network plumbing, never provider credentials or config
  // pointers from the developer shell (including cloud SDK profiles).
  const plumbing = /^(PATH|SystemRoot|ComSpec|TEMP|TMP|TMPDIR|LANG|LC_.*|TZ|SSL_CERT_FILE|NODE_EXTRA_CA_CERTS|HTTPS?_PROXY|ALL_PROXY|NO_PROXY|https?_proxy|all_proxy|no_proxy)$/;
  const variables = { ...Object.fromEntries(Object.entries(process.env).filter(([key]) => plumbing.test(key))),
    HOME: join(env.root, 'home'), PI_CODING_AGENT_DIR: join(env.root, 'agent'),
    PI_OFFLINE: '1', PI_SKIP_VERSION_CHECK: '1' };
  const child = spawn(command, [...entry, '--mode', 'rpc', '--offline', ...args], {
    cwd: env.project, env: variables, stdio: ['pipe', 'pipe', 'pipe'],
  });
  const records = []; let stderr = ''; let sequence = 0;
  const pending = new Map();
  child.stdout.on('data', chunk => reader.push(chunk));
  child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-8192); });
  const reader = new JsonlReader(record => {
    const value = JSON.parse(record); records.push(value);
    const callback = value.type === 'response' ? pending.get(value.id) : undefined;
    if (callback) { pending.delete(value.id); callback.resolve(value); }
  });
  const exited = new Promise(resolve => child.on('exit', (code, signal) => {
    for (const callback of pending.values()) callback.reject(new Error(`Backend exited ${code}/${signal}: ${stderr}`));
    pending.clear(); resolve({ code, signal });
  }));
  t.after(async () => { child.kill('SIGKILL'); await exited; });
  return {
    child, records, exited, stderr: () => stderr,
    request(type, fields = {}) {
      const id = String(++sequence);
      return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => { pending.delete(id); reject(new Error(`Timed out: ${type}; ${stderr}`)); }, 25_000);
        pending.set(id, { resolve: value => { clearTimeout(timeout); resolve(value); }, reject: error => { clearTimeout(timeout); reject(error); } });
        child.stdin.write(JSON.stringify({ ...fields, type, id }) + '\n');
      });
    },
    async ok(type, fields) { const response = await this.request(type, fields); assert.equal(response.success, true, response.error); return response.data; },
    async waitEvent(predicate) {
      // A cold SDK/extension bootstrap can exceed five seconds on CI runners.
      // Keep this bounded by the same budget as correlated requests.
      const deadline = Date.now() + 25_000;
      while (Date.now() < deadline) {
        const record = records.find(predicate); if (record) return record;
        await new Promise(resolve => setTimeout(resolve, 25));
      }
      throw new Error(`Event not received; ${stderr}`);
    },
    async quit() { child.stdin.end(); assert.deepEqual(await exited, { code: 0, signal: null }, stderr); },
  };
}

test('published Pi SDK bootstrap, metadata, protocol identity and clean EOF shutdown', async t => {
  const env = await environment(t); const client = peer(t, env, ['--no-session', '--no-extensions', '--no-skills', '--no-context-files']);
  const info = await client.ok('get_backend_info'); assert.equal(info.piVersion, '0.99.1'); assert.equal(info.protocolVersion, 1); assert.equal(info.nodeVersion, binary ? info.nodeVersion : process.versions.node); assert.equal(typeof info.bunVersion, binary ? 'string' : 'undefined'); assert.deepEqual(info.features, ['fork_cwd']);
  assert.ok(info.commands.includes('list_packages')); assert.ok(info.commands.includes('share'));
  const state = await client.ok('get_state'); assert.equal(state.isStreaming, false); assert.ok(state.sessionId);
  assert.equal(state.sessionFile, undefined);
  // Agent core may expose its explicit "unknown" placeholder without a model.
  assert.ok(state.model === undefined || state.model.provider === 'unknown');
  assert.deepEqual((await client.ok('get_available_models')).models, [], 'No real provider may be available in this test');
  for (const command of ['get_messages', 'get_entries', 'get_tree', 'get_settings', 'get_session_stats',
    'get_available_models', 'get_available_thinking_levels', 'get_auth_providers', 'get_commands', 'list_packages', 'get_project_trust']) {
    assert.ok(await client.ok(command), command);
  }
  const link = { change: 'abc', commit: 'def', after: 1 };
  const { entryId } = await client.ok('append_custom_entry', { customType: 'pi-desktop-turn', data: link });
  assert.deepEqual((await client.ok('get_custom_entries', { customType: 'pi-desktop-turn' })).entries
    .map(entry => [entry.id, entry.data]), [[entryId, link]]);
  assert.deepEqual((await client.ok('get_messages')).messages, [], 'custom entries stay out of the context');
  const foreign = await client.request('append_custom_entry', { customType: 'plan-mode', data: {} });
  assert.equal(foreign.success, false, 'only the desktop\'s own entry types');
  const failed = await client.request('prompt', { message: 'No model is configured', images: [] });
  assert.equal(failed.success, false); assert.equal((await client.ok('get_state')).isStreaming, false);
  await client.quit();
});

test('models, settings, exact scope, handled preflight, reload and extension UI cancellation', async t => {
  const env = await environment(t); const client = peer(t, env, ['--no-session', '-e', extension]);
  const models = (await client.ok('get_available_models')).models.filter(model => model.provider === 'offline-desktop-fixture');
  assert.equal(models.length, 2);
  const first = await client.ok('get_state'); assert.equal(first.model.provider, 'offline-desktop-fixture');
  await client.ok('set_model', { provider: 'offline-desktop-fixture', modelId: 'small', persist: false });
  let settings = await client.ok('get_settings'); assert.equal(settings.defaultModel, undefined);
  await client.ok('set_scoped_models', { patterns: ['offline-desktop-fixture/small'], persist: true });
  settings = await client.ok('get_settings'); assert.deepEqual(settings.scopedModels, ['offline-desktop-fixture/small']);
  await client.ok('set_model_thinking_level', { provider: 'offline-desktop-fixture', modelId: 'small', level: 'low' });
  assert.equal((await client.ok('get_state')).thinkingLevel, 'low');
  const handled = await client.ok('prompt', { message: '/backend-handled' }); assert.equal(handled.disposition, 'handled');
  assert.equal((await client.ok('prompt', { message: 'fixture:handled' })).disposition, 'handled');
  assert.equal(client.records.some(record => record.type === 'agent_start'), false);
  assert.equal((await client.ok('get_messages')).messages.filter(message => message.role === 'user').length, 0);
  const waiting = client.request('prompt', { message: '/backend-confirm' });
  const dialog = await client.waitEvent(record => record.type === 'extension_ui_request' && record.method === 'confirm');
  const blocked = await client.request('reload'); assert.equal(blocked.success, false);
  assert.equal((await client.ok('get_state')).isStreaming, false);
  client.child.stdin.write(JSON.stringify({ type: 'extension_ui_response', id: dialog.id, cancelled: true }) + '\n');
  assert.equal((await waiting).data.disposition, 'handled');
  await client.ok('reload');
  const catalog = await client.ok('get_commands'); assert.ok(catalog.commands.some(command => command.name === 'backend-confirm'));
  assert.equal(JSON.stringify(client.records).includes('offline-fixture-not-a-real-key'), false);
  await client.quit();
  const saved = JSON.parse(await readFile(join(env.root, 'agent', 'settings.json'), 'utf8'));
  assert.deepEqual(saved.enabledModels, ['offline-desktop-fixture/small']);
  assert.equal(saved.modelThinkingLevels['offline-desktop-fixture/small'], 'low');
});

test('a prompt that reaches the model reports started; the fixture model then fails offline', async t => {
  const env = await environment(t); const client = peer(t, env, ['--no-session', '-e', extension]);
  await client.ok('set_model', { provider: 'offline-desktop-fixture', modelId: 'small', persist: false });
  assert.equal((await client.ok('prompt', { message: 'Run the fixture model' })).disposition, 'started');
  await client.waitEvent(record => record.type === 'agent_end');
  assert.equal((await client.ok('get_state')).isStreaming, false);
  await client.quit();
});

test('active tool metadata reflects builtins, extensions, loadout changes and explicit no-tools', async t => {
  const env = await environment(t); const client = peer(t, env, ['--no-session', '-e', extension]);
  let tools = (await client.ok('get_state')).activeTools;
  assert.ok(tools.some(tool => tool.name === 'read' && tool.sourceInfo.source === 'builtin'));
  const custom = tools.find(tool => tool.name === 'backend_probe');
  assert.equal(custom.description, 'Metadata-only fixture tool; validation never executes it.');
  assert.equal(custom.sourceInfo.path, extension);
  assert.equal(tools.some(tool => 'execute' in tool || 'parameters' in tool), false);
  assert.equal((await client.ok('prompt', { message: '/backend-tools' })).disposition, 'handled');
  tools = (await client.ok('get_state')).activeTools;
  assert.deepEqual(tools.map(tool => tool.name), ['read', 'backend_probe']);
  await client.ok('reload');
  assert.deepEqual((await client.ok('get_state')).activeTools.map(tool => tool.name), ['read', 'backend_probe']);
  assert.equal(client.records.some(record => ['agent_start', 'tool_execution_start'].includes(record.type)), false);
  await client.quit();
  const disabled = peer(t, env, ['--no-session', '--no-tools', '--no-extensions']);
  assert.deepEqual((await disabled.ok('get_state')).activeTools, []);
  await disabled.quit();
});

test('shell streams, preserves context policy and failures, handles extensions and aborts without a model', async t => {
  const env = await environment(t); const client = peer(t, env, ['--no-session', '-e', extension]);
  const running = client.request('bash', { command: "printf 'shell begin\\n'; sleep 0.4; printf 'shell end\\n'" });
  const chunk = await client.waitEvent(record => record.type === 'bash_execution_update');
  assert.equal(typeof chunk.id, 'string'); assert.ok(chunk.delta.includes('shell begin'));
  assert.equal((await client.ok('get_state')).isBashRunning, true);
  assert.equal((await client.request('reload')).success, false);
  const result = await running; assert.equal(result.success, true); assert.equal(result.data.exitCode, 0);
  assert.equal(result.data.output, 'shell begin\nshell end\n');
  await client.ok('bash', { command: "printf 'private output\\n'; exit 7", excludeFromContext: true });
  let messages = (await client.ok('get_messages')).messages.filter(message => message.role === 'bashExecution');
  assert.equal(messages.length, 2); assert.equal(messages[0].excludeFromContext, false);
  assert.equal(messages[1].excludeFromContext, true); assert.equal(messages[1].exitCode, 7);
  assert.equal(messages[1].output, 'private output\n');
  const handled = await client.ok('bash', { command: 'fixture:bash' }); assert.equal(handled.output, 'extension handled shell\n');
  messages = (await client.ok('get_messages')).messages.filter(message => message.role === 'bashExecution');
  assert.equal(messages.at(-1).command, 'fixture:bash');
  const aborting = client.request('bash', { command: "printf 'cancellable\\n'; sleep 20" });
  await client.waitEvent(record => record.type === 'bash_execution_update' && record.delta.includes('cancellable'));
  await client.ok('abort_bash'); assert.equal((await aborting).data.cancelled, true);
  assert.equal((await client.ok('get_state')).isBashRunning, false);
  assert.equal(client.records.some(record => ['agent_start', 'tool_execution_start'].includes(record.type)), false);
  assert.equal((await client.request('bash', { command: '', excludeFromContext: 'yes' })).success, false);
  await client.quit();
});

test('standard extension select returns the exact choice, and load metadata contains no executable definitions', async t => {
  const env = await environment(t); const client = peer(t, env, ['--no-session', '-e', extension]);
  const pending = client.request('prompt', { message: '/backend-select' });
  const dialog = await client.waitEvent(record => record.type === 'extension_ui_request' && record.method === 'select');
  assert.deepEqual(dialog.options, ['Allow once', 'Allow for project', 'Block']);
  client.child.stdin.write(JSON.stringify({ type: 'extension_ui_response', id: dialog.id, value: 'Block' }) + '\n');
  assert.equal((await pending).data.disposition, 'handled');
  assert.ok(client.records.some(record => record.method === 'notify' && record.message === 'Block'));
  const trust = await client.ok('get_project_trust');
  const fixture = trust.loadedExtensions.find(resource => resource.path === extension);
  assert.equal(fixture.status, 'loaded'); assert.ok(fixture.commands.includes('/backend-select'));
  assert.equal('handlers' in fixture || 'tools' in fixture || 'execute' in fixture, false);
  assert.deepEqual(trust.projectSettings, {});
  assert.equal(client.records.some(record => record.type === 'agent_start'), false);
  await client.quit();
});

test('extension load failures remain inspectable and reported without killing the session', async t => {
  const env = await environment(t);
  const local = join(env.project, '.pi'); await mkdir(join(local, 'extensions'), { recursive: true });
  await writeFile(join(local, 'extensions', 'broken.ts'), `throw new Error('Synthetic inspectable load failure');`);
  await writeFile(join(local, 'extensions', 'good.ts'), `export default pi => pi.registerCommand('healthy-resource', {handler: async()=>{}});`);
  await writeFile(join(env.project, 'AGENTS.md'), 'Isolated context metadata fixture.');
  await writeFile(join(local, 'settings.json'), JSON.stringify({ defaultThinkingLevel: 'medium', apiKey: 'must-not-be-reported' }));
  const client = peer(t, env, ['--approve']);
  await client.ok('get_state');
  const trust = await client.ok('get_project_trust');
  assert.ok(trust.loadedExtensions.some(resource => resource.status === 'load-error' && resource.error.includes('Synthetic inspectable')));
  assert.ok(trust.loadedExtensions.some(resource => resource.status === 'loaded' && resource.commands.includes('/healthy-resource')));
  assert.deepEqual(trust.contextFiles, [join(env.project, 'AGENTS.md')]);
  assert.equal(trust.projectSettings.defaultThinkingLevel, 'medium');
  assert.equal('apiKey' in trust.projectSettings, false);
  assert.ok(client.records.some(record => record.method === 'notify' && record.notifyType === 'error' && record.message.includes('Synthetic inspectable')));
  assert.equal(client.records.some(record => record.type === 'agent_start'), false);
  await client.quit();
});

test('saved trust stays pending until restart; untrusted project code never loads', async t => {
  const env = await environment(t); const local = join(env.project, '.pi'); await mkdir(join(local, 'extensions'), { recursive: true });
  const marker = join(env.project, 'loaded');
  await writeFile(join(local, 'extensions', 'unsafe.ts'), `import {writeFileSync} from 'node:fs'; export default function(){writeFileSync(${JSON.stringify(marker)}, 'loaded');}`);
  const client = peer(t, env);
  let trust = await client.ok('get_project_trust'); assert.equal(trust.trusted, false); assert.equal(trust.hasProjectResources, true);
  await assert.rejects(access(marker));
  const invalid = await client.request('set_project_trust', { choice: 'not-valid' }); assert.equal(invalid.success, false);
  await client.ok('set_project_trust', { choice: 'trust' });
  trust = await client.ok('get_project_trust'); assert.equal(trust.trusted, false); assert.equal(trust.savedDecision.decision, true);
  const mutation = await client.request('install_package', { source: './forbidden', local: true }); assert.equal(mutation.success, false);
  await client.ok('reload'); await assert.rejects(access(marker)); await client.quit();
  const restarted = peer(t, env); trust = await restarted.ok('get_project_trust'); assert.equal(trust.trusted, true);
  assert.equal(await readFile(marker, 'utf8'), 'loaded'); await restarted.quit();
});

test('local package configuration changes are explicit; removal and reload are separate', async t => {
  const env = await environment(t); const pkg = join(env.root, 'package'); await mkdir(pkg);
  await writeFile(join(pkg, 'package.json'), JSON.stringify({ name: 'offline-backend-fixture', pi: { extensions: ['./extension.ts'] } }));
  await writeFile(join(pkg, 'extension.ts'), `export default pi => pi.registerCommand('local-package-probe', {handler: async()=>{}});`);
  const client = peer(t, env);
  await client.ok('install_package', { source: pkg, local: false });
  assert.ok((await client.ok('list_packages')).packages.some(item => item.installedPath === pkg));
  assert.equal((await client.ok('get_commands')).commands.some(item => item.name === 'local-package-probe'), false);
  await client.ok('reload'); assert.equal((await client.ok('get_commands')).commands.some(item => item.name === 'local-package-probe'), true);
  await client.ok('remove_package', { source: pkg, local: false });
  assert.equal((await client.ok('get_commands')).commands.some(item => item.name === 'local-package-probe'), true);
  await client.ok('reload'); assert.equal((await client.ok('get_commands')).commands.some(item => item.name === 'local-package-probe'), false);
  await client.quit(); assert.ok(await readFile(join(pkg, 'extension.ts'), 'utf8'));
});

test('a fork into another folder is a new session file that names that folder', async t => {
  const env = await environment(t); const file = join(env.root, 'history.jsonl'); const timestamp = new Date().toISOString();
  const other = join(env.root, 'workspace'); await mkdir(other);
  const usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
  const answer = (id, parentId, text) => ({ type: 'message', id, parentId, timestamp, message: { role: 'assistant', content: [{ type: 'text', text }],
    api: 'openai-completions', provider: 'fixture', model: 'offline', stopReason: 'stop', timestamp: Date.now(), usage } });
  const records = [
    { type: 'session', version: 3, id: 'offline-fork', timestamp, cwd: env.project },
    { type: 'message', id: 'user1', parentId: null, timestamp, message: { role: 'user', content: 'first', timestamp: Date.now() } },
    answer('answer1', 'user1', 'One'),
    { type: 'message', id: 'user2', parentId: 'answer1', timestamp, message: { role: 'user', content: [{ type: 'text', text: 'second' }], timestamp: Date.now() } },
    answer('answer2', 'user2', 'Two'),
  ];
  await writeFile(file, records.map(record => JSON.stringify(record)).join('\n') + '\n');
  const client = peer(t, env, ['--session', file, '--no-extensions']);
  const fork = await client.ok('fork', { entryId: 'user2', cwd: other });
  assert.equal(fork.text, 'second');
  const lines = (await readFile(fork.sessionPath, 'utf8')).trim().split('\n').map(line => JSON.parse(line));
  assert.equal(lines[0].cwd, await realpath(other)); assert.equal(lines[0].id, fork.sessionId);
  assert.deepEqual(lines.slice(1).map(entry => entry.id), ['user1', 'answer1']);
  assert.equal((await client.ok('get_state')).sessionId, 'offline-fork', 'this process keeps its session');
  assert.equal((await client.request('fork', { entryId: 'answer1', cwd: other })).success, false, 'only before a user message');
  await client.quit();
});

test('saved history preserves entries and Unicode; navigation/replacement rebind identity', async t => {
  const env = await environment(t); const file = join(env.root, 'history.jsonl'); const timestamp = new Date().toISOString();
  const header = { type: 'session', version: 3, id: 'offline-history', timestamp, cwd: env.project };
  const thinking = { type: 'thinking_level_change', id: 'thinking1', parentId: null, timestamp, thinkingLevel: 'off' };
  const user = { type: 'message', id: 'user1', parentId: thinking.id, timestamp, message: { role: 'user', content: 'one\u2028two\u2029three', timestamp: Date.now() } };
  const assistant = { type: 'message', id: 'answer1', parentId: 'user1', timestamp, message: { role: 'assistant', content: [{ type: 'text', text: 'Offline answer' }],
    api: 'openai-completions', provider: 'fixture', model: 'offline', stopReason: 'stop', timestamp: Date.now(),
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } } } };
  await writeFile(file, [header, thinking, user, assistant].map(record => JSON.stringify(record)).join('\n') + '\n');
  const client = peer(t, env, ['--session', file, '--no-extensions']);
  assert.equal((await client.ok('get_state')).sessionId, header.id);
  assert.equal((await client.ok('get_messages')).messages.find(message => message.role === 'user').content, user.message.content);
  assert.equal((await client.ok('get_entries')).leafId, assistant.id);
  await client.ok('set_label', { targetId: user.id, label: 'original' });
  await client.ok('navigate_tree', { targetId: user.id, summarize: false });
  // Pi navigation to a user entry returns to its parent and restores the draft.
  assert.equal((await client.ok('get_entries')).leafId, thinking.id);
  await client.ok('set_session_name', { name: 'First\u2028Second\u2029Third' });
  assert.equal((await client.ok('get_state')).sessionName, 'First\u2028Second\u2029Third');
  const deletion = await client.request('delete_session', { sessionPath: file }); assert.equal(deletion.success, false);
  await client.ok('new_session'); assert.notEqual((await client.ok('get_state')).sessionId, header.id);
  await client.ok('switch_session', { sessionPath: file }); assert.equal((await client.ok('get_state')).sessionId, header.id);
  const clone = await client.ok('clone'); assert.equal(clone.cancelled, false); assert.notEqual((await client.ok('get_state')).sessionId, header.id);
  const malformed = join(env.root, 'invalid.jsonl'); await writeFile(malformed, '{}\n');
  const invalid = await client.request('switch_session', { sessionPath: malformed }); assert.equal(invalid.success, false);
  const missing = join(env.root, 'nonexistent.jsonl');
  assert.equal((await client.request('set_session_name', { sessionPath: missing, name: 'wrong' })).success, false);
  await assert.rejects(access(missing));
  await client.quit();
});

test('EOF cancels an outstanding extension dialog and exits without hanging', async t => {
  const env = await environment(t); const client = peer(t, env, ['--no-session', '-e', extension]);
  await client.ok('get_state');
  const waiting = client.request('prompt', { message: '/backend-confirm' });
  await client.waitEvent(record => record.method === 'confirm');
  await client.quit(); await waiting.catch(() => {});
});
