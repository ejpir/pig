import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { shareSession } from '../src/share.mjs';
import { loadPi } from '../src/compat.mjs';

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'pi-share-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const pi = {
    radiusGateway: 'https://radius.invalid',
    exportSessionForShare(file) { return import('node:fs').then(() => file); },
    getAuthCredential: auth => auth?.token,
    getShareViewerUrl: id => `https://pi.invalid/share/${id}`,
  };
  // Synchronous branch exporter, like Pi's actual helper.
  const fs = await import('node:fs');
  pi.exportSessionForShare = file => fs.writeFileSync(file, '{"type":"session","id":"fixture"}\n');
  const session = {
    modelRuntime: { getProvider: () => undefined, getAuth: async () => ({ token: 'synthetic-share-token' }) },
    exportToHtml: file => writeFile(file, '<html>Local test fixture, no real conversation.</html>'),
  };
  return { pi, session, root };
}

test('Radius sharing uses Pi export/auth, uploads organization JSONL, returns canonical URL and cleans up', async t => {
  const { pi, session, root } = await fixture(t);
  session.modelRuntime.getProvider = () => ({ id: 'radius' });
  let uploads = 0;
  const result = await shareSession(pi, session, { tmpdir: root,
    run: () => { throw new Error('Radius must not invoke gh'); },
    fetch: async (endpoint, request) => {
      uploads++;
      assert.equal(endpoint.origin, 'https://radius.invalid');
      assert.equal(endpoint.searchParams.get('visibility'), 'organization');
      assert.equal(request.headers.Authorization, 'Bearer synthetic-share-token');
      assert.equal(request.headers['Content-Type'], 'application/x-ndjson');
      assert.equal(request.body.toString(), '{"type":"session","id":"fixture"}\n');
      return { ok: true, status: 200, json: async () => ({ artifact: { canonical_url: 'https://radius.invalid/artifact/fixture' } }) };
    },
  });
  assert.deepEqual(result, { destination: 'radius', url: 'https://radius.invalid/artifact/fixture' });
  assert.equal(uploads, 1); assert.deepEqual(await readdir(root), []);
});

test('private gist fallback uploads Pi HTML, uses argv without a shell, returns viewer/gist URLs', async t => {
  const { pi, session, root } = await fixture(t); const calls = [];
  const result = await shareSession(pi, session, { tmpdir: root,
    fetch: () => { throw new Error('No Radius request expected'); },
    run: async (program, args, options) => {
      assert.equal(program, 'gh'); assert.ok(options.signal); calls.push(args);
      if (args[0] === 'auth') return { code: 0, stdout: '', stderr: '' };
      assert.deepEqual(args.slice(0, 3), ['gist', 'create', '--public=false']);
      assert.match(await readFile(args[3], 'utf8'), /Local test fixture/);
      return { code: 0, stdout: 'https://gist.github.com/fixture/aabbcc\n', stderr: '' };
    },
  });
  assert.deepEqual(result, { destination: 'gist', gistUrl: 'https://gist.github.com/fixture/aabbcc', url: 'https://pi.invalid/share/aabbcc' });
  assert.equal(calls.length, 2); assert.deepEqual(await readdir(root), []);
});

for (const scenario of ['radius-error', 'gh-unauthenticated', 'gist-error', 'bad-gist-url', 'cancel']) {
  test(`sharing ${scenario} fails honestly and removes sensitive temporary exports`, async t => {
    const { pi, session, root } = await fixture(t); const controller = new AbortController();
    if (scenario === 'radius-error' || scenario === 'cancel') session.modelRuntime.getProvider = () => ({ id: 'radius' });
    const result = shareSession(pi, session, { tmpdir: root, signal: controller.signal,
      fetch: async () => {
        if (scenario === 'cancel') controller.abort();
        return { ok: false, status: 403, json: async () => ({ error: 'Do not echo synthetic-share-token' }) };
      },
      run: async (_, args) => {
        if (args[0] === 'auth') return { code: scenario === 'gh-unauthenticated' ? 1 : 0 };
        return { code: scenario === 'gist-error' ? 1 : 0, stdout: 'https://evil.invalid/aabbcc', stderr: 'Do not echo credentials' };
      },
    });
    await assert.rejects(result, error => { assert.ok(!error.message.includes('synthetic-share-token')); return true; });
    assert.deepEqual(await readdir(root), []);
  });
}

test('published Pi private exporter retains branch metadata without a TUI or network', async t => {
  const pi = await loadPi(); const root = await mkdtemp(join(tmpdir(), 'pi-real-export-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const project = join(root, 'project'); await mkdir(project);
  const sessionManager = pi.SessionManager.inMemory(project);
  sessionManager.appendMessage({ role: 'user', content: 'Synthetic offline export only', timestamp: Date.now() });
  const session = { sessionManager, state: { systemPrompt: 'Fixture system prompt', tools: [] } };
  const file = join(root, 'export.jsonl'); pi.exportSessionForShare(file, session);
  const records = (await readFile(file, 'utf8')).trim().split('\n').map(line => JSON.parse(line));
  assert.ok(records.some(record => record.customType === 'pi.share' && record.data.systemPrompt === 'Fixture system prompt'));
  assert.ok(records.some(record => record.message?.content === 'Synthetic offline export only'));
});
