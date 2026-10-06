# Pi in your pocket

**The approved Android screens: Pi keeps working on your computer, and the phone follows its sessions, answers its questions, reviews its changes and starts new work.**

**[Open the gallery](index.html)** · [Overview PNG](overview.png)

These are mocks for [crates/pi_android](../../../crates/pi_android/README.md). They use the Moonstone and Evening themes, IBM Plex Sans, Georgia italic and Commit Mono, and only effects GPUI paints natively: solid and two-stop fills, shadows, radii, solid and dashed borders, opacity and SVG icons.

## Screens

| Screen | What it shows |
| --- | --- |
| [01 Pair](screens/01-pair.png) | Three steps to pair by QR code, with the command to run on the computer. A note on what pairing trusts sits above **Scan computer QR**; SSH key setup is the quiet option below. |
| [02 Home](screens/02-home.png) | The computer is the title and opens the computer switcher. Pi's question comes first, with the command and **Answer**. Working runs are drawn to time on one shared track, coloured by stage. Today's results follow, and a start bar sits at the bottom. |
| [03 New session](screens/03-new.png) | A sheet from the start bar: the project, the composer on the keyboard, and starting points that fill the draft but never send it. |
| [04 Working](screens/04-working.png) | The run line down the gutter, one station per stage with its icon. Under each stage is what Pi did: counts, up to three file and search chips then a count, and the live diff. **Working**, the time and **Stop** sit on the composer. |
| [05 Answer in place](screens/05-answer.png) | The question card takes the composer's place, so the run so far stays readable. The card shows **Needs you**, where the command runs, the command and the choices. Only **Answer** answers. |
| [06 Done](screens/06-done.png) | The finished run line with each stretch's time, Pi's hand-off, and a report card. The card lists each file with its change size, and the check marked **Passed**. **Review** sits above the follow-up. |
| [07 Review](screens/07-review.png) | One file at a time with arrows between files, an edge-to-edge diff with folded unchanged lines, and selected lines attached to the composer. |
| [08 Details](screens/08-details.png) | The run line with a key of stages and times, then cost, turns, changed files, context with the compaction mark, and the model, history and tools. |
| [09 File history](screens/09-history.png) | Points named by the prompts that made them, the current point marked, and **Restore** on each. Restoring saves the current files first. |
| [10 A page Pi made](screens/10-page.png) | The run line with a key that says what Pi did, then the page as a card showing how it looks, with **Open** and the source. |
| [11 Page viewer](screens/11-viewer.png) | The page on the whole screen. **Page**, **Source**, close and reload sit at the bottom. |
| [12 Evening](screens/12-evening.png) | Working in the dark theme. |
| [13 Computers](screens/13-computers.png) | A sheet from the title: computers with their state, **Connect**, pairing another, the sample sessions and Settings. |

## Ratios

| | Scale | Used for |
| --- | --- | --- |
| Space | 4 px grid: 4 · 8 · 12 · 16 · 20 · 24 · 32 · 48 | One 20 px screen gutter |
| Type | ×1.2 from 15 px: 12.5 · 15 · 18 · 22 · 26 · 31 | Meta · body · titles · sheet titles and Pi's headline · first-run heading |
| Radius | 8 · 12 · 16 · 24 · pill | Code · wells and tiles · cards · sheets and composer · buttons and chips |
| Height | 48 targets · 56 one-line rows · 64 two-line rows | Buttons are 48 or 40 tall |

## The run line

A run is drawn through its four stages: Understand, Change, Verify and Hand off. Each stretch is in its stage's colour.

- **In a thread** the line runs down the gutter. Each station carries its stage's icon, as on the desktop, so colour is never the only cue. Stages ahead are dashed, and a hand marks a run waiting for you.
- **On Home**, working runs share one track, so a run that has gone on longer shows longer.
- **Under a finished run**, each stretch shows its time. Where nothing else says what Pi did, as with a page, a key pairs each stage's icon with what it did. In Details, the key names the stages.

## The dock

What moves work forward is at the bottom of each screen:

- the start bar on Home;
- **Working** and **Stop** on the composer while Pi works;
- the question in place of the composer;
- **Review** above the follow-up;
- the selected lines while reviewing.

## Reproduce

```sh
python3 design/android/next4/render.py
```

Needs a local Chromium or headless shell (or set `CHROME`) and Pillow. It renders each `screens/*.html` at 412×915 and 2× scale, then composes [overview.png](overview.png). The page images are the [Aurora sample](../../../crates/pi_android/assets/samples/aurora.html), rendered with the same browser.
