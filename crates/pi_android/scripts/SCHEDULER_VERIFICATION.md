# Native scheduler verification

`test_schedules.py` exercises **the native GPUI Android app**, not the HTML design
study. It reuses `test_interactions.Phone` and requires the separate debuggable
`dev.pi.android.uitest` APK with `ui-test` enabled. It never builds the host,
installs a scheduler/service, uses SSH, starts a provider, or clears app data.
Every injected input rechecks fresh fixture telemetry and sample mode. Device clock skew is measured read-only for fresh-telemetry checks (this emulator
is approximately nine seconds behind the host; its clock is not changed). Existing
preferences are compared byte-for-byte afterward; paired real connection
preferences cause an early refusal rather than being erased.

## Environment found (2026-10-11)

- `emulator-5554`: AVD `pi_api35`, API 35, x86_64, 1080×2400 at 420 dpi.
- Only one configured AVD and one running emulator. Do not claim two emulators
  from two display sizes. Repeat `--serial` if another device becomes available.
- SDK: `$HOME/Android/Sdk`; platform `android-37.2`, build tools `37.0.0`,
  NDK `magisk` (r29), Java 21. `adb` and `emulator` are not on PATH.
- Both Android Rust targets are installed. Build for **x86_64**, not arm64.
- The initial APK predated scheduler hooks. A fresh scheduler-enabled x86_64
  UI-test APK was subsequently built and installed; native verification passed
  on this emulator at both small and large geometries.

Read-only discovery evidence is in ignored
`artifacts/schedules-environment-20261011T134828Z/`.

## Exact commands

The main/build agent builds and installs **after Android implementation is ready**:

```sh
ANDROID_HOME="$HOME/Android/Sdk" python3 crates/pi_android/scripts/build_apk.py \
  --ui-test --target x86_64-linux-android --out dist/pi-ui-test-x86_64.apk
"$HOME/Android/Sdk/platform-tools/adb" -s emulator-5554 install -r dist/pi-ui-test-x86_64.apk
```

Run the script with a new output path (never overwrite old evidence):

```sh
python3 crates/pi_android/scripts/test_schedules.py \
  --serial emulator-5554 --apk dist/pi-ui-test-x86_64.apk \
  --geometry small --geometry large --output artifacts/schedules-native-RUN_ID
```

Small is 720×1280 at 320 dpi (360×640 dp); large is 1080×2400 at 420 dpi
(approximately 411×914 dp). Content viewport excludes system insets. Display
changes are opt-in, emulator-only, and restored in `finally` to the exact
previous overrides (or physical defaults). No AVD configuration is changed.
If interrupted unsafely, first inspect `wm size`, `wm density`, and
`display-before.json` before deciding whether to restore; do not blindly repeat.
Use `--geometry native` (default) for no display changes.
`--case consent-wrapping` checks the full consent label in Save, Run now and
Delete confirmations, in both themes. It asserts label bounds inside the padded
row, taps the last wrapped line, withdraws consent, and verifies that no sample
job or session was changed.

Plural emulator invocation, when a second emulator actually exists:

```sh
python3 crates/pi_android/scripts/test_schedules.py \
  --serial emulator-5554 --serial emulator-5556 \
  --output artifacts/schedules-two-emulators-RUN_ID
```

The script records invocation, APK hash (when supplied), device/package metadata,
every exact ADB command and exit status, screenshots with corresponding JSON
states, assertion reports, PID-filtered logcat, and preference hashes. It does
not clear logcat. Screenshots still require human visual inspection; automated
bounds checks cannot certify typography, contrast, or appearance.

Tooling-only unit checks (no Android/host Rust unit tests):

```sh
python3 -m unittest discover -s crates/pi_android/scripts -p 'test_schedules_tooling.py' -v
```

## Delivered Android hook contract

The script matches the native fixture/telemetry source and has passed on-device.
Initial evidence is in `artifacts/schedules-native-20261011T142711Z/`. The final
rebuilt APK was verified again in `artifacts/schedules-native-20261011T144203Z/`:
both small and large geometry runs passed, no native/Java panic was reported,
preferences were preserved byte-for-byte and the display returned to its
physical defaults. These are two configurations of **one emulator**, not two
emulators. Light list, dark detail, small enable-confirmation and offline
captures were visually inspected; final-run small confirmation and large dark
detail were inspected again. APK SHA-256:
`0cd70d46ae31d6d33404a6203f7c9963155bb00ae5c668adc3b6ee50d508a432`.
A production arm64 APK also built successfully, but was not device-tested.
Later reruns use new evidence directories rather than overwrite these runs.
All routes below must be compiled only into `ui-test` and use inert sample
scheduler data with no remote backend/provider calls:

- `pi://preview/schedules`: online, light, sample definitions + historical runs.
- `pi://preview/schedules-evening`: same data, dark.
- `pi://preview/schedule-offline`: cached/unknown host state, no mutations.
- `pi://preview/schedule-setup`: setup needed, no automatic host setup.
- `pi://preview/schedule-stopped`: service installed but stopped.
- `pi://preview/schedule-unsupported`: helper capability absent.

Existing `files/ui-test-state.json` keys remain: `fixture`, `time`, `sample`,
`route`, `sheet`, `bounds`, `viewport`, `scale`, and `sessions` (numeric IDs).
Add `scheduler` with:

```json
{
  "mode": "sample",
  "status": "online",
  "appearance": "light",
  "selected_id": "review",
  "definitions": [{"id": "review", "revision": 1, "enabled": true, "next_due": "..."}],
  "draft": null,
  "preview": [],
  "preview_error": null,
  "confirmation": null,
  "consent": false,
  "occurrences": [{"id": "run-1", "status": "completed", "session_id": 1}]
}
```

`status` alternatives: `offline`, `setup_required`, `unsupported`. Stopped and
not-installed both report `setup_required`; their native status cards differ;
`appearance`: `light`,
`dark`; `confirmation`: null/`save`/`run`/`delete`. Telemetry can use character
counts for drafts; no real prompt contents or credentials are required.
`accepted`/`running` must remain distinct from `completed`, and no automatic
sample completion should race the admission assertion. The Android agent's inert
sample Run now records `accepted` without starting a provider or new session;
pre-seeded historical occurrences provide the separate normal-session link test.
Definitions should expose
revision and next occurrence so run-now can prove both unchanged.

Selectors:

- Navigation (delivered): `computers` opens the Computers menu, `open-schedules`
  opens its calendar-icon Schedules entry, and `schedule-back` returns to Sessions.
  The old `home-sessions` / `home-schedules` button row is absent.
- List (delivered): `schedule-new`, `schedule-row-<index>`; script maps stable
  definition IDs to the currently rendered telemetry order.
- Native editable fields: `schedule-name`, `schedule-prompt`, `schedule-cron`,
  `schedule-timezone`; `schedule-model` opens the existing native model sheet,
  using its `model` index-zero control. Form supports numeric five-field cron.
- Preview/review: `schedule-preview`, `schedule-review`.
- Confirmations: `schedule-consent`, `schedule-consent-label`, `schedule-confirm`,
  `schedule-cancel`. The label wraps within the consent row instead of overflowing.
  The confirm button remains disabled/inert until unattended execution consent.
- Detail: `schedule-edit`, `schedule-toggle`, `schedule-run-now`,
  `schedule-history`, `schedule-delete`.
- History (delivered): `schedule-session-<index>` opens its linked ordinary
  `Thread(SessionId(N))`; indexes derive from the selected definition’s history; normal Back returns through history to detail.
- Small-screen content: one declared `scroll:...schedule...` viewport per screen
  (existing harness naming), with clipped/hidden targets not treated as tappable.

## Focused coverage

1. Sessions ↔ Schedules under one computer.
2. Native name/prompt editing; invalid cron rejection; read-only cron/timezone
   preview; review; disabled confirmation without consent; explicit enable.
3. Pause; edit + preview + consent; revision changes while staying paused; resume.
4. Run-now open/cancel does not admit; consent + confirm records one inert sample
   admission (no provider/new session); accepted is not completed, and next
   recurring time/pause state is unchanged.
5. History opens the correct normal session; Back returns to scheduler history
   and detail, not the Sessions root. The return-stack regression is covered by
   native touch/Back tests and emulator assertions.
6. Cancel deletion preserves definition; confirmed deletion preserves sessions.
7. Offline/setup states clearly differ from paused/live and cannot mutate/queue.
8. Light/dark list and detail screenshots; controls within small/large viewport.

This verifies UI/sample behavior, not production host scheduling, cron/DST engine
correctness, service lifecycle, provider execution, completion, or exactly-once
external effects. Those are owned by the host/Android agents, not duplicated here.
