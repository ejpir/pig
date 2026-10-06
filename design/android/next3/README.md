# Pi in your pocket, next3

**Each screen taken from the pass that read better, [next](../next/README.md) or [next2](../next2/README.md), with the changes asked for in review.**

**[Open the next / next2 / next3 gallery](index.html)** · [Overview PNG](overview.png)

These are mocks. Nothing here changes `crates/pi_android`, and the other sets are untouched.

## What came from where

| Screen | Base | Changes |
| --- | --- | --- |
| 01 Pair | next | The SSH note moves above the button, as in next2 |
| 02 Home | next | No summary line; no rule between the working sessions; each working run drawn to time on one shared track, so the longest run shows longest |
| 03 New session | next2 | |
| 04 Working | next2 | Stage icons inside the run line's stations; next's detail under each stage (counts, up to three file and search chips then “+4 more”, the diff card); a status dot and the time in the Working strip, without the current action |
| 05 Answer in place | next2 | next's stage detail in the thread; next's question card |
| 06 Done | next2 | Each stretch's time under the finished run line; next's report card with files, change size and the check says what was done |
| 07 Review | next2 | |
| 08 Details | next | The run line and its key added above the numbers |
| 09 File history | next2 | |
| 10 A page Pi made | next | The run line replaces the stage tiles, with a key that says what Pi did |
| 11 Page viewer | next2 | |
| 12 Evening | next3's Working | In the dark theme |
| 13 Computers | next2 | |

## The run line, with a key

The run line from next2 stays: a run is drawn through its stages, each stretch in its stage's colour and as long as the stage took. In review, colour alone didn't say what Pi did. So:

- **In a thread**, every station carries its stage's icon, as on the desktop: eye, pencil, shield, arrow, and a hand when Pi is waiting.
- **Under a finished line**, where nothing else says what Pi did (a page Pi made), a key pairs each stage's icon with what it did and how long it took: “Wrote aurora.html 0:40”. On Done the report card already says it, so only the times sit under the line.
- **On Home**, each working run is drawn to time on one shared track, so a run that has gone on longer shows longer.
- **In Details**, the key names the stages themselves.

The base is next's stylesheet; [next3.css](next3.css) adds the run line, the stations and the key at the end.

## Reproduce

```sh
python3 design/android/next3/render.py
```
