import { runAgentLoop } from '@earendil-works/pi-agent-core';
import { streamSimple } from '@earendil-works/pi-ai/api/openai-completions';

// Application wiring only: upstream loop, validation, provider adapter and SDK are unchanged.
globalThis.startAgent = (inputJson) => {
  const input = JSON.parse(inputJson);
  const text = (value) => ({ content: [{ type: 'text', text: value }], details: {} });
  const tools = [
    {
      name: 'read', label: 'Read', description: 'Read a UTF-8 file in the workspace (256 KiB maximum).',
      parameters: { type: 'object', properties: { path: { type: 'string' } }, required: ['path'], additionalProperties: false },
      execute: async (_id, args) => text(__read(args.path)),
    },
    {
      name: 'write', label: 'Write', description: 'Create or replace a UTF-8 workspace file. Parent directories must exist. No shell.',
      parameters: { type: 'object', properties: { path: { type: 'string' }, content: { type: 'string' } }, required: ['path', 'content'], additionalProperties: false },
      execute: async (_id, args) => { __write(args.path, args.content); return text('Written'); },
    },
  ];
  const model = {
    id: input.model, name: input.model, api: 'openai-completions', provider: 'openai',
    baseUrl: input.baseUrl, reasoning: false, input: ['text'],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: 32768, maxTokens: 2048,
    compat: { supportsDeveloperRole: false },
  };
  const controller = new AbortController();
  globalThis.cancelAgent = () => controller.abort();
  let turns = 0;
  const config = {
    model, convertToLlm: (messages) => messages,
    // Real API key stays in Rust. The HTTP bridge supplies Authorization.
    getApiKey: async () => 'host-managed',
    maxRetries: 0,
    timeoutMs: input.requestTimeoutMs,
    toolExecution: 'sequential',
    prepareRequest: () => { if (++turns > 8) throw new Error('PoC limit: eight model requests'); },
  };
  const prompt = { role: 'user', content: input.prompt, timestamp: Date.now() };
  runAgentLoop([prompt], { messages: [], tools }, config,
    (event) => __emit(JSON.stringify(event)), controller.signal, streamSimple)
    .then((messages) => __finish(JSON.stringify({ messages })),
      (error) => __finish(JSON.stringify({ error: String(error), stack: error?.stack })));
};
