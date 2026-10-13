# Inline HTML pages: native emulator verification

The browser study is [design/android/inline-pages](../../../design/android/inline-pages/README.md). Its iframe checks are **not** native Android tests. The native app now uses an app-attached, clipped `PopupWindow`/`WebView` for the latest page in a conversation; the surrounding thread, source, confirmations and composer remain GPUI.

## Behaviour and limits

- A captured self-contained HTML/HTM artifact runs in its reply slot. Expand resizes the **same** document; Android Back returns to the chat. Source shows the pinned artifact source, not a new browser document.
- A later exact edit offers **Update**. **Keep current** keeps the running inputs; **Load update** deliberately replaces the document. Version numbers count local document adoptions, not backend file-history revisions.
- Unknown reconstruction removes the current preview. A pinned snapshot requires **View saved version instead** and is labelled historical. Renderer/provider failures have a native placeholder, Source, the existing separate viewer, and an explicit, state-resetting Retry; crashing documents are not automatically retried.
- Only the most recently changed page per conversation is inline. Older cards retain posters/the separate viewer. Native retention is bounded to **three WebViews**, **eight documents**, and **2 MiB** per artifact. Running state is not persisted: eviction, process death, or replacing the computer store (including reconnect) can reset it. Expansion, source, themes, ordinary route changes and foreground/background do not reset a healthy cached view.
- Geometry comes from **actual canvas painting**, not virtual-list measurement. The full WebView viewport is preserved while its outer window clips to the chat viewport. Sheets, departing sheets and notices hide the native surface; state remains cached. Boundary drags cancel the HTML press before forwarding the gesture to GPUI. Collapsed cards/bodies cannot flex-shrink in a long list row; otherwise a full-height native canvas can cover the GPUI footer. The suite taps Source on an actually partially clipped page to verify that boundary.
- Insets come from the full-screen parent window. Popup-local IME overlap must never become the parent's keyboard inset: that caused an emulator resize feedback loop, which was fixed before the passing run.

## Local capability boundary

The Java host supplies an intercepted document response **before** untrusted scripts, with trusted CSP headers: an opaque `sandbox allow-scripts` origin, `connect-src 'none'`, blocked frames/workers/forms/objects, and data/blob-only images/media plus inline styles/scripts. Each view also uses an unrelated random `UUID.invalid` URL. No DNS/HTTP load is needed for the document.

`setBlockNetworkLoads` and request interception alone are **not sufficient** for WebSockets or WebRTC. CSP blocks connection APIs; a first trusted script disables the WebRTC/ICE constructors, and the opaque-origin/child-frame policy prevents obtaining fresh constructors in a child realm. File/content access, DOM storage, native JavaScript bridges, permissions, file choosers, downloads, extra windows and native JavaScript dialogs are blocked. Trusted headers also disable DNS prefetching and browser sharing/fullscreen/sensors/wake-lock capabilities. This is a constrained WebView document, not an OS-level resource sandbox; arbitrary scripts can still consume browser CPU/memory. Maintain these checks against supported WebView versions.

This new envelope is scoped to the inline host. Legacy poster/separate-viewer paths have different policies and do not inherit it; do not treat these results as an app-wide network-sandbox proof. Open separately remains an explicit navigation choice.

## Reproduce on the emulator first

Requires an existing Android SDK, NDK, JDK and running x86_64 emulator. The scripts install no Python/browser dependencies.

```sh
export ANDROID_HOME="$HOME/Android/Sdk"
export ANDROID_NDK_ROOT="$ANDROID_HOME/ndk/magisk" # or an installed NDK
export PATH="$ANDROID_HOME/platform-tools:$PATH"
python3 crates/pi_android/scripts/build_apk.py --ui-test \
  --target x86_64-linux-android --out dist/pi-inline-ui-test-x86_64.apk
adb -s emulator-5554 install -r dist/pi-inline-ui-test-x86_64.apk
python3 crates/pi_android/scripts/test_inline_pages.py \
  --serial emulator-5554 --output artifacts/inline-pages-emulator/final
cargo test -p pi_android -p gpui_android --lib --features pi_android/ui-test
cargo test -p pi_markdown --lib
```

Use `--case calculator`, `scrolling`, `keyboard`, `boundary`, or `recovery` to repeat a group. The isolated package is `dev.pi.android.uitest`; fixture sessions are inert samples, not remote/provider work. `inline-calculator`, `inline-calculator-evening`, `inline-input`, `inline-scroll` and `inline-boundary` are additional preview fixtures.

GPUI actions use read-only layout telemetry. HTML actions are real ADB touches using Android accessibility or actual DOM rectangles, constrained to the visible native clip. Chromium 124 can expose only a replacement WebView's title until its first DOM change, so assertions use **read-only, harness-initiated** document readback instead of pressing AC to manufacture an expected reset value. Readback uses a DUMP-permission-protected broadcast only registered in the separate `.uitest` package; Rust also requires a paused inline sample fixture. It installs no JavascriptInterface and does not mutate inputs. `documents.jsonl` records the actual native document/revision/value/geometry. The renderer-termination hook is also guarded by fixture/sample mode.

The network group verifies its local HTTP endpoint with a positive control, then asserts that inline HTTP/image/WebSocket probes never reach it. It creates and removes only its own adb reverse mapping. It also checks file access, cookies, bridge/RTC constructor isolation, dialogs and app navigation. Recovery terminates the actual WebView renderer, checks that the native app PID and conversation survive, and calculates again after an explicit Retry.

## Recorded result

Passing run: **API 35 Android emulator, x86_64, 1080 × 2400, scale 2.625, AOSP WebView 124.0.6367.219, headless/lavapipe**. Compiled with Android platform 37.2/build-tools 37.0.0.

- **80 inline emulator checks passed**: arithmetic (`12 × 8 = 96`), themes, expand/Back, source/permissions and overlay hiding, explicit revisions/reset, saved snapshot retention, clipped chat drags without accidental keys, route/cache retention, background/resume, HTML input/IME sizing/Back, source while an HTML input is focused, first-tap composer handoff, native isolation probes, actual renderer termination and recovery.
- **392 Rust tests passed**: 339 `pi_android` (two existing ignored tests), 36 `gpui_android`, 17 `pi_markdown`.
- Existing **rich-content**, **input**, **gestures** and **models** cases passed **39 emulator checks** together on the final APK (`regressions-pass/`).
- A broader **long-input** composer check remains failing, including in isolation: a first tap selects the word `editable`, and typing `XYZ` replaces it instead of producing the test's expected three-character insertion. No inline page is mounted in that fixture. This run does not claim that the entire interaction suite passes; unrelated composer/subagent working-tree changes were preserved.
- Browser-study checks also pass independently (`build.cjs --check`, `render.cjs --check`).

Evidence: `artifacts/inline-pages-emulator/final/` (native screenshots, document readbacks and `report.json`), plus build/unit/regression logs in its parent directory. Screenshots were visually inspected for real composition, clipping, themes, sheets, keyboard layout and recovery.

Not validated here: a physical device, ARM deployment, Android 11, rotation/landscape/large font scaling, all arbitrary HTML form layouts, view-cache eviction under memory pressure, or a real SSH disconnect/reconnect while editing a widget. Native chat screen-reader support is still a platform limitation. The new APK is the isolated UI-test build, not a production installation.
