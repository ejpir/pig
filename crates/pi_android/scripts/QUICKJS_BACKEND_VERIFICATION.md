# Embedded QuickJS backend — isolated APK verification

The standalone agent fixture remains an **offline proof**. The same opt-in APK also connects Claude/Codex subscription accounts to the existing session/thread/composer screens through the core session reducer and projection. It embeds the experiment's Rust library in the existing `libpi_android.so`; no executable is extracted/launched, and neither Bun nor Node runs on Android.

## Build

```sh
(cd experiments/quickjs-backend && npm ci --ignore-scripts && bun run test)
python3 crates/pi_android/scripts/build_apk.py --quickjs-backend-test \
  --target x86_64-linux-android --out dist/pi-quickjs-test.apk
# Physical arm64 devices:
python3 crates/pi_android/scripts/build_apk.py --quickjs-backend-test \
  --target aarch64-linux-android --out dist/pi-quickjs-test-arm64.apk
```

The flag rebuilds the pinned JS bundles, enables `quickjs-backend-test` (which includes `ui-test`), and selects **`dev.pi.android.quickjstest`**, labelled **Pi local backend tests**. This is a separate, debuggable package/UID from production `dev.pi.android` and the regular UI-test APK. Default builds do not link the backend experiment or expose these controls. The offline **Try the local QuickJS backend** entry and its test links additionally require the APK's Android `FLAG_DEBUGGABLE`; accidentally enabling the Cargo feature in a non-debuggable APK does not expose the fixture. This checks APK debuggability, not Rust optimization: the separately packaged test APK may still use optimized Rust. Installed npm dependency license texts are included in `assets/licenses/QUICKJS-BACKEND-NPM-NOTICES.txt`; this is not a complete release licensing audit.

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

Verified on the x86_64 Android emulator: in-app round-trip, duplicate guard, Stop/background/Back, resume without replay, restart, and visually inspected success screen. ARM64 APK builds. Host `pi_android` tests passed (350 passed, 2 ignored); the experiment's 15 host tests cover the backend and harness failure paths. The normal feature-off Android library also builds. No live provider was contacted.

## Claude/Codex account setup

On Connect, tap **Local providers · Claude + Codex**. Claude opens the system browser then accepts a returned authorization code in a native password-style dialog. Codex displays a device code with **Copy code** and polls upstream. Explicit copying marks the clipboard entry sensitive and shows a confirmation without repeating the code. Check / refresh uses Pi's OAuth refresh logic; Sign out removes local credentials, not remote authorization. API keys/other providers are not exposed.

The account screen has an independent auth owner: browser/background transitions preserve sign-in; Back cancels it. Refresh survives UI navigation to save rotated credentials. Ciphertext and per-provider Android Keystore keys are separate from preferences and agent workspaces. Opening this screen starts no network request. Full auth URLs/codes/credentials are excluded from telemetry. Screenshot protection is currently disabled at the user's request; codes and conversations can appear in screenshots/recents. Native code input remains masked.

```sh
python3 crates/pi_android/scripts/test_provider_accounts.py \
  --serial emulator-5554 --output artifacts/provider-auth-emulator
```

This script opens **only synthetic mode**, uses a separate `no_backup/local-providers/synthetic` namespace, and never contacts real auth/provider endpoints. It tests masked native code entry, device polling, explicit sign-in-code copying with synthetic-only clipboard read-back, background/return, process restart, encrypted persistence/refresh, tampered ciphertext/AAD, missing keys, failed-write preservation, logout/reset and cancellation. It never clears the production or real-account namespace. Synthetic native-dialog accessibility dumps are transient and deleted; no real-code dumping is supported.

See [provider accounts](../../../experiments/quickjs-backend/PROVIDER_AUTH_PLAN.md) for security boundaries and limitations. Real-account interoperability, physical ARM64 execution and hardware-specific Keystore behavior are not validated. Not implemented locally: durable sessions, workspace/tools/shell, generated Visualize execution, queues, compaction, or a background service.

### Copy control and debug-only fixture verification

The updated emulator APK passed both auth and offline-agent suites. A deliberately
non-debuggable APK with the backend feature still linked was also installed on the
emulator: the offline button was absent (visually inspected), and all four offline
fixture links left the fixture report/UI files unchanged. The normal debug test APK
was restored afterward; no app data was cleared. Evidence:
`artifacts/quickjs-release-guard/{report.json,non-debug-connect.png}`.
The first negative check sampled UI telemetry while the prior debug process was
still running and failed the byte comparison; after inspecting its unchanged PID,
the corrected check force-stopped that process before taking its baseline and passed.

The ARM64 test APK including Copy code and the debug guard was successfully updated
on phone **25019PNF3C**; production `dev.pi.android` was untouched. This is installation
evidence, not a live-account or physical-phone execution test.

## Shared local UI validation

Use the top-right **Pi** logo to select **Local** or a saved remote computer. Sign in or choose **Chat on this phone**; the ordinary composer supports model selection, supported thinking levels, text files/images and Stop. Reselecting Local or reopening the same account must not erase conversation/drafts. Switching accounts/backends or signing out clears local chat state.

```sh
python3 crates/pi_android/scripts/test_local_chat.py \
  --serial emulator-5554 --output artifacts/local-chat-emulator
```

The emulator suite uses synthetic accounts only: shared screens, Pi selector, thinking selection, multi-turn canonical history, model changes, cancellation, background/return, account isolation and no automatic inference. Host tests additionally verify actual image/text payloads and thinking events for both upstream providers, four 1-MiB image bodies, full-block/signature limits and failed-turn exclusion. The native importer has separate image/file tests. Synthetic results do **not** establish live provider acceptance.

Always update a phone with `adb install --no-incremental -r`; never uninstall or clear its account data. Screenshots are intentionally enabled, including account/code screens; native code entry remains masked. Do not capture/share real credentials in test evidence.

Latest verification: 23 experiment host tests and Clippy passed; 354 Android host tests passed (2 ignored), and the default feature-disabled Android library checks cleanly. All three emulator suites (accounts, shared local chat, offline agent) passed. The ARM64 APK was rebuilt and installed on the 25019PNF3C with `install --no-incremental -r`, without uninstalling or clearing app data. No live-account request or physical-phone execution test was performed by the assistant.
