# Pi in your pocket, next

**A second pass over the [Android screens](../README.md): same themes, fonts and voice, tighter proportions, fewer boxes, and every action that moves work forward at the bottom of the screen.**

**[Open the before/after gallery](index.html)** · [Overview PNG](overview.png)

These are mocks. Nothing here changes `crates/pi_android` yet, and the [current screens](../screens) are untouched.

## Ratios

Everything is sized from a few numbers, kept in [next.css](next.css):

| | Scale | Used for |
| --- | --- | --- |
| Space | 4 px grid: 4 · 8 · 12 · 16 · 20 · 24 · 32 · 48 | One 20 px screen gutter everywhere (the current screens mix 12, 16 and 20) |
| Type | ×1.2 from 15 px: 12.5 · 15 · 18 · 22 · 26 · 31 | Meta · body · titles · sheet titles and Pi's headline · home summary · first-run heading |
| Line | 16 · 20 · 22 · 24 · 28 · 36 | Meta, UI and prose lines on the same scale |
| Radius | 8 · 12 · 16 · 24 · pill | Code · wells and tiles · cards · sheets and composer · buttons and chips. An inner radius is the outer one minus its padding. |
| Height | 48 targets · 56 one-line rows · 64 two-line rows | Buttons are 48 or 40 tall and always pills |

Three voices, as before: sans for the interface, italic serif for Pi's words and headings, mono for anything the computer runs.

## What changed

- **The dock.** The bottom of each screen holds what you do next: the start bar on Home, Stop on a live strip above the composer, the question in place of the composer, Review above the follow-up, and the lines you selected while reviewing. Strips that belong to the composer share its border.
- **Rules instead of boxes.** Lists are hairline rows on the canvas. Cards are kept for things that stand alone: a question, a hand-off summary, a page.
- **The run rail, smaller where it repeats.** In the thread it stays as tiles. On Home it's four 4 px segments in the same colors, and a finished run collapses to four tiles.
- **Answer in place.** A question no longer covers the thread with a scrim. It takes the composer's place, so the run so far stays readable. Picking a choice still doesn't answer; Answer does.
- **Home is a launcher.** The computer is the title and opens a switcher sheet, which replaces the drawer. Starting work opens a sheet with the composer already on the keyboard.
- **People's words over ids.** File history names each point by the prompt that made it; the operation id moves into the details.
- **Pages show themselves.** The card in the thread shows the page as it looked once loaded. The viewer gives the page the whole screen and puts its controls at the bottom.

Kept as they are: typing (08), notifications (10) and settings (12) already work within these ratios.

## What it would take

Most of this reuses the app's existing pieces: `ui::card`, `ui::row`, the composer, and the stage tiles. The parts that are new to build:

- the live strip and the answer dock, both attached to the composer;
- the stage segments on Home rows;
- the computer switcher sheet, replacing the drawer;
- a page snapshot: the web view captures a bitmap after the page first loads, and the card shows it with `img()`;
- moving `PageActivity`'s bar to the bottom.

## Reproduce

```sh
python3 design/android/next/render.py
```

The same needs as [../render.py](../render.py): a local Chromium or headless shell, plus Pillow. It renders each `screens/*.html` at 412×915 and 2× scale, then composes [overview.png](overview.png). The page images are the [Aurora sample](../../../crates/pi_android/assets/samples/aurora.html), rendered with the same browser.
