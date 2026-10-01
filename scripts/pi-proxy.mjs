#!/usr/bin/env node
// Runs pi for the native capture scripts (as PI_DESKTOP_RPC_ENTRY) and logs what passes
// between the desktop and pi: pi's stdin and stdout, and the desktop channel to Pi
// Desktop's extension, which this forwards through a socket of its own.
//
// PI_PROXY_PI       the pi executable
// PI_PROXY_ARGS     a JSON array of arguments put before the desktop's
// PI_PROXY_LOG      folder for commands.jsonl (each command sent), events.jsonl (each
//                   record received) and responses.jsonl (each response in full)
// PI_PROXY_PROMPTS  optional JSON array: the only prompts allowed; another one stops pi
//
// pi gets only plumbing environment variables, never inherited provider secrets.
import { spawn } from 'node:child_process';
import { appendFileSync, mkdtempSync } from 'node:fs';
import { createConnection, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const log = process.env.PI_PROXY_LOG;
const record = (file, value) => appendFileSync(join(log, file), `${JSON.stringify(value)}\n`);
const prompts = process.env.PI_PROXY_PROMPTS ? JSON.parse(process.env.PI_PROXY_PROMPTS) : undefined;
const plumbing = /^(PATH|HOME|XDG_[A-Z_]+|PI_CODING_AGENT_DIR|PI_OFFLINE|PI_SKIP_VERSION_CHECK|PI_DESKTOP_LSP(?:_TOKEN)?|PI_DESKTOP_JJ_TOOLS|PI_DESKTOP_CHANNEL_TOKEN|SystemRoot|ComSpec|TEMP|TMP|TMPDIR|LANG|LC_.*|TZ|SSL_CERT_FILE|NODE_EXTRA_CA_CERTS|NODE_USE_SYSTEM_CA|HTTPS?_PROXY|ALL_PROXY|NO_PROXY|https?_proxy|all_proxy|no_proxy)$/;
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => plumbing.test(key)));

const command = value => ({
  type: value.type,
  ...(value.type === 'bash' ? { excludeFromContext: value.excludeFromContext } : {}),
  ...(value.type === 'prompt' ? { message: value.message } : {}),
  ...(value.type === 'extension_ui_response'
    ? { id: value.id, cancelled: value.cancelled, value: value.value, confirmed: value.confirmed }
    : {}),
});
const event = value => ({
  type: value.type,
  ...(value.method ? { method: value.method } : {}),
  ...(value.command ? { command: value.command } : {}),
  ...((value.type === 'extension_ui_request' && value.method !== 'notify') || value.type === 'extension_ui_cancel'
    ? { id: value.id }
    : {}),
});
const received = value => {
  record('events.jsonl', event(value));
  if (value.type === 'response') record('responses.jsonl', value);
};

function lines(stream, each) {
  let pending = '';
  stream.setEncoding('utf8');
  stream.on('data', chunk => {
    pending += chunk;
    for (let end = pending.indexOf('\n'); end >= 0; end = pending.indexOf('\n')) {
      const line = pending.slice(0, end);
      pending = pending.slice(end + 1);
      each(line);
    }
  });
}

// pi's extension connects here; each connection goes on to the desktop's socket.
const desktop = process.env.PI_DESKTOP_CHANNEL;
if (desktop) {
  const address = join(mkdtempSync(join(tmpdir(), 'pi-proxy-')), 'channel');
  const port = /^tcp:(\d+)$/.exec(desktop)?.[1];
  const server = createServer(extension => {
    const upstream = port ? createConnection({ host: '127.0.0.1', port: Number(port) }) : createConnection(desktop);
    lines(extension, line => {
      received(JSON.parse(line));
      upstream.write(`${line}\n`);
    });
    lines(upstream, line => {
      record('commands.jsonl', command(JSON.parse(line)));
      extension.write(`${line}\n`);
    });
    extension.on('close', () => upstream.end());
    upstream.on('close', () => extension.end());
    extension.on('error', () => upstream.destroy());
    upstream.on('error', () => extension.destroy());
  });
  await new Promise(resolve => server.listen(address, resolve));
  server.unref();
  env.PI_DESKTOP_CHANNEL = address;
}

const child = spawn(process.env.PI_PROXY_PI, [...JSON.parse(process.env.PI_PROXY_ARGS ?? '[]'), ...process.argv.slice(2)],
  { env, stdio: ['pipe', 'pipe', 'inherit'] });
lines(process.stdin, line => {
  const value = JSON.parse(line);
  if (prompts && value.type === 'prompt' && !prompts.includes(value.message)) {
    console.error(`pi-proxy: this capture forbids the prompt ${JSON.stringify(value.message)}`);
    process.exit(1);
  }
  record('commands.jsonl', command(value));
  child.stdin.write(`${line}\n`);
});
lines(child.stdout, line => {
  received(JSON.parse(line));
  process.stdout.write(`${line}\n`);
});
process.stdin.on('end', () => child.stdin.end());
child.on('error', error => {
  console.error(error.message);
  process.exit(1);
});
child.on('exit', code => process.exit(code ?? 1));
