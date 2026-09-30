#!/usr/bin/env node
import { outputGuard, loadPi } from './compat.mjs';
import { parseOptions, createRuntime } from './runtime.mjs';
import { createCommands, EXCLUSIVE } from './commands.mjs';
import { createUi } from './ui.mjs';
import { bindSession, createOutput, serve } from './protocol.mjs';

outputGuard.takeOverStdout();
process.title = 'pi-desktop-backend';
process.env.PI_CODING_AGENT = 'true';
process.env.AI_AGENT = 'pi';
if (process.argv.includes('--offline')) {
  process.env.PI_OFFLINE = '1';
  process.env.PI_SKIP_VERSION_CHECK = '1';
}
let host;
let pi;
let detachSession = () => {};
let detachInput = () => {};
let interaction;
let closing = false;
let output;
const operations = new AbortController();
const shutdownRequested = { requested: false, check() { if (this.requested) void shutdown(); } };
async function shutdown(code = 0) {
  if (closing) return;
  closing = true;
  operations.abort();
  detachInput();
  interaction?.cancel();
  // Extensions and package operations can hang. Never leave a closed window's
  // backend running forever; the desktop process-tree supervisor is the backstop.
  const timeout = setTimeout(() => { pi?.killTrackedDetachedChildren(); process.exit(code || 1); }, 5_000);
  try {
    if (host) {
      host.session.abortBash();
      await host.session.abort();
      detachSession();
      await host.dispose();
      await host.session.settingsManager.flush();
    }
    pi?.killTrackedDetachedChildren();
    await outputGuard.flushRawStdout();
  } catch {
    console.error('Desktop backend shutdown could not finish cleanly');
    code ||= 1;
  }
  clearTimeout(timeout);
  process.exit(code);
}
for (const [signal, code] of [['SIGTERM', 143], ['SIGINT', 130], ...(process.platform === 'win32' ? [] : [['SIGHUP', 129]])]) {
  process.on(signal, () => { void shutdown(code); });
}
try {
  pi = await loadPi();
  const options = parseOptions(pi, process.argv.slice(2));
  if (options.help) {
    console.error('pi-desktop-backend [--mode rpc] [--session /absolute/session.jsonl] [-e extension] [--offline]\nLocal JSONL stdin/stdout adapter. See packages/pi-desktop-backend/README.md.');
    process.exit(0);
  }
  if (options.version) { console.error(`pi-desktop-backend 0.1.0 (Pi ${pi.VERSION})`); process.exit(0); }
  output = createOutput(outputGuard, error => { console.error(error.message); void shutdown(1); });
  host = await createRuntime(pi, options);
  interaction = createUi(output, pi.theme);
  detachSession = await bindSession(pi, host, output, interaction, shutdownRequested);
  const close = code => { void shutdown(code); };
  close.check = () => shutdownRequested.check();
  detachInput = serve(process.stdin, output, createCommands(pi, host, output, { signal: operations.signal }), interaction, EXCLUSIVE, close);
  shutdownRequested.check();
} catch (error) {
  console.error(`Desktop backend startup failed: ${error instanceof Error ? error.message : 'Unknown error'}`);
  await shutdown(1);
}
