# Calls in the thread

**A mock for following Pi while it works: its tool calls stream into the thread as quiet one-line rows, the call still running is the only one open, and the run line folds into the dock until the run is done.**

[Overview PNG](overview.png) · [Gallery](index.html) · Built on [next4](../next4/README.md)'s stylesheet and proportions. It changes [04 Working](../next4/screens/04-working.png) and keeps the rest.

Built in [crates/pi_android](../../../crates/pi_android/src/screens/stream.rs). Try it with `cargo run -p pi_android --example preview -- working`, or `verifying`, `call`, `run-line` and `calls-shown`.

## Why

The run line in 04 Working is good for seeing a run as a whole, but hard to follow while calls are still coming in. Stages group the calls, so the newest one is not always at the bottom, and chips for files read sit beside the live diff with the same weight. While Pi works, the thread now reads like a chat: newest last, one line per finished call, and Pi's own words between them.

## Screens

| Screen | What it shows |
| --- | --- |
| [01 Streaming](screens/01-changing.png) | Pi's narration, then the calls it made: **Read 5 files** folded with the first names, and **Searched**. The edit in progress is the one open card, with its diff and a caret where Pi is typing. The strip carries the run line folded to four segments, the stage, the time and **Stop**. |
| [02 Output as it comes](screens/02-verifying.png) | Finished edits as one line each with their change size. A type check that failed stays one line, in coral, with the line that matters. The test run is open: its last four lines of output, newest at the bottom, and how many tests have run. |
| [03 A call opened](screens/03-call.png) | Tapping a finished call opens it as a sheet: the command, where it ran, its exit code and the full output. A link goes to what Pi did next. **Ask about this** attaches the call to the composer. |
| [04 The run line on demand](screens/04-runline.png) | The strip's chevron pulls the existing run line up as a sheet, with each stage's counts and time. Each stage jumps to its first call in the thread. |
| [05 Done, folded](screens/05-done.png) | When Pi hands off, the calls fold under the run line into **11 calls · 1 failed, fixed**, with **Show** to unfold them. Then Pi's hand-off and the report card, as in next4's 06 Done. |
| [06 Evening](screens/06-evening.png) | Screen 02 in the dark theme. |

## How the stream works

- **One line per call.** A row is 40 tall: the tool's icon, a verb in the past tense, what it touched, and its time or change size. Long targets end in an ellipsis; the time never moves.
- **Each call's icon wears its stage's colour**, so the stream and the run line agree without the rail drawn beside it.
- **Calls of one kind in a row fold together**, such as five reads into **Read 5 files** with the first names under it. The chevron unfolds them in place.
- **Only the live call is open.** An edit shows its diff and a command shows its last four output lines. When it finishes, it folds to one line and the next call opens below it.
- **Failures stay visible.** A failed call keeps its one line, in coral, with the error that matters. Pi's next call usually says what it did about it.
- **Pi's words sit between calls** as plain secondary text, never in a bubble, so the thread reads top to bottom.
- **The run line is one tap away.** It lives in the strip as four segments while Pi works, opens as a sheet, and is drawn in full under a finished run.

## Reproduce

```sh
CHROME=/path/to/chrome python3 design/android/stream/render.py
```

Same as next4's renderer, except that it renders into a taller window and crops to the phone, because new headless Chrome's viewport is shorter than its window.
