// Trusted platform adapter. The Agent, provider transforms, thinking semantics and
// SSE parser are the pinned, unchanged upstream implementation.
import { Agent } from '@earendil-works/pi-agent-core';
import { createModels, clampThinkingLevel } from '@earendil-works/pi-ai/models';
import { anthropicProvider } from '@earendil-works/pi-ai/providers/anthropic';
import { openaiCodexProvider } from '@earendil-works/pi-ai/providers/openai-codex';

// The presentation protocol never receives SDK diagnostics or raw provider errors.
function publicMessage(m) {
  const {role, content, api, provider, model, usage, stopReason, timestamp, thinkingLevel} = m;
  return {role, content, api, provider, model, usage, stopReason, timestamp, thinkingLevel,
    ...(['error', 'aborted'].includes(stopReason) ? {errorMessage: 'Reply did not complete.'} : {})};
}
globalThis.startChat = async json => {
  try {
    const input = JSON.parse(json);
    const resolved = JSON.parse(__resolvedAuth());
    if (resolved.source !== 'OAuth' || !resolved.auth) throw Error('Missing prepared auth');
    const provider = input.provider === 'anthropic' ? anthropicProvider() : openaiCodexProvider();
    const models = createModels({ authContext: { env: async () => undefined, fileExists: async () => false } });
    models.setProvider(provider);
    const model = models.getModel(input.provider, input.model);
    if (!model) throw Error('Unknown model');
    const auth = resolved.auth;
    const agent = new Agent({
      initialState: { model, messages: input.messages.slice(0, -1), tools: [],
        thinkingLevel: clampThinkingLevel(model, input.thinkingLevel) },
      transport: 'sse',
      finishTurn: () => ({action: 'end'}),
      // Auth was resolved by the independent refresh owner. Never re-resolve it
      // or give the inference VM access to the refresh-token vault.
      streamFn: (m, context, options = {}) => provider.streamSimple(
        auth.baseUrl ? {...m, baseUrl: auth.baseUrl} : m, context, {
          ...options, apiKey: auth.apiKey,
          headers: {...auth.headers, ...m.headers, ...options.headers}, env: resolved.env,
          transport: 'sse', maxRetries: 0, maxTokens: 4096, timeoutMs: 180000, toolChoice: 'none',
        }),
    });
    const encoder = new TextEncoder();
    let outputBytes = 0, eventBytes = 0, events = 0, exceeded = false;
    const blocks = new Map();
    const fail = () => { exceeded = true; agent.abort(); throw Error('Output limit'); };
    const bytes = value => encoder.encode(value).length;
    const checkMessage = m => {
      const size = (m.content || []).reduce((sum,b) => sum + bytes(b.text || b.thinking || ''), 0);
      if (size > 256 * 1024 || bytes(JSON.stringify(m)) > 512 * 1024) fail();
    };
    const emit = record => {
      const encoded = JSON.stringify({kind: 'record', record});
      eventBytes += encoder.encode(encoded).length;
      if (++events > 20000 || eventBytes > 4 * 1024 * 1024) fail();
      __emit(encoded);
    };
    agent.subscribe(event => {
      if (exceeded) return;
      // User admission is already represented natively, with attachment labels;
      // don't echo multi-megabyte base64 images back over the presentation bridge.
      if (['message_start', 'message_end'].includes(event.type) && event.message.role === 'assistant') {
        checkMessage(event.message);
        emit({type: event.type, message: publicMessage(event.message)});
      } else if (event.type === 'message_update') {
        const e = event.assistantMessageEvent;
        if (/^(text|thinking)_(start|delta|end)$/.test(e.type)) {
          const old = blocks.get(e.contentIndex) || 0;
          const size = e.type.endsWith('_delta') ? old + bytes(e.delta) : bytes(e.content || '');
          outputBytes += size - old;
          blocks.set(e.contentIndex, size);
          if (outputBytes > 256 * 1024) fail();
        }
        if (/^(text|thinking)_(start|delta|end)$/.test(e.type)) {
          const {type, contentIndex, delta, content} = e;
          emit({type: 'message_update', assistantMessageEvent: {type, contentIndex, delta, content}});
        }
      }
    });
    await agent.prompt(input.messages.at(-1));
    const message = agent.state.messages.at(-1);
    if (message?.role !== 'assistant' || !['stop', 'length'].includes(message.stopReason)) throw Error('Incomplete response');
    if (exceeded) throw Error('Output limit');
    checkMessage(message);
    __finish(JSON.stringify({ok: true, message}));
    // Native code emits agent_settled only after the worker/account lease is gone.
  } catch {
    __finish(JSON.stringify({ok: false}));
  }
};
