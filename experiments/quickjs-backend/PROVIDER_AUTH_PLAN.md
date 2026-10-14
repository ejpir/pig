# Phone provider accounts — Claude and Codex

## Implemented in the opt-in APK

The **Pi local backend tests** APK (`dev.pi.android.quickjstest`) has a top-right **Pi → Local / remote computer** selector, with **Local providers · Claude + Codex** account management. This is separate from SSH-computer settings and the offline agent fixture. Normal production builds remain SSH-based and do not expose provider accounts.

| Account | Upstream provider ID | Phone flow |
| --- | --- | --- |
| Claude subscription | `anthropic` | Sign in → Open browser → return → Enter authorization code (`code#state`) |
| OpenAI Codex subscription | `openai-codex` | Sign in → Open browser → enter displayed device code; upstream polls |

**Check / refresh** calls upstream `models.getAuth()` and refreshes only when its validity window requires it. **Sign out** removes this phone's credential/key; it does not revoke the provider's remote authorization. API-key entry, other providers, and the distinct `openai` ChatGPT callback flow are deliberately not exposed.

No real accounts or provider calls were used to validate this change. Live provider acceptance still needs manual, explicitly authorized verification. **Saved accounts and subscription-backed Agent inference are wired into the existing session/thread/composer screens.** The old standalone Chat Completions path must not be given these subscription tokens.

## Unmodified upstream / small platform adapters

`js/auth.js` uses public exports from pinned `@earendil-works/pi-ai@1.2.0`: `createModels`, `anthropicProvider`, `openaiCodexProvider`, and `registerBunOAuthFlows`. Upstream owns selection, PKCE, state checking, polling/backoff, token exchanges, refresh and provider-specific credential fields. Native code selects only `copy_code` / `device_code`; it does not reproduce those protocols.

The static registration hook bundles all OAuth loaders, but only the two providers are registered. Separate Node HTTP/crypto shims throw for unsupported branches. Bun's `import.meta.url` definition is a build adapter for classic-script IIFE evaluation, not an upstream source patch. Web API adapters supply host-backed SHA-256/randomness/base64, form-body serialization, and modern AbortSignal behavior.

Auth has its own worker-owned QuickJS VM and **no workspace read/write capability**. HTTPS policy is exact, provider-scoped token/device endpoints; redirects/proxies are disabled, and no global bearer is injected. System DNS is used through two bounded process-lifetime lookup workers (two queued requests maximum), so uncancellable OS DNS cannot hold per-request runtime teardown or create unlimited detached lookup threads. A stuck resolver fails closed when capacity is exhausted.

## Credentials and lifecycle

- Complete OAuth records are preserved, including refresh tokens, expiry and Codex `accountId`.
- Android Keystore AES-256-GCM keys are per provider and namespace. Ciphertext is under `getNoBackupFilesDir()/local-providers/accounts/`, outside every agent workspace and `Prefs`.
- AAD binds package, schema, namespace, provider and generation. Atomic replacement syncs the file and containing directory before acknowledging a commit. Missing/invalidated keys and malformed/tampered records fail closed; there is no plaintext fallback or silent reset.
- A durable generation tombstone plus key deletion implements local logout. `auth::logout` additionally revokes attempts at admission, including a worker that has not yet read the vault. Callers must use that coordinated operation rather than calling the low-level `Vault.clear` primitive directly.
- Admission/revocation are scoped by **vault namespace and provider**, so synthetic logout cannot invalidate a real-account refresh. Serialized upstream `CredentialStore.modify` prevents concurrent refresh rotation; native generation CAS rejects late writes after logout. Login cancellation cannot silently hide an already-committed account.
- Browser/background transitions do **not** cancel sign-in. Explicit Cancel/Back does. Login has a 15-minute-plus-exchange ceiling; refresh has a separate owner and deadline, surviving caller cancellation/navigation to persist rotated tokens. Android can still terminate background work: Activity/process destruction interrupts unfinished sign-in and requires an explicit restart. No foreground service or resumable PKCE session is claimed.
- Opening the screen reads stored-account metadata only. No new auth, refresh or inference starts automatically on launch/resume.

Tokens necessarily exist in the **trusted packaged auth runtime's memory**. They never enter model messages, generated Visualize programs or model-readable files. Keystore protection is at rest, not invisibility to trusted JS. This APK is debuggable: authorized adb/debuggers/root can access its process. It is not a production credential-isolation claim.

## Sensitive UI

Claude code entry is a native password-style dialog: no GPUI text drafts, autosave, autofill, personalized learning, automatic clipboard reads/copies, or code-bearing deep links. Replies are request-scoped and one-use; queued/stale dialogs are rejected when the Activity retires. Browser launch uses the system browser, not a model-generated WebView.

The displayed device sign-in code has an explicit **Copy code** button. It copies only the current active code, never tokens or the authorization URL, and sets Android's sensitive-clipboard marker to suppress clipboard previews where supported. Nothing is copied automatically; copy confirmation contains no code.

Screenshot protection is currently **disabled at the user's request**, including on the provider page and native code dialog. Codes, conversations, and recent-app previews may be captured; be careful sharing screenshots. Native authorization-code input remains masked. Link-launch failures no longer log full URLs or exceptions. Raw upstream HTTP/code/token errors are not placed in UI snapshots or logs.

Real accounts emit no geometry/auth telemetry. Synthetic tests emit only status, account-presence booleans, and geometry through a coalescing worker—never auth URLs, device/manual codes or credentials.

## Offline validation

```sh
# Host: real upstream flows running inside QuickJS, synthetic HTTP only.
(cd experiments/quickjs-backend && bun run build && cargo test --locked \
  && cargo clippy --locked --all-targets -- -D warnings)

# Build/install the opt-in APK first; see QUICKJS_BACKEND_VERIFICATION.md.
python3 crates/pi_android/scripts/test_provider_accounts.py \
  --serial emulator-5554 --output artifacts/provider-auth-emulator
```

The explicit test URL `pi://test/local-providers/synthetic` selects an independent **synthetic** vault namespace. It does not accept credentials, destinations or prompts. Only this mode remaps the already-validated upstream destinations to loopback. The ordinary `pi://test/local-providers` URL merely opens the real-account screen; it never starts login.

Verified on the x86_64 emulator under APK UID 10150: Claude native code entry, Codex device polling, browser-style background/return, encrypted persistence across process restart, refresh writes with fresh nonces, tampered ciphertext/AAD and missing-key rejection, failed-write preservation, independent providers, logout/reset, native-dialog dismissal, Cancel/Back, and log/telemetry redaction. Tests use only synthetic credentials and never touch the real-account namespace. Host coverage also checks PKCE, wrong Claude state, stale prompts, logout-before-initial-read and logout-vs-refresh races, refresh failure retention, transport allowlists and bounded DNS capacity.

The updated ARM64 APK was installed successfully via adb on the connected 25019PNF3C phone, in the separate `dev.pi.android.quickjstest` package. Installation is not a physical-device execution/login test.

Known limitation: native Keystore/filesystem operations are synchronous on credential workers and cannot be interrupted by the JS/HTTP deadlines. A stalled OS storage operation can retain admission beyond those deadlines; the UI does not join it, and new conflicting refreshes fail closed rather than abandoning a possibly committed token rotation.

Still unverified: real provider interoperability, physical ARM64 execution, hardware-specific Keystore behavior, and actual power-cut durability/key invalidation. Native Activity-retirement guards are implemented, but process-restart tests are not a complete Activity-recreation stress test. Full slow-down/15-minute device-expiry timing remains upstream behavior rather than a complete accelerated fixture matrix.

## Next increment

Connect the stored accounts to upstream provider-aware streaming (`models.streamSimple`, `anthropic-messages`, `openai-codex-responses` with SSE), not the PoC's fixed Chat Completions adapter. Keep refresh ownership independent of agent cancellation, add explicit inference destination policies, then integrate a local-session UI. Durable sessions, a shell/development environment, background services and release hardening remain separate work.

## Shared local sessions

**Chat on this phone** opens the ordinary thread/composer. Reopening the same provider preserves its in-memory sessions/drafts; selecting the already-active Local backend only dismisses the picker. Changing provider/backend or signing out clears local state. Process restart loses conversations, not saved encrypted accounts. No provider calls start on launch/resume or model/thinking selection; Send is explicit.

The unchanged upstream `Agent` emits normalized message/thinking records through `pi_core::session::Session::apply` and the same Android projection as SSH. Model and supported thinking catalogs are bundled from upstream; they do not establish subscription entitlement. Native text-file and image attachments use the existing importer (eight attachments, four images, each image ≤1 MiB, each text file ≤256 KiB). Text-only models reject image context rather than silently losing it. Only successful canonical upstream turns enter later inference; stopped/failed turns remain visibly excluded.

Local execution has no workspace/tools, command queue, compaction, generated Visualize execution, durable history, or background execution service. These unsupported capabilities are gated individually; thinking, attachments, models and Stop use the normal screens. Stop/navigation/background requests cancellation; the worker keeps draining through credential/socket teardown before another run is admitted.

Bounds: eight chats, 32 turns/chat, 64 context messages, 8 MiB serialized context, 10 MiB inference HTTP body; auth/workspace PoC bodies remain 1 MiB. The VM envelope accommodates JSON escaping. Output text+thinking is capped at 256 KiB, canonical assistant messages at 512 KiB, presentation records at 20,000 / 4 MiB. The 185-second VM deadline does not interrupt a stalled native Keystore call. Model thinking budgets/provider behavior mean `maxTokens:4096` is not a universal output-token cap.
