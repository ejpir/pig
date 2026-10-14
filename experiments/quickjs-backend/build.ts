// Build tooling only: Bun is never shipped or launched by the PoC.
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

if (Bun.version !== '1.4.2') throw new Error(`Use Bun 1.4.2, not ${Bun.version}`);
const result = await Bun.build({
  entrypoints: ['js/encoding.js', 'js/platform.js', 'js/agent.js'],
  outdir: 'generated', target: 'browser', format: 'iife',
});
if (!result.success) throw new AggregateError(result.logs, 'QuickJS bundle failed');
// Include the installed tree's full notices in opt-in APKs (a conservative superset
// of tree-shaken JS). This is not a complete Rust/APK release licensing audit.
let notices = 'Pi QuickJS backend experiment — installed npm dependency notices\n\n';
function collect(directory: string) {
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    if (!entry.isDirectory() || entry.name.startsWith('.')) continue;
    const path = join(directory, entry.name);
    if (entry.name.startsWith('@')) { collect(path); continue; }
    const manifest = JSON.parse(readFileSync(join(path, 'package.json'), 'utf8'));
    notices += `\n===== ${manifest.name} ${manifest.version} (${manifest.license ?? 'see package license'}) =====\n`;
    const files = readdirSync(path, { withFileTypes: true });
    for (const file of files.filter(file => file.isFile() && /^(license|copying|notice)/i.test(file.name)).sort((a, b) => a.name.localeCompare(b.name))) {
      notices += readFileSync(join(path, file.name), 'utf8') + '\n';
    }
    if (files.some(file => file.isDirectory() && file.name === 'node_modules')) collect(join(path, 'node_modules'));
  }
}
collect('node_modules');
writeFileSync('generated/QUICKJS-BACKEND-NPM-NOTICES.txt', notices);
const inputs = ['build.ts', 'package.json', 'package-lock.json', 'js/encoding.js', 'js/platform.js', 'js/agent.js'];
const outputs = ['generated/encoding.js', 'generated/platform.js', 'generated/agent.js', 'generated/QUICKJS-BACKEND-NPM-NOTICES.txt'];
const files = Object.fromEntries([...inputs, ...outputs].map(path => [path,
  createHash('sha256').update(readFileSync(path)).digest('hex')]));
const packages = Object.fromEntries(['@earendil-works/pi-agent-core', '@earendil-works/pi-ai', 'openai'].map(name => [name,
  JSON.parse(readFileSync(`node_modules/${name}/package.json`, 'utf8')).version]));
writeFileSync('generated/manifest.json', JSON.stringify({ bun: Bun.version, packages, files }, null, 2) + '\n');
console.log(JSON.stringify(packages));
for (const path of outputs) console.log(`${path}: ${readFileSync(path).length} bytes`);
