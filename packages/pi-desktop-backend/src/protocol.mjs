export const MAX_RECORD_BYTES = 16 * 1024 * 1024;
export const MAX_PENDING_BYTES = 32 * 1024 * 1024;

// Split bytes only on LF. Unicode line/paragraph separators are string data.
export class JsonlReader {
  parts = [];
  size = 0;
  constructor(onLine, limit = MAX_RECORD_BYTES) { this.onLine = onLine; this.limit = limit; }
  push(chunk) {
    chunk = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    let start = 0;
    for (let index = 0; index < chunk.length; index++) {
      if (chunk[index] !== 10) continue;
      this.append(chunk.subarray(start, index));
      if (this.size + 1 > this.limit) throw new Error('RPC input record exceeds byte limit');
      this.onLine(new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(this.parts, this.size)));
      this.parts = []; this.size = 0; start = index + 1;
    }
    this.append(chunk.subarray(start));
  }
  append(bytes) {
    if (this.size + bytes.length > this.limit) throw new Error('RPC input record exceeds byte limit');
    if (bytes.length) { this.parts.push(bytes); this.size += bytes.length; }
  }
  end() { if (this.size) throw new Error('RPC stdin ended with an unterminated record'); }
}

export function createOutput(guard, onError) {
  let pendingBytes = 0;
  let failed = false;
  const output = record => {
    if (failed) return;
    const text = JSON.stringify(record) + '\n';
    const bytes = Buffer.byteLength(text);
    if (bytes > MAX_RECORD_BYTES || pendingBytes + bytes > MAX_PENDING_BYTES) {
      failed = true; onError(new Error('RPC output exceeds bounded buffer')); return;
    }
    pendingBytes += bytes;
    guard.writeRawStdout(text);
    void guard.waitForRawStdoutBackpressure().then(() => { pendingBytes -= bytes; }, error => {
      if (!failed) { failed = true; onError(error); }
    });
  };
  output.drain = () => guard.waitForRawStdoutBackpressure();
  return output;
}

export async function bindSession(pi, host, output, interaction, shutdownRequested) {
  let detach = () => {};
  const rebind = async () => {
    detach();
    interaction.cancel();
    const session = host.session;
    const unsubscribe = session.subscribe(event => {
      output(pi.toJsonEvent(event));
      if (event.type === 'agent_settled') shutdownRequested.check();
    });
    const unsubscribeBackpressure = session.agent.subscribe(async () => { await output.drain(); });
    detach = () => { unsubscribe(); unsubscribeBackpressure(); };
    await session.bindExtensions({
      mode: 'rpc', uiContext: interaction.ui,
      commandContextActions: {
        waitForIdle: () => session.waitForIdle(),
        newSession: options => host.newSession(options),
        switchSession: (file, options) => host.switchSession(file, options),
        fork: async (entry, options) => { const result = await host.fork(entry, options); return { cancelled: result.cancelled }; },
        navigateTree: async (entry, options) => { const result = await session.navigateTree(entry, options); return { cancelled: result.cancelled }; },
        reload: () => session.reload(),
      },
      shutdownHandler: () => { shutdownRequested.requested = true; },
      onError: error => output({ type: 'extension_error', extensionPath: error.extensionPath, event: error.event, error: error.error }),
    });
    for (const diagnostic of host.diagnostics ?? []) {
      interaction.ui.notify(diagnostic.message, diagnostic.type);
    }
  };
  host.setRebindSession(rebind);
  await rebind();
  return () => { detach(); interaction.cancel(); host.setRebindSession(undefined); };
}

export function serve(input, output, handle, interaction, exclusive, close) {
  const inFlight = new Set();
  let mutation;
  let stopped = false;
  const response = (command, success, fields) => output({ type: 'response', id: typeof command?.id === 'string' ? command.id : undefined,
    command: typeof command?.type === 'string' ? command.type : 'parse', success, ...fields });
  const line = text => {
    if (stopped) return;
    let command;
    try { command = JSON.parse(text); } catch { response(undefined, false, { error: 'Invalid JSON record' }); return; }
    // Dialog answers must bypass mutation gates, otherwise confirmations deadlock.
    if (command?.type === 'extension_ui_response') { interaction.respond(command); return; }
    if (inFlight.size >= 64) { response(command, false, { error: 'Too many outstanding RPC commands' }); return; }
    const mutating = exclusive.has(command?.type);
    if (mutating && mutation) { response(command, false, { error: 'Another session mutation or prompt preflight is in progress' }); return; }
    const task = Promise.resolve().then(() => handle(command)).then(data => {
      response(command, true, data === undefined ? {} : { data });
    }, error => { response(command, false, { error: error instanceof Error ? error.message : 'Command failed' }); })
      .finally(() => {
        inFlight.delete(task);
        if (mutation === task) mutation = undefined;
        close.check?.();
      });
    inFlight.add(task);
    if (mutating) mutation = task;
  };
  const reader = new JsonlReader(line);
  const stop = error => {
    if (stopped) return;
    stopped = true; cleanup();
    if (error) response(undefined, false, { error: error.message });
    close(error ? 1 : 0);
  };
  const data = chunk => { try { reader.push(chunk); } catch (error) { stop(error); } };
  const end = () => { try { reader.end(); stop(); } catch (error) { stop(error); } };
  const cleanup = () => { input.off('data', data); input.off('end', end); input.off('error', stop); input.pause(); };
  input.on('data', data); input.on('end', end); input.on('error', stop);
  return cleanup;
}
