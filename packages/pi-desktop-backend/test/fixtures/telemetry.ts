import type { ExtensionAPI } from '@earendil-works/pi-coding-agent';

// Synthetic notification only. This fixture never invokes a model or a tool.
export default function (pi: ExtensionAPI) {
  pi.on('session_start', (_, ctx) => {
    ctx.ui.notify('TPS 87.3 tok/s. out 2,344, in 8,132, cache r/w 13,824/0, total 24,300, 26.9s', 'info');
  });
}
