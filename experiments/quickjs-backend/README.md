# Pi in QuickJS — Android backend PoC

**Yes: upstream Pi's agent loop and provider implementation run in QuickJS without source patches.** This experiment runs their published JavaScript (built from their TypeScript), not a rewritten agent or a replacement SSE parser.

It lives in its own Cargo workspace so the standalone backend can be tested without GPUI or a sibling Zed checkout. An opt-in, separately packaged build of the existing Android app now embeds this library and provides an offline test screen. Production builds and SSH sessions are unchanged.

## What is reused unchanged

| Component | Pinned dependency | Responsibility |
| --- | --- | --- |
| Agent loop | `@earendil-works/pi-agent-core@1.2.0` | turns, tool execution, events, follow-up requests |
| Pi AI | `@earendil-works/pi-ai@1.2.0` | transcript conversion, schema validation, streaming provider adapter |
| Provider SDK | `openai@7.19.0` (through Pi AI) | HTTP requests, SSE parsing, streamed tool arguments, provider errors |
| JS runtime | `rquickjs@0.14.0` | QuickJS embedded in the Rust process |

Only public package exports are imported. No copied upstream source, dependency patches, provider aliases, or replacement `pi-ai` implementation. npm's lockfile pins the complete dependency tree.

```text
Rust worker / future Android UI event channel
    ↕ JSON events and host callbacks
QuickJS
    ├── unmodified Pi agent loop
    ├── unmodified Pi OpenAI Chat Completions adapter + OpenAI SDK
    ├── ordinary read/write tool registrations → Rust workspace capability
    └── Web API polyfills + small fetch/timer bridge
                ↕
Rust: HTTPS + cancellation, file access, monotonic clock, OS randomness
```

The tools are deliberately small host-provided tools, **not** the full stock coding-agent tool set. `pi-coding-agent`/`pi-durable`, their sessions/extensions, and other providers have not been ported or validated.

## Run locally

Requires Rust (the repository's toolchain), npm, and **Bun 1.4.2 at build time only**. Linux/macOS hosts; file capabilities also build for Android. Windows is outside this experiment.

```sh
cd experiments/quickjs-backend
npm ci --ignore-scripts
bun run build
cargo test --locked
cargo run --locked -- --fixture
```

`--fixture` uses only an ephemeral loopback HTTP server, a synthetic credential, and temporary files. It never calls a model provider or accesses a real project. It drives the **actual upstream SDK and agent** through:

1. Stream a `read(seed.txt)` tool call.
2. Receive the file result in the next model request.
3. Stream a `write(result.txt)` call with arguments in multiple deltas.
4. Read the result back, then stream `Saved 🐷`.

The fixture deterministically delivers HTTP body bytes to JS **one at a time**, including inside UTF-8 characters and SSE delimiters; this does not depend on TCP chunk boundaries. It asserts tool IDs/results in follow-up requests, file contents, and the final assistant reply. JSONL on stdout contains upstream events.

Tests also cover invalid tool arguments, truncated tool calls, path traversal, symlinks/hardlinks, HTTP 401, broken response bodies, the upstream SDK's timer/AbortSignal timeout, and cancellation/deadlines/drop closing the HTTP connection before and after response headers.

For an explicitly requested **live** run:

```sh
mkdir -p /tmp/pi-quickjs-playground
export PI_QUICKJS_API_KEY='your-provider-key'
cargo run --locked -- --live /tmp/pi-quickjs-playground \
  https://api.openai.com/v1 YOUR_CHAT_COMPLETIONS_MODEL \
  'Write a short greeting into hello.txt, read it back, and confirm.'
```

This can incur provider charges and overwrite files in the chosen workspace. Use a disposable directory. Choose a model supporting Chat Completions and function tools. No live provider call was made during validation.

## Embedded Android app test (recommended)

From the repository root, after installing the experiment's npm dependencies:

```sh
python3 crates/pi_android/scripts/build_apk.py --quickjs-backend-test \
  --target x86_64-linux-android --out dist/pi-quickjs-test.apk
adb -s emulator-5554 install -r dist/pi-quickjs-test.apk
python3 crates/pi_android/scripts/test_quickjs_backend.py \
  --serial emulator-5554 --output artifacts/quickjs-apk-emulator
```

Package **`dev.pi.android.quickjstest`**, label **Pi local backend tests**. On Connect, tap **Try the local QuickJS backend**. This links QuickJS into `libpi_android.so` and runs under the app UID, not adb shell. There is no executable extraction. The device test covers the streamed tool cycle, duplicate admission, Stop, background/Back cancellation, and restart. Use `--target aarch64-linux-android` for a phone APK.

See [APK verification](../../crates/pi_android/scripts/QUICKJS_BACKEND_VERIFICATION.md). Provider login and paid local sessions are not exposed yet; see the [provider-auth plan](PROVIDER_AUTH_PLAN.md).

## Standalone Android shell probe

The earlier shell-only proof is still available:

```sh
rustup target add aarch64-linux-android x86_64-linux-android
export ANDROID_NDK_HOME=/path/to/ndk
./build-android.sh aarch64-linux-android
./build-android.sh x86_64-linux-android

python3 test-android.py --serial emulator-5554 --target x86_64-linux-android
# An arm64 device can run the same isolated fixture:
python3 test-android.py --serial DEVICE --target aarch64-linux-android
```

The script creates a unique `/data/local/tmp/pi-quickjs-poc.*` directory, pushes only the native test executable, runs the offline fixture, saves evidence under `generated/`, then removes its executable and empty directory. It does not install an APK or change app settings, keys, sessions, or device services. If interrupted, inspect the printed directory before retrying; cleanup does not recursively delete unknown files.

**Verified:** standalone host tests and Clippy; Android x86_64 emulator shell and APK fixtures (50 upstream events); arm64 Android executable and APK compilation; 349 Android host tests passed (2 ignored). An attempted physical-device run lost responsive adb-shell access and was not counted as a pass; its temporary directory could not yet be inspected/cleaned. No repeated execution was attempted.

**Important boundary:** the shell probe alone does not prove APK-UID behavior. That boundary is now exercised separately by the embedded app test above. It remains an offline fixture: production local-session UI, provider credentials, background services, durability, and release readiness are still outstanding.

## Compatibility layer and upgrade path

- `js/agent.js`: application wiring, explicit model descriptor and two tool registrations. No agent loop or provider parsing code.
- `js/encoding.js`: standard encoding polyfill loaded first.
- `js/platform.js`: existing URL, Headers, AbortController, streams and structured-clone packages, plus host-backed timers/fetch/random/clock. `Response` is a narrow JSON/stream adapter. `FormData` intentionally throws: uploads are unsupported.
- `src/lib.rs`: a separate VM per run, promise-job pumping, typed host callbacks, events, cancellation and deadline handling. Never reuse the privileged backend VM for model-generated `@Visualize` programs.
- `src/http.rs`: transport bytes only; fixed destination, Rust-owned credential, TLS, request/response limits, cancellable I/O. No SSE or provider protocol implementation.
- `src/workspace.rs`: descriptor-relative UTF-8 read/write; rejects symlink traversal and hardlinked/non-regular files.
- `src/fixture.rs`: synthetic provider for reproducible compatibility tests, with bounded/cancellable request parsing because loopback is not UID-private.
- `src/app_fixture.rs`: offline app-view owner, bounded snapshots, cancellation/cleanup admission, private telemetry, and terminal failure supervision.

To upgrade, update both Pi package versions together, regenerate `package-lock.json`, then:

```sh
npm ci --ignore-scripts
bun run test                     # rebuilds JS, then runs Cargo tests
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
./build-android.sh x86_64-linux-android
python3 test-android.py --serial emulator-5554 --target x86_64-linux-android
```

No patches to rebase. Upstream API/runtime changes can still require shim changes: QuickJS is not an upstream-supported Node replacement. `build.ts` records dependency versions and input/output hashes in `generated/manifest.json`; `build.rs` rejects stale bundles, so Cargo cannot silently validate old JS after a source/lockfile update. Generated bundles are ignored, embedded at compile time, and require neither npm nor Bun at runtime.

## Limits and safeguards

- Text-only Chat Completions with API-key auth; no OAuth, discovery, images, uploads, shell, Git, extensions, subagents, swarm, or durable history.
- In-memory transcript, one prompt/run, at most eight model requests, default 90-second live deadline. File writes are not transactional and cancellation does not roll back completed effects or guarantee a provider stops billing.
- Backend JS is **trusted packaged code**. Host functions are capabilities, not an OS sandbox. This is not a safe executor for arbitrary generated JS. Same-UID malicious filesystem races are outside the file boundary.
- Existing workspace directory and parent directories required; files limited to 256 KiB. Absolute paths and parent traversal rejected. Rust credentials are not exposed to JS or model events; callers must not store credentials inside the model-readable workspace.
- HTTPS only for live calls; no redirects/proxies, exact configured endpoint only. Loopback HTTP is fixture-only. Credentials are injected by Rust rather than forwarded from JS.
- 96 MiB JS heap, 1 MiB JS stack, 1 MiB request body, 4 MiB response body, bounded host message channel. JS stream buffering is bounded by response/heap limits, **not** demand-driven end-to-end backpressure.
- Dropping a run requests cancellation; joining waits for its worker and HTTP cleanup. The callback runs on the worker and must not block indefinitely. A UI must marshal events onto its own event queue.
- Polyfills cover the exercised path, not every Web/Node/Bun API. The legacy `text-encoding` package is a convenient PoC dependency; reassess polyfills and dependency notices before shipping.
