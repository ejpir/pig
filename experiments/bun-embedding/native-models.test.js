import { test, expect, beforeEach, afterEach } from "bun:test";
import { phoneModels, installNativeFetch } from "./native-models.js";
const user = {
  role: "user",
  content: "offline synthetic transport test",
  timestamp: 1,
};
for (const provider of ["anthropic", "openai-codex"])
  test(
    provider +
      " uses native request auth/transport and releases before terminal",
    async () => {
      let prepared = 0,
        released = 0,
        requests = [];
      const payload = Buffer.from(
        JSON.stringify({
          "https://api.openai.com/auth": {
            chatgpt_account_id: "acct_synthetic_only",
          },
        }),
      ).toString("base64url");
      const fake =
        provider === "anthropic"
          ? "sk-ant-oat01-synthetic-only"
          : "synthetic." + payload + ".synthetic";
      const bridge = {
        async call(engine, type) {
          if (type === "account_status") return true;
          if (type === "prepare") {
            prepared++;
            return {
              operation: 1,
              resolved: { auth: { apiKey: fake }, source: "OAuth" },
            };
          }
          if (type === "release") {
            released++;
            return true;
          }
          throw Error("unexpected operation");
        },
        async fetch(operation, request) {
          requests.push(request);
          return new Response(
            JSON.stringify({
              error: {
                type: "invalid_request_error",
                message: "synthetic rejected " + fake,
              },
            }),
            { status: 400, headers: { "content-type": "application/json" } },
          );
        },
      };
      installNativeFetch(bridge);
      const models = phoneModels(1, bridge, false, { mode: "Reply" });
      const model = (await models.getAvailable()).find(
        (m) => m.provider === provider && m.input.includes("text"),
      );
      expect(model).toBeDefined();
      const events = [];
      for await (const event of models.streamSimple(
        model,
        { messages: [user], systemPrompt: "" },
        { maxTokens: 32, maxRetries: 0 },
      )) {
        if (event.type === "done" || event.type === "error")
          expect(released).toBe(1);
        events.push(event);
      }
      expect(prepared).toBe(1);
      expect(released).toBe(1);
      expect(requests.length).toBeGreaterThan(0);
      expect(JSON.stringify(events)).not.toContain(fake);
      if (
        provider === "openai-codex" &&
        requests[0].headers["content-encoding"] === "zstd"
      )
        expect(typeof requests[0].bodyBase64).toBe("string");
    },
  );
test("metadata checks never prepare inference auth", async () => {
  let preparations = 0;
  const bridge = {
    async call(engine, type) {
      if (type === "prepare") preparations++;
      return false;
    },
  };
  const models = phoneModels(2, bridge, false, { mode: "Reply" });
  expect((await models.getAvailable()).length).toBe(0);
  expect(preparations).toBe(0);
});

let previousFetch;
beforeEach(() => {
  previousFetch = globalThis.fetch;
});
afterEach(() => {
  globalThis.fetch = previousFetch;
});
