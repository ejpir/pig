/** Reuse stock Pi's model/auth runtime, not its agent/session/extension runtime. */
import { ModelRuntime, type CreateModelRuntimeOptions } from "@earendil-works/pi-coding-agent";
import { registerBunOAuthFlows } from "@earendil-works/pi-ai/bun-oauth";
import { bedrockProviderModule } from "@earendil-works/pi-ai/bedrock-provider";
import { setBedrockProviderModule } from "@earendil-works/pi-ai/compat";

export async function desktopModels(options: CreateModelRuntimeOptions = {}): Promise<ModelRuntime> {
  // Match stock Pi's standalone setup without loading its CLI/agent/extension runtime.
  // OAuth and Bedrock use bundler-opaque imports otherwise: catalog discovery works,
  // but the compiled binary fails when a request first derives auth or loads Bedrock.
  registerBunOAuthFlows();
  setBedrockProviderModule(bedrockProviderModule);
  // Same auth.json locks, token refresh, built-ins, models.json and cached catalogs as stock Pi.
  // No catalog/provider network requests just to start or enumerate the GUI's models.
  return ModelRuntime.create({ ...options, allowModelNetwork: false });
}
