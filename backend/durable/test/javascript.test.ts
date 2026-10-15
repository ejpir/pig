import { afterEach, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  executeJavaScript,
  JAVASCRIPT_OUTPUT_BYTES,
} from "../src/javascript-execution.ts";

const directories: string[] = [];
async function fixtures(files: Record<string, string>) {
  const cwd = await mkdtemp(join(tmpdir(), "pi-javascript-test-"));
  directories.push(cwd);
  for (const [name, source] of Object.entries(files))
    await writeFile(join(cwd, name), source);
  return cwd;
}
afterEach(async () => {
  for (const cwd of directories.splice(0))
    await rm(cwd, { recursive: true, force: true });
});

test("JavaScript workers run ESM, CommonJS and Node APIs with literal argv and separate globals/env", async () => {
  const cwd = await fixtures({
    "dependency.cjs":
      "module.exports=require('node:crypto').createHash('sha256').update('abc').digest('hex');",
    "input.txt": "kept",
    "main.mjs": `import {readFile,writeFile} from 'node:fs/promises';import {join} from 'node:path';import {workerData} from 'node:worker_threads';
import digest from './dependency.cjs';await writeFile(join(workerData.cwd,'output.txt'),await readFile(new URL('./input.txt',import.meta.url),'utf8'));
console.log(JSON.stringify({args:process.argv.slice(2),cwd:workerData.cwd,envCwd:process.env.PI_JAVASCRIPT_CWD,bridge:typeof globalThis.__piPhone,sentinel:process.env.PI_JAVASCRIPT_TEST_PRIVATE,digest}));console.error('stderr kept');`,
  });
  const globals = globalThis as any;
  const previous = globals.__piPhone,
    env = process.env.PI_JAVASCRIPT_TEST_PRIVATE;
  globals.__piPhone = { syntheticSentinel: true };
  process.env.PI_JAVASCRIPT_TEST_PRIVATE = "not-an-account";
  try {
    const args = ["a b", "; echo never", "🌍"];
    const result = await executeJavaScript({ file: "main.mjs", args }, cwd);
    expect(result.reason).toBe("exit");
    expect(result.exitCode).toBe(0);
    expect(result.terminationConfirmed).toBe(true);
    expect(JSON.parse(result.stdout)).toEqual({
      args,
      cwd,
      envCwd: cwd,
      bridge: "undefined",
      digest:
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    });
    expect(result.stderr).toContain("stderr kept");
    expect(await readFile(join(cwd, "output.txt"), "utf8")).toBe("kept");
  } finally {
    if (previous === undefined) delete globals.__piPhone;
    else globals.__piPhone = previous;
    if (env === undefined) delete process.env.PI_JAVASCRIPT_TEST_PRIVATE;
    else process.env.PI_JAVASCRIPT_TEST_PRIVATE = env;
  }
});

test("CommonJS main and process.exit are worker-local; repeated invocations are fresh", async () => {
  const cwd = await fixtures({
    "main.cjs": `if(require.main!==module)throw Error('not main'); globalThis.count=(globalThis.count||0)+1; console.log(globalThis.count);process.exit(7);`,
  });
  for (let i = 0; i < 2; i++) {
    const result = await executeJavaScript({ file: "main.cjs" }, cwd);
    expect(result.stdout.trim()).toBe("1");
    expect(result.exitCode).toBe(7);
    expect(result.terminationConfirmed).toBe(true);
  }
});

test("exceptions, missing files and invalid inputs are errors, not successful empty output", async () => {
  const cwd = await fixtures({
    "throw.js": "throw Error('synthetic-script-failure');",
    "plain.txt": "not JS",
    "ok.js": "console.log('ok')",
  });
  const thrown = await executeJavaScript({ file: "throw.js" }, cwd);
  expect(thrown.reason).toBe("error");
  expect(thrown.stderr).toContain("synthetic-script-failure");
  expect(thrown.terminationConfirmed).toBe(true);
  for (const request of [
    { file: "missing.js" },
    { file: "plain.txt" },
    { file: "ok.js", timeout: 0 },
    { file: "ok.js", timeout: 301 },
    { file: "ok.js", args: ["bad\0arg"] },
  ]) {
    const result = await executeJavaScript(request, cwd);
    expect(result.reason).toBe("error");
    expect(result.stdout).toBe("");
    expect(result.stderr.length).toBeGreaterThan(0);
  }
});

test("timeout terminates synchronous infinite JS without blocking the parent or future workers", async () => {
  const cwd = await fixtures({
    "spin.js": "while(true){}",
    "ok.js": "console.log('after timeout')",
  });
  let heartbeat = false;
  setTimeout(() => {
    heartbeat = true;
  }, 30);
  const result = await executeJavaScript({ file: "spin.js", timeout: 1 }, cwd);
  expect(heartbeat).toBe(true);
  expect(result.reason).toBe("timeout");
  expect(result.terminationConfirmed).toBe(true);
  expect((await executeJavaScript({ file: "ok.js" }, cwd)).exitCode).toBe(0);
});

test("cancellation terminates a running worker; pre-cancelled scripts never start", async () => {
  const cwd = await fixtures({
    "spin.js": "console.log('started');while(true){}",
    "effect.js": `require('node:fs').writeFileSync(require('node:path').join(process.env.PI_JAVASCRIPT_CWD,'must-not-exist'),'bad')`,
  });
  const controller = new AbortController();
  setTimeout(() => controller.abort(), 100);
  const result = await executeJavaScript(
    { file: "spin.js" },
    cwd,
    controller.signal,
  );
  expect(result.reason).toBe("cancelled");
  expect(result.terminationConfirmed).toBe(true);
  const cancelled = await executeJavaScript(
    { file: "effect.js" },
    cwd,
    controller.signal,
  );
  expect(cancelled.reason).toBe("cancelled");
  await expect(readFile(join(cwd, "must-not-exist"))).rejects.toThrow();
});

test("combined output is bounded and a second simultaneous script is not started", async () => {
  const cwd = await fixtures({
    "loud.js": "process.stdout.write('x'.repeat(100000));",
    "spin.js": "while(true){}",
    "ok.js": "console.log('must not start')",
  });
  const loud = await executeJavaScript({ file: "loud.js" }, cwd);
  expect(loud.reason).toBe("output_limit");
  expect(loud.terminationConfirmed).toBe(true);
  expect(
    Buffer.byteLength(loud.stdout) + Buffer.byteLength(loud.stderr),
  ).toBeLessThanOrEqual(JAVASCRIPT_OUTPUT_BYTES);
  const controller = new AbortController();
  const running = executeJavaScript(
    { file: "spin.js" },
    cwd,
    controller.signal,
  );
  await Bun.sleep(50);
  const second = await executeJavaScript({ file: "ok.js" }, cwd);
  expect(second.reason).toBe("busy");
  expect(second.stdout).toBe("");
  controller.abort();
  expect((await running).terminationConfirmed).toBe(true);
});

test("Durable registers JavaScript as an unsafe tool with bounded streaming and structured output", async () => {
  const { BACKGROUND_CONTEXT: context } = await import(
    "@earendil-works/chord/context"
  );
  const { createModels } = await import("@earendil-works/pi-ai/models");
  const { fauxProvider, fauxAssistantMessage, fauxToolCall } = await import(
    "@earendil-works/pi-ai/providers/faux"
  );
  const { createRegistry, Harness, MemoryStorage } = await import(
    "@earendil-works/pi-durable"
  );
  const { NodeExecutionEnv } = await import(
    "@earendil-works/pi-durable/env/node"
  );
  const { JavascriptTools } = await import("../src/javascript.ts");
  const cwd = await fixtures({
    "sdk.cjs": "console.log('sdk-output');console.error('sdk-stderr');",
  });
  const models = createModels(),
    faux = fauxProvider();
  models.setProvider(faux.provider);
  faux.setResponses([
    fauxAssistantMessage(
      fauxToolCall("javascript", { file: "sdk.cjs" }, { id: "script" }),
      { stopReason: "toolUse" },
    ),
    fauxAssistantMessage("done"),
  ]);
  const registry = createRegistry();
  registry.install(JavascriptTools);
  const harness = await Harness.open(
    new MemoryStorage(),
    { models, registry, env: () => new NodeExecutionEnv({ cwd }) },
    context,
  );
  try {
    const model = faux.getModel();
    const root = await harness.root(context, {
      agent: { model: { provider: model.provider, modelId: model.id } },
    });
    const agent = await root.agent(context);
    expect(agent.tools.map((t) => t.name)).toContain("javascript");
    const submission = await root.submit(
      { type: "input", content: "test JS" },
      context,
    );
    await submission.wait(context);
    await root.waitForIdle(context);
    const messages = (await root.context(context)).messages;
    const result = messages.find((m) => m.role === "toolResult");
    expect(result?.role).toBe("toolResult");
    if (result?.role === "toolResult") {
      expect(result.isError).not.toBe(true);
      expect(JSON.stringify(result.content)).toContain("sdk-output");
      expect(JSON.stringify(result.content)).toContain("sdk-stderr");
      expect(result.details).toMatchObject({
        exitCode: 0,
        reason: "exit",
        terminationConfirmed: true,
      });
    }
  } finally {
    await harness.close(context);
  }
});

test("decoded multibyte and invalid UTF-8 output cannot expand beyond the byte limit", async () => {
  const cwd = await fixtures({
    "euro.js": "process.stdout.write('€'.repeat(21846));",
    "invalid.js": "process.stdout.write(Buffer.alloc(65536,255));",
    "split.js":
      "process.stdout.write(Buffer.from([226]));setTimeout(()=>{process.stderr.write('ERR');setTimeout(()=>process.stdout.write(Buffer.from([130,172])),20);},20);",
  });
  for (const file of ["euro.js", "invalid.js"]) {
    const result = await executeJavaScript({ file }, cwd);
    expect(result.reason).toBe("output_limit");
    expect(
      Buffer.byteLength(result.stdout) + Buffer.byteLength(result.stderr),
    ).toBeLessThanOrEqual(JAVASCRIPT_OUTPUT_BYTES);
  }
  const streamed: string[] = [];
  const result = await executeJavaScript(
    { file: "split.js" },
    cwd,
    undefined,
    (_stream, text) => streamed.push(text),
  );
  expect(result.stdout).toBe("€");
  expect(result.stderr).toBe("ERR");
  expect(streamed.join("")).toContain("€");
  expect(streamed.join("")).not.toContain("�");
});

test("Durable Stop cancels an actual JavaScript tool and releases its worker slot", async () => {
  const { BACKGROUND_CONTEXT: context } = await import(
    "@earendil-works/chord/context"
  );
  const { createModels } = await import("@earendil-works/pi-ai/models");
  const { fauxProvider, fauxAssistantMessage, fauxToolCall } = await import(
    "@earendil-works/pi-ai/providers/faux"
  );
  const { createRegistry, Harness, MemoryStorage } = await import(
    "@earendil-works/pi-durable"
  );
  const { NodeExecutionEnv } = await import(
    "@earendil-works/pi-durable/env/node"
  );
  const { JavascriptTools } = await import("../src/javascript.ts");
  const cwd = await fixtures({
    "stop.cjs":
      "require('node:fs').writeFileSync(require('node:path').join(process.env.PI_JAVASCRIPT_CWD,'started'),'yes');for(;;){}",
    "after.cjs": "console.log('after-stop');",
  });
  const models = createModels(),
    faux = fauxProvider();
  models.setProvider(faux.provider);
  faux.setResponses([
    fauxAssistantMessage(
      fauxToolCall(
        "javascript",
        { file: "stop.cjs", timeout: 3 },
        { id: "script" },
      ),
      { stopReason: "toolUse" },
    ),
  ]);
  const registry = createRegistry();
  registry.install(JavascriptTools);
  const harness = await Harness.open(
    new MemoryStorage(),
    { models, registry, env: () => new NodeExecutionEnv({ cwd }) },
    context,
  );
  try {
    const model = faux.getModel(),
      root = await harness.root(context, {
        agent: { model: { provider: model.provider, modelId: model.id } },
      });
    await root.submit({ type: "input", content: "test Stop" }, context);
    const { readFile } = await import("node:fs/promises");
    let started = false;
    for (let i = 0; i < 200; i++) {
      try {
        started = (await readFile(join(cwd, "started"), "utf8")) === "yes";
      } catch {}
      if (started) break;
      await Bun.sleep(5);
    }
    expect(started).toBe(true);
    const stopStart = Date.now();
    await root.abort(context, { background: true });
    expect(Date.now() - stopStart).toBeLessThan(2500);
    await root.waitForIdle(context);
    const after = await executeJavaScript({ file: "after.cjs" }, cwd);
    expect(after.exitCode).toBe(0);
    expect(after.stdout).toBe("after-stop\n");
  } finally {
    await harness.close(context);
  }
});
