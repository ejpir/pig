/** Native, pinned standalone artifact plus complete dependency notices for embedding in the Rust helper. */
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

if (Bun.version !== "1.4.2") throw new Error(`Use Bun 1.4.2, not ${Bun.version}, for reproducible runner builds`);
const root = resolve(import.meta.dir, "../..");
const output = process.env.PI_DESKTOP_DURABLE_OUTPUT_DIR
  ? resolve(process.env.PI_DESKTOP_DURABLE_OUTPUT_DIR)
  : join(root, "artifacts/durable");
mkdirSync(output, { recursive: true });
const build = spawnSync(process.execPath, ["build", "src/main.ts", "--compile", "--minify", "--outfile", join(output, "pi-desktop-durable")], { cwd: import.meta.dir, stdio: "inherit" });
if (build.status !== 0) throw new Error("Durable standalone build failed");
let notices = `Pi Desktop durable runner\nBuilt with Bun ${Bun.version}; pi-durable/pi-ai/chord pinned in package-lock.json.\nInstalled dependency notices follow (tree-shaking may omit some packages).\n\n`;
for (const path of [join(root, "LICENSE"), join(root, "licenses/DURABLE-BUN-LICENSE.md")]) notices += readFileSync(path, "utf8") + "\n\n";
function packages(directory: string) {
  for (const item of readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    if (!item.isDirectory() || item.name.startsWith(".")) continue;
    const path = join(directory, item.name);
    if (item.name.startsWith("@")) { packages(path); continue; }
    const manifest = JSON.parse(readFileSync(join(path, "package.json"), "utf8")) as { name: string; version: string; license?: string };
    notices += `\n===== ${manifest.name} ${manifest.version} (${manifest.license ?? "see license"}) =====\n`;
    for (const file of readdirSync(path, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (file.isFile() && /^(license|copying|notice)/i.test(file.name)) notices += readFileSync(join(path, file.name), "utf8") + "\n";
    }
    // Nested, version-specific dependencies have their own notices too.
    if (readdirSync(path).includes("node_modules")) packages(join(path, "node_modules"));
  }
}
packages(join(import.meta.dir, "node_modules"));
writeFileSync(join(output, "NOTICES.txt"), notices);
console.log(`Built PRODUCTION durable runner for ${process.platform}/${process.arch}: ${join(output, "pi-desktop-durable")}`);
