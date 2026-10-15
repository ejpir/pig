# Embedded Bun / Durable Android — experimental

Android Local is integrated into the shared UI with native Claude/Codex accounts,
stock Durable tools and persistent SQLite sessions. **Bun 1.4.3** is source-linked
as a genuine shared library; Pi/Durable stays pinned and unchanged at **1.2.0**.
Bun's CLI `main` is never called. The stock shell tool may spawn Android sh.

## Release/CI build

`.github/workflows/desktop.yml` source-builds the pinned ARM64 runtime in a separate
job with LLVM 23, NDK **29.0.14206865**, nightly **2026-09-15** Rust/rust-src and
bootstrap Bun **1.4.2**. `prepare.py` checks the source archive SHA-256 and adds the
explicit embedding module. Upstream `--ci=ON --lto=OFF` embeds built-in JavaScript;
matching JSC is pinned at `0c06faadf65bf8e8c8ad3a5a8aca83e1e9ed653f`.
The public Bun source revision is `c6da4a4d3010e5553438c60f6bd76d981976867c`.

```sh
python3 experiments/bun-embedding/build_runtime.py \
  --work-dir /scratch/bun-build --ndk /path/to/android-ndk-r29 \
  --llvm /usr/lib/llvm-23 --out-dir /scratch/bun-runtime --jobs 2
(cd crates/pi_local && npm ci --ignore-scripts)
(cd backend/durable && npm ci --ignore-scripts)
python3 crates/pi_android/scripts/build_apk.py \
  --bun-library /scratch/bun-runtime/libbunembeddingpoc.so --out dist/pi.apk
```

CI packages that library plus the bundled SDK in the **normal non-debuggable
`dev.pi.android` APK**, checks architecture/package/version/bundle hashes/notices,
and publishes runtime and LGPL relink-object archives alongside desktop/helpers.
The separate synthetic `--bun-backend-test` APK retains
`dev.pi.android.quickjstest` and its existing signing key/data. Never uninstall or
clear it; authorized upgrades use `adb install --no-incremental -r`.

## Runtime boundary

One permanent native thread owns one Bun VM. Its ABI exports `pi_bun_init`,
`pi_bun_eval`, `pi_bun_pump`; actual timers/microtasks/I/O use cooperative zero-wait
pumping. There is no patched executable, CLI subprocess, SDK fork, Node shim,
custom tool engine, memory-storage fallback or companion APK requirement.
`backend/durable/android/local.js` imports the desktop `backend/durable/src/run.ts` host boundary;
`backend/durable/android/native-models.js` wraps public provider APIs with request-scoped native auth/HTTP.
Native Vault alone owns stored OAuth and refresh. Local/SSH share the same
`pi_core::durable` presentation projection; SQLite owns execution/recovery.

The earlier QuickJS chat/compaction prototypes are archived at
`archive/quickjs-before-bun-cleanup`, not used as a fallback. QuickJS remains for
native-owned upstream OAuth only. Standalone JNI/SDK probes in this directory
remain useful offline runtime regressions, not a second product backend.

## Verified scope and limitations

ARM64 phone integration and completed-session persistence were confirmed by the
user. Synthetic emulator checks exercised unchanged stock tools, real SQLite,
fresh-process reopening, request deduplication, navigation without cancellation,
and explicit Stop. Source/observations and preserved APKs are documented in
[device verification](../../crates/pi_android/scripts/BUN_LOCAL_VERIFICATION.md).
Automated checks do not inspect real credentials or call live providers.

**Experimental and deliberately not sandboxed.** Android sh is not GNU Bash or a
Linux userland. No complete Activity/lifecycle, long-background execution,
arbitrary mid-tool crash recovery, production security or comprehensive live
provider validation is claimed. Bun changes process-wide stdio/signals and ICU
hooks; the VM cannot be unloaded or reinitialized. Debug-derived runtime size is
large. Completed-session recovery is not exactly-once shell/filesystem recovery.

## JavaScript tool verification

The shared Durable `javascript` tool runs `.js`/`.mjs`/`.cjs` files using real
Node-compatible workers in the same embedded Bun library, not the SDK/auth realm
or a Bun CLI. See [tool semantics and limitations](../../backend/durable/README.md#javascript-files-experimental).

The offline `javascript_tool_probe.ts` bundles the actual executor and SDK tool.
Build it with bootstrap Bun, package it with the existing JNI probe builder and
run the emulator-only verifier (omit `--durable`, whose assertions describe the
older fixed stock-tool probe):

```sh
bun build experiments/bun-embedding/javascript_tool_probe.ts --target=bun \
  --outfile=/scratch/javascript-probe.mjs
python3 experiments/bun-embedding/build_apk.py \
  --library /scratch/x64/libbunembeddingpoc.so --abi x86_64 \
  --sdk-bundle /scratch/javascript-probe.mjs \
  --sdk-notices crates/pi_local/generated/BUN-DURABLE-NOTICES.txt \
  --out /scratch/javascript-probe.apk
python3 experiments/bun-embedding/verify_android.py \
  --adb "$HOME/Android/Sdk/platform-tools/adb" --serial emulator-5554 \
  --apk /scratch/javascript-probe.apk --output /scratch/javascript-evidence
```

This updates only `dev.pi.bunembedding` on the emulator, preserves its old SQLite
state, and creates unique fixture directories. Assert every `durable.checks`
value and `durable.results.sdk.passed` in the fresh report. Tests cover real Node
imports, relative ESM/CJS imports, literal argv, independent globals/environment,
worker-local exit, filesystem effects, UTF-8/output bounds, infinite-loop
cancellation/timeout, parent liveness, and a subsequent successful worker.

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
export ANDROID_NDK_ROOT="/path/to/android-ndk-r29"
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
