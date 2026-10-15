// Build tooling only: Bun is never shipped or launched by the PoC.
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { getSupportedThinkingLevels, clampThinkingLevel } from '@earendil-works/pi-ai/models';
import { anthropicProvider } from '@earendil-works/pi-ai/providers/anthropic';
import { openaiCodexProvider } from '@earendil-works/pi-ai/providers/openai-codex';

if (Bun.version !== '1.4.2') throw new Error(`Use Bun 1.4.2, not ${Bun.version}`);
const result = await Bun.build({
  entrypoints: ['js/encoding.js', 'js/platform.js', 'js/agent.js', 'js/auth.js', 'js/chat.js'],
  outdir: 'generated', target: 'browser', format: 'iife',
  // Bundler-opaque fallback in upstream's OAuth loader is never used after registration.
  // Bun 1.4.2 otherwise leaves import.meta.url in a classic-script IIFE.
  define: { 'import.meta.url': JSON.stringify('file:///pi-ai-auth.js') },
  plugins: [{ name: 'mobile-oauth-node-boundaries', setup(build) {
    build.onResolve({ filter: /^node:(http|crypto)$/ }, args => ({
      path: join(import.meta.dir, args.path === 'node:http' ? 'js/no-node-http.js' : 'js/no-node-crypto.js'),
    }));
  } }],
});
if (!result.success) throw new AggregateError(result.logs, 'QuickJS bundle failed');
// Public coding-agent SDK on QuickJS. Optional Node transport/process/native-code
// capabilities are explicitly denied; provider HTTP uses our existing native fetch.
const nodeAliases: Record<string,string> = {
 path:'path.js',buffer:'buffer.js',events:'events.cjs',process:'process.js',os:'os.js',url:'url.js',crypto:'crypto.js',
 fs:'fs.cjs','fs/promises':'fs-promises.js',module:'module.js',timers:'timers.js','timers/promises':'timers-promises.js',
 string_decoder:'string-decoder.js',util:'util.js',stream:'stream.js',undici:'undici.js',
};
const deniedBuiltins=new Set(['child_process','readline','worker_threads','stream/promises','tty','net','http','https','zlib','async_hooks','vm','assert','v8','sqlite','dns','tls','querystring','http2','perf_hooks']);
const sdkResult=await Bun.build({
 entrypoints:['js/sdk-platform.js','js/sdk-probe.js','js/session.js'],outdir:'generated',target:'browser',format:'iife',
 define:{'import.meta.url':JSON.stringify('file:///pi-coding-agent/dist/config.js')},
 plugins:[{name:'admitted-sdk-platform',setup(build){build.onResolve({filter:/^(?:node:)?[a-z0-9_]+(?:\/[a-z0-9_]+)?$/},args=>{
  const name=args.path.replace(/^node:/,'');
  if(nodeAliases[name])return {path:join(import.meta.dir,'js/node',nodeAliases[name])};
  if(deniedBuiltins.has(name))return {path:join(import.meta.dir,'js/node/unavailable.js')};
 });}}],
});
if(!sdkResult.success)throw new AggregateError(sdkResult.logs,'Upstream session SDK bundle failed');
// Separate IIFEs: bootstrap globals must exist before SDK module initialization.
for(const name of ['sdk-probe','session'])writeFileSync(`generated/${name}.js`,readFileSync('generated/sdk-platform.js','utf8')+'\n'+readFileSync(`generated/${name}.js`,'utf8'));
// Read-only configuration asset, not a native filesystem mount.
writeFileSync('generated/sdk-package.json',readFileSync('node_modules/@earendil-works/pi-coding-agent/package.json'));
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
// Static upstream catalog only; building never resolves auth or contacts providers.
writeFileSync('generated/chat-models.json', JSON.stringify(Object.fromEntries(
  [anthropicProvider(), openaiCodexProvider()].map(p => [p.id, p.getModels().map(m => ({...m, supportedThinkingLevels: getSupportedThinkingLevels(m), clampedThinkingLevels: Object.fromEntries(['off','minimal','low','medium','high','xhigh','max'].map(l => [l, clampThinkingLevel(m,l)]))}))])), null, 2) + '\n');
const inputs = ['build.ts', 'package.json', 'package-lock.json', 'js/encoding.js', 'js/platform.js', 'js/agent.js', 'js/auth.js', 'js/chat.js', 'js/no-node-http.js', 'js/no-node-crypto.js', 'js/sdk-platform.js', 'js/sdk-probe.js', 'js/session.js', ...readdirSync('js/node').sort().map(name=>`js/node/${name}`)];
const outputs = ['generated/encoding.js', 'generated/platform.js', 'generated/agent.js', 'generated/auth.js', 'generated/chat.js', 'generated/chat-models.json', 'generated/QUICKJS-BACKEND-NPM-NOTICES.txt', 'generated/sdk-platform.js', 'generated/sdk-probe.js', 'generated/sdk-package.json', 'generated/session.js'];
const files = Object.fromEntries([...inputs, ...outputs].map(path => [path,
  createHash('sha256').update(readFileSync(path)).digest('hex')]));
const packages = Object.fromEntries(['@earendil-works/pi-agent-core', '@earendil-works/pi-ai', '@earendil-works/pi-coding-agent', 'openai'].map(name => [name,
  JSON.parse(readFileSync(`node_modules/${name}/package.json`, 'utf8')).version]));
writeFileSync('generated/manifest.json', JSON.stringify({ bun: Bun.version, packages, files }, null, 2) + '\n');
console.log(JSON.stringify(packages));
for (const path of outputs) console.log(`${path}: ${readFileSync(path).length} bytes`);
