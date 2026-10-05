# Pi for Android

The [Android screens](../../design/android/README.md) as an app, in GPUI on
[gpui_android](../gpui_android/README.md): follow the sessions on a
computer, answer Pi's questions, review its changes and start new work.

The phone connects to a computer over SSH with its own key and uses Pi
Desktop's helper there (`pi-desktop-remote`). Sessions it starts are
**durable**: their state lives in a database on the computer, so they carry on
while the phone is away and pick up where they were. “Look around with sample
sessions” on the first screen opens sample sessions that run on their own
instead, with no computer.

## Set up the computer

1. Turn on SSH: on a Mac, **System Settings → General → Sharing → Remote
   Login**.
2. Install a helper built with durable sessions, from the repository root on
   that computer (see [backend/durable](../../backend/durable/README.md)):

   ```sh
   cd backend/durable && npm ci --ignore-scripts \
     && npm exec --yes --package=bun@1.4.2 -- bun run build && cd ../..
   PI_DESKTOP_DURABLE_BINARY="$PWD/artifacts/durable/pi-desktop-durable" \
     cargo build --release -p pi_remote --features bundled-durable
   mkdir -p ~/.pi/desktop/bin/phone
   cp target/release/pi-desktop-remote ~/.pi/desktop/bin/phone/
   ```

   The phone uses the newest helper under `~/.pi/desktop/bin/` that runs
   durable sessions, lets several apps watch one, and lists sessions. A helper
   Pi Desktop installed over SSH counts too.
3. Give the durable runner a model provider, as backend/durable's README
   describes. The phone sets the model from its settings, or the first the
   computer offers.

## Connect

Enter `you@computer` (an IP address or a Tailscale name works where `.local`
names don't), and tap Connect. The first time, the computer turns the phone's
key down: copy it from the second step into `~/.ssh/authorized_keys` there, and
connect again. The computer's host key is kept then; a different one later is
refused until the computer is forgotten in Settings.

## What works

- **Sessions on the computer:** every session Pi Desktop or the phone started
  over SSH, from the helper's list. Running ones are watched at once, so the
  list stays current and notifications fire; others attach when opened. Pi
  Desktop and the phone can watch one session together.
- **New sessions:** in a folder the computer has sessions in, or one typed in
  the project sheet. The prompt shows at once, then Pi's turns: the stages its
  tools went through, the files it changed with Pi's own diffs, the last check
  it ran, and its answer.
- **Follow-ups, Stop and the queue:** a follow-up during a run waits for it;
  Stop aborts the run on the computer.
- **Questions:** a stock Pi session's confirm or select dialog opens as a
  sheet and is answered from the phone. Durable sessions have no extensions,
  so they don't ask.
- **Review:** one file at a time; tapping lines names them (“About lines
  211–212 of src/app.rs”) in a follow-up.
- **Reconnecting:** when the connection drops, the phone connects again and
  attaches to what it was watching.
- **Back, notifications, links, selecting text, settings:** as in the sample.

Not yet: images and files from the phone (only the text is sent), Pi's
commands and resources in durable sessions, staying connected in the
background, and several computers at once. Each update carries the whole
session, which is fine on Wi-Fi and heavy for long sessions on mobile data.

## Build and install

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
`connect`, `sessions`, `search`, `start`, `working`, `waiting`, `done`,
`review`, `typing`, `details`, `evening`, `settings`, and the sheets `model`,
`attach`, `more`, `project`, `models` and `resources`. The tests render each
one.

`live::tests::a_durable_session_runs_over_ssh` runs the whole path against a
real SSH server: set `PI_ANDROID_TEST_SSH` (`user@host:port`),
`PI_ANDROID_TEST_KEYS` (that account's authorized_keys, which the test adds
its key to) and `PI_ANDROID_TEST_PROJECT`, then run it with `--ignored`. With
backend/durable's faux test runner as `PI_DESKTOP_DURABLE_RUNNER` there, no
provider is called.

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
