# gpui_android

A GPUI platform for Android. The demo app has run on a phone: drawing, touch,
the on-screen keyboard, the clipboard with images, file pickers and links
work. Back and fixture links into the app are also tested on a device;
notification delivery still needs device coverage. The plain-Rust parts are
unit-tested on any host.
[Pi for Android](../pi_android/README.md) is built on it.

## What it does

- **App:** a `NativeActivity` through [`android-activity`], extended by a
  small Java class, `GpuiActivity` (`java/`), for what Android offers only to
  Java. The `android_main` thread is GPUI's main thread; it waits in Android's
  looper, and work queued from other threads wakes it.
- **Window:** Vulkan (with a GL fallback) through the same wgpu renderer as
  Linux. When Android destroys the window (backgrounding) only the surface is
  dropped; the renderer and its glyph atlas survive. Rotation and theme
  changes resize the window instead of restarting the activity.
- **Frames:** on demand, at vsync, from Android's Choreographer. The window
  asks Android for the display's fastest refresh rate (90 or 120 Hz where
  available) while it draws; an idle window draws nothing.
- **Lifecycle:** `onStart`/`onResume`/`onPause`/`onStop` become GPUI's
  `AppLifecyclePhase`; focus becomes the window's active state.
- **Touch:** every finger and every batched sample, fed to GPUI's own
  recognizers (taps, long presses, pans, flings) with Android's thresholds and
  fling curve.
- **Keyboard:** tapping a focused text input shows the on-screen keyboard;
  focus leaving text hides it. The keyboard edits a mirror of the text around
  the selection, and each batch of edits becomes calls on GPUI's input handler,
  so autocorrect, suggestions, swipe typing and composed input (underlined
  while composing) work. The app's own changes go back to the keyboard. The
  input's `TextInputConfiguration` sets autocorrect, capitalization,
  suggestions and the action key; the return and action keys arrive as enter.
  Hardware keyboards send keys, and unhandled character keys type. A change a
  key caused (such as the keyboard's delete) does not restart the keyboard, so
  holding delete repeats and speeds up as in Android's own fields.
  `activity::long_press_feedback` gives the buzz of a long press that selects.
  Keyboard activation is resolved after a rendered frame so a newly focused
  GPUI handler exists. The shared Android editor retires its InputConnection
  on blur and restarts it for a new logical field; stale composition cannot
  write into another field.
- **Screen:** the app draws edge to edge. The system bars, display cutouts and
  the keyboard are reported as insets (`window.fully_visible_bounds()`),
  following the keyboard as it slides. Density sets the scale factor, dark
  mode sets the appearance and the status bar icons; an app whose theme
  differs from the system's sets the icons with `activity::set_bar_icons`.
  Android's `adjustNothing` leaves IME layout to GPUI's animated insets instead
  of panning the native surface independently of its input coordinates.
- **Back:** the back button and gesture arrive as the `back` key, so an app
  binds it to an action (close a sheet, go up a screen). When nothing handles
  it, Android does, and the app goes to the background.
- **Clipboard:** text and images (PNG, JPEG, WebP, GIF and the other formats
  GPUI knows) go through the system clipboard. Copied images are served to
  the apps that paste them by a small content provider.
- **Files:** `cx.prompt_for_paths` shows Android's file picker. Picked files
  are copied into the app's cache under their own names, so apps read them
  from ordinary paths; folders cannot be picked. `cx.prompt_for_new_path`
  picks a document to save to and returns a path in the cache; each time the
  app finishes writing it, the file is copied to the document.
- **Links:** `cx.open_url` opens links in the browser. Links that open the
  app, such as a notification's, arrive at `Application::on_open_urls`, also
  while it runs (the activity is single-task).
- **Notifications:** `activity::notify` posts one on a channel the app names,
  with an importance, a link to open and up to three action buttons that are
  links too; `ongoing` makes it stay until cancelled. The app's icon is a
  drawn π for now. `activity::request_notification_permission` asks on
  Android 13 and later, once; `activity::notifications_enabled` says whether
  they show.
- **Fonts:** fonts in `/system/fonts` and `/product/fonts` are memory-mapped
  for language fallback. Roboto is the default UI font. A compatible Noto
  Color Emoji bitmap font is bundled because newer Android COLRv1 fonts
  render blank with the current Swash renderer. The unmodified font comes
  from [googlefonts/noto-emoji](https://github.com/googlefonts/noto-emoji/tree/e20cbc2bbec1926686be9f9bee7d1d2cfa1fea0e/2D/fonts)
  (SHA-256 `15671215ab769fdc7162a045d56fd7d7e477c51b04e6b3c761d914d8fdd6cc44`);
  its OFL license is in `assets/fonts/OFL.txt` and included in each APK.

Not yet: screen readers, stored credentials, and mouse or stylus as anything
but touch.

## Try it

`examples/touch/` is a small app: a message field with Send, Paste, Copy,
Copy image, Attach…, Save… and Open a link, a tap counter, and a long list to
fling.

`scripts/build_apk.py` builds it without Gradle. It needs a JDK and the
Android SDK with a platform (API 30 or later), build-tools and the NDK, all of
which Android Studio installs:

```sh
rustup target add aarch64-linux-android
export ANDROID_HOME=~/Library/Android/sdk     # macOS default
python3 crates/gpui_android/scripts/build_apk.py
adb install -r dist/gpui-touch.apk
```

Logs and panics go to logcat:

```sh
adb logcat -s gpui-touch GpuiActivity
```

The APK is signed with the Android debug key, targets Android 15 (API 35) and
runs on Android 11 and later on arm64. Other apps build their APK through the
same script with their own package, library and permissions; see
`crates/pi_android/scripts/build_apk.py`.

[`android-activity`]: https://github.com/rust-mobile/android-activity
