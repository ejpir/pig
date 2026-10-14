# Embedded QuickJS backend — isolated APK verification

This is an **offline proof**, not a local-session transport or a provider login screen. It embeds the experiment's Rust library in the existing `libpi_android.so`; no executable is extracted/launched, and neither Bun nor Node runs on Android.

## Build

```sh
(cd experiments/quickjs-backend && npm ci --ignore-scripts && bun run test)
python3 crates/pi_android/scripts/build_apk.py --quickjs-backend-test \
  --target x86_64-linux-android --out dist/pi-quickjs-test.apk
# Physical arm64 devices:
python3 crates/pi_android/scripts/build_apk.py --quickjs-backend-test \
  --target aarch64-linux-android --out dist/pi-quickjs-test-arm64.apk
```

The flag rebuilds the pinned JS bundles, enables `quickjs-backend-test` (which includes `ui-test`), and selects **`dev.pi.android.quickjstest`**, labelled **Pi local backend tests**. This is a separate, debuggable package/UID from production `dev.pi.android` and the regular UI-test APK. Default builds do not link the backend experiment or expose these controls. Installed npm dependency license texts are included in `assets/licenses/QUICKJS-BACKEND-NPM-NOTICES.txt`; this is not a complete release licensing audit.

## Use and verify

```sh
adb -s emulator-5554 install -r dist/pi-quickjs-test.apk
python3 crates/pi_android/scripts/test_quickjs_backend.py \
  --serial emulator-5554 --output artifacts/quickjs-apk-emulator
```

Or open the app, tap **Try the local QuickJS backend** on Connect, then **Run offline agent**. **Test cancellation** starts a deliberately stalled fixture; Stop/Back/Home must terminate it.

The explicit test URL `pi://test/quickjs-backend` only opens the screen. Fixed `/run`, `/stall`, `/cancel` suffixes operate synthetic fixtures. There are no URL parameters for credentials, endpoints, prompts or workspaces. Do not add a public auth callback to this handler.

The device test:

- uses only the dedicated package (never clears app data or touches production sessions);
- taps real GPUI controls using fresh, visible, process/frame-matched geometry;
- asserts the native backend's UID equals the installed app UID, and its PID is the app process;
- verifies three upstream tool results, one agent completion, Unicode reply and file cleanup;
- rejects duplicate admission without replacing a running fixture;
- checks Stop, background, Back and restart, including actual HTTP socket closure;
- checks that background/resume does not replay a run, and Back hides the panel;
- saves a screenshot, synthetic result records and app logs. Screenshots still need visual inspection.

## Boundaries and lifecycle

Backend execution, HTTP, result reports and joins run on workers. The UI starts/cancels/polls bounded synthetic snapshots and writes small, test-only geometry telemetry. Leaving the page, backgrounding, or dropping its owner requests cancellation; admission stays occupied through cleanup. A failed report write still cancels/joins the VM, and harness panics produce terminal failure state rather than leaving a permanently busy UI.

Fixture workspaces are temporary children of `files/quickjs-backend-test/`, **not** the app files directory that contains SSH keys/preferences. Only synthetic `report.json` and geometry `ui.json` remain. Report replacement is atomic. Reports contain no provider credentials or real prompts. Loopback is not UID-private on Android: the fixture HTTP parser has request-line/total-header/body limits, an absolute request deadline, and cancellation-aware reads.

The backend and model-generated visuals use separate QuickJS contexts/runtimes, with one unified native `rquickjs` version (`=0.14.0`). The experiment's independent Cargo workspace remains usable; the APK resolves dependencies through the root lockfile.

## Evidence and remaining work

Verified on the x86_64 Android emulator: in-app round-trip, duplicate guard, Stop/background/Back, resume without replay, restart, and visually inspected success screen. ARM64 APK builds. Host `pi_android` tests passed (349 passed, 2 ignored); the experiment's 11 host tests cover the backend and harness failure paths. The normal feature-off Android library also builds. No live provider was contacted.

Not implemented: provider login, encrypted credential storage, live local-session UI, durable sessions, shell/development environment, or background execution. The [provider-auth plan](../../../experiments/quickjs-backend/PROVIDER_AUTH_PLAN.md) describes the next increment; it is a plan, not an available sign-in feature.
