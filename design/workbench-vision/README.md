# Less interface. More work.

**A free-rein design direction for Pi Desktop. Static proposals, not implemented UI.**

## Open the concept

- **[Eight-screen vector SVG](../desktop-workbench-vision.svg)** — the whole direction on one zoomable sheet.
- [Individual-screen gallery](index.html) · [Overview PNG](overview.png).
- [Earlier, more conservative streamline study](../desktop-streamline-study.svg) — preserved separately.

The master embeds the existing Plex Sans and Commit Mono fonts, their licenses and the vector artwork. No screenshots are wrapped in SVG. Serif headings use Georgia/DejaVu Serif system fallbacks; PNGs preserve the reviewed Linux rendering. Clicking a screen title opens its individual SVG while the repository folder structure is intact.

## Eight moments in the same workbench

| Screen | What changes |
| --- | --- |
| [01 · Working](screens/01-work.svg) | Project/session hierarchy only. Thread and Changes are the permanent work views; edited files are useful links rather than buried tool output. |
| [02 · Reviewing](screens/02-review-wide.svg) | The 1600px window earns its width with a side-by-side diff. File navigation and revision requests live beside the work, not in an inspector. |
| [03 · File details](screens/03-file-details.svg) | A file opens in the main area. A deliberately opened inspector explains provenance, history and context without being required for core actions. |
| [04 · Waiting](screens/04-waiting.svg) | A pending extension question presents one decision with an explicit submit. Choosing a row alone does not answer it. |
| [05 · Evening](screens/05-work-evening.svg) | The same layout and hierarchy in the ink-blue theme, with neutral selection and readable supporting text. |
| [06 · Tools](screens/06-tools.svg) | Models, resources, context and history are still reachable through search. “Settings & tools” also provides a mouse-accessible entry point. |
| [07 · Compact review](screens/07-review-compact.svg) | At 1000×720 the file rail becomes a picker and the diff becomes unified. Open File and revision actions stay visible. |
| [08 · Starting](screens/08-start.svg) | Lead with the task and the fluid composer. Starters populate a draft; they should not silently run a prompt. |

## Implemented: 03, 05 and 08

- **03 · File details.** An opened file shows a project breadcrumb, a "Current file" header and a code card. The lines the latest observed edit wrote are shaded when that text is still in the file. The composer stays below. The inspector, opened on request, shows "Selected edit": the session, the edit, Reveal in thread, honest file history, and disclosures for path, context, tree and run details (`desktop/files/provenance.rs`).
- **05 · Evening.** The `evening` theme already uses this sheet's ink-blue palette. The new surfaces were checked in it.
- **08 · Starting.** A new session leads with "What should we change?" and the composer itself. Starting points fill the draft, never replacing or sending it. Pi's templates attach as a command, and the project and file-history state close the page (`desktop/landing.rs`).

## Interaction contract

- **Quiet shell:** keep the 36px title bar and its four controls. No new permanent dashboard, navigation rail or branding strip. Native window-control appearance is not being redesigned here.
- **Use the width:** shared 24px work gutters, full-width tools/diffs/composer, and a reading measure for prose only. A file rail is local to Changes, not a universal fourth column.
- **Details are requested:** closed by default, never opened because a run finishes or a window grows. On compact windows use an explicit overlay with Escape/close and focus restoration; that overlay is not illustrated in this sheet.
- **One control point:** working, waiting, finished and ready are explicit states near the input/action area. Stop remains independent of the draft. Queue follow-up is the less disruptive primary action during a run; Steer now remains explicit. Preserve drafts and queue state across views.
- **Inspecting is not applying:** selecting a file/model/command does not mutate anything. A revision is an explicit new prompt with visible file/line context. Changing the selected file must not silently retarget an existing draft's attachment.
- **Honest change review:** these are observed tool edits, not a complete Git working-tree diff. The edit is already on disk; there is no misleading “Accept” button. Unavailable snapshots stay visible, and restore must never be implied when unsupported.
- **Keyboard parity:** each mouse action needs a keyboard route, visible focus and predictable focus restoration. Platform shortcut labels should adapt. Background session requests/errors must remain discoverable even when their project is collapsed.

## Boundaries

Names, code, test output, timestamps, counts and extension questions are illustrative. “Checks passed” here is sample content, not a result of running the app or backend. The serif response headline is sample assistant-authored text, not an invented client-side summary. Test success must come from actual execution results; a finished run alone does not imply success.

The extension question is a proposed presentation of a correlated pending request, not a general security sandbox or a promise that other processes cannot run. Timeout, cancellation and stale-request handling still matter.

This is a direction study, not a complete specification for settings, resource trust, errors, long histories or every responsive state. No production code or existing review assets were changed to implement it.

## Reproduce

```sh
python3 design/workbench-vision/render.py
```

Requires Pillow and a local Chromium/headless shell (or set `CHROME`). Rendering uses local assets, makes no provider calls and installs nothing. The generator checks text bounds and SVG structure, renders all eight previews, and renders the actual font-embedded master for `overview.png`. [manifest.json](manifest.json) records dimensions and composer geometry. Native accessibility, keyboard behavior and high-DPI app rendering remain implementation-stage validation.
