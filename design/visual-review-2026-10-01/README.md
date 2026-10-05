# A quieter Pi Desktop

**Visual review · 1 October 2026 · baseline `df8a946`**

## Recommendation

Refine the existing **warm, editorial workbench** rather than redesigning it as another chat dashboard. The paper-like light theme, ink-blue dark theme, serif titles and compact shell already give Pi a recognizable identity. The problem is not a lack of decoration: it is that small metadata, duplicated information and too many simultaneous columns compete with the work.

**Do first:** readable secondary text, a working compact inspector, independent search, and consistent control dimensions. Then simplify the sidebar and inspector. No new navigation layer, gradients, hero dashboards or branding in the title bar.

- **[Open the complete 18-screen SVG](../desktop-streamline-study.svg)** — the whole proposal in one zoomable, two-column study.
- **[Browse all 18 screens individually](all-screens.html)** — full-size SVGs and PNGs, with captions.
- **[Overview preview](overview.png)** — the full sheet at a smaller size.
- **[Open the visual comparison gallery](index.html)** — selected actual captures beside proposed designs.
- **[Measured current screen](measurements.png)** — dimensions and annotated trouble spots.
- **[Raw captures](captures/manifest.json)** — 26 real GPUI screenshots, not browser recreations.
- **[Measurements and contrast calculations](measurements.json)**.

The images in `concepts/` and `design/desktop-streamline-study.svg` are **proposals, not implemented UI**. The originals in `captures/` are unannotated app screenshots. No production code was changed.

The 18-screen vector sheet covers Thread (light/dark/wide), the idle state, global search (results/empty), New Session (folder/worktree), Sessions, Models (docked/compact drawer), Resources (packages/trust), Settings, Appearance, Changes, Tree and Context. It embeds actual vector layouts, **not screenshots wrapped in SVG**. Mockup content is illustrative and sometimes shortened; it is not evidence of successful backend actions.

## Implemented (4 October 2026)

The 18 screens now use the app's current theme: sentence-case headings, serif titles, soft tinted callouts, hairline rows and switches. Where this review conflicts with the later [workbench vision](../workbench-vision/README.md), such as one "Settings & tools" entry instead of four sidebar rows, the vision wins.

- **P0.** Narrow windows open requested details as a drawer (Escape closes it). Global search has keyboard selection, an empty state and key hints, and Ctrl/Cmd+K works from every view.
- **Shared.** `label()` and `section()` render readable sentence case. Each app view has its own search field, which never filters the sidebar.
- **Views.**
  - Thread inspector: context, cost and edits first; details behind disclosures; the no-snapshot warning stays visible.
  - Sessions: Resume, then Rename, Fork and More.
  - Models: compact provider filters.
  - Resources: one trust callout and sentence-case tabs.
  - Settings: inline descriptions, a thinking-level dropdown, Advanced, "Show JSON keys", and Appearance as theme previews.
  - Context: plain figures, narrow bars on a scale with a legend, and a switch for auto-compaction.
  - Changes: a snapshot callout.
  - New Session: "Thinking · inherited", and no disabled option.

`capture.py` predates "Settings & tools" and cannot reach the catalog views any more; use Ctrl+K to open them.

## Inspection scope

Captured with Xvfb, software Vulkan, Linux aarch64, at 1×:

- **1344×740:** running and idle Thread, light/dark, Changes, Tree, Context, search/results-empty, New Session/worktree, Sessions, Models, Resources and Settings.
- **1600×900:** transcript width and spacing.
- **1000×680:** automatic inspector hiding, explicit toggle attempts, Settings, Models and Resources.
- **960×600:** minimum-window dialog and worktree layout.

Used `target/check/debug/pi-desktop`, isolated temporary home/config/project folders, the offline demo and `fixtures/catalog-rpc.py`. No provider calls, package installation or real project edits. Also ran the existing first-screen and catalog capture checks successfully.

This is a visual/interaction sample, not a full accessibility audit. Native macOS/Windows rendering, Retina scaling, traffic lights, touch targets on those platforms, large datasets, terminal/editor detail and every error state still need separate checks. Fixture messages such as “not reported” are intentional missing data; the recommendation concerns their **presentation**, not inventing backend capabilities.

## Keep these strengths

- The **36px shared title bar**, its four icon controls, and absence of repeated view names.
- Warm light surfaces and a genuinely dark Evening theme; do not flatten both into generic gray.
- The serif voice for human-readable titles, paired with Plex Sans and monospace code/data.
- Contextual inspector content, neutral row selection and collapsible sidebar sections.
- Grouped tool calls, explicit queued follow-ups, distinct Steer/Follow-up/Stop actions.
- The branching Tree visualization. It is distinctive and useful; it does not need replacement with cards.
- The New Session dialog's `+` beside close. Improve its hit area and tooltip, not its visual footprint with another large button.

## Findings, in priority order

### 1. Compact mode must not remove actions · P0

**Observed:** at 1000px the inspector disappears. Clicking the top-right toggle off and on does not reveal it. Compare [wide Models](captures/16-models.png) with [compact Models](captures/24-models-compact.png), and [Settings after toggling](captures/23-settings-compact-after-toggle.png). “Use in this session”, selected-item details and reset/help controls are no longer visible.

This is supported by `Layout::show_inspector`: the 1150px gate remains in effect after `toggle_inspector` resets `inspector_requested`.

**Propose:** keep the automatic hiding, but make an explicit inspector request open a **320–360px overlay drawer**. Do not squeeze sidebar + table + inspector into 1000px. Pin important actions to its footer, let its body scroll, provide close/Escape, and restore focus to the originating row. Merely selecting a model must still never switch it.

Dock only when the remaining work area is at least about **720px**, calculated from the actual resized pane widths. With the proposed defaults, this is around 1256px total; a simple initial 1280px breakpoint is reasonable. Respect the user's explicit close until they request details again.

**Acceptance:** at 960×600 and 1000×680, all model/session/resource actions remain reachable without resizing the window. No accidental model changes or hidden confirmation dialogs.

[Proposed compact drawer](concepts/compact-models.png)

### 2. Minimal should not mean faint and tiny · P0

**Observed:** much of the navigation and supporting information is 9–11px; section labels use 10px spaced-out capitals. Some essential helper text uses `theme.faint`:

| Flat foreground/background | Current contrast | Suggested replacement | Contrast |
| --- | ---: | --- | ---: |
| Light caption `#8b847d` on panel `#f2efeb` | 3.22:1 | `#6b645d` | 5.08:1 |
| Dark caption `#737981` on panel `#1a212b` | 3.69:1 | `#959ca5` | 5.84:1 |
| Light amber warning `#b97a14` on white | 3.59:1 | `#8b631f` | 5.38:1 |

These are theme-color calculations, not antialiased pixel samples. They fall below the usual **4.5:1 normal-text target** in these combinations. Decorative separators can remain quiet; explanatory text should not have to be deciphered.

**Propose:** 13px navigation, 12px supporting text, 15px/24px transcript prose. Keep 11px only for nonessential short counters or timestamps. Use sentence-case labels; reserve tracked monospace capitals for the occasional small section marker, not the primary navigation. Keep actual code and identifiers monospaced.

The dark selected fill is still visibly blue (`#273748`), despite neutral borders. Try a neutral charcoal (`#2a3038`) and warm neutral light selection (`#e7e3de`). Keep blue for actions, links and meaningful data. A **keyboard focus outline must remain distinct** from persistent selection; neutral does not mean invisible.

**Acceptance:** meaningful text ≥4.5:1 on its actual surface; essential control boundaries/focus cues ≥3:1 where required. Check hover, selected, disabled and focused states independently.

### 3. Search needs a single, visible scope · P0

**Observed:** the [palette](captures/03-search.png) says “Search sessions” but lists app views and actions. Typing a [nonmatching query](captures/04-search-empty.png) leaves an empty “QUICK ACTIONS” heading and filters away the sidebar behind the overlay. The same search entity is consumed by other views too.

**Propose:**

- Rename the palette placeholder to **“Search sessions and actions…”**.
- Give the palette its **own query**; opening it should not silently filter the underlying view.
- Add a plain empty state: **“No matches for ‘…’”**, then “Try a session, project or action name.”
- Keep palette geometry stable enough that empty results do not look like a broken popup.
- Keep `Ctrl/Cmd+K` global. Catalog/Settings filtering belongs in a small local search field or expandable search affordance in that view's own toolbar, with a visible clear action.
- Support highlighted result, arrow keys, Enter, Escape and focus restoration. This is a proposed acceptance requirement, not a claim that a complete keyboard audit was performed.

No extra permanent search field in the title bar.

### 4. Establish one spacing and control vocabulary · P1

**Observed:** the broad layout is already sensible, but small inconsistencies accumulate. The transcript starts 24px inside its pane; the composer starts 20px inside. Controls mix 20, 22, 24, 26, 28 and 30px heights. Primary actions are only 24px tall. Settings and Resources begin their toolbars differently.

| Element | Current | Proposed starting point |
| --- | --- | --- |
| Shared title bar | 36px | **Keep 36px** |
| Session tabs | 40px | **Keep 40px**, 13px sentence-case labels |
| App-view toolbar | e.g. Settings 46px; Sessions 44px | **44px**, scope/filter/actions only |
| Sidebar / inspector | 208 / 328px | **216 / 320px**, still resizable; this is a minor refinement, not the main fix |
| Main content gutter | mixed 20 / 24px | **24px** for transcript, composer and view content |
| Inspector gutter | 20px | **Keep 20px** |
| Normal sidebar rows | 28–30px | **32px**; dense Settings category rail may remain 28px |
| Button / icon target | mostly 24px; dialog icons 20px | **30px buttons, ≥28px icon targets**, 14–16px glyphs |
| New Session width | 500px | **Keep 500px**; scroll body, fixed footer |
| Status bar | 24px | **Keep 24px**, less text, 11–12px |
| Corner radii | mostly 5–10px | **6px controls/rows, 8px input surfaces, 10px overlays** |

Use 4/8/12/16/24px spacing tokens instead of individually tuning every card. These are desktop mouse/keyboard targets, not a claim of touch-size compliance. Do not change the approved macOS traffic-light appearance based on Linux captures.

### 5. Use the work area; constrain prose, not the layout · P1

**Observed:** at [1600×900](captures/10-thread-wide.png), the middle pane is **1064px** wide. That width is useful for tools and editing; only prose becomes harder to read when it spans the entire pane.

**Revision after feedback:** the first proposal incorrectly capped the whole conversation and composer at 760px. That withheld 256px of usable work width, creating an extra 128px blank margin on each side. It also contradicted the full-width treatment of the other views. The default-size Thread happened to fit 760px already, masking the inconsistency.

**Propose one rule across the study:**

- Use the available work pane, with **24px outer gutters**. At the proposed pane sizes, the content width is **760px at 1344px** and **1016px at 1600px**. These are available widths, not a maximum-width setting.
- Keep **read-only prose paragraphs** left-aligned with a comfortable reading measure (roughly 65–85 characters where feasible, at most about 760px). Do not center the entire turn around that text.
- Let user-turn surfaces, tool rows/output, diffs and the **composer span the usable width**. Anchor composer actions to the same right gutter. Do not impose a prose cap on editable prompts, code or pasted snippets.
- Use the same fluid composer and gutters in the idle state. Tables, files and settings keep their existing fluid work areas; dialogs and explicitly requested compact drawers retain their bounded widths.
- Expanding an existing tool can use the extra space; resizing must **not** open it automatically. Do not add a panel or dashboard merely to fill space.

The [revised wide concept](concepts/thread-wide.png) shows a user-expanded Edit with an inline diff and a full-width composer. It is an illustrative expanded state, not a content-identical comparison to the raw capture. Short content may still leave whitespace; the layout should not reserve empty outer rails.

### 6. Reduce duplicate navigation and status · P1

**Observed:** [Thread](captures/01-thread-light.png) highlights “Qwen signatures” twice, under Active and again under its project. Its running state is repeated in the tool row, “Running tools…” line, inspector and footer, in addition to sidebar indicators. The [idle fixture](captures/14-idle-session.png) is also under “Active”, although it is not running.

**Propose:** rename **Active → Open**. When both sections are expanded, show each open session once in Open and closed/saved sessions under Projects. If Open is collapsed, include open sessions in their project list so they do not become difficult to find. Counts must say what they count; never silently reinterpret “total sessions” as “visible rows”. Preserve search across both groups.

Keep the detailed live activity beside the active tool and the relevant composer actions. Use compact sidebar indicators for background work. Collapse the footer to connection/process state plus a run-details affordance; keep important disconnected/error states prominent. Metrics remain available in Context/inspector, not removed.

### 7. The inspector should explain the selection, not repeat a dashboard · P1

**Observed:** the default Thread inspector shows long missing-history explanations before the useful context meter. Usage occupies six rows; a session file path and extension status continue below the visible area. [Changes](captures/05-changes.png) repeats general session usage below file actions. [Resources](captures/18-resources-skill.png) switches from large serif titles to a tiny resource heading, so its hierarchy changes abruptly.

**Propose:** one consistent inspector anatomy:

1. **Selected item title**; human name in the serif voice, exact identifier/path below in monospace. Allow a second title line before truncation; expose the full value on hover/copy.
2. **The two or three facts needed now**, e.g. context + cost + changed-file count, or model limits + price.
3. **Relevant actions**, with one filled primary action at most. Anchor critical catalog actions at the bottom.
4. **Disclosures for detailed usage, session file, extension status and technical explanation.** Persist disclosure state per view.

Missing data should be an honest compact value, not a paragraph in every section. Keep safety-critical information visible: “No snapshots; past edits cannot be restored” is important. Do not hide trust consequences or imply that an unavailable undo will work.

Use a switch, not a filled “Enabled · turn off” button for auto-compaction. Keep filled treatment for intentional actions such as Compact or Use model.

[Thread proposal, light](concepts/thread-light.png) · [dark](concepts/thread-dark.png)

### 8. Settings needs fewer simultaneous columns · P1/P2

**Observed:** [Settings](captures/19-settings.png) combines a 208px app sidebar, 180px category rail, 628px settings area and 328px inspector. A setting's explanation is far away, while its JSON key is always underneath its name. [Appearance](captures/20-settings-appearance.png) is an entire four-column screen for one short row.

**Propose:** keep the app sidebar and category rail, but make settings self-explanatory **inline**. Put a short description below the label; move JSON keys, provenance, allowed values and reset details behind a details disclosure/optional inspector. Keep source/scope and changed/inherited status visible when relevant. The user's file-vs-project distinction must remain explicit.

Use a dropdown for seven thinking levels instead of squeezing them all into a segmented control. Keep the common settings first; group model-specific overrides and budgets under Advanced. Search must still find and reveal advanced matches. The existing inspector toggle can open technical details when wanted; it should not be permanently required to understand an ordinary setting.

The [Settings concept](concepts/settings.png) changes presentation, not available settings. Do not add controls for features the backend does not support.

## Smaller view-specific refinements · P2

| View | Evidence | Suggested refinement |
| --- | --- | --- |
| Sessions | [15](captures/15-sessions.png) | Replace the inspector's grid of eight competing buttons with Resume, then Rename/Fork and a More menu for export/share/delete. Keep destructive actions separated and confirmed. Preserve row names and dates; use less zebra striping. |
| Models | [16](captures/16-models.png), [24](captures/24-models-compact.png) | Keep a table, not model cards. At narrow widths retain model/provider, context and a details affordance; move secondary columns into details. Collapse provider summaries into a compact filter when they consume too much vertical space. Distinguish selected, current-session and saved-default states. |
| Resources | [17](captures/17-resources-project.png), [18](captures/18-resources-skill.png) | One scope toolbar, one tab row, then content. Remove the empty 100px settings-preview box when there is no reported data. Keep one visible trust status and its explanation; avoid repeating two equal-weight Trust actions. Never equate trust with sandboxing. |
| Changes | [05](captures/05-changes.png) | The inner file rail is about 240px, leaving only about 568px for the diff at the default size. Try 200px/collapsible files and conditionally overlay details. Put restore/undo actions in one consistent place rather than both bracket links and inspector buttons. Keep unavailable-history explanation close to those actions. |
| Context | [07](captures/07-context.png) | Three samples become three very wide slabs. Cap sparse bar widths around 24–32px; add a clear color legend and useful scale. Use a compact empty state when there is no usage. Do not invent request-level metrics from session totals. |
| Tree | [06](captures/06-tree.png) | Preserve the branch geometry. Apply the common text/control scale, make the selected-entry title wrap, and keep branch-changing actions clearly distinct from inspecting a row. |
| New Session | [08](captures/08-new-session.png), [13b](captures/13b-new-worktree-minimum.png) | The 500px dialog already works. Increase invisible hit areas around `+` and close, clarify “Saved” as an inherited thinking choice, and remove the disabled automatic-worktree-removal option from the default form until usable. Retain the folder-overlap warning with readable contrast. Keep branch/path fields conditional and the footer reachable at 960×600. |
| Idle Thread | [14](captures/14-idle-session.png) | Keep the small Pi mark here, not in the title bar. Lead with “What are we working on?” and the composer, then useful prompt/skill starters. Collapse the four metadata cards into “Available in this project”; do not lead an empty conversation with “Manifest not exposed by RPC”. |

## Suggested implementation order

### Pass 1 — immediate consistency, no new information architecture

1. Separate readable secondary text from decorative faint color; normalize control targets and gutters.
2. Fix explicit inspector opening below the breakpoint; preserve action access and focus.
3. Split global/local queries and add a palette empty state.
4. Neutralize dark selection; preserve visible keyboard focus.

**Likely files:** [`theme.rs`](../../crates/pi_desktop/src/theme.rs), [`components.rs`](../../crates/pi_desktop/src/components.rs), [`desktop.rs`](../../crates/pi_desktop/src/desktop.rs), [`chrome.rs`](../../crates/pi_desktop/src/desktop/chrome.rs).

### Pass 2 — less repetition, stronger reading hierarchy

1. Share fluid work-area gutters; constrain read-only prose independently of tools and the composer.
2. Deduplicate visible sidebar sessions and rename Open.
3. Reorder inspector essentials; disclose technical detail without hiding safety information.
4. Apply the same title/action anatomy across catalogs.

**Likely files:** [`transcript.rs`](../../crates/pi_desktop/src/desktop/transcript.rs), [`composer.rs`](../../crates/pi_desktop/src/desktop/composer.rs), [`sidebar.rs`](../../crates/pi_desktop/src/desktop/sidebar.rs), [`inspector.rs`](../../crates/pi_desktop/src/desktop/inspector.rs), [`app_views/`](../../crates/pi_desktop/src/desktop/app_views).

### Pass 3 — view-specific polish

Inline Settings descriptions, catalog action grouping, sparse Context charts, smaller Changes file rail and the idle-state cleanup. Do these after the shared primitives, not as independent redesigns.

**Validation gate for each pass:** repeat light/dark captures at 1344×740, 1000×680, 960×600 and 1600×900; test the 1150/1280 transition region, long names, 200% text scaling, hover/focus/selected states and keyboard-only operation. Ensure no model/package/session mutation happens from mere inspection. Check native macOS/Windows before changing window chrome. Preserve auto-hiding scrollbars, discoverability on hover/scroll, and sensible minimum scroll-thumb sizes.

## Reproduce

From the repository root, with a current native build:

```sh
xvfb-run -a -s '-screen 0 1920x1080x24' \
  python3 design/visual-review-2026-10-01/capture.py \
  --binary target/check/debug/pi-desktop

python3 design/visual-review-2026-10-01/render.py
```

Capture dependencies: Xvfb, xdotool, ImageMagick `import`, Tesseract, Pillow. Rendering uses the installed Chromium headless shell (or `CHROME`), local fonts and the existing licensed app icons. No CDN, provider or hosted design service. Capture filenames/manifest record the tested states; exact antialiasing, spinner frames and temporary paths can vary.
