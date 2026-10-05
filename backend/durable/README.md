# Experimental durable SSH backend

This is the first vertical slice, **not yet the default or a stock Pi replacement**. It pins pi-durable/pi-ai/chord and the stock Pi model/auth SDK at 1.0.2 and compiles the JavaScript and Bun 1.4.2 runtime into a standalone worker. Rust keeps ownership/SSH/files; SQLite owns recoverable execution state. No provider is called by the tests.

## Native recovery tests (including on a Mac)

Run from the repository root. You need the project's Rust toolchain and npm on the build machine. The sibling Zed checkout is required by the existing workspace. These tests do not need SSH, credentials or a model subscription:

```sh
cd backend/durable
npm ci --ignore-scripts
npm run check
npm exec --yes --package=bun@1.4.2 -- bun test
npm exec --yes --package=bun@1.4.2 -- bun build test/fixture.ts --compile \
  --outfile ../../artifacts/durable/pi-desktop-durable-fixture
cd ../..
PI_DESKTOP_TEST_DURABLE_RUNNER="$PWD/artifacts/durable/pi-desktop-durable-fixture" \
  cargo test --locked -p pi_remote --test durable -- --ignored --nocapture
```

Four tests cover disconnect/reattach, worker and daemon SIGKILL, persisted follow-ups, safe tool recovery, unsafe non-replay, idempotent admission/collision rejection, and the inherited writer lock after the Rust owner is killed. The fixture is a separate executable containing only a faux model; it is never included in the production runner. Forced crashes may print a truncated-JSONL disconnect diagnostic; the resumed state and receipts are what the tests assert.

Locally validated on Linux arm64; CI runs native Linux amd64/arm64 and macOS arm64 tests. macOS results are not yet locally verified. Windows durable hosts are explicitly refused pending ownership/runtime validation; stock Windows SSH remains supported.

## Build a durable-enabled helper

Build **on the SSH host's OS/CPU**, or on a matching build machine. A Mac helper cannot run on a Linux SSH host.

```sh
cd backend/durable
npm ci --ignore-scripts
npm exec --yes --package=bun@1.4.2 -- bun run build
cd ../..
PI_DESKTOP_DURABLE_BINARY="$PWD/artifacts/durable/pi-desktop-durable" \
  cargo build --locked -p pi_remote --features bundled-durable
```

`build.ts` generates the native runner and its dependency/runtime `NOTICES.txt`. Set `PI_DESKTOP_DURABLE_OUTPUT_DIR` at build time to stage both outside a shared checkout without overwriting another OS's artifacts. The Rust helper embeds both, extracts the worker to a hash-specific cache and verifies the cached bytes before execution. Keep old caches: a running session may use them. Add `bundled-backend` and the existing `PI_DESKTOP_BACKEND_ARCHIVE` when the helper also needs self-contained stock Pi/interactive setup. Durable never invokes that stock executable.

For development without embedding, set `PI_DESKTOP_DURABLE_RUNNER` **on the SSH host** to its standalone executable. This is distinct from `PI_DESKTOP_REMOTE_HELPER`, a desktop-side helper file uploaded over SSH.

An upgraded helper does not hot-reload an already running daemon/runner. A fresh SSH session uses the new code; reconnecting a live old session keeps its existing owner until it is explicitly shut down. Do not kill working sessions merely to update their model metadata.

### `cannot execute binary file` on a Mac

The production runner and the test fixture are different artifacts. Compiling `test/fixture.ts` updates **only** `pi-desktop-durable-fixture`; it does not replace `pi-desktop-durable`, which the helper embeds. In a shared sandbox/host checkout, the production artifact may still be a Linux ELF even though the Rust helper was rebuilt as a Mac executable.

Run `npm exec --yes --package=bun@1.4.2 -- bun run build` in `backend/durable` from your **Mac terminal**, then verify `file artifacts/durable/pi-desktop-durable` from the repository root says **Mach-O 64-bit arm64**, not ELF. Rebuild the helper with the same `PI_DESKTOP_DURABLE_BINARY` command above, fully quit/relaunch the GUI with `PI_DESKTOP_REMOTE_HELPER` pointing to that rebuilt helper, and reconnect. The changed binary/helper hashes select fresh installation/cache paths; do not delete durable session storage or old running caches.

The helper build now checks the embedded runner's executable header against Cargo's **target** OS/CPU (not the build host), and runtime overrides/caches are checked against the execution host. A mismatch fails with the production-build instructions before executing the runner. Header validation does not prove shared-library availability or a complete valid executable.

## Test the app on a Mac using that same Mac as SSH host

1. Enable **System Settings → General → Sharing → Remote Login** for your user.
2. Configure SSH keys/agent and verify the host with `ssh localhost`. Confirm `ssh -o BatchMode=yes localhost true` works without a password prompt. The desktop never disables host verification.
3. Build the helper natively using the commands above. Fully quit old desktop instances before launching the rebuilt desktop.
4. The runner reuses stock Pi's credentials **on the SSH host**: `~/.pi/agent/auth.json` (or the host's `PI_CODING_AGENT_DIR`), including stored API keys and OAuth/subscription tokens. If you already ran `/login` there, no extra API key is needed. Otherwise sign in with stock Pi on that host. Environment/ambient credentials also work when available to the non-interactive SSH shell; desktop credentials are never copied.
5. Launch from the repository root, using an existing absolute project path:

```sh
PI_DESKTOP_REMOTE_HELPER="$PWD/target/debug/pi-desktop-remote" \
  cargo run --locked -p pi-desktop -- \
  --ssh localhost --project "$PWD" --remote-backend durable
```

Choose a configured model before submitting a text prompt. Alternatively use **New session → SSH → Durable · experimental**. Start harmless work, disconnect/close the app and reconnect the saved session. **Stop** is an explicit abort; closing is only detachment. For crash behavior, prefer the no-provider tests above instead of killing arbitrary production shell commands.

For a Linux remote host, run/build the helper on matching Linux, copy that resulting helper to your Mac, and point `PI_DESKTOP_REMOTE_HELPER` at the copied Linux executable. Use your real SSH alias/project in the launch command. You still do not install Node or Bun on the remote execution machine if you supply the bundled helper.

## Credentials and providers

The runner uses the pinned stock SDK's public `ModelRuntime`—not its agent or extension host. This registers all 42 built-in providers, including Kimi Coding, GitHub Copilot, OpenAI Codex and OpenRouter, and loads the host's global `models.json` overrides/custom endpoints and cached catalogs. Provider extensions are still not loaded.

Stored OAuth tokens refresh at request time under stock Pi's credential-file lock and are written back to the same `auth.json`; concurrent sessions/stock processes use that locking protocol. A failed refresh preserves the credential and does not fall back to an environment API key. The GUI reports actual API-key/OAuth status through non-secret metadata; the credential store is not copied to the desktop or durable state. Public model metadata excludes request headers and endpoint credentials. As with stock Pi, tools are not sandboxed and can expose secrets if asked to read or print them.

**Models → Refresh** reloads the host's auth/model configuration without restarting the runner. Catalog discovery is offline/cache-only, so startup and model browsing do not fetch provider catalogs or exchange OAuth tokens. Configured credential commands retain stock Pi's execution/caching behavior. Dynamic providers need an existing cached catalog populated by stock Pi. Creating a new login still uses stock Pi's interactive `/login` on the SSH host; there is no durable-specific GUI/browser login flow yet. No stock Pi agent process is started by the durable runner.

The Bun tests use only temporary credential files and synthetic OAuth refresh functions. They cover built-in registration, stored keys/subscription discovery, locked rotation, failed-refresh retention, model refresh and non-secret wire metadata; they never call a model or a real token endpoint.

## Headless use from a phone

The standalone `pi-desktop-durable` is an internal JSONL worker, **not a network or browser server**. It needs `--state`, `--cwd`, `--key` and the Rust owner's inherited writer lock. Do not bypass that lock by setting internal ownership variables yourself.

Build the durable-enabled Rust helper on the Mac/SSH host, then install it under `~/.pi/desktop/bin/<sha256>/pi-desktop-remote` (or let the desktop upload it). The current phone client discovers helpers there and needs `durable`, `watchers` and `sessions` in `--capabilities`. See [Pi for Android](../../crates/pi_android/README.md) for setup.

Enable SSH, add the phone's public key to the account's `~/.ssh/authorized_keys`, and connect the phone to `user@<computer-IP-or-name>`, **not `localhost`**. You do not need to keep Pi Desktop or a foreground worker running: the phone invokes `connect --stdio` over SSH and creates/reconnects detached daemons on demand. New phone sessions use durable and the host's own credentials. There is no separate Pi HTTP port or browser UI to expose.

## Scope and recovery semantics

- Text prompts, coding tools, streaming committed-state snapshots, steering/follow-ups, stop/clear queue, naming, model/thinking controls and reconnection.
- SQLite WAL uses `synchronous=FULL`; snapshots are not recovery checkpoints. Storage is `~/.pi/desktop/durable/<key>/`, not the binary cache. `PI_DESKTOP_REMOTE_STATE_DIR` redirects test endpoints/storage.
- A detached daemon owns a Rust storage owner, which holds an OS writer lock inherited by the runner. A surviving orphan cannot silently admit a second writer. Owner-pipe EOF closes the harness without withdrawing its pending work. Old process descendants share the daemon-owned process group.
- After worker/daemon/host failure, **explicit reconnect** opens the same storage and calls `resume()`. No unattended boot service or automatic daemon-restart loop yet.
- Unsafe interrupted tools return an interrupted error, not an automatic replay. The model may choose subsequent actions; no task-success or exactly-once external-side-effect guarantee.
- Prompt `requestId` is separate from transport correlation IDs. Admission is acknowledged after commit. `get_submission` resolves an uncertain admission; explicit same-key/same-payload retry deduplicates, a payload collision fails. There is no automatic retry or persistent desktop outbox yet. Receipt retention is bounded at 10,000 prompts without silent key eviction.
- Stock host-side API-key/OAuth credentials, built-in providers and global model configuration are integrated. No interactive durable login, stock extension/skill/template host, images, manual compaction, session-tree/fork migration or Windows durable ownership yet. Unsupported commands fail instead of starting stock Pi or local services.
- Files work through the existing independent SSH channel. Remote terminals and LSP remain separate milestones.

Before making this the default: validate native runtimes/live SSH and real provider/OAuth flows, add resource/extension compatibility, explicit capabilities/UI gating, a persisted desktop outbox and supported migration/version policy. Existing stock sessions retain their original backend.
