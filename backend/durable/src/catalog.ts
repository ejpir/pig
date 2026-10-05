/** Non-secret GUI projections. Kept separate so the faux fixture does not import stock Pi. */
import type { Api, Model } from "@earendil-works/pi-ai";
import type { Models } from "@earendil-works/pi-ai/models";

/** Only public model metadata crosses SSH. In particular, no configured headers or credentials. */
export function publicModel(model: Model<Api>) {
  return {
    id: model.id, provider: model.provider, name: model.name, api: model.api,
    reasoning: model.reasoning, input: model.input, cost: model.cost,
    contextWindow: model.contextWindow, maxTokens: model.maxTokens,
  };
}

export async function authProviders(models: Models) {
  return Promise.all(models.getProviders().map(async (provider) => {
    const check = await models.checkAuth(provider.id);
    return {
      id: provider.id, name: provider.name,
      authType: provider.auth.oauth ? (provider.auth.apiKey ? "api_key_or_oauth" : "oauth") : "api_key",
      status: check ? { type: check.type, source: check.source ?? (check.type === "oauth" ? "OAuth" : "configured") } : null,
    };
  }));
}
