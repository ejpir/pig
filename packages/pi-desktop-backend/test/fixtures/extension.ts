import type { ExtensionAPI } from '@earendil-works/pi-coding-agent';
import { Type } from 'typebox';

export default function (pi: ExtensionAPI) {
  // Synthetic, local-only metadata. The stream function forbids model requests.
  pi.registerProvider('offline-desktop-fixture', {
    baseUrl: 'http://127.0.0.1:1/forbidden',
    apiKey: 'offline-fixture-not-a-real-key',
    api: 'openai-completions',
    models: ['large', 'small'].map(id => ({ id, name: `Offline ${id}`, reasoning: true,
      input: ['text', 'image'], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: 10000, maxTokens: 1000 })),
    streamSimple: () => { throw new Error('Fixture forbids model calls'); },
  });
  pi.registerTool({
    name: 'backend_probe', label: 'Backend probe',
    description: 'Metadata-only fixture tool; validation never executes it.',
    parameters: Type.Object({}),
    execute: async () => { throw new Error('Fixture forbids tool execution'); },
  });
  pi.registerCommand('backend-tools', {
    description: 'Change the fixture loadout without running a tool or model',
    handler: async () => { pi.setActiveTools(['read', 'backend_probe']); },
  });
  pi.registerCommand('backend-handled', {
    description: 'Metadata-only fixture command',
    handler: async (_, ctx) => { ctx.ui.notify('Handled without a model', 'info'); },
  });
  pi.registerCommand('backend-confirm', {
    description: 'Fixture cancellation probe',
    handler: async (_, ctx) => {
      const confirmed = await ctx.ui.confirm('Fixture confirmation', 'Cancel is safe');
      ctx.ui.notify(confirmed ? 'Confirmed' : 'Cancelled', 'info');
    },
  });
  pi.registerCommand('backend-select', {
    description: 'Synthetic extension-owned decision; no permission enforcement',
    handler: async (_, ctx) => {
      const choice = await ctx.ui.select('Synthetic permission request', ['Allow once', 'Allow for project', 'Block']);
      ctx.ui.notify(choice ?? 'Cancelled', 'info');
    },
  });
  pi.registerCommand('backend-dialog-queue', {
    description: 'Synthetic queued native decisions; no model or tool calls',
    handler: async (_, ctx) => {
      const first = ctx.ui.confirm('Queued fixture confirmation', 'Cancel is safe');
      const second = ctx.ui.select('Queued fixture selection', ['Allow once', 'Block']);
      await first;
      ctx.ui.notify((await second) ?? 'Cancelled', 'info');
    },
  });
  pi.registerCommand('backend-dialog-timeout', {
    description: 'Synthetic timeout probe; no model or tool calls',
    handler: async (_, ctx) => {
      await ctx.ui.confirm('Timed fixture confirmation', 'The backend cancels this request.', { timeout: 3000 });
      ctx.ui.notify('Timed request finished', 'info');
    },
  });
  pi.on('user_bash', async event => event.command === 'fixture:bash' ? {
    result: { output: 'extension handled shell\n', exitCode: 0, cancelled: false, truncated: false },
  } : undefined);
  pi.on('input', async event => event.text === 'fixture:handled' ? { action: 'handled' } : undefined);
}
