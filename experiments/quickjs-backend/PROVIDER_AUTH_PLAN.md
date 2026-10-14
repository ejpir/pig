# Phone-side provider accounts — next increment (not implemented)

The local backend needs its own provider accounts. SSH-helper credentials stay on the computer and must not be copied automatically. The current APK screen is offline-only and has **no provider login or credential persistence**.

## Reuse upstream, don't fork auth

Inspection of pinned `@earendil-works/pi-ai@1.2.0` found the active API is `createModels(...)`, provider factories, and:

- `models.login(providerId, "oauth" | "api_key", interaction, options)`
- `models.logout(providerId)`, `checkAuth`, `getAuth`
- `AuthInteraction.prompt`: text, secret, select, manual_code
- `AuthInteraction.notify`: auth_url, device_code, progress, info

The old `/oauth` entry is type-only; it does not supply an OAuth runtime registry. `registerBunOAuthFlows()` from `/bun-oauth` is the existing static bundling hook. Its implementations have Node imports that need narrow, fail-closed build shims; it must not be assumed QuickJS-compatible just because the agent loop works.

Proposed first accounts:

| Account | Upstream provider ID | Phone flow |
| --- | --- | --- |
| OpenAI Codex subscription | `openai-codex` | Select `device_code`; show verification URL/code, open system browser, poll with expiry/cancel |
| Claude subscription | `anthropic` | Select `copy_code`; open system browser, collect returned authorization code |
| API-key account | exact provider ID | Native password-style entry, explicit save/remove; never label this OAuth login |

These are real upstream branches that avoid a local callback listener. They have **not** been exercised against live accounts here. Other device-code providers can follow. The separate `openai` ChatGPT login and OpenRouter require loopback listeners in this version and should not be advertised as supported until implemented/tested.

## Credential ownership and UI

- A **Local providers** screen, separate from remote-computer settings: Add account, browser/device-code progress, Connected/Expired/Needs sign-in, Sign out.
- Store the complete provider credential including refresh token, expiry, account ID and provider-specific fields. Native storage should use Android Keystore-backed authenticated encryption with ciphertext in app-private, non-backed-up storage, **outside every model-readable workspace**.
- No secrets in `Prefs`, fixtures, screenshots/telemetry, clipboard auto-copy, deep-link query logs, or backups. Secret fields must disable saved-instance-state/autofill as appropriate. Do not ask for provider passwords in the app.
- Inject a serialized per-provider `CredentialStore` (`read/list/modify/delete`) into upstream `createModels`. Keep refresh/persistence semantics upstream. Logout must invalidate pending writes so an in-flight refresh cannot resurrect an account.
- Unmodified Codex/Anthropic implementations inspect actual tokens, so they must be available inside the **trusted packaged backend/auth VM**. The PoC's placeholder-key/Rust-only-token scheme cannot be retained unchanged. Tokens must never reach model messages or the separate, unprivileged `@Visualize` VM.

## Backend/shim changes required

1. Route model calls through upstream provider registration and `models.streamSimple`, not the PoC's fixed `openai-completions` function. Codex needs `openai-codex-responses` (SSE first); Claude needs `anthropic-messages`, including upstream OAuth headers and request transformations.
2. Allow only explicit provider authorization/token/inference destinations in Rust. Auth needs form-encoded POST and some bodyless GETs. Do **not** attach a bearer credential globally to every request or loosen the bridge into an unrestricted fetch proxy.
3. Extend Web API shims with PKCE SHA-256, atob/btoa, abort reasons/throwIfAborted/any/timeout, and supported request body types. An unavailable `node:http` shim can throw for the selected non-loopback flows; it cannot silently pretend a callback server exists.
4. Handle external-browser lifecycle separately from agent runs: opening the browser must not cancel the authorization session as ordinary foreground-only agent work is cancelled. Bound auth lifetime and handle process death explicitly.
5. Upstream refresh starts within five minutes of expiry and deliberately survives caller cancellation (bounded to 15 seconds) so rotated tokens can be saved. Do not destroy its VM mid-refresh when an agent run stops; define a supervised auth-owner lifetime.

## Acceptance before enabling paid local sessions

Use synthetic auth endpoints and credentials first: device polling/cancel/expiry, browser-code input, wrong-state rejection where applicable, refresh rotation and concurrent updates, logout-vs-refresh race, missing/invalidated Keystore key, process restart, and redaction. Verify encrypted persistence on an APK UID. Then manually validate each advertised provider flow with explicit user consent; no unattended real sign-in or provider calls.
