import assert from 'node:assert/strict';
import { PassThrough } from 'node:stream';
import { setTimeout as delay } from 'node:timers/promises';
import test from 'node:test';
import { JsonlReader, createOutput, serve } from '../src/protocol.mjs';
import { createUi } from '../src/ui.mjs';
import { validate } from '../src/commands.mjs';
import { promptWithDisposition } from '../src/compat.mjs';

test('JSONL frames bytes only on LF, preserves split UTF-8/U+2028/U+2029 and CRLF', () => {
  const lines = [];
  const reader = new JsonlReader(line => lines.push(JSON.parse(line)));
  const record = { type: 'fixture', message: '日本語\u2028one\u2029two' };
  const bytes = Buffer.from(JSON.stringify(record) + '\r\n');
  for (const byte of bytes) reader.push(Buffer.from([byte]));
  reader.end();
  assert.deepEqual(lines, [record]);
});

test('input bounds and incomplete records fail closed', () => {
  const reader = new JsonlReader(() => {}, 5);
  reader.push('12345');
  assert.throws(() => reader.push('6'), /byte limit/);
  assert.throws(() => reader.end(), /unterminated/);
  assert.throws(() => new JsonlReader(() => {}).push(Buffer.from([0xff, 10])), /encoded data/);
});

test('output buffer is bounded while the consumer is stalled', async () => {
  const writes = []; let release;
  const blocked = new Promise(resolve => { release = resolve; });
  const errors = [];
  const guard = { writeRawStdout: text => writes.push(text), waitForRawStdoutBackpressure: () => blocked };
  const output = createOutput(guard, error => errors.push(error));
  const record = { type: 'fixture', body: 'x'.repeat(12 * 1024 * 1024) };
  output(record); output(record); output(record); output({ type: 'ignored' });
  assert.equal(writes.length, 2);
  assert.match(errors[0].message, /bounded buffer/);
  release(); await output.drain();
});

test('shape validation rejects invalid trust choices/booleans/images before mutation', () => {
  for (const record of [null, [], { type: 'set_project_trust', choice: 'oops' },
    { type: 'install_package', source: './local', local: 'true' }, { type: 'set_auto_retry' },
    { type: 'prompt', message: 'fixture', images: [{ type: 'image', data: 2 }] },
    { type: 'set_scoped_models', patterns: [42] }]) assert.throws(() => validate(record));
});

test('dialog replies bypass mutation gate; reads work while mutations serialize', async () => {
  const input = new PassThrough(); const records = []; const output = record => records.push(record);
  const interaction = createUi(output, {});
  const cleanup = serve(input, output, async command => {
    if (command.type === 'mutate') return { confirmed: await interaction.ui.confirm('Fixture', 'Cancel?') };
    return { read: true };
  }, interaction, new Set(['mutate']), () => {});
  input.write('{"type":"mutate","id":"one"}\n'); await delay(0);
  input.write('{"type":"mutate","id":"two"}\n{"type":"read","id":"read"}\n'); await delay(0);
  assert.equal(records.find(record => record.id === 'two').success, false);
  assert.equal(records.find(record => record.id === 'read').success, true);
  const request = records.find(record => record.type === 'extension_ui_request');
  input.write(JSON.stringify({ type: 'extension_ui_response', id: request.id, cancelled: true }) + '\n');
  await delay(0);
  assert.deepEqual(records.find(record => record.id === 'one').data, { confirmed: false });
  cleanup();
});

test('dialog cancellation, timeout and abort resolve defaults', async () => {
  const interaction = createUi(() => {}, {});
  const controller = new AbortController();
  const waiting = interaction.ui.confirm('Fixture', '?', { signal: controller.signal });
  controller.abort(); assert.equal(await waiting, false);
  assert.equal(await interaction.ui.input('Fixture', '', { timeout: 1 }), undefined);
  const editor = interaction.ui.editor('Fixture', 'original'); interaction.cancel();
  assert.equal(await editor, undefined);
});

function sessionFor(outcome) {
  const subscribers = new Set();
  return {
    subscribe(fn) { subscribers.add(fn); return () => subscribers.delete(fn); },
    _runAgentPrompt() { return outcome === 'nested' ? Promise.resolve() : new Promise(() => {}); },
    async prompt(text, options) {
      assert.equal(options.source, 'rpc');
      if (outcome === 'nested') await this._runAgentPrompt([]);
      if (outcome === 'error') { options.preflightResult(false); throw new Error('preflight failed'); }
      if (outcome === 'queued') for (const fn of subscribers) fn({ type: 'queue_update', steering: [text], followUp: [] });
      options.preflightResult(true);
      if (outcome === 'started') await this._runAgentPrompt([]);
    },
  };
}
for (const outcome of ['started', 'handled', 'queued', 'nested', 'error']) {
  test(`released SDK boolean preflight becomes truthful ${outcome} disposition`, async () => {
    const session = sessionFor(outcome); const original = session._runAgentPrompt;
    const result = new Promise((resolve, reject) => promptWithDisposition(session,
      { message: 'fixture', streamingBehavior: 'steer' }, resolve, reject));
    if (outcome === 'error') await assert.rejects(result, /preflight failed/);
    else assert.equal(await result, outcome === 'nested' ? 'handled' : outcome);
    assert.equal(session._runAgentPrompt, original);
  });
}
