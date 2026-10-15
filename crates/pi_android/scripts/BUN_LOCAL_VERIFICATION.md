# Experimental Bun Local verification

Release APKs include `local-backend`, the source-linked ARM64 Bun library and the
unchanged SDK bundle. Release packaging rejects an SSH-only/test APK, a missing
or wrong-architecture runtime, missing notices, and a declared SDK bundle absent
from the native application. No second installation or Bun CLI is needed.

The **separate synthetic test build** retains `dev.pi.android.quickjstest`, its
existing signing key and app data; it enables `bun-backend-test`/`ui-test` and is
debuggable. Do not substitute it for the normal `dev.pi.android` release APK.

```sh
python3 crates/pi_android/scripts/build_apk.py --bun-backend-test \
  --bun-library /path/to/arm64/libbunembeddingpoc.so \
  --out dist/pi-bun-local-test-arm64.apk
# Only when explicitly authorized; never uninstall or clear app data:
adb install --no-incremental -r dist/pi-bun-local-test-arm64.apk
```

For the emulator, use the x86_64 library and `--target x86_64-linux-android`.
`scripts/test_bun_local_navigation.py` checks that a stock shell tool finishes
while Local is parked behind another backend and that explicit Stop still works.
Native-account and transport fixtures use synthetic accounts/loopback only.

Retained pre-cleanup evidence (ignored local artifacts):

- `artifacts/bun-durable-emulator`: stock write/read/edit/sh, real SQLite,
  fresh-process reopen and persistent request deduplication without new model calls.
- `artifacts/bun-local-emulator`: integrated shared UI, synthetic reply/tool and reopen.
- `artifacts/bun-local-navigation-emulator`: parked tool completion and explicit Stop.
- `artifacts/bun-local-phone`, `artifacts/bun-local-navigation-phone`: authorized
  upgrades of the existing ARM64 test installation; user confirmed persistence.

The latest working phone APK is preserved as
`dist/pi-bun-local-navigation-fix-arm64.apk`, SHA-256
`05df39ccd41082feefdfac1ded9b18e01f91c7242947f9a8352a45cfe984ed0f`.
Release preparation must not silently replace that installation. Historical
observations are not claims that every later APK/device/lifecycle case passed.

Local is experimental, in-process and not sandboxed. Android sh is not GNU Bash
or a bundled Linux userland. No unattended real-account inference, credential
inspection, isolation/security certification, arbitrary mid-tool recovery or
full Android background-service guarantee is part of these checks.
