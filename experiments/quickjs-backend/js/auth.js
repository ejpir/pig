// Trusted packaging adapter only. Provider flows, PKCE, polling, refresh and conversion
// remain upstream. This VM has no workspace or model-generated program capability.
import { createModels } from '@earendil-works/pi-ai/models';
import { registerBunOAuthFlows } from '@earendil-works/pi-ai/bun-oauth';
import { anthropicProvider } from '@earendil-works/pi-ai/providers/anthropic';
import { openaiCodexProvider } from '@earendil-works/pi-ai/providers/openai-codex';
registerBunOAuthFlows();

const emit = event => __emit(JSON.stringify(event));
globalThis.startAuth = async (json) => {
  try {
    const options = JSON.parse(json);
    const provider = options.provider;
    if (!['anthropic', 'openai-codex'].includes(provider)) throw new Error('Unsupported provider');
    const check = id => { if (id !== provider) throw new Error('Wrong credential scope'); };
    let lock = Promise.resolve();
    const credentials = {
      async read(id) { check(id); return JSON.parse(__vaultRead()) ?? undefined; },
      async list() { const value = await this.read(provider); return value ? [{ providerId: provider, type: value.type }] : []; },
      async modify(id, update) {
        check(id);
        const work = lock.then(async () => {
          const current = await this.read(id);
          const next = await update(current);
          if (next !== undefined) {
            // Fail closed on malformed token responses before persisting upstream output.
            if (next.type !== 'oauth' || typeof next.access !== 'string' || !next.access ||
                typeof next.refresh !== 'string' || !next.refresh || !Number.isFinite(next.expires) ||
                (provider === 'openai-codex' && (typeof next.accountId !== 'string' || !next.accountId)))
              throw new Error('Invalid OAuth credential');
            __vaultWrite(JSON.stringify(next));
          }
          return next ?? current;
        });
        lock = work.catch(() => {});
        return work;
      },
      // Native logout changes the generation atomically, including while login/refresh
      // is in flight. It is not queued behind an interactive prompt or network request.
      async delete() { throw new Error('Logout belongs to the native account owner'); },
    };
    const models = createModels({ credentials, authContext: { env: async () => undefined, fileExists: async () => false } });
    models.setProvider(provider === 'anthropic' ? anthropicProvider() : openaiCodexProvider());
    let resolved;
    if (options.operation === 'refresh') {
      resolved = await models.getAuth(provider, options.handoff ? { minOAuthValidityMs: 5 * 60 * 1000 } : undefined);
      if (!resolved || resolved.source !== 'OAuth') throw new Error('No OAuth account');
    } else if (options.operation === 'login') {
      let nextPrompt = 0;
      const pending = new Map();
      globalThis.hostInput = event => {
        const prompt = pending.get(event.id);
        if (!prompt) return;
        pending.delete(event.id); prompt.clean();
        emit({ kind: 'submitted' }); prompt.resolve(event.value);
      };
      await models.login(provider, 'oauth', {
        prompt(prompt) {
          if (prompt.type === 'select') {
            const choice = provider === 'anthropic' ? 'copy_code' : 'device_code';
            if (!prompt.options.some(option => option.id === choice)) throw new Error('Mobile login method unavailable');
            return Promise.resolve(choice);
          }
          if (provider !== 'anthropic' || prompt.type !== 'manual_code') throw new Error('Unsupported mobile auth prompt');
          return new Promise((resolve, reject) => {
            prompt.signal?.throwIfAborted();
            const id = `${options.session}:${++nextPrompt}`;
            const abort = () => { pending.delete(id); reject(prompt.signal.reason); };
            const clean = () => prompt.signal?.removeEventListener('abort', abort);
            pending.set(id, { resolve, clean });
            prompt.signal?.addEventListener('abort', abort, { once: true });
            emit({ kind: 'prompt', id });
          });
        },
        notify(event) {
          if (event.type === 'auth_url') emit({ kind: 'browser', url: event.url });
          if (event.type === 'device_code') {
            emit({ kind: 'browser', url: event.verificationUri });
            emit({ kind: 'device', code: event.userCode });
          }
        },
      });
    } else throw new Error('Unsupported auth operation');
    // Native-only handoff: this resolved request auth never enters UI snapshots.
    __finish(JSON.stringify({ ok: true, ...(options.handoff ? { resolved } : {}) }));
  } catch {
    // Upstream error messages may contain HTTP bodies, refresh tokens, or pasted codes.
    // Never copy them into UI snapshots or telemetry.
    __finish(JSON.stringify({ ok: false }));
  }
};
