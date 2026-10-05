# Visual workflow directions

**[Open the comparison](index.html)**. Eight alternatives that make a run's shape visible and give the UI more range. They build on [workbench-vision](../workbench-vision/README.md), [thread-polish-mocks](../thread-polish-mocks/README.md) and the [streamline study](../desktop-streamline-study.svg).

These are static design proposals, not implemented UI. Content, timings and check results are illustrative.

| Screen | Idea |
| --- | --- |
| [A · Run rail](screens/a-run-rail.html) | The turn as a timeline with coloured step nodes, a live pulse and a dashed queued future. **Recommended base.** |
| [B · Stage flow](screens/b-stage-flow.html) | A pipeline strip (Understand → Change → Verify → Hand off), with prose on the left and a result board for files and checks. |
| [C · Studio](screens/c-studio.html) | A material layer: blurred window, floating sheet, glossy tool tiles and an ambient state glow on the composer dock. |
| [D · Session map](screens/d-session-map.html) | A node-graph view of the turn, with session-tree forks as ghost branches. |
| [E · Follow mode](screens/e-follow-mode.html) | Compact steps beside a live stage showing the file or command just touched, with a step scrubber. |
| [F · Mission control](screens/f-mission-control.html) | A board of all sessions: Needs you / Working / Queued / Done today. |
| [G · Signal](screens/g-signal.html) | A bold editorial identity: ink sidebar, one loud accent, a state band and big numerals. |
| [H · Timeline tracks](screens/h-timeline.html) | Video-editor-style lanes for model, explore, edit and run, with a playhead. |

## Implemented: A + E

The run rail and follow mode are in the app:

- **Rail.** `components/rail.rs` holds the shared pieces: step kinds and hues, soft tinted tiles with a breathing live halo, diff blocks and the status dot. Each activity group in `desktop/transcript` becomes a step with a tile. The step the run is in sits on a soft card, as in E.
- **The flow upfront.** A turn follows a fixed template: Understand → Change → Verify → Hand off. The rail draws the stages ahead as dashed tiles the moment a turn starts, and each one disappears as the real step that fills it lands above. The settled turn's closing text gets the filled Hand off tile. Calls pick their stage from what they do: reads, searches and look-only commands understand; edits, writes and other commands change; commands like `npm run check` or `cargo test` verify. The template is the plan, so no prompt asks the agent for one, and an unreached stage is never shown as done. The follow scrubber shows the same stages as dashed ticks. Try it with `PI_DESKTOP_DEMO_WORKBENCH_FRESH=1` (see `captures/08-fresh-light.png`).
- **Follow mode.** The stage is `desktop/follow.rs`. Open it with the eye button in the session tabs or Ctrl+Shift+F. It follows the latest call. Picking a step in the thread or on the scrubber pauses following, and the switch resumes it. When the main area is under 900px wide, the thread keeps the full width.
- **Theme.** New `violet` and `orange` hues and `Theme::tint`. Reading and running share `steel`, as in the study's legend.

## Constraint

The mocks use only effects GPUI paints natively: 2-stop linear gradients, layered and inset box shadows, dashed borders, `pattern_slash`, `PathBuilder` paths, keyframe and spring animation, and window-level blur. They avoid backdrop-filter, radial or conic gradients, CSS filters, blend modes and masks.

## Reproduce

```sh
python3 design/visual-workflow/render.py            # all screens
python3 design/visual-workflow/render.py c-studio   # one screen
```

This needs a local Chromium or headless shell (or set `CHROME`). The mocks load fonts from `assets/fonts/` and make no network calls.
