import { test, expect, beforeAll, afterAll, afterEach } from "bun:test";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { install } from "./local.js";
test("native HTTP stream cancel forwards cancellation and ignores late body events", async () => {
  const response = fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: { "x-pi-native-operation": "1" },
    body: "{}",
  });
  const request = JSON.parse(__piPhone.drain())[0];
  expect(request.type).toBe("http");
  __piPhone.http({ id: request.id, kind: "headers", status: 200, headers: {} });
  const reader = (await response).body.getReader();
  await reader.cancel();
  expect(JSON.parse(__piPhone.drain())[0].type).toBe("http_cancel");
  expect(() =>
    __piPhone.http({ id: request.id, kind: "chunk", bytes: [65] }),
  ).not.toThrow();
  expect(() => __piPhone.http({ id: request.id, kind: "done" })).not.toThrow();
  __piPhone.http({ id: request.id, kind: "released" });
});
test("shared embedded runner: real SQLite, synthetic stock tool, restart and request dedup", async () => {
  const root = await mkdtemp(join(tmpdir(), "pi-bun-local-host-"));
  await mkdir(join(root, "workspace"));
  await mkdir(join(root, "home"));
  await mkdir(join(root, "tmp"));
  const options = {
    state: join(root, "session"),
    cwd: join(root, "workspace"),
    key: "local-host",
    synthetic: true,
    provider: "anthropic",
    model: "claude-fable-5",
    thinking: "high",
    home: join(root, "home"),
    temp: join(root, "tmp"),
    shellPath: "/bin/sh",
  };
  __piPhone.init({ home: options.home, temp: options.temp });
  let records = [];
  let closed = false;
  async function pump(until) {
    const deadline = Date.now() + 25000;
    while (!until()) {
      for (const item of JSON.parse(__piPhone.drain())) {
        if (item.channel === "host") {
          expect(item.type).toBe("assert_owner");
          __piPhone.deliver(item.id, true, null);
        } else if (item.channel === "protocol") records.push(item.record);
        else if (item.channel === "closed") closed = true;
      }
      if (Date.now() > deadline)
        throw new Error(
          "Embedded runner timeout: " +
            JSON.stringify(records.map((r) => [r.type, r.command, r.error])),
        );
      await Bun.sleep(5);
    }
  }
  try {
    __piPhone.open(101, options);
    await pump(() => records.some((r) => r.type === "durable_state"));
    const prompt = {
      type: "prompt",
      id: "p1",
      requestId: "persistent-host-tool",
      message: "offline tools",
    };
    __piPhone.send(101, prompt);
    await pump(() =>
      records.some((r) =>
        JSON.stringify(r).includes("Tool completed: phone-tool-ok"),
      ),
    );
    expect(
      records.some((r) => JSON.stringify(r).includes("tool_execution")),
    ).toBe(true);
    __piPhone.close(101);
    await pump(() => closed);
    records = [];
    closed = false;
    __piPhone.open(102, options);
    await pump(() => records.some((r) => r.type === "durable_state"));
    expect(JSON.stringify(records)).toContain("Tool completed: phone-tool-ok");
    __piPhone.send(102, { ...prompt, id: "p2" });
    await pump(() =>
      records.some((r) => r.type === "response" && r.id === "p2"),
    );
    const admitted = records.find(
      (r) => r.type === "response" && r.id === "p2",
    );
    expect(admitted.success).toBe(true);
    expect(
      records.filter(
        (r) =>
          r.type === "durable_events" &&
          JSON.stringify(r).includes("generation_start"),
      ).length,
    ).toBe(0);
    __piPhone.close(102);
    await pump(() => closed);
  } finally {
    __piPhone.close(101);
    __piPhone.close(102);
    await rm(root, { recursive: true, force: true });
  }
}, 60000);

for (const headers of [false, true])
  test(`native cancellation settles ${headers ? "body" : "fetch"} before release`, async () => {
    const response = fetch("https://api.anthropic.com/v1/messages", {
      method: "POST",
      headers: { "x-pi-native-operation": "1" },
      body: "{}",
    });
    const request = JSON.parse(__piPhone.drain())[0];
    let rejected;
    if (headers) {
      __piPhone.http({
        id: request.id,
        kind: "headers",
        status: 200,
        headers: {},
      });
      rejected = (await response).text();
    } else rejected = response;
    __piPhone.http({ id: request.id, kind: "released" });
    await expect(rejected).rejects.toThrow(
      "Native request ended before completion",
    );
    expect(() =>
      __piPhone.http({
        id: request.id,
        kind: "headers",
        status: 200,
        headers: {},
      }),
    ).not.toThrow();
  });

for (const signedIn of [false, true])
  test(`production Local control/admission survives with account metadata ${signedIn}`, async () => {
    const root = await mkdtemp(join(tmpdir(), "pi-bun-production-control-"));
    for (const dir of ["workspace", "home", "tmp"])
      await mkdir(join(root, dir));
    const engine = signedIn ? 202 : 201;
    let prepares = 0;
    const records = [];
    let closed = false;
    async function pump(until) {
      const deadline = Date.now() + 5000;
      while (!until() && !closed) {
        for (const item of JSON.parse(__piPhone.drain())) {
          if (item.channel === "host") {
            if (item.type === "assert_owner")
              __piPhone.deliver(item.id, true, null);
            else if (item.type === "account_status")
              __piPhone.deliver(item.id, signedIn, null);
            else if (item.type === "prepare") {
              prepares++;
              __piPhone.deliver(
                item.id,
                null,
                "Offline regression: native authorization intentionally unavailable",
              );
            } else
              throw new Error(
                "Unexpected HTTP/auth operation in offline production regression: " +
                  item.type,
              );
          } else if (item.channel === "protocol") records.push(item.record);
          else if (item.channel === "closed" && item.engine === engine)
            closed = true;
        }
        if (Date.now() > deadline)
          throw new Error("Production control regression timed out");
        await Bun.sleep(5);
      }
    }
    try {
      __piPhone.open(engine, {
        state: join(root, "session"),
        cwd: join(root, "workspace"),
        key: "production-control",
        synthetic: false,
        provider: "anthropic",
        model: "claude-fable-5",
        thinking: "high",
        home: join(root, "home"),
        temp: join(root, "tmp"),
        shellPath: "/bin/sh",
      });
      await pump(() => records.some((r) => r.type === "durable_state"));
      // This internal, deliberately uncorrelated record was sent before every
      // Android prompt. It must never enter the public SDK command stream.
      __piPhone.send(engine, { type: "phone_fixture_mode", mode: "Tool" });
      __piPhone.send(engine, { type: "get_state", id: "after-control" });
      await pump(() =>
        records.some((r) => r.type === "response" && r.id === "after-control"),
      );
      expect(closed).toBe(false);
      expect(
        records.find((r) => r.type === "response" && r.id === "after-control")
          ?.success,
      ).toBe(true);
      expect(prepares).toBe(0);
      if (signedIn) {
        const prompt = {
          type: "prompt",
          id: "production-prompt",
          requestId: "production-offline-request",
          message: "offline admission regression",
        };
        __piPhone.send(engine, prompt);
        await pump(
          () =>
            prepares > 0 &&
            records.some((r) => r.type === "response" && r.id === prompt.id),
        );
        expect(
          records.find((r) => r.type === "response" && r.id === prompt.id)
            ?.success,
        ).toBe(true);
        __piPhone.send(engine, { ...prompt, id: "production-retry" });
        await pump(() =>
          records.some(
            (r) => r.type === "response" && r.id === "production-retry",
          ),
        );
        const retry = records.find(
          (r) => r.type === "response" && r.id === "production-retry",
        );
        expect(retry?.success).toBe(true);
        expect(retry?.data.duplicate).toBe(true);
        expect(prepares).toBe(1);
        expect(closed).toBe(false);
      }
    } finally {
      __piPhone.close(engine);
      await pump(() => closed);
      await rm(root, { recursive: true, force: true });
    }
  });

let originalFetch, originalPhone, originalHome, originalTemp;
beforeAll(() => {
  originalFetch = globalThis.fetch;
  originalPhone = globalThis.__piPhone;
  originalHome = process.env.HOME;
  originalTemp = process.env.TMPDIR;
  install();
});
afterEach(() => {
  if (originalHome === undefined) delete process.env.HOME;
  else process.env.HOME = originalHome;
  if (originalTemp === undefined) delete process.env.TMPDIR;
  else process.env.TMPDIR = originalTemp;
});
afterAll(() => {
  globalThis.fetch = originalFetch;
  if (originalPhone === undefined) delete globalThis.__piPhone;
  else globalThis.__piPhone = originalPhone;
  if (originalHome === undefined) delete process.env.HOME;
  else process.env.HOME = originalHome;
  if (originalTemp === undefined) delete process.env.TMPDIR;
  else process.env.TMPDIR = originalTemp;
});
