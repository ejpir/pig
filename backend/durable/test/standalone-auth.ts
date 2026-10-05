/** Compiled regression probe: synthetic credentials only, no inference/login/token exchanges. */
import { readFileSync } from "node:fs";
import { desktopModels } from "../src/models.ts";

const originalFetch = globalThis.fetch;
let networkRequests = 0;
globalThis.fetch = Object.assign(async () => {
  networkRequests++;
  throw new Error("Standalone auth regression must not make network requests");
}, { preconnect: originalFetch.preconnect }) as typeof fetch;
try {
  const path = process.argv[2];
  const provider = process.argv[3];
  if (!path || !provider) throw new Error("Expected isolated auth.json path and provider ID");
  const credential = JSON.parse(readFileSync(path, "utf8"))[provider];
  if (credential?.access !== "FAKE-ACCESS-NEVER-SEND" || credential?.refresh !== "FAKE-REFRESH-NEVER-SEND"
      || !(credential.expires > Date.now() + 600_000)) throw new Error("Only unexpired synthetic test credentials are accepted");
  const models = await desktopModels({ authPath: path, modelsPath: null });
  const resolved = await models.getAuth(provider);
  const access = resolved?.auth.apiKey ?? resolved?.auth.headers?.Authorization;
  if (!access?.includes(credential.access) || networkRequests !== 0) throw new Error("Expected non-network auth derivation");
  // Only a boolean crosses stdout; credentials/headers must never be printed.
  console.log(JSON.stringify({ provider, derived: true, networkRequests }));
} catch (error) {
  // This executable only receives synthetic test credentials; inspect the otherwise-hidden loader cause.
  for (let current: unknown = error; current instanceof Error; current = current.cause) {
    console.error(current.message);
  }
  process.exitCode = 1;
}
