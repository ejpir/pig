# Embedded Bun / Durable Android PoC

**Proven on the x86_64 Android emulator:** real Bun **1.4.3-debug** is source-linked into an APK shared library and runs the unchanged **pi-durable 1.2.0** SDK, SQLite adapter and coding tools through JNI.

The trusted offline probe executes **write → read → edit → bash → read**, commits a synthetic model answer, closes/reopens SQLite, and checks persisted history and request deduplication. The verifier then stops **only the test package**, starts a fresh Android process/VM, reopens the **same database/conversation/submission**, verifies the answer/tool results, and confirms retrying the completed request performs **zero new model calls**.

Bun itself is embedded, **not a CLI subprocess**, an executable with patched ELF flags, or JavaScriptCore-only evaluation. The command tool intentionally spawns an actual child shell via the stock adapter. The library contains Bun's real Rust/native object graph, runtime hooks and matching static JavaScriptCore. Its generated executable link inputs are relinked with `-shared`, our JNI caller, `-z defs`, and `-u,main` to retain the full runtime. **`main` is never called.**

## Artifacts and evidence

- SDK/tool APK: `dist/pi-bun-durable-test.apk` (~92 MiB, debug).
- Initial synchronous-only APK retained: `dist/pi-bun-embedding-poc.apk` (~91 MiB).
- Separate package: `dev.pi.bunembedding`; no permissions, credentials or network access. Only predetermined synthetic model/tool inputs.
- SDK evidence: [`artifacts/bun-durable-emulator`](../../artifacts/bun-durable-emulator/).
  - `runtime-report-create.json`: six faux model calls, five real tool executions, committed SQLite results.
  - `runtime-report-recover.json`: different Java/native/Bun PID, same database/conversation/submission, restored receipts/history, zero new model calls.
  - `report.json`, `screen.png`: automated checks and post-return process/Activity liveness.
  - `bundle-provenance.json`, `build-provenance.json`, link commands and ELF/JNI exports: source, dependency and linkage provenance.
  - `attempts/`: retained failed early attempts; **not** passes.
- Initial synchronous-only evidence remains in [`artifacts/bun-embedding-emulator`](../../artifacts/bun-embedding-emulator/). Its `desktop-sdk-smoke.json` is explicitly desktop-only, not Android evidence.

**Reuse confirmed:** stock `CodingTools`, `NodeExecutionEnv`, `Harness`, `SqliteStorage`, and the public `storage/sqlite/node` adapter run unchanged. Bun provides the Node filesystem/process APIs and **`node:sqlite`**. Android needs only the adapter's existing `shellPath: '/system/bin/sh'` option for this command test. The tool is named `bash`, but this does **not** mean GNU Bash, a Linux userland, Git or a compiler is installed.

No real provider requests, credential inspection, phone installation, working-app upgrade or production companion dependency were involved. Existing QuickJS/compaction work is preserved; its Local UI remains unchanged. The shared Cargo `target/` cache remains absent; Bun scratch, APKs and evidence are retained.

## Current scope

This is a successful **embedded SDK/tools/persistence PoC**, not the completed Local backend:

- **Not yet wired to the shared Local UI or native Claude/Codex accounts.** No live inference validation.
- x86_64 emulator only; ARM64 phone build/load remains untested.
- Proves completed-session persistence and deduplication across a process restart, **not** mid-tool crash recovery or power-loss durability of filesystem effects.
- Async imports, timers, I/O, child-process completion and SQLite run through a cooperative, zero-kernel-wait event pump. Budgets apply **between turns**, not within callbacks or synchronous native operations.
- One VM, one permanent thread, process lifetime. No destruction, unload, reinitialization or general Activity lifecycle support.
- Bun initialization changes process-wide stdio/signals and installs ICU allocator hooks. Wider JVM/ICU/signal compatibility testing remains.
- **Deliberately not sandboxed**, per the current PoC scope; isolation is not a gate for these tests. There are no credentials in this package. `cwd` is not a security boundary.
- No automatic Android background-service/restart guarantee. Android can stop the process; this verifier explicitly relaunches it.
- Debug library/APK are large. Release optimization and complete redistribution/licensing review remain; this is a local development test, not a production release.

## Implementation

`native/embedding.rs` is mounted as `src/runtime/pi_embedding.rs` in a pinned Bun source checkout, with one `mod pi_embedding;` line added to its runtime crate. It initializes real `VirtualMachine::init`, evaluates through `Bun__REPL__evaluate`, and pumps real tasks/microtasks/timers/uSockets with zero-wait polling. It never enters CLI dispatch or shutdown. UTF-8 results use caller-owned buffers and JNI byte arrays; JSC values stay native.

`native/jni.cpp` is the real JNI caller. `ProbeActivity.java` keeps every VM call on one permanent 8-MiB-stack worker. It copies the signed SDK asset to private storage, verifies its SHA-256, dynamically imports it, then pumps/polls a JSON completion mailbox. Java/native/Bun PIDs are independently measured, not supplied by a shell.

`durable-probe.js` uses public SDK APIs and an empty in-memory model collection containing only the faux provider. No SDK/Node compatibility shims, SDK source patches, file-auth loading or memory-storage fallback. SQLite is actual app-private storage with `synchronous=FULL`.

**Packaged built-ins matter:** build Bun with `--ci=ON`. Ordinary local debug builds define `BUN_DYNAMIC_JS_LOAD_PATH` and attempt to read built-in JS from the developer's host filesystem; Node module import then aborts on Android. Upstream's CI build semantics embed those sources instead. No extra Bun source patch is needed for this.

Upstream Pi npm code is unchanged. Bun receives an explicit experimental bridge patch; this is not a supported upstream embedding API.

## Reproduce

Needs bootstrap Bun, Git/Python/JDK/npm, LLVM **23**, NASM, Android SDK/NDK, and pinned **nightly-2026-09-15** Rust with `rust-src`. Tested NDK: ONDK r29.5/r29. Use scratch storage (several GiB), not the shared application Cargo target.

```sh
cd experiments/bun-embedding
export WORK=/tmp/pi-bun-embedding-poc
python3 prepare.py --work-dir "$WORK"
export SOURCE="$WORK/source/bun-bun-v1.4.3"
export BUILD="$SOURCE/build/android-x64"
export OUTPUT="$WORK/output-durable"
export RUSTUP_HOME="$WORK/rustup"
export CARGO_HOME="$WORK/cargo"
export RUSTUP_TOOLCHAIN=nightly-2026-09-15
export GIT_SHA=c6da4a4d3010e5553438c60f6bd76d981976867c
rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal \
  --component rust-src --target x86_64-linux-android --no-self-update

# Must contain LLVM 23 tools. Its clang resource directory must be writable:
# Bun links NDK compiler runtimes into it. Do not modify /usr as root.
# Tested: /usr/bin/clang{,++} wrappers adding a scratch -resource-dir;
# scratch resource include -> /usr/lib/clang/23/include; LLVM tools symlinked.
export BUN_TOOLCHAIN_LLVM="$WORK/llvm"
export ANDROID_NDK_ROOT="$HOME/Android/Sdk/ndk/magisk"
export BUN_BUILD_CACHE_DIR="$HOME/.bun/build-cache"
(cd "$SOURCE" && bun scripts/build.ts --profile=android --arch=x64 \
  --ci=ON --lto=OFF --android-api-level=28 --build-dir="$BUILD" \
  --target=bun -d keeprsp -j8)

NDK_TOOLS="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64"
"$NDK_TOOLS/bin/x86_64-linux-android28-clang++" -std=c++20 -fPIC \
  -fno-exceptions -fno-rtti -c native/jni.cpp -o "$WORK/jni-async.o"
python3 relink.py --source "$SOURCE" --build "$BUILD" \
  --ninja "$HOME/.bun/build-cache/ninja/bun-ninja-5ecd8831/ninja" \
  --jni-object "$WORK/jni-async.o" --out "$OUTPUT/libbunembeddingpoc.so"
cp "$OUTPUT/libbunembeddingpoc.so" "$OUTPUT/libbunembeddingpoc.unstripped.so"
"$NDK_TOOLS/bin/llvm-strip" --strip-debug "$OUTPUT/libbunembeddingpoc.so"

# Normal checkout: npm ci --ignore-scripts --omit=dev
# Alternatively install package.json/lock in scratch and symlink node_modules.
npm ci --ignore-scripts --omit=dev
python3 build_sdk_probe.py --out-dir "$WORK/sdk-bundle"
python3 build_apk.py --library "$OUTPUT/libbunembeddingpoc.so" \
  --sdk-bundle "$WORK/sdk-bundle/durable-probe.bundle.mjs" \
  --sdk-notices "$WORK/sdk-bundle/NOTICES.txt" \
  --out ../../dist/pi-bun-durable-test.apk
python3 verify_android.py --durable \
  --adb "$HOME/Android/Sdk/platform-tools/adb" \
  --apk ../../dist/pi-bun-durable-test.apk \
  --output ../../artifacts/bun-durable-emulator
```

The verifier is emulator-only and installs with `--no-incremental -r`. It force-stops **only this synthetic package** and removes **only its previous report file**, retaining SQLite state across the two processes. It never uninstalls/clears the account-bearing app. Standard Android debug signing is used.

`prepare.py` verifies the exact public source archive SHA-256 before extraction. Bun's build fetches pinned dependencies/matching Android WebKit/JSC. The base Git SHA is supplied explicitly because a source archive lacks commit metadata; the bridge patch and dependency lock are recorded separately.

## Integrated Local APK (2026-10-15)

The existing `dev.pi.android.quickjstest` Local UI now has an opt-in
`bun-backend-test` build. It embeds this source-linked Bun library and imports
`backend/durable/src/run.ts` through its optional in-process host boundary.
Execution, tool definitions, compaction, SQLite and prompt deduplication remain
in the unchanged public SDK. SSH and Local share the extracted durable protocol
projection in `pi_core`. Native Vault/AccountLease and native bounded HTTP remain
the account-refresh and provider-transport owners; Bun receives request-scoped
resolved auth, not stored OAuth credentials.

Installed on the ARM64 phone using **`adb install --no-incremental -r`**, with
no uninstall/data clearing and unchanged first-install time/signing/package:

- Retained installed APK: `dist/pi-bun-local-installed-arm64.apk`
- SHA256: `7ca485b9a8b70bc688adb6293cde9960bebf1db24fe61a93e6cd49fcecb4cd7f`
- User confirmed that it works and persistence works. Do not replace this working
  installation automatically while the user is testing.
- Installation/signature/package evidence: `artifacts/bun-local-phone/`.
- A subsequent build is retained separately as `dist/pi-bun-local-test-arm64.apk`;
  it is **not** the installed snapshot above.

Offline checks additionally exercised native-auth provider wrappers for both
Claude/Codex with synthetic transport (including Codex binary/zstd bodies),
release-before-terminal handling, stream cancellation/late events, and the shared
runner's SQLite reopen and persistent-request deduplication. The integrated x64
emulator UI completed a synthetic reply, stock Android-sh tool turn, and reopened
its saved session in a new process (PIDs 21584 and 21943). Evidence is under
`artifacts/bun-local-emulator/`. No real provider inference was performed by these
automated checks, and no real credentials were inspected.

Build an architecture-matching app with:

```sh
CARGO_TARGET_DIR=/home/nick/repos/pig/target \
python3 crates/pi_android/scripts/build_apk.py --bun-backend-test \
  --bun-library /path/to/libbunembeddingpoc.so \
  --out dist/pi-bun-local-test.apk
```

Place the pinned Bun `LICENSE.bun.md` beside that library. The builder generates
SDK notices and an embedded-bundle source/hash manifest. The app includes both
native libraries; no companion APK or Bun CLI process is required.

This remains an in-process PoC, not a sandbox or production lifecycle/security
validation. Full Activity-recreation races, long background execution, arbitrary
mid-tool crash recovery and exhaustive UI/account/compaction regressions remain
follow-up work. Native Local discovery/admission metadata is a presentation and
recovery index; SDK SQLite remains execution authority. Android's stock tool named
`bash` uses `/system/bin/sh`, not GNU Bash or a bundled Linux userland.

### Navigation cancellation regression

The first phone build retained disposable QuickJS navigation cancellation:
leaving a thread, parking Local behind Remote, and navigation URLs could issue
Abort. The Bun feature now treats these as presentation changes, while explicit
Stop, deletion, native account invalidation and the existing app-background
cancellation policy remain separate. QuickJS's original feature behavior is
unchanged.

`crates/pi_android/scripts/test_bun_local_navigation.py` is emulator-only. It
starts the stock SDK `bash` tool using Android sh, switches to the sample Store
(the same Store parking seam as SSH), verifies that the tool completes while
parked, returns to the same Local session, then confirms explicit Stop cancels
a second tool. The regression passed; evidence, including a first failed
**telemetry verifier** attempt and the corrected passing run, is retained under
`artifacts/bun-local-navigation-emulator/`. The first attempt's actual tool also
completed correctly; core projection stores tool results in `tools`, not ordinary
message records.

Ready ARM64 update: `dist/pi-bun-local-navigation-fix-arm64.apk`. This update has
not been installed on the user's phone; the confirmed working installed snapshot
is left untouched pending a convenient time to restart the app.

The user subsequently authorized installing the navigation fix. It was upgraded
in place on the phone at 04:07:52 using `adb install --no-incremental -r`, then
launched successfully (PID 31353). No uninstall or data/account clearing was done;
first-install time remained unchanged. Installed update SHA256:
`05df39ccd41082feefdfac1ded9b18e01f91c7242947f9a8352a45cfe984ed0f`.
Evidence: `artifacts/bun-local-navigation-phone/`. The earlier working APK remains
retained separately.
