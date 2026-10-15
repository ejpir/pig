import { expect, test } from "bun:test";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";

test("SQLite recovery reports an interrupted JavaScript tool without repeating its side effect", async () => {
  const directory = await mkdtemp(join(tmpdir(), "pi-javascript-crash-"));
  await writeFile(
    join(directory, "job.cjs"),
    `const fs=require('node:fs'),path=require('node:path');const {workerData}=require('node:worker_threads');fs.appendFileSync(path.join(workerData.cwd,'executions.log'),'execution\\n');console.log('script started');setInterval(()=>{},1000);`,
  );
  const processes: ReturnType<typeof Bun.spawn>[] = [];
  const start = () => {
    const proc = Bun.spawn(
      [
        ...(process.env.PI_JAVASCRIPT_TEST_RUNNER
          ? [resolve(process.env.PI_JAVASCRIPT_TEST_RUNNER)]
          : [
              process.execPath,
              join(import.meta.dir, "javascript-crash-fixture.ts"),
            ]),
        join(directory, "state"),
        directory,
      ],
      { stdin: "pipe", stdout: "pipe", stderr: "pipe" },
    );
    processes.push(proc);
    const records: any[] = [];
    const reading = (async () => {
      let pending = "";
      for await (const chunk of proc.stdout) {
        pending += Buffer.from(chunk).toString();
        let end;
        while ((end = pending.indexOf("\n")) >= 0) {
          const line = pending.slice(0, end);
          pending = pending.slice(end + 1);
          if (line) records.push(JSON.parse(line));
        }
      }
    })();
    const errors = new Response(proc.stderr).text();
    return { proc, records, reading, errors };
  };
  const wait = async (check: () => Promise<boolean> | boolean) => {
    for (let i = 0; i < 600; i++) {
      if (await check()) return;
      await Bun.sleep(10);
    }
    throw Error("JavaScript recovery deadline");
  };
  try {
    const first = start();
    await wait(() => first.records.some((r) => r.type === "durable_state"));
    first.proc.stdin.write(
      JSON.stringify({
        type: "prompt",
        id: "prompt",
        requestId: "persisted-script",
        message: "run offline script",
      }) + "\n",
    );
    await wait(async () => {
      try {
        return (
          await readFile(join(directory, "executions.log"), "utf8")
        ).includes("execution");
      } catch {
        return false;
      }
    });
    first.proc.kill("SIGKILL");
    await first.proc.exited;
    await first.reading;
    expect(
      (await readFile(join(directory, "executions.log"), "utf8")).trim(),
    ).toBe("execution");
    const second = start();
    await wait(() =>
      JSON.stringify(second.records).includes(
        "JavaScript interruption observed",
      ),
    );
    expect(JSON.stringify(second.records)).toContain("interrupted");
    expect(
      (await readFile(join(directory, "executions.log"), "utf8")).trim(),
    ).toBe("execution");
    second.proc.stdin.write(
      JSON.stringify({
        type: "prompt",
        id: "duplicate",
        requestId: "persisted-script",
        message: "run offline script",
      }) + "\n",
    );
    await wait(() => second.records.some((r) => r.id === "duplicate"));
    expect(
      second.records.find((r) => r.id === "duplicate")?.data.duplicate,
    ).toBe(true);
    expect(
      (await readFile(join(directory, "executions.log"), "utf8")).trim(),
    ).toBe("execution");
    second.proc.stdin.end();
    await second.proc.exited;
    await second.reading;
    expect(await second.errors).toBe("");
  } finally {
    for (const proc of processes) {
      if (proc.exitCode === null) proc.kill("SIGKILL");
      await proc.exited;
    }
    await rm(directory, { recursive: true, force: true });
  }
}, 15000);
