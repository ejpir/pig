# Pi in your pocket, next2

**A second pass over [next](../next/README.md), made with a written design plan that was checked against generic defaults before any screen was drawn. Same Moonstone and Evening themes, fonts and proportions; one memorable element and everything else quiet.**

**[Open the next / next2 gallery](index.html)** · [Overview PNG](overview.png)

These are mocks. Nothing here changes `crates/pi_android`, and the [current screens](../screens) and [next](../next) are untouched.

## Brief

- **Subject:** Pi, a coding agent that works on someone's computer.
- **Audience:** developers away from their desk.
- **Job:** show what needs them and what is running, let them answer and review, and start work. Editing and terminals stay on the desktop.

## Plan

**Colour.** The Moonstone palette as before. The canvas `#faf9f7`, ink `#252f3d` and slate accent `#4b607c` carry the interface. The four stage colours carry the run line and appear nowhere else: read `#2f7fa8`, edit `#b0661a`, check `#2e7950` and hand-off `#4b607c`, plus waiting `#a57514`.

**Type.** Each family has one job:

- IBM Plex Sans is the interface, with tabular figures so times and counts line up in columns.
- Georgia italic is Pi's voice: its questions, its hand-off and its replies. The one other use is the first-run question.
- Commit Mono is the computer's voice: commands, paths and code.

**Layout.** Everything is left-aligned on a 20 px gutter. Lines in a thread hang from a 16 px gutter column. The bottom third of the screen is the dock, holding one primary action per screen.

```
Home                              Thread
studio-mac ▾           ⌕  ≡       ←  Qwen signatures      ⓘ ⋮
╭──────────────────────────╮         you ▸ ┌────────────────┐
│ Qwen signatures   2 min  │               └────────────────┘
│ Run the provider tests?  │      ●  Understood          0:21
│ $ pnpm test …            │      ┃  openai-completions.ts
│ In pi           [Answer] │      ◉  Changing           +3 −1
╰──────────────────────────╯      ┊  ┌ diff ┐
Streaming retry       2:40        ○  Verify
━━━━━━━━━━━━━━◉                   ○  Hand off
Changing 2 files        pi        ┌ Working 1:12      ■ Stop ┐
──────────────────────────        │ Queue a follow-up…       │
✓ Mistral thinking   14:02        └──────────────────────────┘
[ What should we change?  (+) ]
```

**The one ornament: the run line.**

- A run is one line through its stages, each stretch in its stage's colour. Where the run is now, a ring.
- On Home every working session's line shares one time scale, so a glance shows which run has been going longest and where its time went.
- In a thread, the line runs down the gutter with a station per stage.
- A finished run collapses to one line between your prompt and Pi's answer, with each stretch's time under it.
- Details labels the line with its stage names.
- The line appears only while a run is alive or being summed up, never as decoration.

## Review of the first pass

The first pass ([next](../next)) was checked against generic defaults. Five things were cut or changed:

| In next | Why it was generic | In next2 |
| --- | --- | --- |
| 51 meta strings joined with middle dots (“pi · studio-mac”) | Template chrome that appears on any subject | Plain phrases (“pi on studio-mac”, “Turn 3, 4 min ago”) or a right-hand column |
| Mono for counts and small data | Mono used as decoration | Tabular sans for numbers; mono only for what the computer runs |
| Labels above groups (“Working”, “Today”, “Pair in under a minute”) | Labels that the content already makes clear | Removed; a session's state shows in its run line or icon |
| A serif summary line on Home (“1 needs you, 2 working”) | Serif used for the app's voice, not Pi's | Removed; Pi's question is the hero, in Pi's voice |
| Tiles, badges and boxed lists | The card kit, with the same treatment everywhere | Cards only for things that stand alone: Pi's question, a page, code |

What stayed from the first pass: the brief's theme, the ratios, the dock at the bottom, answering in place, the computer switcher, and the page poster and viewer.

## Reproduce

```sh
python3 design/android/next2/render.py
```

The same needs as [../render.py](../render.py): a local Chromium or headless shell, plus Pillow.
