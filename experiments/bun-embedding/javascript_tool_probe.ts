// Offline probe of the *same* executor as the tool, in the actual Android .so.
import { mkdir, mkdtemp, writeFile, readFile } from "node:fs/promises";
import { join } from "node:path";
import {
  executeJavaScript,
  JAVASCRIPT_OUTPUT_BYTES,
} from "../../backend/durable/src/javascript-execution.ts";

import { javascriptSdkProbe } from "../../backend/durable/test/javascript-android-probe.ts";

export async function runProbe({ root }: { root: string }) {
  const directory = await mkdtemp(join(root, "javascript-worker-"));
  const fixtures: Record<string, string> = {
    "input.txt": "node-on-android",
    "dependency.cjs": `module.exports = require('node:crypto').createHash('sha256').update('abc').digest('hex');`,
    "dependency.mjs": `export const marker = 'relative-esm-import';`,
    "main.mjs": `import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto';
import {workerData} from 'node:worker_threads'; import digest from './dependency.cjs'; import {marker} from './dependency.mjs';
const value=fs.readFileSync(new URL('./input.txt',import.meta.url),'utf8');
fs.writeFileSync(path.join(workerData.cwd,'output.txt'),value+'-written');
console.log(JSON.stringify({value,digest,marker,args:process.argv.slice(2),cwd:workerData.cwd,env:process.env.PI_JAVASCRIPT_CWD,
pid:process.pid,bun:Bun.version,platform:process.platform,arch:process.arch,bridge:typeof globalThis.__piPhone,
parentEnv:process.env.PI_WORKER_PARENT_SENTINEL,crypto:crypto.randomBytes(8).length}));console.error('stderr-ok');`,
    "main.cjs": `if(require.main!==module)throw Error('not CommonJS entry'); console.log(require('node:path').basename(__filename)); process.exit(7);`,
    "throw.js": `throw new Error('offline-script-failure');`,
    "spin.js": `while(true){}`,
    "loud.js": `process.stdout.write('x'.repeat(100000));`,
    "invalid.js": "process.stdout.write(Buffer.alloc(65536,255));",
    "euro.js": "process.stdout.write('€'.repeat(21846));",
    "split.js":
      "process.stdout.write(Buffer.from([226]));setTimeout(()=>{process.stderr.write('ERR');setTimeout(()=>process.stdout.write(Buffer.from([130,172])),20);},20);",
  };
  for (const [file, source] of Object.entries(fixtures))
    await writeFile(join(directory, file), source);
  await mkdir(join(directory, "tmp"));
  const home = process.env.HOME,
    temp = process.env.TMPDIR;
  process.env.HOME = directory;
  process.env.TMPDIR = join(directory, "tmp");
  process.env.PI_WORKER_PARENT_SENTINEL = "must-not-be-inherited";
  (globalThis as any).__piPhone = { privateSentinel: "not-a-real-credential" };
  const checks: Record<string, boolean> = {};
  const results: Record<string, unknown> = {};
  const assert = (name: string, okay: boolean) => {
    checks[name] = okay;
    if (!okay)
      throw Error(
        "Worker probe failed: " + name + " " + JSON.stringify(results),
      );
  };
  try {
    const esm = await executeJavaScript(
      { file: "main.mjs", args: ["a b", ";not-shell", "🌍"] },
      directory,
    );
    results.esm = esm;
    assert(
      "esm_exit",
      esm.exitCode === 0 && esm.reason === "exit" && esm.terminationConfirmed,
    );
    const value = JSON.parse(esm.stdout.trim());
    assert(
      "node_esm_cjs_imports",
      value.value === "node-on-android" &&
        value.marker === "relative-esm-import" &&
        value.crypto === 8 &&
        value.digest ===
          "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
    assert(
      "argv",
      JSON.stringify(value.args) ===
        JSON.stringify(["a b", ";not-shell", "🌍"]),
    );
    assert(
      "worker_context",
      value.bridge === "undefined" &&
        value.parentEnv === undefined &&
        value.cwd === directory &&
        value.env === directory,
    );
    assert(
      "same_embedded_process",
      value.pid === process.pid &&
        value.platform === "android" &&
        value.arch === "x64" &&
        value.bun.startsWith("1.4.3"),
    );
    assert(
      "filesystem",
      (await readFile(join(directory, "output.txt"), "utf8")) ===
        "node-on-android-written",
    );
    assert("stderr", esm.stderr.includes("stderr-ok"));
    const cjs = await executeJavaScript({ file: "main.cjs" }, directory);
    results.cjs = cjs;
    assert(
      "commonjs_exit_does_not_exit_app",
      cjs.exitCode === 7 && cjs.stdout.includes("main.cjs"),
    );
    const thrown = await executeJavaScript({ file: "throw.js" }, directory);
    results.thrown = thrown;
    assert(
      "errors",
      thrown.reason === "error" &&
        thrown.stderr.includes("offline-script-failure") &&
        thrown.terminationConfirmed,
    );
    let heartbeat = false;
    setTimeout(() => {
      heartbeat = true;
    }, 30);
    const spin = await executeJavaScript(
      { file: "spin.js", timeout: 1 },
      directory,
    );
    results.timeout = spin;
    assert(
      "timeout_and_parent_liveness",
      spin.reason === "timeout" && spin.terminationConfirmed && heartbeat,
    );
    const controller = new AbortController();
    setTimeout(() => controller.abort(), 100);
    const cancelled = await executeJavaScript(
      { file: "spin.js" },
      directory,
      controller.signal,
    );
    results.cancelled = cancelled;
    assert(
      "cancellation",
      cancelled.reason === "cancelled" && cancelled.terminationConfirmed,
    );
    const loud = await executeJavaScript({ file: "loud.js" }, directory);
    results.output = {
      ...loud,
      stdoutBytes: Buffer.byteLength(loud.stdout),
      stdout: undefined,
    };
    assert(
      "bounded_output",
      loud.reason === "output_limit" &&
        loud.terminationConfirmed &&
        Buffer.byteLength(loud.stdout) + Buffer.byteLength(loud.stderr) <=
          JAVASCRIPT_OUTPUT_BYTES,
    );
    const again = await executeJavaScript(
      { file: "main.mjs", args: [] },
      directory,
    );
    assert(
      "usable_after_termination",
      again.exitCode === 0 &&
        again.reason === "exit" &&
        again.terminationConfirmed,
    );
    for (const file of ["invalid.js", "euro.js"]) {
      const result = await executeJavaScript({ file }, directory);
      assert(
        file + "_bytebound",
        result.reason === "output_limit" &&
          Buffer.byteLength(result.stdout) + Buffer.byteLength(result.stderr) <=
            JAVASCRIPT_OUTPUT_BYTES,
      );
    }
    const streamed: string[] = [];
    const split = await executeJavaScript(
      { file: "split.js" },
      directory,
      undefined,
      (_stream, chunk) => streamed.push(chunk),
    );
    assert(
      "independent_utf8_streams",
      split.stdout === "€" &&
        split.stderr === "ERR" &&
        !streamed.join("").includes("�"),
    );
    const sdk = await javascriptSdkProbe(directory);
    results.sdk = sdk;
    assert("sdk_tool", sdk.passed);
    return {
      passed: true,
      pid: process.pid,
      bun: Bun.version,
      checks,
      results,
    };
  } finally {
    delete (globalThis as any).__piPhone;
    delete process.env.PI_WORKER_PARENT_SENTINEL;
    if (home === undefined) delete process.env.HOME;
    else process.env.HOME = home;
    if (temp === undefined) delete process.env.TMPDIR;
    else process.env.TMPDIR = temp;
  }
}
