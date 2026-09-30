// Private Pi imports are deliberately isolated here. Tested against the npm
// artifact, not the desktop-patched sibling checkout. Never resolve from cwd.
// Pi does not export these files, so they are named by path; every path is a
// literal so the standalone build (scripts/build-binary.mjs) can bundle it.
import manifest from '../node_modules/@earendil-works/pi-coding-agent/package.json' with { type: 'json' };

export const PI_VERSION = '0.99.1';
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
