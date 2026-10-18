---
name: android-local-install
description: Rebuild and activate the full durable host helper, then safely install the normal dev.pi.android APK with bundled Local mode. Preserve live sessions, signing identity, and app data.
compatibility: x86-64 Linux host with ADB, Android SDK/NDK, Rust, Java, npm, GitHub CLI, and an arm64 Android device.
---

# Host Helper + Android Local Install

Use this skill after a release to rebuild and activate `pi-desktop-remote`, then install or upgrade `dev.pi.android` on the physical phone.

## Safety invariants

- Build the production Durable runner with the exact pinned Bun version; never embed the faux test fixture.
- Build the helper with both `bundled-durable` and `bundled-backend` so it carries Durable and stock Pi.
- Activate through the helper's content-addressed `activate` command; never replace a running executable in place.
- Do not delete old helper objects, Durable runner caches, credentials, pairing keys, or session state.
- Preserve scheduler running/autostart state while repinning an existing service to the new helper.
- Do not stop existing durable session daemons merely to update the helper; fresh sessions use the new binary.
- Never run `adb uninstall`, `pm clear`, or install with a different certificate.
- Always inspect the currently installed APK signer before updating.
- Use `adb install -r --no-incremental`; preserve app data and accounts.
- Install the normal `dev.pi.android` package, never the UI-test or backend-test package.
- Require bundled Local assets: `libbunembeddingpoc.so`, `bun-local.mjs`, and `local-runtime.json`.
- Require Local metadata to match the workspace version and pinned Bun/Pi SDK versions.
- Prefer a non-emulator physical device. If several physical devices are connected, pass `--serial`.
- Put generated Cargo/build files under a temporary directory and clean them afterward.
- Do not stage, commit, reset, or alter unrelated working-tree files.

## Standard command

From the repository root:

```bash
./.pi/skills/android-local-install/scripts/install.sh
```

Optional arguments:

```bash
./.pi/skills/android-local-install/scripts/install.sh \
  --tag v0.0.20 \
  --serial 'adb-device-serial'

# Run only one half when diagnosing or repeating an operation:
./.pi/skills/android-local-install/scripts/install.sh --helper-only
./.pi/skills/android-local-install/scripts/install.sh --android-only --serial 'adb-device-serial'
```

Use `--keep` only when build artifacts are needed for diagnosis.

## Workflow

The script performs these steps fail-closed:

1. Reads the workspace version and controlled Bun/Pi SDK pins.
2. Installs the pinned host Bun in a temporary directory.
3. Builds the production Durable runner and notices, fetches the checksum-pinned stock Pi archive, and builds `pi-desktop-remote` with both bundled engines.
4. Verifies helper version/capabilities, activates it under `~/.pi/desktop/bin/<sha256>/`, and atomically updates the stable symlink.
5. If the scheduler was running or installed for login, cooperatively stops only the scheduler, repins its service, and restores its previous state. Existing durable session daemons continue.
6. Selects a physical ADB device and pulls its installed `dev.pi.android` APK.
7. Records the installed certificate SHA-256 digest.
8. Downloads and checksum-verifies the published release APK.
9. If the release signer matches, installs that exact APK.
10. If it does not match, verifies that the configured local keystore matches the installed signer, downloads the release's source-linked Bun runtime, installs the pinned host Bun, and builds the normal Local APK with that key.
11. Rechecks signer and Local assets before installation.
12. Installs in place and verifies the installed version, signer, and Local metadata.
13. Removes temporary runners, helpers, APKs, runtimes, and Cargo targets unless `--keep` was requested.

By default the fallback uses `~/.android/debug.keystore`. To use a release keystore, provide the same variables consumed by the APK builder:

```bash
export ANDROID_KEYSTORE=/path/to/release.keystore
export ANDROID_KEYSTORE_PASSWORD=...
export ANDROID_KEY_ALIAS=...
export ANDROID_KEY_PASSWORD=...
```

The script refuses to install if that keystore's certificate does not match the existing app.

## Required completion report

Report:

- rebuilt helper version, capabilities, and content hash
- scheduler running/autostart state restoration
- confirmation that existing durable session daemons were not stopped
- selected device serial (do not include IP addresses)
- installed version/versionCode
- certificate match result
- whether the published APK or locally re-signed build was used
- bundled Local Bun and SDK versions
- confirmation that app data was preserved
- cleanup result
