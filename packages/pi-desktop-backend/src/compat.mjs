// Private Pi imports are deliberately isolated here. Tested against the npm
// artifact, not the desktop-patched sibling checkout. Never resolve from cwd.
// Pi does not export these files, so they are named by path; every path is a
// literal so the standalone build (scripts/build-binary.mjs) can bundle it.
import manifest from '../node_modules/@earendil-works/pi-coding-agent/package.json' with { type: 'json' };

export const PI_VERSION = '0.87.1';
if (manifest.version !== PI_VERSION) {
  throw new Error(`pi-desktop-backend requires Pi ${PI_VERSION}; found ${manifest.version}. Run npm ci in the backend package.`);
}

// Guard stdout before loading the full SDK (or any user extensions).
export const outputGuard = await import('../node_modules/@earendil-works/pi-coding-agent/dist/core/output-guard.js');
export async function loadPi() {
  const [sdk, ai, trust, trustResolution, models, events, theme, shell, http, trustContext, sharing, auth, config, radius, extensions] = await Promise.all([
    import('@earendil-works/pi-coding-agent'),
    import('@earendil-works/pi-ai'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/core/trust-manager.js'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/core/project-trust.js'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/core/model-resolver.js'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/modes/json-event.js'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/modes/interactive/theme/theme.js'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/utils/shell.js'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/core/http-dispatcher.js'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/cli/project-trust.js'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/modes/interactive/session-share.js'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/cli/auth-command.js'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/config.js'),
    import('@earendil-works/pi-ai/providers/radius-config'),
    import('../node_modules/@earendil-works/pi-coding-agent/dist/extensions/index.js'),
  ]);
  const api = { ...sdk, getSupportedThinkingLevels: ai.getSupportedThinkingLevels,
    ...trust, ...trustResolution, createProjectTrustContext: trustContext.createProjectTrustContext,
    resolveModelScopeFromModels: models.resolveModelScopeFromModels,
    toJsonEvent: events.toJsonEvent, theme: theme.theme,
    killTrackedDetachedChildren: shell.killTrackedDetachedChildren,
    configureHttpDispatcher: http.configureHttpDispatcher,
    applyHttpProxySettings: http.applyHttpProxySettings,
    exportSessionForShare: sharing.exportSessionForShare,
    getAuthCredential: auth.getAuthCredential,
    getShareViewerUrl: config.getShareViewerUrl,
    radiusGateway: radius.DEFAULT_RADIUS_GATEWAY,
    builtInExtensions: extensions.builtInExtensions };
  for (const name of ['createAgentSessionRuntime', 'createAgentSessionServices', 'createAgentSessionFromServices',
    'resolveProjectTrusted', 'createProjectTrustContext', 'resolveModelScopeFromModels', 'toJsonEvent',
    'exportSessionForShare', 'getAuthCredential', 'getShareViewerUrl', 'configureHttpDispatcher']) {
    if (typeof api[name] !== 'function') throw new Error(`Unsupported Pi ${PI_VERSION} API: ${name}`);
  }
  return api;
}

// Released 0.87.1 reports boolean preflight results rather than dispositions.
// Observe its actual agent-entry boundary, not prompts/filenames or completion.
// The shim is session-local, restored on preflight or failure, and never mutates
// messages, queues, events, or run lifecycle. Only one prompt preflight at a time.
export function promptWithDisposition(session, command, accepted, failed) {
  if (typeof session._runAgentPrompt !== 'function') {
    throw new Error('Unsupported Pi prompt implementation: missing agent-entry boundary');
  }
  const original = session._runAgentPrompt;
  let started = false;
  let preflightPassed = false;
  let acknowledged = false;
  let queued = false;
  const unsubscribe = session.subscribe(event => {
    if (event.type === 'queue_update' && (event.steering.length || event.followUp.length)) queued = true;
  });
  const restore = () => {
    if (session._runAgentPrompt === observe) session._runAgentPrompt = original;
    unsubscribe();
  };
  function observe(...args) {
    // Extension handlers may run their own model work before consuming the
    // input. That is not a run started by this prompt's completed preflight.
    if (preflightPassed) started = true;
    return original.apply(this, args);
  }
  session._runAgentPrompt = observe;
  const acknowledge = disposition => {
    if (acknowledged) return;
    acknowledged = true;
    restore();
    accepted(disposition);
  };
  void session.prompt(command.message, {
    images: command.images,
    streamingBehavior: command.streamingBehavior,
    source: 'rpc',
    preflightResult: value => {
      if (typeof value === 'string') acknowledge(value);
      else if (value === true) {
        preflightPassed = true;
        queueMicrotask(() => acknowledge(started ? 'started' : queued ? 'queued' : 'handled'));
      }
    },
  }).catch(error => {
    restore();
    if (!acknowledged) failed(error);
  });
}
