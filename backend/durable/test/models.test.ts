import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createProvider, type Api, type Model } from "@earendil-works/pi-ai";
import { getBuiltinModels } from "@earendil-works/pi-ai/providers/all";
import { desktopModels } from "../src/models.ts";
import { authProviders, publicModel } from "../src/catalog.ts";

const roots: string[] = [];
const fetch = globalThis.fetch;
let networkRequests = 0;
afterEach(() => {
  globalThis.fetch = fetch;
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  expect(networkRequests).toBe(0); // Also catches requests swallowed by best-effort catalog refresh.
});
function authFile(credentials: unknown) {
  // Any provider/catalog request is a failure, including OAuth refresh to a real service.
  networkRequests = 0;
  globalThis.fetch = Object.assign(async () => { networkRequests++; throw new Error("Tests must not call a provider"); }, { preconnect: fetch.preconnect }) as typeof fetch;
  const root = mkdtempSync(join(tmpdir(), "pi-durable-auth-"));
  roots.push(root);
  const path = join(root, "auth.json");
  writeFileSync(path, JSON.stringify(credentials), { mode: 0o600 });
  return path;
}

const model: Model<Api> = { ...getBuiltinModels("anthropic")[0], id: "test-model", provider: "test-oauth", headers: { Authorization: "NEVER-SEND-HEADER" } };
function oauthProvider(refresh: NonNullable<ReturnType<typeof createProvider>["auth"]["oauth"]>["refresh"]) {
  return createProvider({
    id: "test-oauth", auth: { oauth: {
      name: "Test subscription",
      login: async () => { throw new Error("No login/network in these tests"); },
      refresh,
      toAuth: async (credential) => ({ apiKey: credential.access }),
    } }, models: [model],
    api: { stream: () => { throw new Error("No model calls in these tests"); }, streamSimple: () => { throw new Error("No model calls in these tests"); } },
  });
}

test("stock model runtime registers all built-ins including OAuth providers without network", async () => {
  const runtime = await desktopModels({ authPath: authFile({}), modelsPath: null });
  const ids = runtime.getProviders().map((provider) => provider.id);
  expect(ids.length).toBeGreaterThan(30);
  for (const id of ["kimi-coding", "github-copilot", "openai-codex", "openrouter", "amazon-bedrock", "radius"]) expect(ids).toContain(id);
  expect(runtime.getProvider("kimi-coding")?.auth.oauth).toBeDefined();
});

test("existing stored Pi API keys work and auth status is not reported as environment", async () => {
  const runtime = await desktopModels({ authPath: authFile({ anthropic: { type: "api_key", key: "STORED-KEY-SECRET" } }), modelsPath: null });
  expect((await runtime.getAuth("anthropic"))?.auth.apiKey).toBe("STORED-KEY-SECRET");
  const provider = (await authProviders(runtime)).find((provider) => provider.id === "anthropic")!;
  expect(provider.status?.type).toBe("api_key");
  expect(provider.status?.source).not.toBe("environment");
  expect(JSON.stringify(provider)).not.toContain("STORED-KEY-SECRET");
  expect(await runtime.getAvailable("anthropic")).not.toHaveLength(0);
});

test("existing Pi OAuth credentials appear in the GUI without refreshing tokens", async () => {
  const path = authFile({ "kimi-coding": { type: "oauth", access: "ACCESS-SECRET", refresh: "REFRESH-SECRET", expires: 1 } });
  const runtime = await desktopModels({ authPath: path, modelsPath: null });
  const providers = await authProviders(runtime);
  const provider = providers.find((provider) => provider.id === "kimi-coding")!;
  expect(provider.authType).toBe("api_key_or_oauth");
  expect(provider.status?.type).toBe("oauth");
  expect(await runtime.getAvailable("kimi-coding")).not.toHaveLength(0);
  const wire = JSON.stringify({ providers, models: (await runtime.getAvailable("kimi-coding")).map(publicModel) });
  expect(wire).not.toContain("ACCESS-SECRET");
  expect(wire).not.toContain("REFRESH-SECRET");
  expect(JSON.parse(readFileSync(path, "utf8"))["kimi-coding"].expires).toBe(1);
});

test("two durable runtimes serialize expired OAuth refresh and persist rotated credentials", async () => {
  const path = authFile({ "test-oauth": { type: "oauth", access: "old-access", refresh: "old-refresh", expires: 1 }, anthropic: { type: "api_key", key: "unrelated" } });
  const first = await desktopModels({ authPath: path, modelsPath: null });
  const second = await desktopModels({ authPath: path, modelsPath: null });
  let calls = 0;
  const provider = oauthProvider(async (credential) => {
    calls++;
    expect(credential.refresh).toBe("old-refresh");
    await new Promise((resolve) => setTimeout(resolve, 20));
    return { ...credential, access: "rotated-access", refresh: "rotated-refresh", expires: Date.now() + 3_600_000 };
  });
  first.registerNativeProvider(provider);
  second.registerNativeProvider(provider);
  const results = await Promise.all([first.getAuth("test-oauth"), second.getAuth("test-oauth")]);
  expect(calls).toBe(1);
  expect(results.map((result) => result?.auth.apiKey)).toEqual(["rotated-access", "rotated-access"]);
  const persisted = JSON.parse(readFileSync(path, "utf8"));
  expect(persisted["test-oauth"].refresh).toBe("rotated-refresh");
  expect(persisted.anthropic.key).toBe("unrelated");
});

test("failed OAuth refresh retains the stored credential and does not switch auth methods", async () => {
  const credential = { type: "oauth", access: "old-access", refresh: "old-refresh", expires: 1 };
  const path = authFile({ "test-oauth": credential });
  const runtime = await desktopModels({ authPath: path, modelsPath: null });
  let apiKeyResolutions = 0;
  const provider = oauthProvider(async () => { throw new Error("simulated token rotation failure"); });
  runtime.registerNativeProvider({ ...provider, auth: { ...provider.auth, apiKey: {
    name: "Environment key", resolve: async () => { apiKeyResolutions++; return { auth: { apiKey: "DO-NOT-FALL-BACK" } }; },
  } } });
  await expect(runtime.getAuth("test-oauth")).rejects.toThrow("simulated token rotation failure");
  expect(apiKeyResolutions).toBe(0);
  expect(JSON.parse(readFileSync(path, "utf8"))["test-oauth"]).toEqual(credential);
});

test("model projection removes request headers and endpoint details", () => {
  const wire = JSON.stringify(publicModel({ ...model, baseUrl: "https://user:ENDPOINT-SECRET@example.invalid" }));
  expect(wire).not.toContain("NEVER-SEND-HEADER");
  expect(wire).not.toContain("ENDPOINT-SECRET");
  expect(JSON.parse(wire).id).toBe("test-model");
});

test("global stock models.json endpoints reuse configured auth without exposing private headers", async () => {
  const path = authFile({});
  const modelsPath = join(path, "..", "models.json");
  writeFileSync(modelsPath, JSON.stringify({ providers: { custom: {
    api: "openai-completions", baseUrl: "https://example.invalid/v1", apiKey: "CUSTOM-KEY-SECRET",
    headers: { "x-private": "CUSTOM-HEADER-SECRET" }, models: [{ id: "custom-model" }],
  } } }));
  const runtime = await desktopModels({ authPath: path, modelsPath });
  const available = await runtime.getAvailable("custom");
  expect(available[0]?.id).toBe("custom-model");
  expect((await runtime.getAuth("custom"))?.auth.apiKey).toBe("CUSTOM-KEY-SECRET");
  const wire = JSON.stringify(available.map(publicModel));
  expect(wire).not.toContain("CUSTOM-KEY-SECRET");
  expect(wire).not.toContain("CUSTOM-HEADER-SECRET");
});

test("GUI model refresh sees new host-side credentials without restarting the runner", async () => {
  const path = authFile({});
  const runtime = await desktopModels({ authPath: path, modelsPath: null });
  writeFileSync(path, JSON.stringify({ "kimi-coding": { type: "oauth", access: "new-access", refresh: "new-refresh", expires: Date.now() + 3_600_000 } }));
  await runtime.refresh({ allowNetwork: false });
  const status = (await authProviders(runtime)).find((provider) => provider.id === "kimi-coding")?.status;
  expect(status?.type).toBe("oauth");
  expect(await runtime.getAvailable("kimi-coding")).not.toHaveLength(0);
});
