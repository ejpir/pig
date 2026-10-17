# Design studies

## Historical scripted mini-app prototype

**[Browse the archived GPUI calculator prototype](scripted-calculator/README.md)**: JSON/Rhai sources, a standalone native runner and its [Moonstone](scripted-calculator/screens/02-moonstone.png) and [Evening](scripted-calculator/screens/01-evening.png) captures are retained as design history. The current native [`@Visualize`](../crates/pi_android/README.md#native-mini-apps-with-visualize) runtime uses JavaScript bundles instead.

## Latest: Android screens

**[Browse the twelve phone screens](android/index.html)**: Pi keeps running on your computer or server, and the phone follows sessions, answers questions, reviews changes and starts work. Connect, Sessions, Starting, Working, Waiting, Done, Review, Typing, Details, Notifications, Evening and Settings, in the Moonstone and Evening themes. See the [notes and phone contract](android/README.md) and the [overview](android/overview.png).

## Android: inline pages

**[Open the interactive inline-page study](android/inline-pages/index.html)**: HTML pages become usable parts of the reply. Try a real calculator, expand it without losing state, inspect its source, choose when to load a revision, and explore offline/unavailable states. See the [design contract](android/inline-pages/README.md) and [overview](android/inline-pages/overview.png). The browser study is separate from the native inline host documented in the design notes.

## Android: swarm communication

**[Open the interactive swarm study](android/swarm/index.html)**: isolated agents exchange findings, questions and immutable artifacts through a durable broker on the computer. Eight screens cover a live peer map, exchange receipts, agent details, decisions, a grouped 24-agent swarm, offline state, completion evidence and Evening. See the [architecture and interaction contract](android/swarm/README.md) and [overview](android/swarm/overview.png). Design only; no peer messaging runtime is implemented.

## Android: schedules

**[Open the interactive scheduling study](android/schedules/index.html)**: recurring Pi jobs on the computer, managed from the phone. Six screens cover schedules, creation, details, completed runs, offline state and Evening, with local confirmation and cron-preview flows. See the [design contract](android/schedules/README.md) and [overview](android/schedules/overview.png). This is a prototype, not an implemented scheduler.

## Android: subagents

**[Browse the seven subagent screens](android/subagents/index.html)**: work Pi hands off to scouts, planners, workers and reviewers, shown inside the session that started it, including a chain picked up after the computer restarts. See the [notes](android/subagents/README.md) and the [overview](android/subagents/overview.png).

## Visual workflow directions

**[Compare eight directions](visual-workflow/index.html)**: Run rail, Stage flow, Studio, Session map, Follow mode, Mission control, Signal and Timeline tracks. Each is limited to GPUI-paintable effects, with a capability table and a recommended path. See the [notes](visual-workflow/README.md).

## Thread result polish

**[Compare the three focused mocks](thread-polish-mocks/index.html)** — alternatives for changed-file scan distance, honest unknown counts, actionable issue status, command presentation, and idle-composer density.

- [Combined SVG](thread-polish-mocks.svg) · [Overview PNG](thread-polish-mocks/overview.png)
- [Decision notes and reproduction](thread-polish-mocks/README.md)

## Workbench vision

**[Open the eight-screen workbench SVG](desktop-workbench-vision.svg)** — a separate, free-rein direction: task-first navigation, first-class change review, optional details and explicit working/waiting states.

- [Browse the screens](workbench-vision/index.html) · [Overview](workbench-vision/overview.png)
- [Interaction notes and reproduction](workbench-vision/README.md)

## Streamline review

**[Open the complete 18-screen SVG](desktop-streamline-study.svg)**

The whole-app proposal includes Thread, Sessions, Models, Resources, Settings, Appearance, Changes, Tree, Context, search, New Session/worktree, idle and compact layouts, with light/dark examples.

- [Browse all proposed screens](visual-review-2026-10-01/all-screens.html)
- [Full-sheet PNG preview](visual-review-2026-10-01/overview.png)
- [Before/after comparisons](visual-review-2026-10-01/index.html)
- [Screenshot-based findings, dimensions and priorities](visual-review-2026-10-01/README.md)
- [26 actual Xvfb captures](visual-review-2026-10-01/captures/manifest.json)

The proposals are static design studies, not implemented UI. The actual captures use baseline `df8a946` and isolated offline data.

Regenerate the proposal SVGs, PNGs and full sheet:

```sh
python3 design/visual-review-2026-10-01/render.py
```

Earlier `desktop-*-study.svg` files remain historical studies; this review does not overwrite them.
