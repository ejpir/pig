// Headless adaptation of Pi's MIT-licensed session sharing flow. Pi owns the
// branch export and credential resolution; no TUI components or auth-file reads.
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export function runCommand(program, args, { signal } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, { signal, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
    let stdout = ''; let stderr = '';
    child.stdout.on('data', bytes => { stdout = (stdout + bytes.toString('utf8')).slice(-8192); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes.toString('utf8')).slice(-2048); });
    child.on('error', reject);
    child.on('close', code => resolve({ code, stdout, stderr }));
  });
}
function url(value) {
  if (typeof value !== 'string') throw new Error('Share service did not return a URL');
  const parsed = new URL(value);
  if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') throw new Error('Invalid share URL');
  return value;
}

export async function shareSession(pi, session, options = {}) {
  const fetchRequest = options.fetch ?? fetch;
  const run = options.run ?? runCommand;
  // Bounded within the desktop's existing 30s request deadline. Shutdown/Abort
  // cancels uploads and gh, and finally always removes the temporary exports.
  const signal = AbortSignal.any([AbortSignal.timeout(25_000), ...(options.signal ? [options.signal] : [])]);
  signal.throwIfAborted();
  const directory = await mkdtemp(join(options.tmpdir ?? tmpdir(), 'pi-desktop-share-'));
  const jsonl = join(directory, 'session.jsonl');
  const html = join(directory, 'session.html');
  try {
    pi.exportSessionForShare(jsonl, session);
    const token = session.modelRuntime.getProvider('radius')
      ? pi.getAuthCredential(await session.modelRuntime.getAuth('radius', { minOAuthValidityMs: 5 * 60_000, signal }))
      : undefined;
    signal.throwIfAborted();
    if (token) {
      const endpoint = new URL('/v1/artifacts', pi.radiusGateway);
      endpoint.searchParams.set('visibility', 'organization');
      endpoint.searchParams.set('title', 'Pi session');
      const body = await readFile(jsonl);
      const response = await fetchRequest(endpoint, { method: 'POST', signal,
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/x-ndjson',
          'Content-Length': String(body.byteLength) }, body });
      const data = await response.json().catch(() => null);
      signal.throwIfAborted();
      if (!response.ok || !data?.artifact) {
        // Do not echo a remote response body; it might contain credential material.
        throw new Error(`Radius share upload failed (HTTP ${response.status})`);
      }
      return { destination: 'radius', url: url(data.artifact.canonical_url) };
    }
    let auth;
    try { auth = await run('gh', ['auth', 'status'], { signal }); }
    catch (error) {
      signal.throwIfAborted();
      if (error.code === 'ENOENT') throw new Error('GitHub CLI (gh) is not installed');
      throw error;
    }
    if (auth.code !== 0) throw new Error('GitHub CLI is not logged in. Run gh auth login first.');
    await session.exportToHtml(html);
    signal.throwIfAborted();
    const gist = await run('gh', ['gist', 'create', '--public=false', html], { signal });
    signal.throwIfAborted();
    if (gist.code !== 0) throw new Error('Creating the private gist failed');
    const gistUrl = url(gist.stdout.trim());
    const parsed = new URL(gistUrl);
    const id = parsed.pathname.split('/').filter(Boolean).at(-1);
    if (parsed.hostname !== 'gist.github.com' || !id || !/^[a-f\d]+$/i.test(id)) throw new Error('Invalid gist URL from GitHub CLI');
    return { destination: 'gist', gistUrl, url: url(pi.getShareViewerUrl(id)) };
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}
