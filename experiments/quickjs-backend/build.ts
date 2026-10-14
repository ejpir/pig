// Build tooling only: Bun is never shipped or launched by the PoC.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';

if (Bun.version !== '1.4.2') throw new Error(`Use Bun 1.4.2, not ${Bun.version}`);
const result = await Bun.build({
  entrypoints: ['js/encoding.js', 'js/platform.js', 'js/agent.js'],
  outdir: 'generated', target: 'browser', format: 'iife',
});
if (!result.success) throw new AggregateError(result.logs, 'QuickJS bundle failed');
const inputs = ['build.ts', 'package.json', 'package-lock.json', 'js/encoding.js', 'js/platform.js', 'js/agent.js'];
const outputs = ['generated/encoding.js', 'generated/platform.js', 'generated/agent.js'];
const files = Object.fromEntries([...inputs, ...outputs].map(path => [path,
  createHash('sha256').update(readFileSync(path)).digest('hex')]));
const packages = Object.fromEntries(['@earendil-works/pi-agent-core', '@earendil-works/pi-ai', 'openai'].map(name => [name,
  JSON.parse(readFileSync(`node_modules/${name}/package.json`, 'utf8')).version]));
writeFileSync('generated/manifest.json', JSON.stringify({ bun: Bun.version, packages, files }, null, 2) + '\n');
console.log(JSON.stringify(packages));
for (const path of outputs) console.log(`${path}: ${readFileSync(path).length} bytes`);
