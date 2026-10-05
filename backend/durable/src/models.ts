/** Reuse stock Pi's model/auth runtime, not its agent/session/extension runtime. */
import { ModelRuntime, type CreateModelRuntimeOptions } from "@earendil-works/pi-coding-agent";

export async function desktopModels(options: CreateModelRuntimeOptions = {}): Promise<ModelRuntime> {
  // Same auth.json locks, token refresh, built-ins, models.json and cached catalogs as stock Pi.
  // No catalog/provider network requests just to start or enumerate the GUI's models.
  return ModelRuntime.create({ ...options, allowModelNetwork: false });
}
