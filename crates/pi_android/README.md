# Pi for Android

The [Android screens](../../design/android/next4/README.md) as an app, in GPUI on
[gpui_android](../gpui_android/README.md): follow the sessions on a
computer, answer Pi's questions, review its changes and start new work.

The phone connects to a computer over SSH with its own key and uses Pi
Desktop's helper there (`pi-desktop-remote`). Sessions it starts are
**durable**: their state lives in a database on the computer, so they carry on
while the phone is away and pick up where they were. Sample sessions that run
on their own, with no computer, are under “Set up with an SSH key instead” on
the first screen, and in the computer switcher (tap the computer's name on
Home).

## Set up the computer

1. Turn on SSH: on a Mac, **System Settings → General → Sharing → Remote
   Login**.
2. Install a helper built with durable sessions. The macOS and Linux helpers
   in the releases (`pi-desktop-remote-*`) have it built in; put one in
   `~/.pi/desktop/bin/`. Or build it from the repository root on that computer
   (see [backend/durable](../../backend/durable/README.md)):

   ```sh
   cd backend/durable && npm ci --ignore-scripts \
     && npm exec --yes --package=bun@1.4.2 -- bun run build && cd ../..
   PI_DESKTOP_DURABLE_BINARY="$PWD/artifacts/durable/pi-desktop-durable" \
     cargo build --release -p pi_remote --features bundled-durable
   mkdir -p ~/.pi/desktop/bin/phone
   cp target/release/pi-desktop-remote ~/.pi/desktop/bin/phone/
   ln -sfn ~/.pi/desktop/bin/phone/pi-desktop-remote ~/.pi/desktop/bin/pi-desktop-remote
   ```

   The phone uses the newest helper under `~/.pi/desktop/bin/` that runs
   durable sessions, lets several apps watch one, and lists sessions. A helper
   Pi Desktop installed over SSH counts too.
3. Give the durable runner a model provider, as backend/durable's README
   describes. The phone sets the model from its settings, or the first the
   computer offers.

## Connect

The easiest first connection is QR pairing. On the computer, run:

```sh
~/.pi/desktop/bin/pi-desktop-remote pair
```

Then tap **Scan computer QR** in the Android app, scan the terminal code, and
compare the six-digit confirmation code shown on both devices. Press Enter on
the computer to allow it. The QR expires after two minutes and its one-use key
is replaced with the phone's permanent Ed25519 key only after confirmation.
The installed key is restricted to Pi helper commands, and pairing uses the
computer's existing SSH server—there is no extra listening port or pairing
daemon. The QR also pins the SSH host fingerprint, so the first connection is
not trust-on-first-use.

Manual setup is behind **Set up with an SSH key instead**. Enter `you@computer` (an IP
address or a Tailscale name works where `.local` names don't), and tap Connect.
The first time, the computer turns the phone's key down: copy it from the second
step into `~/.ssh/authorized_keys` there, and connect again. The computer's host
key is kept then; a different one later is refused until the computer is
forgotten in Settings.

Connecting is step one. Step two offers recent projects and a read-only folder
browser on that computer: Home, parent folders, hidden folders, and an optional
path field. Choose **Use this folder** to open the new-session composer. No
session starts while browsing. **Just view sessions** skips project selection.
The last chosen project is remembered separately for each computer. Folder
browsing requires the updated helper's `directories` command; an older helper
shows an update message and leaves recent projects available.

## What works

- **Sessions on the computer:** every session Pi Desktop or the phone started
  over SSH, from the helper's list. Running ones are watched at once, so the
  list stays current and notifications fire; others attach when opened. Pi
  Desktop and the phone can watch one session together.
- **New sessions:** choose a recent project or browse folders remotely, with
  optional manual path entry. The prompt shows at once, then Pi's turns: the stages its
  tools went through, the files it changed with Pi's own diffs, the last check
  it ran, and its answer.
- **Follow-ups, Stop and the queue:** a follow-up during a run waits for it;
  Stop aborts the run on the computer. New tasks, follow-ups and revisions use
  the same multiline composer, with separate model and thinking controls.
  Models are searchable by name, provider and ID; the search stays above the
  scrolling results. The computer's read-only model catalog loads immediately
  after connecting, before any session exists, and can be reloaded after
  configuring a provider. This requires the helper's `models` command.
  Stop lives in the composer toolbar. In an existing
  session, those choices change that session, not the default for new tasks.
- **Questions:** a stock Pi session's confirm or select dialog opens as a
  sheet and is answered from the phone. Durable sessions have no extensions,
  so they don't ask.
- **Review:** one file at a time; tapping lines names them (“About lines
  211–212 of src/app.rs”) in a follow-up.
- **Reconnecting:** when the connection drops, the phone connects again and
  attaches to what it was watching. Unacknowledged prompts retry with their
  original admission ID and complete image payload. Obsolete connection
  updates are ignored. Rejected prompts stay available as **Edit and retry**;
  recovery never replaces a draft already being edited.
  Opening the app connects to the last computer and says so; if it can't
  reach it, the app shows why and tries again after 5, 10, 20, then every 30
  seconds. **Use a different computer** goes to setup.
- **Back, notifications, links, selecting text, settings:** as in the sample.
- **Computers, sheets and the dock:** Home's title is the computer; tap it to
  switch computers, pair another, try the sample sessions or reach Settings.
  What moves work forward sits at the bottom: the start bar on Home, Working
  and Stop on the composer, Pi's question in the composer's place, Review over
  the follow-up. Swipe down from a sheet's handle (or its content when
  scrolled to the top) to dismiss it. Back and close buttons remain available.
- **Copying:** long-press a prompt, a reply, a question's command or a line in
  Review to open it as selectable text: hold a word and drag the handles to
  copy part of it, or copy all of it.
  Panels follow the finger until release, then settle smoothly; partial and
  cancelled drags return to their starting position. Reduced motion is honored.
  Sheet headers and close buttons remain visible while long content scrolls.
- **Delete sessions:** swipe a row right, or use the session's More menu, then
  confirm permanent deletion. The computer deletes only that durable session's
  history; project edits are kept. Working sessions must be stopped first.
  This requires an updated remote helper/daemon. Older helpers report the
  requirement without deleting anything; legacy stock-Pi history is managed
  on the computer. A server-side tombstone prevents stale clients recreating
  a deleted session; daemon and writer locks protect the deletion.
- **Readable conversations:** numbered user turns, full Markdown replies,
  copyable code blocks, and an activity rail that expands after completion.
  Tap a stage for commands, full tool output and file details. Live commentary
  and tool previews stay visible while the run works; scrolling up pauses
  following, and **Latest reply** returns to the bottom.
- **Pages Pi makes:** when Pi writes an `.html` file, a card under the turn
  opens it full screen, with a Preview tab (a web view) and a Code tab. The
  phone already has the page from Pi's `write`, and applies later exact edits
  itself; an edit it can't follow says so instead of showing an old page. Ask
  for one self-contained file: other files next to it aren't loaded. The page
  runs its scripts and may use the network, but has no access to the app, its
  files or a native bridge; links it follows on a tap open in the browser. The
  desktop preview opens pages in the browser.
- **Long drafts:** bounded, scrollable composers; horizontal scrolling for
  one-line inputs; Unicode-aware cursor/deletion. Return adds a line by
  default. With Return sends enabled, Shift+Return still adds a line.
- **Attachments:** the system document picker imports images and UTF-8 text.
  Images have a thumbnail, separate remove target, and full preview. Image-only
  messages work. Import/normalization runs off the UI thread; Send waits until
  it finishes. Up to four images (eight attachments total); each source image
  is limited to 20 MB, resized to at most 2048 pixels and 1 MiB for upload.
  Text files are limited to 256 KiB; their contents, not just filenames, are
  included. A vision-capable model and image-enabled helper/daemon are required.
  Running old daemons are not restarted automatically.
- **File mentions:** typed, pasted and suggested `@path` references have a
  bordered highlight. Paths with spaces use `@"my folder/file.rs"`. The text
  remains directly editable. Mentions reference computer-side files; the
  paperclip uploads phone-side bytes.
- **Large lists and replies:** proportional scrollbars expose overflow and
  allow direct dragging, independently of sheet dismissal.

Not yet: Pi's commands/resources in durable sessions, background connection
service, multiple computers at once, reopening past-message images on the phone,
and persistent phone-side unsent drafts across process death. Computer-side admitted history is durable. Session
updates still carry the full text projection; image bytes are replaced with
small content-hash references and can be retrieved with `get_image`.

## Build and install

Each release has a signed `pi-android-arm64.apk` (Android 11 or newer, arm64): download it on the phone and open it. To build it yourself:

```sh
rustup target add aarch64-linux-android
export ANDROID_HOME=~/Library/Android/sdk     # macOS default
python3 crates/pi_android/scripts/build_apk.py
adb install -r dist/pi.apk
adb logcat -s pi GpuiActivity
```

The script is gpui_android's, which lists what it needs.

## On the desktop

The views compile on any host, so a phone-sized window shows them without a
phone, and it connects like the phone does:

```sh
cargo run -p pi_android --example preview             # as on the phone
cargo run -p pi_android --example preview -- waiting  # a named state
```

The named states use the sample sessions and match the design's screens:
`connect`, `reconnecting`, `unreachable`, `sessions`, `search`, `start`, `working`, `waiting`, `done`,
`review`, `typing`, `details`, `evening`, `settings`, and the sheets `model`,
`attach`, `more`, `project`, `models` and `resources`. Stress states include
`long-input`, `long-reply`, `streaming-reply`, `long-labels`, `empty-search`,
`failed`, `stopped`, `computers`, `many-files`, `activity`, `markdown`, `multi-turn` and
`tool-output`, `follow-up-input`, `delete`, `delete-running`, `projects`,
`project-empty`, `project-error`, `project-loading`, `project-long-path`, `project-tree`, `project-search`, `project-file`, `tool-image`,
`model-long-list`, `model-no-match`, `thinking`, `mentions`, `image-input`
and `image-only`. The tests
render every state at 320×640, 384×854 and 640×360.

## Device UI regression checks

Use the isolated test package so fixture resets never affect a real app's
settings, keys or sessions:

```sh
cargo test -p pi_android -p gpui_android --lib --features pi_android/ui-test
python3 crates/pi_android/scripts/build_apk.py --ui-test
adb install -r dist/pi-ui-test.apk
python3 crates/pi_android/scripts/test_device.py --serial DEVICE --output /tmp/pi-phone-screens
python3 crates/pi_android/scripts/test_interactions.py --serial DEVICE --output /tmp/pi-phone-interactions
adb shell am start -n dev.pi.android.uitest/dev.pi.gpui.GpuiActivity \
  -a android.intent.action.VIEW -d pi://preview/long-input
```

The **Pi UI tests** app is `dev.pi.android.uitest`; it can also connect normally.
Fixture links are compiled only with `ui-test`. This separate APK is debuggable
so the test runner can read app-private fixture telemetry via `run-as`; normal
release APKs are not. The telemetry contains routes, bounds and character/image
counts, never credentials or prompt contents, and is disabled for real sessions.
Screenshots require visual
inspection: the script checks process survival and crash logs, not appearance.
`test_interactions.py` waits for settled layouts and asserts the QR connect screen
and native camera scanner, typing, model search, thinking selection, long Unicode
edits, Stop, actual system-picker image import, image preview/send, scrollbars,
sheet drags, confirmed swipe deletion, completed activity details, long
tool/reply scrolling and project selection.
It saves screenshots and a JSON report. Use `--case input`, `--case picker`,
`--case models`, etc. to repeat a case. The picker case supports English AOSP
DocumentsUI and Xiaomi's picker; it creates a uniquely named synthetic PNG and
removes only that generated device file afterward. The local evidence is kept.
The `--case ime` test taps real Gboard keys to exercise composition, deletion
and switching fields. It expects English portrait Gboard with four rows and
no extra number row or toolbar; other layouts can run the remaining cases
without changing the phone's keyboard settings.
Fixture sessions are paused intentionally. Restart the app
or use the computer switcher's sample sessions to try animated sample runs.
`pi://test/grow` (UI-test build only) streams a growing write and reply into
the paused Qwen sample, to check that growing content does not move the
thread.

Check short/empty/multiline drafts, switching from a populated field to an
empty field, long-paste editing and scrolling, send with the keyboard open,
back from New session, sheet scrims, completed activity expansion,
tool-output scrolling, and reading earlier text while updates arrive.

`test_ssh.py` runs both the durable-session regression and the complete QR
enrollment through a temporary loopback-only OpenSSH server, a separate
authorized-keys file and the faux-only durable runner. It verifies model/folder
discovery before a session exists, prompts, image bytes reaching the provider,
follow-ups, a second watcher, host-key rejection and deletion that keeps project
files. It then creates a one-use QR offer, pins the host key, exchanges the
bootstrap key after confirmation, reconnects with the phone key, and proves the
permanent forced-command key cannot run a shell. It never changes the account's
SSH service, real authorized keys or provider configuration; detached test
daemons and temporary keys are cleaned up. The Rust tests refuse keys/projects
outside this harness's temporary folder.

```sh
python3 crates/pi_android/scripts/test_ssh.py \
  --helper target/release/pi-desktop-remote \
  --runner artifacts/durable/pi-desktop-durable-fixture \
  --output /tmp/pi-ssh-tests
```

Build the helper and the standalone `backend/durable/test/fixture.ts` runner
first. The helper can be the production build: the test endpoint explicitly
sets the isolated state directory and faux runner; no real provider is called.

The backend's ignored `images_survive_queue_cancellation_completion_and_reconnection`
test exercises a real SQLite durable session with a faux provider: actual image
bytes, exact queued-message cancellation, image references/retrieval, reconnect,
duplicate admission and changed-payload collision rejection. Set
`PI_DESKTOP_TEST_DURABLE_RUNNER` to the compiled `backend/durable/test/fixture.ts`
runner and run `cargo test -p pi_remote --test durable images_survive_queue -- --ignored`.

## Layout

| File | What |
| --- | --- |
| `app.rs` | The app: screens as a stack, one sheet, back, connecting, notifications, links |
| `screens/` | One file per screen, and the sheets |
| `ssh.rs` | SSH: the phone's key, the computer's host key, commands over one connection |
| `remote.rs` | The helper on the computer: finding it, listing sessions, attaching |
| `live.rs` | The connected computer: watched sessions, prompts, models, reconnecting |
| `projection.rs` | Pi's session state as turns, stages, changed files and checks |
| `store.rs` | What the screens read: a computer's sessions, live or sample |
| `composer.rs`, `text_area.rs` | The composer and the text field under it, with Android's keyboard |
| `model.rs`, `demo.rs` | Sessions as the phone shows them, and the sample ones |
| `alerts.rs` | Notifications and the `pi://` links they open |
| `ui.rs`, `theme.rs`, `assets.rs` | Building blocks, Moonstone and Evening, icons and fonts |
