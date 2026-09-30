import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { COMMANDS } from '../src/commands.mjs';

test('all commands currently emitted by the Rust desktop have backend handlers', async () => {
  const source = await readFile(new URL('../../../crates/pi_core/src/protocol.rs', import.meta.url), 'utf8');
  const section = source.split('pub fn name(&self)')[1].split('/// Only LF')[0];
  const names = [...section.matchAll(/=>\s*"([a-z_]+)"/g)].map(match => match[1]);
  assert.ok(names.length >= 30);
  assert.deepEqual(names.filter(name => !COMMANDS.includes(name)), []);
});
