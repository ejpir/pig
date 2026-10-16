# Pi in your pocket, next5

**A proposal on top of [next4](../next4/README.md): Home becomes an inbox of what needs you, the run line shows what happened in order, answers are the buttons themselves, review ends in a decision, and the phone can reach you while the app is closed.**

**[Open the before/after gallery](index.html)** · [Overview PNG](overview.png)

These began as mocks; most of them are now in [the app](#built). Same Moonstone and Evening themes, fonts, proportions and GPUI-only effects as next4; 04 Working builds on [stream](../stream/README.md), which is what the app ships.

## Why

Each change starts from something in the app or the screens as they are.

1. **Pi rarely asks, and results pile up.** A question only comes from a stock Pi extension's `confirm` or `select` ([projection.rs:1064](../../../crates/pi_android/src/projection.rs)). Durable sessions, the kind the phone starts, never ask, and a question never carries a command (`command: String::new()`, line 1099). next4's Home leads with a permission card built around a command, while finished runs you haven't looked at have no state at all.
2. **The run line is tidier than the run.** Stages come from tool names: `edit` and `write` are Change, any `bash` is Verify, everything else is Understand ([projection.rs:304](../../../crates/pi_android/src/projection.rs)). Time is summed into four buckets ([model.rs:401](../../../crates/pi_android/src/model.rs)), so order is lost. A run that failed its type check, fixed it and passed looks the same as one that went straight through. Exploring with `rg` or `ls` through bash is painted as Verify, in the green of Passed.
3. **The stage colours collide.** Change and Waiting are ΔE 9.5 apart (1.5 with deuteranopia). Understand and Hand off are both blue (ΔE 14.3, and 7.2 in Evening). Verify is the same hex as Passed, additions and Connected. On Home and under a finished run there are no icons, so colour is the only cue there. See [12 Palette check](screens/12-palette.png).
4. **Answering takes a choice and then a generic button.** A radio list plus **Answer** is a desktop dialog. Separately, a `select` with more than three options silently loses the rest: its options are zipped onto the three-value `Answer` ([projection.rs:1087](../../../crates/pi_android/src/projection.rs)).
5. **Review ends nowhere.** After the last file there is no "reviewed", no undo nearby, and nothing that clears the run from Home.
6. **The phone only knows while it is open.** There is no foreground service, notification actions can only reopen the app ([Notifications.java:85](../../../crates/gpui_android/java/dev/pi/gpui/Notifications.java)), and the manifest has no share target, only `MAIN` and `VIEW` ([build_apk.py:309](../../../crates/gpui_android/scripts/build_apk.py)). Yet "start a task with a file, screenshot or log attached" is one of the app's five jobs.
7. **First run hides the easy ways in.** Local is reached by tapping the logo ("Choose Local from the Pi logo…", [connect.rs:344](../../../crates/pi_android/src/screens/connect.rs)), and the sample sessions only appear after **Set up with an SSH key instead**.

## Screens

| Screen | What it shows |
| --- | --- |
| [01 First run](screens/01-first-run.png) | Two doors, **On your computer** and **On this phone**, each saying what it means, and **Look around with sample sessions** at the bottom. The computer door leads to next4's pairing steps. |
| [02 Home](screens/02-home.png) | **Needs you** first: a yes/no question answered in place, a question with four choices, a result you haven't reviewed and a run that stopped, each with a dot until you open it. Then working runs on the shared track, drawn in order with a failed check where it happened. What you've seen folds into **Seen today**. |
| [03 Caught up](screens/03-caught-up.png) | The same Home with nothing left: "Nothing needs you." The answered question stays on its run's track as a hand, and reviewed runs go quiet. |
| [04 Working](screens/04-working.png) | stream's thread, with the strip's run line drawn in order: the failed type check is a coral point, the tests still running are grey. Tapping the title opens Details, so the info button goes. The composer is one line until you type. |
| [05 A question](screens/05-question.png) | All four of an extension's choices as buttons. Tapping one answers; **Later** leaves it waiting. |
| [06 Done](screens/06-done.png) | The run as it happened, with each kind's total in a key with icons; Pi's hand-off; **Worth a look**, Pi's pointers to what to check, including what it couldn't test; then the files and the check, which failed once and then passed. |
| [07 Review](screens/07-review.png) | Pi's note pinned above the lines it is about. **Looks good** marks the run reviewed and clears it from Home; **Undo this turn** restores the files from before it, saving the current ones first, as File history does. |
| [08 Lock screen](screens/08-lock-screen.png) | A working run as an Android 16 Live Update, its run line drawn by `ProgressStyle` as segments and points. The questions and the result sit under it; answering asks you to unlock first. |
| [09 Share to Pi](screens/09-share.png) | Pi in the system share sheet, with recent sessions as Direct Share targets. Sharing starts a session with the attachment, or adds it to a session's next message. The status bar carries the Live Update chip. |
| [10 Details](screens/10-details.png) | Opened from the title: the run line, then each stretch in order with what it did, each check with its result, then context, model and file history. |
| [11 Evening](screens/11-evening.png) | 06 Done in the dark theme. |
| [12 Palette check](screens/12-palette.png) | The old and new run lines as most people see them, then simulated for deuteranopia and protanopia, with the numbers. |

## The run line

A run is drawn as what happened, in order and to time.

- **Three kinds of stretch.** Looking (blue): reading, searching, exploring, and Pi's own words between calls. Changing (orange): edits and writes. Checking (grey): tests, builds, linters and type checks, the commands `is_check` already recognises ([projection.rs:970](../../../crates/pi_android/src/projection.rs)). Any other bash command counts as Looking.
- **A check is grey until it has a result.** It ends in a point: green with a tick if it passed, coral with a cross if it failed. A run that took four attempts looks like it.
- **Waiting for you is dotted amber and ends in a hand.** Amber is never a stretch of work, so it can't be mistaken for Changing.
- **Hand off is gone.** Pi's closing words count as Looking, and the header says when it finished.
- **Colour is never the only cue.** Points carry glyphs, waiting is dotted, the key under a finished line has icons, and on Home each run says in words what it's doing.

## Colour

| Token | next4 | next5 | Why |
| --- | --- | --- | --- |
| Checking | `--h-check` `#2e7950`, the green of Passed | `#949ca6` (Evening `#6c7683`) | A check isn't good news until it passes. ΔE from Looking 19.3, from Changing 33.0 |
| Results | green and coral, shared with Verify | green and coral, results only | Passed, Failed, added, removed |
| Needs you | `#a57514` as text on its tint, 3.25:1 | ink `#74510f`, 5.73:1 | Fill and ink are separate tokens |
| Passed text | `#2e7950` on its tint, 4.13:1 | ink `#276b46`, 5.00:1 | As above |
| Faint | `#8a837c`, 3.55:1 (Evening 4.48:1) | `#77706a`, 4.63:1 (Evening `#8d949c`, 5.53:1) | Stages ahead and hints are information |
| Hand off | `--accent`, ΔE 14.3 from Understand | none | One blue |

ΔE is CIEDE2000. Contrast is WCAG 2.

## What it would take

- **Run line:** keep an ordered list of stretches in the projection instead of `times[4]`, split bash with `is_check`, and keep each check's result on the line.
- **Questions:** carry an option's index instead of `AllowOnce`/`AllowSession`/`Deny`, so every option survives; render the title and message as Markdown; answer `confirm` from Home.
- **Inbox:** a "seen" mark per session and turn, set when its screen opens; "reviewed" set by **Looks good**. Keeping it on the computer would let Pi Desktop agree with the phone.
- **Worth a look:** the durable runner asks for the hand-off to end with pointers (file, line, reason, and anything it couldn't check), sent as data rather than prose.
- **Reaching you:** a foreground service that holds the SSH connection while a session works; `Notification.ProgressStyle` on Android 16 and an ordinary progress notification before it; an `ACTION_SEND` filter for images and text in the manifest, and Direct Share shortcuts for recent sessions.
- **The rest:** the title opens Details; the composer shows the model and effort only when focused; first run offers both doors and the samples.

## Built

In [crates/pi_android](../../../crates/pi_android/README.md): try `cargo run -p pi_android --example preview -- <screen>` with `connect`, `pair`, `sessions`, `verifying`, `waiting`, `done` or `details`.

- **01 First run:** the two doors, **Look around with sample sessions** below them, and pairing a step of its own. The phone door appears where the app is built with Local.
- **02 and 03 Home:** the inbox, its dots, **Seen today** and “Nothing needs you.”, with when you last looked at each session kept per computer. A yes-or-no question is answered in place.
- **04 Working:** the strip's run line in order, the title opening Details, the one-line composer.
- **05 A question:** a button per choice, all of a select's options (the cut at three is fixed), Pi's message under the title.
- **08 Lock screen:** the working notification as an Android 16 Live Update with the run line as its progress bar. It updates while the app's process runs; there is no background connection yet.
- **10 Details:** the run line and each stretch in order.
- **The run line everywhere:** ordered stretches from each call's stamped end, checks by what they run, results as marks, waits dotted, Hand off folded into looking.

Not built: 06's **Worth a look**, 07's **Looks good** and **Undo this turn**, 09 Share to Pi, and the colour changes of 12 beyond the run line's grey checks.

## Not mocked

- **One inbox across computers.** The computer becomes a property of each session, not a mode of the app. It needs a connection to each computer at once.
- **System font size.** The scale comes from `density / 160` alone ([window.rs:613](../../../crates/gpui_android/src/window.rs)), so Android's font size setting changes nothing. Meta text is 12.5 dp and can't grow.
- **TalkBack.** `gpui_android` has no accessibility bridge (the only AccessKit adapter in `Cargo.lock` is macOS's), so TalkBack can read inline pages but not the app itself.
- **One serif everywhere.** The mocks use Georgia, phones render Noto Serif ([theme.rs:9](../../../crates/pi_android/src/theme.rs)) and Linux DejaVu Serif. Bundling an OFL italic such as IBM Plex Serif would make Pi's voice the same on every screen.
- **Pairing from Pi Desktop.** Pairing is only `pi-desktop-remote pair` in a terminal; the desktop app could show the same QR.

## Reproduce

```sh
swift design/android/next5/render.swift            # every screen and overview.png
swift design/android/next5/render.swift 02-home    # one screen
```

The renderer is the [subagents](../subagents/README.md) one, using the system's WebKit on macOS with no browser to install and no network calls. Its base path is made absolute, so it also runs from inside this folder.
