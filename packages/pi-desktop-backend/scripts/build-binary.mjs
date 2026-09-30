#!/usr/bin/env node
// Builds the standalone backend: one Bun executable plus the pi files that pi
// reads from beside its executable (the layout of pi's own release binary), and
// that folder as a zstd-compressed tar, which pi-desktop's `bundled-backend`
// feature embeds (PI_DESKTOP_BACKEND_ARCHIVE).
//
// Usage: node scripts/build-binary.mjs [--platform darwin-arm64] [--out dist]
// Needs `npm ci` in this package and `bun` on PATH (or BUN=/path/to/bun).
// Platforms: darwin-arm64, darwin-x64, linux-x64, linux-arm64, windows-x64, windows-arm64.
//
// Output: <out>/<platform>/, <out>/pi-desktop-backend-<platform>.tar.zst and
// <out>/pi-desktop-backend-notices.txt, the licenses of the bundled npm packages.
//
// On macOS, PI_DESKTOP_CODESIGN_IDENTITY signs the executable with the hardened
// runtime and packaging/macos/backend.entitlements before it is packed; without
// it the executable keeps the ad-hoc signature Bun gives it.
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { constants, zstdCompressSync } from 'node:zlib';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const pi = join(root, 'node_modules/@earendil-works/pi-coding-agent');
const hostPlatform = () => {
  const os = { darwin: 'darwin', linux: 'linux', win32: 'windows' }[process.platform];
  const arch = { arm64: 'arm64', x64: 'x64' }[process.arch];
  if (!os || !arch) throw new Error(`No standalone backend for ${process.platform}-${process.arch}`);
  return `${os}-${arch}`;
};
const { values } = parseArgs({ options: { platform: { type: 'string' }, out: { type: 'string' } } });
const platform = values.platform ?? hostPlatform();
const platforms = ['darwin-arm64', 'darwin-x64', 'linux-x64', 'linux-arm64', 'windows-x64', 'windows-arm64'];
if (!platforms.includes(platform)) throw new Error(`Unknown platform ${platform}; one of ${platforms.join(', ')}`);
const out = resolve(values.out ?? join(root, 'dist'));
const folder = join(out, platform);
const windows = platform.startsWith('windows-');

rmSync(folder, { recursive: true, force: true });
mkdirSync(folder, { recursive: true });
// x64 uses Bun's baseline build, which runs on CPUs without AVX2, as pi's release does.
const target = `bun-${platform}${platform.endsWith('-x64') ? '-baseline' : ''}`;
// Bun embeds a worker only when it is an entry point. Autoloading .env and
// bunfig.toml from the session's folder is off: Node never reads them either.
const program = join(folder, windows ? 'pi-desktop-backend.exe' : 'pi-desktop-backend');
execFileSync(process.env.BUN ?? 'bun', ['build', '--compile', `--target=${target}`,
  '--no-compile-autoload-dotenv', '--no-compile-autoload-bunfig',
  'src/binary.mjs', join(pi, 'dist/utils/image-resize-worker.js'),
  '--outfile', program], { cwd: root, stdio: 'inherit' });
const identity = process.env.PI_DESKTOP_CODESIGN_IDENTITY;
if (identity && platform.startsWith('darwin-')) {
  // Bun's engine compiles JavaScript at run time, which the hardened runtime
  // allows only with these entitlements.
  execFileSync('codesign', ['--force', '--options', 'runtime', '--timestamp', '--sign', identity,
    '--entitlements', join(root, '../../packaging/macos/backend.entitlements'), program], { stdio: 'inherit' });
}

// Files and folders only; fs.cpSync also copies permissions, which some mounts refuse.
function copy(from, to, keep = () => true) {
  if (!statSync(from).isDirectory()) return copyFileSync(from, to);
  mkdirSync(to, { recursive: true });
  for (const name of readdirSync(from).filter(keep)) copy(join(from, name), join(to, name), keep);
}
// The same files pi's scripts/build-binaries.sh puts beside its binary.
for (const file of ['package.json', 'README.md', 'CHANGELOG.md']) copy(join(pi, file), join(folder, file));
copy(join(pi, 'node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm'), join(folder, 'photon_rs_bg.wasm'));
copy(join(pi, 'dist/modes/interactive/theme'), join(folder, 'theme'), name => name.endsWith('.json'));
copy(join(pi, 'dist/modes/interactive/assets'), join(folder, 'assets'));
copy(join(pi, 'dist/core/export-html'), join(folder, 'export-html'), name => !/\.(map|d\.ts)$/.test(name));
copy(join(pi, 'docs'), join(folder, 'docs'));
copy(join(pi, 'examples'), join(folder, 'examples'));
const native = platform.replace('windows-', 'win32-');
const prebuilds = `native/${native.split('-')[0]}/prebuilds/${native}`;
copy(join(pi, 'node_modules/@earendil-works/pi-tui', prebuilds), join(folder, prebuilds));
copy(join(root, 'PI-LICENSE'), join(folder, 'PI-LICENSE'));

const tar = join(out, `pi-desktop-backend-${platform}.tar`);
const archive = `${tar}.zst`;
rmSync(tar, { force: true });
// Relative names only: Git for Windows' GNU tar reads `D:\…` as a remote host.
execFileSync('tar', ['-cf', basename(tar), '-C', platform, '.'], { cwd: out, stdio: 'inherit' });
writeFileSync(archive, zstdCompressSync(readFileSync(tar), { params: { [constants.ZSTD_c_compressionLevel]: 19 } }));
rmSync(tar);

// Bundling drops each package's license file, so collect them: every
// installed production package in the lock file, with its license text or,
// without one, what its package.json says.
const lock = JSON.parse(readFileSync(join(root, 'npm-shrinkwrap.json'), 'utf8'));
const notices = ['Licenses of the npm packages bundled into pi-desktop-backend.\n'];
for (const [path, entry] of Object.entries(lock.packages)) {
  if (!path || entry.dev || !existsSync(join(root, path))) continue;
  const manifest = JSON.parse(readFileSync(join(root, path, 'package.json'), 'utf8'));
  const file = readdirSync(join(root, path)).find(name => /^(licen[cs]e|copying|notice)/i.test(name));
  const text = file ? readFileSync(join(root, path, file), 'utf8').trim()
    : `License: ${manifest.license ?? entry.license ?? 'not stated'}${manifest.author ? `\nAuthor: ${JSON.stringify(manifest.author)}` : ''}`;
  notices.push(`\n== ${manifest.name}@${manifest.version} ==\n\n${text}\n`);
}
const noticesFile = join(out, 'pi-desktop-backend-notices.txt');
writeFileSync(noticesFile, notices.join(''));
console.log(`Built ${folder}\n${archive}\n${noticesFile}`);
