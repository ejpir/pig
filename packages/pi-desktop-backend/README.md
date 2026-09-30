# Pi Desktop backend (trial)

A local Node subprocess that speaks Pi Desktop's JSONL protocol and uses the **published, unpatched Pi 0.99.1 SDK**. It is not a server, a second agent, or a Pi extension. Each desktop session still owns one backend and one Pi session runtime.

## Try it

Node **22.19+** is required. From the repository root:

```sh
npm ci --prefix packages/pi-desktop-backend
export PI_DESKTOP_RPC_ENTRY="$PWD/packages/pi-desktop-backend/src/cli.mjs"
cargo run --release --locked -p pi-desktop -- --project /absolute/path/to/project
```

PowerShell:

```powershell
npm ci --prefix packages/pi-desktop-backend
$env:PI_DESKTOP_RPC_ENTRY = (Resolve-Path packages/pi-desktop-backend/src/cli.mjs).Path
cargo run --release --locked -p pi-desktop -- --project C:\path\to\project
```

This does not require `../pi`, its source resolver, or a globally installed `pi`. `PI_DESKTOP_NODE` still overrides the Node executable. The existing `scripts/pi-rpc.mjs` source launcher remains unchanged as a fallback.

## Standalone build

`node scripts/build-binary.mjs [--platform darwin-arm64] [--out dist]` compiles the backend with Bun (`bun` on PATH or `BUN=…`; tested with 1.4.2) into one executable, plus the pi files pi reads from beside it, and writes that folder as `pi-desktop-backend-<platform>.tar.zst`. It needs `npm ci` first. pi-desktop's `bundled-backend` feature embeds that archive (`PI_DESKTOP_BACKEND_ARCHIVE`), so users need neither Node.js nor pi; see [Built-in backend](../../docs/architecture.md#built-in-backend). `PI_DESKTOP_BACKEND_BINARY=<executable> npm test` runs the suite against a build. `get_backend_info` then also reports `bunVersion`. The build also writes `pi-desktop-backend-notices.txt`, the licenses of the bundled npm packages, and on macOS signs the executable for the hardened runtime when `PI_DESKTOP_CODESIGN_IDENTITY` is set.

**Normal launches use your Pi configuration and can execute tools/extensions, load project resources after trust, and make model calls.** Use an isolated HOME/`PI_CODING_AGENT_DIR` and `--offline` for tests. Offline disables startup network work; it is not a sandbox or a guarantee that a requested prompt cannot run tools. Never point a validation run at your real conversation or credentials.

## What was extracted

- Desktop command handling for session state/history, navigation/labels, fork/clone, naming/listing, models/thinking/cycle scope, settings, resources/packages/trust, export and sharing.
- Correlated responses, delta events, extension UI dialogs/notifications, prompt acknowledgement, queue controls and shutdown. Standard confirm/select responses retain exact values/request IDs; backend timeout/abort emits cancellation to the native dialog host.
- `bash` / `abort_bash` use SDK `user_bash`, `executeBash` and `recordBashResult`, preserving `excludeFromContext` and SDK-owned history/results. This is not a separate desktop execution engine.
- Resource metadata includes SDK-reported loaded extensions/errors, context paths and allow-listed user/project settings keys—not a complete settings-file export or an inferred resource scope. Extension load failures remain inspectable and notify the desktop instead of terminating startup; service/flag errors remain fatal.
- Runtime replacement uses Pi's `AgentSessionRuntime` and recreates cwd-bound services. Subscriptions and extension bindings are rebound to the new session.
- `fork {entryId, cwd}` with `cwd` writes a new session file whose header names that folder, such as a jj workspace, from the branch before that user message; this process keeps its session and returns `sessionPath`/`sessionId`. `get_backend_info.features` lists `fork_cwd`.
- `append_custom_entry {customType, data}` appends a data-only `custom` entry, which pi keeps out of the model's context and copies into forks; only types starting with `pi-desktop-`, since extensions keep their own state in custom entries, and only while idle. `get_custom_entries {customType}` returns every entry of that type from all branches. The desktop keeps which jj change each turn made this way (`pi-desktop-turn`).
- `get_state.activeTools` reports the current SDK loadout, including descriptions and source metadata, using public `getActiveToolNames()`/`getAllTools()` calls. It is not reconstructed from historical tool calls; `[]` means none active.

`get_backend_info` reports backend/protocol/Pi/Node versions, supported command names and limitations. The desktop asks for it once per session and shows the answer under Settings → General → Backend; plain `pi --mode rpc` answers "Unknown command", which the desktop treats as no backend info. The contract test checks every command currently emitted by its `Command::name()` implementation.

### Private APIs, intentionally

`src/compat.mjs` is the only module that resolves Pi's internal module paths. It uses Pi's own project-trust resolver, model-scope resolver, streaming serializer, stdout guard, HTTP setup, built-in extension factories and share exporter. The SDK package version is checked at startup, and `npm-shrinkwrap.json` locks the dependency graph and tarball integrity. Updating Pi is an explicit compatibility/test task.

`prompt` acknowledges with the disposition pi reports from its preflight (`started`, `queued` or `handled`), as pi's own RPC mode does; `steer` and `follow_up` return pi's disposition. `agent_settled`, not `agent_end` or an acknowledgement, remains the completion boundary.

### Sharing

Uses Pi's private **branch exporter**, including its `pi.share` system-prompt/tool metadata, without constructing fake TUI components. The headless wrapper uploads JSONL to Radius with organization visibility when Pi supplies a Radius credential; otherwise it exports Pi HTML and runs `gh gist create --public=false`. It returns the existing structured `{ destination, url, gistUrl? }` response.

Only an explicit, confirmed desktop Share operation uploads data. Credentials are resolved by Pi, never by desktop auth-file reads, and never returned to the renderer. Uploads and `gh` are cancellable and bounded to 25 seconds, below the desktop's current 30-second request deadline. Temporary exports are removed on success, failure and cancellation. Real uploads have **not** been exercised; tests mock the network and `gh`.

## Safety and current limits

- Stdout contains only JSONL. Diagnostics and extension logging go to stderr. Input is strict LF-framed UTF-8 with a 16 MiB record limit; output buffers and concurrent requests are bounded.
- Mutation/preflight gates prevent overlapping destructive session operations. Dialog answers bypass those gates. Background metadata reads and Abort remain available.
- Saved project trust is independent of effective process trust and applies on restart. Untrusted project packages cannot be installed/removed. Package changes do not trigger an automatic reload or update check.
- Session file paths must be absolute, existent and have a bounded valid Pi header. Deleting the active session is refused. Deletion requires a working `trash` executable; unlike the patched CLI, it **never silently falls back to permanent unlink**.
- Native authentication continues to use the desktop's terminal handoff. That handoff needs a runnable `pi` command in the shell; this package's dependency also supplies `node_modules/.bin/pi`, but the desktop does not add it to the terminal PATH automatically. JSON `login`/`logout` requests return an explicit unsupported error, not fabricated auth state.
- This is not a full replacement for the CLI: `--continue`, `--resume`, import/fork startup flags, `--api-key`, interactive modes and CLI maintenance/migration flows are not implemented. Unsupported startup flags fail clearly. Standard user resources and the desktop's explicit LSP extension load through Pi.
- Startup does not refresh model catalogs over the network. Cached/registered available models are reported; unknown metadata remains unknown. Real provider streaming, remote package operations and native macOS/Windows remain unverified.
- No automatic fallback to a different backend after a failure. This package is private and unpublished; release packages embed its standalone build.

## Validate

```sh
npm test --prefix packages/pi-desktop-backend
npm run check --prefix packages/pi-desktop-backend
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-backend.sh
```

Tests launch the actual backend against the locked npm SDK with isolated configuration. They exercise metadata, settings, handled commands, shell context policy/history, confirm/select cancellation, inspectable extension load failures, trust restart semantics, known local package install/remove/explicit reload, saved history and runtime replacement. Transport/sharing tests use controlled peers, synthetic data and upload mocks. The native capture checks the actual desktop's Thread, Models, Resources and Sessions, and asserts a metadata-only command allow-list; it never submits a model prompt or executes a displayed tool.

The adapter follows Pi's MIT-licensed RPC and session-sharing implementations; attribution is in `PI-LICENSE` and the repository's `THIRD_PARTY.md`. The backend package itself is GPL-3.0-or-later, like Pi Desktop.
