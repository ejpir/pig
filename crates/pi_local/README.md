# Android Local runtime (experimental)

The Android Local backend embeds **Bun 1.4.3** in-process and uses the unchanged
public **Pi/Durable 1.2.0** SDK and the shared desktop Durable runner. This crate
owns the native VM thread, bounded HTTP transport, writer locks and native
Claude/Codex account leases. It is an opt-in Android/Linux Cargo workspace so
ordinary desktop/Windows builds do not compile an Android embedding runtime.

**QuickJS remains only for native-owned upstream OAuth login/refresh.** The old
QuickJS agent, custom tools, Node shims and proof UI have been removed. Their
source is preserved at `archive/quickjs-before-bun-cleanup`
(`7e3a1fc48400046ca5b55dccf9a1828fb2d8ba62`).

## Execution and accounts

- One permanent Bun VM/owning thread; never Bun's CLI `main`, an executable
  subprocess, a patched executable, or a companion APK.
- Unchanged Durable dispatch, coding tools, SQLite, checkpoints and compaction.
  Local and SSH share `pi_core::durable` for disposable presentation only.
- SQLite is execution authority. The native atomic index discovers sessions and
  retains pending prompt request IDs for idempotent admission after reopening.
- Native encrypted Vault/AccountLease exclusively own stored credentials and
  OAuth refresh/rotation. Bun gets only request-scoped resolved authorization;
  no account files, credential environment variables, transcript/SQLite auth,
  or ambient SDK account loading. Native HTTP teardown is acknowledged before
  delivering a terminal model event.
- Navigation parks Local rather than aborting it. Stop/account invalidation,
  deletion and the existing application-background policy are separate.
- Failed teardown quarantines the session's writer ownership until process exit;
  it is not treated as a successful close.

## Offline checks

Requires the repository Rust toolchain, npm and bootstrap **Bun 1.4.2**.

```sh
(cd crates/pi_local && npm ci --ignore-scripts && bun run build)
(cd backend/durable && npm ci --ignore-scripts)
python3 experiments/bun-embedding/build_local.py
cargo test --locked --manifest-path crates/pi_local/Cargo.toml
cargo clippy --locked --manifest-path crates/pi_local/Cargo.toml --all-targets -- -D warnings
bun test backend/durable/android/native-models.test.js backend/durable/android/local.test.js
cargo test --locked -p pi_android --features bun-backend-test
```

These are synthetic transport/auth/tool fixtures, not permission to inspect real
credentials or call a live provider. See [build/runtime documentation](../../experiments/bun-embedding/README.md)
and [device verification](../pi_android/scripts/BUN_LOCAL_VERIFICATION.md).

## Limitations

Local is **experimental and not sandboxed**. Tools access the app's permissions
and run Android `/system/bin/sh`; the stock tool named `bash` does not supply GNU
Bash, Git, compilers or a Linux userland. App-background execution, arbitrary
mid-tool crash recovery, Activity recreation and production credential/runtime
isolation have not been comprehensively validated. Completed-session persistence
and request deduplication do not promise exactly-once shell/filesystem effects.
