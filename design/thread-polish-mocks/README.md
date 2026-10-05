# Thread result polish mocks

Focused static proposals based on the real-session screenshot shared on 2026-10-02. They explore the tail of a completed turn: shell guidance, changed files, a failed tool call, and the idle composer.

These are **illustrative design assets**, not native captures and not evidence that the sample commands or checks ran.

## Options

### 01 · Balanced operational rail — recommended

- Keeps every changed-file row full-width and clickable.
- Holds labels, counts, and review navigation in a 1040px metadata rail to reduce eye travel.
- Uses `Modified` when no trustworthy line counts exist instead of displaying `+0`.
- Gives project-root files an explicit `./` location.
- Replaces the passive terminal state with an actionable issue notice.
- Uses a shorter idle composer that can grow with content.

### 02 · Grouped result card — selected

- Treats all changed files as one bounded result object.
- Scans quickly and creates a strong relationship between the file set and its review action.
- More contained, but also more card-like than the current editorial direction.

### 03 · Fluid minimal ledger

- Preserves the fully fluid work surface.
- Keeps change metadata near filenames rather than at the far window edge.
- Adds a shell-language header and copy affordance while retaining honest command content.
- Best if avoiding a new operational width constraint is more important than strict alignment.

## Decision

**02 · Grouped result card** was selected for implementation. The production pass keeps file navigation explicit, uses an honest `Modified` label when counts are unavailable, places failed-tool review beside the affected turn, and lets an empty settled composer collapse without changing working, queued, revision, or attachment states.

The slash character visible in the source screenshot was incidental and is not represented as a product state here.

## Files

- [`index.html`](index.html) — browsable comparison
- [`overview.png`](overview.png) — combined raster preview
- [`../thread-polish-mocks.svg`](../thread-polish-mocks.svg) — standalone combined vector sheet
- [`screens/`](screens/) — individual SVG and PNG proposals
- [`manifest.json`](manifest.json) — asset metadata

Regenerate locally:

```sh
python3 design/thread-polish-mocks/render.py
```
