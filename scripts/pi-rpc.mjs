#!/usr/bin/env node
// Run the sibling checkout's current RPC source, not potentially stale dist files.
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(process.env.PI_DESKTOP_PI_SOURCE ?? resolve(dirname(fileURLToPath(import.meta.url)), '../../pi'));
const resolver = resolve(root, 'packages/coding-agent/src/experimental/source-resolver.ts');
const entry = resolve(root, 'packages/coding-agent/src/rpc-entry.ts');
if (!existsSync(resolver) || !existsSync(entry)) {
  console.error(`Pi source checkout not found at ${root}. Set PI_DESKTOP_PI_SOURCE.`);
  process.exit(1);
}
const child = spawn(process.execPath, ['--experimental-strip-types', '--import', pathToFileURL(resolver).href, entry, ...process.argv.slice(2)], {
  stdio: 'inherit',
  windowsHide: true,
});
child.on('error', error => { console.error(error.message); process.exitCode = 1; });
child.on('exit', (code, signal) => { process.exitCode = code ?? (signal ? 1 : 0); });
