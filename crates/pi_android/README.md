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
   in the releases (`pi-desktop-remote-*`) have it built in; make the downloaded
   file executable and run its `activate` command. Or build and activate it from
   the repository root on that computer (see
   [backend/durable](../../backend/durable/README.md)):

   ```sh
   cd backend/durable && npm ci --ignore-scripts \
     && npm exec --yes --package=bun@1.4.2 -- bun run build && cd ../..
   PI_DESKTOP_DURABLE_BINARY="$PWD/artifacts/durable/pi-desktop-durable" \
     cargo build --release -p pi_remote --features bundled-durable
   target/release/pi-desktop-remote activate
   ```

   Activation verifies and stores the executable under
   `~/.pi/desktop/bin/<sha256>/`, then atomically selects it through the stable
   `~/.pi/desktop/bin/pi-desktop-remote` symlink. A helper Pi Desktop installed
   over SSH is activated the same way and counts too.
3. Give the durable runner a model provider, as backend/durable's README
   describes. The phone sets the model from its settings, or the first the
   computer offers.

## Native mini-apps with `@Visualize`

Type **`@Visualize build me a calculator`**, or choose **@Visualize · Native mini-app** in the `@` picker, then Send. The mention requests one plain JavaScript program in a closed `javascript pi-visual` fence, not an escaped JSON/Rhai bundle. Picking the mention only edits the draft.

QuickJS executes a small `app`/`ui`/drawing API; GPUI paints native rows, columns, text, 48dp buttons/sliders and interactive canvases. No HTML/WebView or per-app Rust compilation. Try **`@Visualize create an interactive spirograph`**. Computed text is separate from fixed flat persisted state; changing source creates a new local state identity. Copy/Source exposes the original program; Reset restores its initial state. Input/ticks do not call the model, and available replies can run without a connection.

The opt-in enables native blocks in subsequent assistant replies while the admitted turn remains loaded. File/code/quote/attachment occurrences do not opt in, ordinary JavaScript fences remain code, and partial/oversized fences never execute. Guest contexts expose no file/network/process/DOM/agent APIs. Heap/stack/cooperative time limits and schema/geometry checks are in-process safeguards, not OS isolation. Event callbacks and uncached presentation use isolated VMs so closures/globals cannot escape rollback.

Omitted animation fps follows this device's actual display; explicit caps remain honored. Latest drag input is coalesced per display frame. Animation requires painted, foreground/focused canvases, stops behind sheets, and respects Remove animations. Frames never persist to disk; controls persist on release. The optional diagnostics distinguish view-update gaps/build time from script, input, layout/prepaint, paint/path cache, and native CPU submission work; they do not claim actual presentation FPS.

Rhai has been removed. Old JSON/Rhai visuals remain inert and source-readable; their original source and saved local data are not erased. Regenerate using @Visualize after updating the client. Existing admitted guides retain retry identity and clean labels. See [verification](scripts/VISUALIZE_VERIFICATION.md) and [`pi_visual`](../pi_visual/README.md). Native JavaScript preview fixtures include calculator, spirograph, and growing trace; they never call a provider.

## Scheduled jobs

Open **Schedules** from the **Computers** menu (tap the computer’s name on Home), marked with a calendar-and-clock icon. Back returns to your sessions. Schedules manages recurring Pi prompts on that host; the phone does not keep timers or need to stay open.

- Choose a project, model, prompt, numeric five-field cron expression and IANA timezone. Preview upcoming occurrences before reviewing the schedule and explicitly allowing unattended execution.
- Each occurrence uses a fresh durable session. Pause affects future times, not an existing run. Editing a paused schedule keeps it paused. Delete keeps sessions and project files.
- **Run now** requires a separate confirmation and does not move the recurring clock. Pending/accepted admission is not completion; the history distinguishes finished, failed, interrupted, skipped, missed and unknown work. Open an occurrence’s session, then Back returns to its schedule history.
- Cached schedules are unknown when the computer is unreachable; offline changes/submissions are not silently queued. Conflicts preserve the draft. An uncertain mutation requires inspection rather than an automatic fresh-key retry.
- The computer must be awake with a configured provider and an updated durable helper advertising `schedules`. Missing/stopped scheduler setup and unsupported helpers are shown separately. Setup happens explicitly **on the computer**, never automatically from the phone.

```sh
~/.pi/desktop/bin/pi-desktop-remote scheduler start
# Optional login autostart; does not start the service immediately:
~/.pi/desktop/bin/pi-desktop-remote scheduler install --confirm
```

Linux/macOS hosts only for now. See [host setup and recovery](../pi_remote/README.schedules.md). Jobs can run commands and edit files as the host account; a read-only prompt is not a sandbox. Missed times are skipped by default, same-job overlaps are skipped, and timeouts request a best-effort Stop. Unknown admission can block further occurrences and needs inspection; this version has no administrative force-clear API. Task success and exactly-once external side effects are not guaranteed. Phone schedule caches/drafts and uncertain operation identities currently live in memory, not across process restarts.

The [HTML design](../../design/android/schedules/index.html) is still a local prototype. Native sample-mode screens and emulator tests never submit real scheduled work; [verification instructions](scripts/SCHEDULER_VERIFICATION.md) describe that boundary.

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
- **Native swarm observation:** an opted-in handoff opens **Map / Exchanges /
  Agents / Results**, in Moonstone or Evening. The map keeps real roster slots
  stable and draws only recorded, directed peer exchanges; dashed coordinator
  links are assignments, not dependencies. Exchanges filter by kind and exact
  peer ID, expand broker previews, and link replies to their exact question when
  it remains in the bounded snapshot. Groups contain actual roster members.
  Opening a peer retains its exact swarm/peer identity even if result order changes.
  Results use the same Markdown renderer as conversation replies, including
  headings, lists, links and copyable code blocks. Missing outputs keep an explicit
  placeholder; the source and full-conversation link are preserved.
  Ordinary crews and chains keep their existing navigation.
  Queued, Received and Cancelled are distinct; Received means explicit retrieval,
  not agreement or successful work. At most 32 recent message events and
  500-character text previews are available, not complete mailbox history.
  Disconnected or recovering session pipes show **Last known**, the last accepted
  phone receipt time, and no live input/Stop. Navigation and cached inspection
  remain available. Send rechecks connectivity, peer identity and the displayed
  child turn; a late Send cannot silently steer a replacement run or become a
  follow-up. Peer Stop requires exact-target confirmation and targets the current
  child conversation, including later follow-ups; original assignment summaries
  stay frozen. Uncertain Stop requests may be explicitly retried but are never
  replayed on reconnect.
  Structured user decisions, artifacts/worktrees, patches/integration/check
  records and more than eight peers remain explicitly unavailable—not simulated.
  See [swarm usage and limits](../../backend/durable/README.md#peer-communication-opt-in-swarm).
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
  copy part of it, or copy all of it. Tool output also supports long-press
  selection; call sheets put a Copy icon beside the Output heading, with
  “Ask about this” on its own full-width row. Edit call sheets show each call’s
  full diff and update while open; input-only previews are labeled “Requested
  changes” until an applied diff is reported.
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
- **Pages Pi makes:** the latest self-contained HTML artifact is interactive
  directly in its Android reply: calculators, inputs and local scripts work
  without opening another activity. Expand and Source retain its running state;
  later exact edits require **Load update**, which explicitly resets inputs.
  An untracked edit shows Unavailable, with an explicitly labelled saved-version
  choice. Inline pages cannot use the network, app files/data, native bridges or
  permission requests. Three native views and eight documents are cached, with
  a 2 MiB HTML limit; eviction, reconnect/store replacement and process death
  can reset them. Source stays readable after a browser failure; Retry starts
  fresh without restarting chat. Older cards and **Open separately** retain the
  existing viewer; desktop previews open in the browser. Ask for one complete
  file with embedded assets. [Design](../../design/android/inline-pages/README.md)
  · [native architecture, limits and emulator evidence](scripts/INLINE_PAGES_VERIFICATION.md).
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

Not yet: background connection service, multiple computers at once, reopening
past-message images on the phone, and persistent phone-side unsent drafts across
process death. Computer-side admitted history is durable. Session
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

## Experimental on-device backend proof

`build_apk.py --quickjs-backend-test` builds a separate **Pi local backend tests** APK (`dev.pi.android.quickjstest`). Tap **Try the local QuickJS backend** on Connect to run the unmodified upstream agent/provider code inside this app, with an offline read/write fixture. No Bun/Node executable, computer, API key, or paid provider is used. The normal release remains SSH-based.

The same opt-in APK offers **Local providers · Claude + Codex** with browser/device-code login, Keystore-encrypted credentials, refresh and local logout. The original Pi/computer selector and Computers sheet choose Local or a remote computer, without extra toolbar chrome. Account management does not launch a provider-specific chat. Local uses the existing session/thread/composer screens and both providers in the shared model picker; provider/model changes and account management preserve conversations and thread drafts, with model-supported thinking, image/text attachments, streaming and Stop. Synthetic auth/storage/inference tests pass; live-account interoperability is not yet validated. Conversations are memory-only; saved accounts survive upgrades/restarts. Stock tools and a credential-isolated shell host remain unfinished; this is not a completed local execution backend. This debuggable build is not production credential isolation. See [build/device verification](scripts/QUICKJS_BACKEND_VERIFICATION.md) and [provider account details](../../experiments/quickjs-backend/PROVIDER_AUTH_PLAN.md).

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
`project-empty`, `project-error`, `project-loading`, `project-long-path`, `project-tree`, `project-search`, `project-file`, `tool-image`, `media-sample` (the shared sample both apps' tests draw), `subagents`, `subagents-many`, `subagents-done`, `subagent` (work handed to subagents, a crew of 24, and one subagent's own screen),
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
python3 crates/pi_android/scripts/test_inline_pages.py --serial DEVICE --output /tmp/pi-inline-pages
adb shell am start -n dev.pi.android.uitest/dev.pi.gpui.GpuiActivity \
  -a android.intent.action.VIEW -d pi://preview/long-input
```

The **Pi UI tests** app is `dev.pi.android.uitest`; it can also connect normally.
Fixture links are compiled only with `ui-test`. This separate APK is debuggable
so the test runner can read app-private fixture telemetry via `run-as`; normal
release APKs are not. The telemetry contains routes, bounds and character/image
counts, never credentials or prompt contents, and is disabled for real sessions.
The inline-page harness additionally records synthetic fixture document values
and geometry through a shell-only, DUMP-protected readback. It does not add a
page-callable JavaScript interface; see the [verification notes](scripts/INLINE_PAGES_VERIFICATION.md).
Screenshots require visual
inspection: the script checks process survival and crash logs, not appearance.
`--case draft-targets` checks first taps and holds across the draft’s text and
padding, including a system-clipboard Copy/Paste round trip. `--case call-output`
checks the top Copy icon, first-open selection menu and full-width Ask action.
`--case swarm-results` checks rendered peer results, exact Unicode code copying,
and opening the full peer conversation. `--case swarm-toggle` checks the session
More menu's Swarm switch and captures the Home crew/rail gap. Swarm is a host-owned
permission for **new** explicit swarm crews; switching it off leaves ordinary
subagents and admitted crews alone. Older runners show Unavailable rather than a
local-only switch; update the helper and start a fresh session to use it.
`--case subagent-input` checks the shared regular editor without selection controls,
its integrated Working/Stop header, first taps across its padding, IME resize/typing,
and independent Send/Stop targets. This dedicated fixture has no transport. Host touch tests also preserve catch-only taps inside a moving viewport
while allowing the first tap on controls outside it.
`--case diff-spacing` captures activity and per-call diff cards with four-digit
line numbers and wrapped code. Their shared gutter keeps a 12 dp inset from the
border and grows for larger numbers without misaligning the code rows.
`--case gestures` scrolls the target row fully inside the list viewport before
swiping, then verifies short swipes, cancellation and deletion of exactly the
confirmed sample session. Targeting tests run without a device:
`python3 crates/pi_android/scripts/test_interactions_tooling.py`.
`test_interactions.py` waits for settled layouts and asserts the QR connect screen
and native camera scanner, typing, model search, thinking selection, long Unicode
edits, Stop, actual system-picker image import, image preview/send, scrollbars,
sheet drags, confirmed swipe deletion, completed activity details, long
tool/reply scrolling, project selection, prompt templates and skills in
Resources and the `/` suggestions (`--case commands`), and subagents: the
hand-off card, a subagent's screen with Stop, and back, and a crew of 24 that
shows five, all, or folds (`--case subagents`).
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
