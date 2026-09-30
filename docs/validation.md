# Validation

Validation runs use the Linux aarch64 sandbox, rustc 1.98.1, and a separate `CARGO_TARGET_DIR=target/linux` so host-native build artifacts are not overwritten.

## Built-in backend

Local Linux aarch64 evidence, Bun 1.4.2: the standalone backend (`build-binary.mjs`) passes the backend suite **31/31** with `PI_DESKTOP_BACKEND_BINARY` and answers its first command in about 0.2 s (Node: about 8 s on this shared mount). Its archive is 35 MB. A debug `pi-desktop --features bundled-backend`, started under Xvfb with an isolated HOME/cache and no `node`, `npm` or `pi` on PATH, unpacked it in 0.61 s and ran its session on it (`--mode rpc` with the LSP extension; tools, session file and extension commands reported). **114 desktop tests** pass, including the unpack tests; desktop Clippy passes with and without the feature. **10 packaging tests** pass, including the backend notices and licenses. `package_desktop.py --notices` was run on that debug binary; the release wrappers and the hosted CI jobs were not run (a full release build of the Zed crates is too slow here). Developer ID signing of the backend, macOS and Windows are **unverified**. The "Node/Pi are not bundled" statement in the snapshot below predates this change.

## Release pipeline (current pipeline snapshot)

The pipeline targets **Linux amd64/arm64, macOS arm64 and Windows amd64**, with native hosted runners and the pinned Zed checkout. Pushes/PRs upload separately named distributable archives; version tags publish all four plus `SHA256SUMS` only after checks and all native builds succeed. An incomplete matrix, unexpected release files and overwriting published assets are refused. Validation logs/screenshots are separate from distributable packages.

Local Linux aarch64 evidence: **197 workspace tests pass, 0 failures, 1 opt-in ignored**, including **112 desktop tests** (`pipeline-workspace-tests.{log,exit}`); the separate fake-LSP suite passes **5 tests**; workspace and fake-LSP all-target Clippy pass with warnings denied. The backend passes **31/31** (`pipeline-backend-tests.{log,exit}`). **9 offline packaging/release tests** pass (`packaging-tests.{log,exit}`), including target-header validation, legal files, isolated staging, exact-matrix/checksum enforcement and rejected unsafe tags. macOS signing/archiving commands in these tests are **mocked**, not native signing evidence. Workflow `actionlint`, own-package format and shell-syntax checks pass. A cold SDK bootstrap exposed the old five-second test-event budget; it now uses the same bounded 25-second budget as test requests.

Hosted macOS/Windows builds, real release upload, Developer ID/notarization and Windows signing have **not run here**. macOS archives use an ad-hoc signature; Windows archives are unsigned. Node/Pi are not bundled, and launch defaults are unchanged. Generated previews, build/dist output, diagnostics and scratch files are excluded from the source commit; earlier tracked study reference images remain intentional design assets.

## Projects, shell, notifications and directory mentions (previous snapshot)

**176 workspace tests pass, 0 failures, 1 opt-in skipped**, including **102 desktop tests** (`artifacts/projects-shell-workspace-tests.{log,exit}`). The published-SDK backend passes **30/30 Node tests** (`projects-shell-backend-tests.{log,exit}`). Desktop all-target warnings-denied Clippy, scoped formatting, native build, backend syntax and whitespace checks pass. Historical counts below describe their own earlier snapshots.

- Native `capture-projects-shell.sh` now completes successfully with isolated HOME/XDG/agent/project fixtures and the published, unpatched Pi 0.87.1 SDK. Captures cover the study chooser/floating initial-model picker in both themes, Project Resources wide/narrow/project disclosure, User Extensions/Skills without repeated settings JSON, real directory completion/chip selection, small shell copy icons, the owned-PID process popup, FIFO confirm/select decisions and backend timeout cancellation. Choosing models/previewing worktrees does not create a session or mutate the existing model.
- The shell probe explicitly executes one harmless local `!!printf`, verifies excluded-context submission and SDK-reported resource/load-error metadata, and checks correlated cancellation/selection IDs and exact `Block` response. The only prompt commands are guarded local extension fixtures, not model prompts. No agent/tool-execution events occur. `projects-shell-native-protocol.json` records the allow-listed proof; clean native exit passes. Worktree creation itself is covered by isolated Git tests, not this preview-only native capture.
- Notification regression verifies the shared floating card has the same geometry in session/catalog views and does not reflow the composer; both dismiss paths clear the owning notice. Native snapshots show the card in the established palette. Directory regressions verify filtering, Enter/Tab, no file preview/content attachment and sent path references. Matching now uses Zed's `fuzzy` crate (`match_path_sets` / `match_strings`) on cancellable background tasks, with tests for Unicode highlights, scattered matches, all metadata groups, bounded results and stale/dismissed queries.
- The enhanced `capture-status-tools.sh` also completes successfully. `status-tools-tool-tooltip.png` shows original SDK description/source details only on hover; wide/narrow inspectors show tool names only. `status-tools-tooltip.png` verifies full TPS data in its tooltip, with no transcript banner. These screenshot/log files now describe the current snapshot, while the older named workspace/backend test logs below retain their historical counts.
- Native SDK process checks cover opening, Escape/click-away and return to the composer. `capture-process-scrollbar.sh` additionally passes native many-row overflow, visible-thumb, track-click and drag checks with unchanged header/footer (`process-scrollbar-{top,bottom,drag-top}.png`, `process-scrollbar-validation.json`). This geometry probe uses explicitly labelled demo rows with **no Pi subprocesses or fabricated PIDs**; live multi-process selection/load is not claimed. Automated tests cover many-session scrolling and selection/ownership. Real provider streaming/authentication, remote package operations/sharing uploads, native macOS/Windows and broader live-project behavior remain unverified.

```sh
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-projects-shell.sh
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-status-tools.sh
```

## Quiet run statistics and live tool inventory (previous milestone)

This change passes **157 workspace tests, 0 failures, 1 opt-in skipped**, including **86 desktop tests** (`artifacts/status-tools-workspace-tests.log`), and **27 backend Node tests** (`status-tools-backend-tests.log`). Desktop warnings-denied Clippy, affected-file formatting and native build pass (`status-tools-{clippy,build}.log`). The initial package-only test attempt encountered a cached `svgtypes` link-metadata mismatch; the full-workspace run passed without source or dependency workarounds.

- The project TPS extension's info notification is kept verbatim in the 24px bottom status bar with a full-data hover tooltip, rather than an amber conversation banner. Arbitrary info, warnings/errors and multiline notices are not swallowed. Metrics clear on the next agent start and stay session-local.
- The SDK backend adds optional `activeTools` metadata to `get_state`, using the public `getActiveToolNames()`/`getAllTools()` APIs. Names appear in the inspector before and after the first prompt; descriptions/reported source metadata are now hover-only. Unknown inventory remains distinct from a reported empty loadout; no tool calls or filenames are used to infer it. Runtime loadout changes and reload are tested without executing a tool/model.
- `capture-status-tools.sh` uses the native rebuilt app and published SDK with synthetic notifications and guarded extension metadata. Wide/narrow screenshots show the quiet footer and real SDK-reported tool list (`status-tools-{wide,narrow,tooltip}.png`); protocol logging verifies metadata-only commands and no agent/tool-execution events. `capture-backend.sh` also passes with the rebuilt app (`status-tools-backend-capture.log`). Native macOS/Windows remain unverified.

```sh
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-status-tools.sh
```

## Standalone Pi SDK backend (previous trial milestone)

`packages/pi-desktop-backend` passes **26 Node tests** against the published, shrinkwrapped Pi **0.87.1** npm package (`artifacts/backend-tests.log`). Syntax checks and the distributable-package dry run pass (`backend-check.log`, `backend-pack.json`). No sibling Pi source was changed for the extraction. No Rust source was changed or workspace suite rerun for this backend-only work; Rust counts below describe earlier milestones, not a new combined result.

- Actual backend subprocess tests cover metadata/bootstrap, settings/model scope/thinking persistence, handled-command acknowledgements, extension dialogs and cancellation, trust save/restart isolation, local package install/remove with explicit reload, Unicode history and session replacement. Test environments allow-list process plumbing rather than inherit provider credentials/config pointers.
- Protocol tests cover LF/UTF-8 framing, record/output bounds, mutation gates with responsive metadata/dialog replies, and the released SDK's boolean-preflight compatibility shim. A contract check requires handlers for every command currently emitted by the Rust desktop.
- Sharing tests reuse the published Pi private exporter and verify preserved `pi.share` branch metadata. Radius and private-gist upload paths, errors, cancellation and temporary-file cleanup are tested with synthetic data and mocked network/`gh`; **no real conversation was uploaded**. Real provider streaming, cloud authentication and remote package operations remain unverified.
- `scripts/capture-backend.sh` passes using the actual native desktop and SDK backend, not a fake RPC peer (`backend-native-capture.log`). `backend-native-{thread,models,resources,sessions}.png` capture saved history and real SDK-derived fixture metadata; Models/Resources were inspected. The recorded command-name allow-list (`backend-native-commands.json`) contains metadata reads only, with no model prompt or displayed-tool execution. The existing native binary was reused; this validates backend interoperability, not a new Rust build.
- The older `capture-live-startup.sh` also reached Ready with the saved history through this backend, but failed its stale message-count OCR crop (which now contains Cost). It is **not** counted as a passing probe; the dedicated backend capture above checks current UI and clean exit.
- Native macOS/Windows, real sharing uploads, default-launch changes and installer bundling are still pending. The source launcher remains unchanged as a fallback.

```sh
npm ci --prefix packages/pi-desktop-backend
npm test --prefix packages/pi-desktop-backend
npm run check --prefix packages/pi-desktop-backend
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-backend.sh
```

## Multiline composer scrollbar (previous Rust milestone)

This Linux build passed **152 workspace tests, 0 failures, 1 opt-in skipped**, including **84 desktop tests** (`artifacts/input-scrollbar-workspace-tests.log`). Desktop all-target Clippy with warnings denied, input formatting, and the native build pass (`input-scrollbar-{clippy,build}.log`). An intermediate desktop run timed out in the terminal typed-command test; the final full-workspace run passed, without changing that test.

- Multiline inputs expose the shared native scrollbar when they overflow, in both capped and expanded mode. Tests cover wrapped text, track clicks, dragging outside the input, wheel scrolling, unchanged text/selection, shrinking to non-overflow content, and caret following across resizing.
- `capture-composer-scrollbar.sh` pastes a 40-line draft into the actual light-theme app using an isolated metadata-only peer. Pixel checks require a visible idle thumb; native drag, track-click and wheel checks reach the expected lines. Expanding preserves a manually scrolled position, and real clipboard copy equals the original draft. No prompt or tool call is submitted.
- Inspected native screenshots: `input-scrollbar-{bottom,top,expanded}.png`; other evidence includes `input-scrollbar-{drag-top,track-bottom,empty}.png`, `input-scrollbar-copy.txt`, and `input-scrollbar-capture.log`. Native macOS/Windows remain unverified.

```sh
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-composer-scrollbar.sh
```

## Failed activity collapse and outcome markers (previous milestone)

This milestone passed **148 workspace tests, 0 failures, 1 opt-in skipped**, including **80 desktop tests** (`artifacts/activity-fix-workspace-tests.log`). Desktop all-target Clippy with warnings denied, package formatting, and the native build pass (`activity-fix-{clippy,build}.log`).

- Settled activity now collapses even with failed tool calls. Live/unfinished groups stay open by default; manual expansion survives settlement. A green check marks all-successful completion, while a coral warning and failure count remain visible for failures. Unit/GPUI tests cover these states, reopening original error output, and session-local disclosure.
- `scripts/capture-activity-status.sh` uses an isolated metadata-only peer to load successful and failed historical turns into the real app. Native dark screenshots `activity-{passed,failed}-{collapsed,expanded}.png` verify collapsed headers and reopening original calls. The peer never executes the displayed commands or invokes a model.
- The First Message inspector card no longer has a blue left border (`catalog-sessions.png`). The full catalog probe and existing chat readability/animated-status probe were rerun successfully (`activity-{card,readability}-capture.log`).

```sh
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-activity-status.sh
```

## Models, Resources and app-view refinements (previous milestone)

This milestone passed **146 workspace tests, 0 failures, 1 opt-in skipped**, including **78 desktop tests** (`artifacts/catalog-workspace-tests.log`). Desktop all-target Clippy with warnings denied and the native build pass (`catalog-clippy.log`, `catalog-build.log`). Core Clippy separately exposed an existing `collapsible_if` warning in `history.rs`; it is not included in the desktop-only lint result.

`capture-catalogs.sh` launches the actual GPUI app with an isolated HOME/project and `fixtures/catalog-rpc.py`. The peer provides synthetic metadata only, never invokes a model or installer, and logs commands for allow-list assertions. Native checks cover model selection/detail updates, session-only model switching, search, resource tabs/raw clipboard details, confirmation cancellation (including background-click/global-search isolation), and a narrow window. `catalog-{models,model-selected,model-used,resources,skills,resources-narrow}.png` are native captures, not the study. Dark Models/Resources were also captured and inspected (`catalog-{models,resources}-dark.png`).

The follow-up screenshot regressions are covered by GPUI tests and inspected native images:

- Multiline session previews stay inside 34px rows without changing saved messages; the sort popup is deferred above the table (`catalog-sessions{,-sort,-sorted}.png`).
- Model price headings and values share right-aligned column bounds. Cycle membership is a 30×18 switch, not an On/Off text button.
- Session notices appear as dismissible overlays without moving rows (`catalog-{notice,notice-dismissed}.png`). Successful resource reload replaces the old reload-needed notice.
- Saved and effective trust are compared. A fresh process reporting both as trusted shows **active in this session**, not a restart warning (`catalog-trust-active.png`). GPUI tests also cover a saved/current mismatch. This verifies rendering from reported state, not real trust-file mutation.
- Catalogs have their own focus target; diagnostics can be opened from them. Drafts, model selection on inspection, and session isolation are tested.

```sh
PI_CATALOG_NOTICE=1 xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-catalogs.sh
PI_CATALOG_TRUSTED=1 xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-catalogs.sh
PI_CATALOG_DARK=1 PI_CATALOG_NOTICE=1 xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-catalogs.sh
```

Provider login hands off to the interactive Pi terminal flow. Real authentication, network package installation/update/removal, and native macOS/Windows execution remain unverified. Command-discoverable resources are not a complete loaded manifest; context-file inventories and unsupported per-model thinking levels remain explicitly unavailable. Older Pi versions may lack the newer catalog/mutation RPC commands.

## Full-width chat, diff wrapping, diagnostics and startup metadata (previous milestone)

This milestone passed **117 workspace tests, 0 failures, 1 opt-in skipped**, including **61 desktop tests** (`artifacts/readability-workspace-tests.log`). Targeted all-target Clippy with warnings denied, package formatting, and the native build pass (`readability-clippy.log`, `readability-build.log`).

- `capture-chat-readability.sh`: actual native completed activity folds/expands/re-folds, preserves the answer, and shows one transcript progress indicator. Two captured inspector frames verify the running mark animates. Captures at 1344/1600/1000px show full-width chat and contained Context tile text (`readability-{collapsed,expanded,wide,context,context-narrow}.png`). GPUI tests additionally check width growth, tile bounds, failure visibility, session-local disclosure and manual expansion surviving settlement.
- `capture-changes-wrapping.sh`: a metadata-only peer supplies a synthetic historical write with a long line and Unicode call ID. The line end is visible with default wrapping and absent with wrapping off; raw Copy diff retains the entire original line and ID. Native images are `wrapping-{on,off}.png`. GPUI tests check the Changed by row's bounds and wrapped document height. No project file is written and no prompt is sent.
- `capture-session-diagnostics.sh`: a deliberately failing subprocess supplies a fixture Node-style stack. The native banner, Ctrl+Shift+D console and real clipboard retain exit status and stderr (`session-diagnostics-{banner,console}.png`, `session-diagnostics-copy.txt`). Newest entries appear first, so the failure is visible immediately. GPUI tests cover banner dismissal without log loss, credential-like-line redaction and bounded retention. **The fixture error is not a diagnosis of the user's original crash.**
- `capture-files-shortcut.sh` now verifies that the reported command catalog appears at startup without opening the slash menu, in addition to its existing jj/Files/narrow-window checks.
- Native edit/bash drag selection, keyboard/context-menu copy and file-action regressions pass (`readability-tool-selection.log`, `readability-file-actions.log`). Tool headers are located from rendered text rather than stale pre-grouping coordinates.

All native probes use offline data or metadata-only peers; none submit prompts/model calls. Diagnostics retain at most 128 entries / 48 KiB and the captured exit stderr tail (up to 8 KiB), not complete process output. Review captured errors/stderr before sharing. Logs and jj turn-to-snapshot mappings remain in memory: **closing/reopening a session still loses its jj mappings**, although jj changes remain. That persistence gap is not fixed by these UI changes. Native macOS/Windows remain unverified.

```sh
xvfb-run -a -s '-screen 0 1700x1000x24' bash scripts/capture-chat-readability.sh
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-changes-wrapping.sh
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-session-diagnostics.sh
```

## Files shortcut and inspector clarity (previous milestone)

The Files icon beside Terminal opens the active session's existing file browser, including when the inspector was hidden or the window is narrow. Inspector Overview/Tree/Files controls now share tab styling and alignment. The project history status uses a compact `jj enabled` indicator and a plain-language explanation. Unrecorded Changes now says **Tool-reported edits · No snapshot**, with one explanation above the patch rather than repeated “incomplete” labels.

- **52 desktop tests pass**, including Files navigation without draft loss and aligned inspector tabs (`files-shortcut-tests.log`). Targeted all-target warnings-denied Clippy and the native build pass (`files-shortcut-{clippy,build}.log`).
- `scripts/capture-files-shortcut.sh` uses a disposable git repository, opts only that repository into jj, verifies its status, opens Files from a hidden inspector, resizes to 1000px, and checks that the test file is unchanged and no model calls were sent. Native screenshots are `files-shortcut-{enabled,browser,narrow}.png`; `files-shortcut-capture.log` records the pass.
- `clarity-changes.png` captures the revised legacy-history labels in the actual native app.
- `design/chat-readability-study.{svg,png}` is the earlier **visual proposal**, not a native screenshot. Its proposed width cap was rejected; the current implementation above uses the available width.

## Landing, quiet thread, per-turn Changes and session closing (previous milestone)

This milestone passed **106 workspace tests, 1 opt-in test skipped** (`artifacts/layout-final-tests.log`), including 50 desktop tests. Targeted all-target Clippy with warnings denied and formatting checks pass (`layout-final-clippy.log`, `layout-final-fmt.log`). Native linking initially hit an open-file limit; retrying with a 65,536-file limit and Rust's bundled lld succeeded (`layout-build-retry.log`). These workspace totals also include concurrently developed terminal tests; the probes below specifically exercise layout, selection, file actions and session lifecycle.

Actual GPUI windows were captured and inspected under Xvfb:

- `capture-workspace-layout.sh`: pre-session landing/inspector, two-turn quiet Thread, per-turn Changes with old/new positions, narrow landing, and active/project-session-row close hover. `layout-close-time-hover.png` shows the close button replacing—not overlapping—the “now” timestamp. Raw **Copy diff** is checked through the real X11 clipboard and excludes display numbers.
- `compare-workspace-study.py`: side-by-side native/study content and inspector comparison (`layout-study-comparison.png`). The landing mark matches within 1px; the Changes divider is x=447 versus x=448 in the study. A specific collapsed-tool border/background probe passes. These are geometry checks, **not full pixel equality**: fixture contents, fonts, honest unavailable fields and the requested usage statistics differ.
- `capture-session-close.sh`: draft-confirmation Escape cancellation, owned RPC process cleanup, closing the final session without creating another process, retained/changed project selection determining New Session's working directory, and confirmed project removal. It checks that the confirmation actually disappears and the removed project leaves the sidebar. Project files and saved history remain intact; the metadata-only peer rejects model/tool/prompt requests.
- `capture-tool-selection.sh`: actual edit/bash mouse-drag selection, keyboard clipboard copy and context-menu copy after the compact row changes. It checks single-line content against a fresh clipboard sentinel, not stale selection data.
- `capture-file-actions.sh`: creation, no overwrite, containment, rename, dirty-buffer guard, save, confirmation cancellation and system Trash regression all pass against temporary files, with no model calls.

```sh
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-workspace-layout.sh
python3 scripts/compare-workspace-study.py
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-session-close.sh
CARGO_TARGET_DIR=target/linux xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-tool-selection.sh
```

Capture logs: `layout-capture.log`, `layout-comparison.log`, `close-capture.log`, `layout-tool-selection.log`, and `layout-file-actions.log` under `artifacts/`. Diff display uses retained Zed Markdown/code surfaces with calculated old/new columns, not a live-Git ProjectDiff or editable buffer. Earlier unassociated tool edits have explicitly limited coverage; they cannot authorize undo. Restore File remains unavailable. Project/saved-session hiding is currently in-memory only. Native macOS/Windows execution remains unverified. Historical checks below are retained as earlier milestones, not substitutes for current evidence.

## File-browser and diagnostic regressions (previous milestone)

The native Linux app was exercised with real temporary files, isolated HOME/XDG directories and a metadata-only RPC peer (`fixtures/files-rpc.py`). The peer rejects prompts/tools/model calls. `scripts/capture-file-actions.sh` verifies:

- `--printenv` emits parseable JSON without initializing a window or Pi process; the shell probe no longer reports an unknown option.
- New File/New Folder icons, context-menu rename, and watcher-driven tree/tab updates.
- Existing files are not truncated; `../` names are rejected.
- Unsaved buffers block deletion; save writes the expected text.
- The deletion confirmation is visibly rendered, Escape cancels it, and a subsequent confirmation moves the directory and contents into the isolated system Trash.
- Clean exit, no forbidden RPC commands, and preservation of an unrelated existing file.

`scripts/capture-diagnostic-hover.sh` starts a **local deterministic LSP process** (one diagnostic, hover text and one quick fix), opens a real YAML file, grants project trust explicitly, receives `publishDiagnostics`, and hovers the underlined text. OCR checks the problem card: the server's message with its source and code, the hover text, "Quick fix: Close the sequence" and "Ask pi to fix". Clicking the quick fix must send the fixed text to the server; clicking **Ask pi to fix** must send exactly one prompt, naming `broken.yaml:1:1` and the message, to the metadata-only fake pi (which refuses it, so no model is involved). It also checks there is no `no rendered diagnostic` error. This verifies the embedded editor's language-server integration, not every production language server or npm mirror. npm is forced offline for this probe; background formatter/cache probes may log expected offline/cache-miss errors, but no packages are downloaded and no model calls are made.

`scripts/capture-mentions.sh` drives the `@` menu in the real window. A temporary project has two TypeScript files and the YAML file above; the local language server also answers a workspace-symbol query, the fake pi reports one saved session, and a shell prints a line. OCR checks the menu for `@` (Files, Sessions, Terminal, Problems), `@br` (the server's symbol) and `@op` (the file preview with its size and language). ⏎ must insert a chip, the inspector must list it under IN THIS PROMPT, and sending must give the fake pi exactly `Compare @packages/ai/src/providers/<file>.ts with the tests`. A second, offline demo run sends a message from a new session and checks that it shows the chip.

```sh
CARGO_TARGET_DIR=target/linux xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-file-actions.sh
CARGO_TARGET_DIR=target/linux xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-diagnostic-hover.sh
CARGO_TARGET_DIR=target/linux xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-mentions.sh
```

Artifacts: `file-actions-{browser,created,duplicate,containment,menu,renamed,nested,unsaved-guard,delete-confirm,deleted}.png`, `diagnostic-hover.png`, and `{file-actions,diagnostic}-{app,capture}.log` under `artifacts/`. macOS/Windows native verification is still outstanding. Formatting and targeted all-target Clippy with warnings denied pass (`files-fmt.log`, `files-clippy.log`). The fresh targeted test run passes **37 desktop tests and 1 editor-adapter test** (`files-tests.log`), including both Tree regressions. The earlier synthetic-font width assertion was replaced by a connected-segment assertion; actual font geometry is checked in the Xvfb comparison.

## New views and Tree visual correction (in progress)

The first combined native build with Tree/Changes/Context and embedded Zed file tabs passed. Core tests: **35 passed, 1 opt-in test ignored** (`artifacts/views-core-tests.log`). These checks do not yet certify the complete new feature set or live file/LSP workflows.

The actual GPUI app was launched under Xvfb, not a browser/mockup. `artifacts/views-before-tree.png` exposed disconnected glyph rails, incorrect text-column placement and an oversized inspector. The corrected `artifacts/views-tree.png` was inspected against the supplied Tree study. It uses vector nodes/connected parent edges, 36px rows, the study's text column, a segmented filter, compact quote/actions and an inline label editor. `tree-study-comparison.png` places the supplied rendered SVG crop beside the actual window; the current-path rail is at **x=251** in both images, with **100% inter-node continuity** in the sampled span. This is a specific geometry check, not a claim of total pixel equality. Sample data, unavailable RPC fields, platform fonts, and the intentionally unchanged newer sidebar differ.

```sh
CARGO_TARGET_DIR=target/linux PI_CAPTURE_FILES=1 \
  xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-views.sh
python3 scripts/compare-tree-study.py
```

The script captures Tree, keyboard navigation, No-tools filtering, Context, Changes and optionally a read-only offline editor preview, then checks clean exit. It sends no prompts/model requests. Native macOS/Windows remain unverified. New UI regression tests and broader validation are still being completed; older results below predate these features.

## Ownership refactor and tool details (previous milestone)

These historical checks cover the component/controller split, targeted notifications, virtualized lists, and collapsed/selectable tool details. Final run results and logs are recorded below; configuring CI is not evidence that a platform passed.

| Check | Result |
| --- | --- |
| Formatting | Passed (`artifacts/refactor-fmt.log`) |
| Clippy, all targets, warnings denied | Passed (`artifacts/refactor-clippy.log`) |
| Workspace tests | 65 passed; 1 opt-in integration test skipped (`artifacts/refactor-tests.log`) |
| Native Linux build | Passed (`artifacts/refactor-build.log`) |
| Native Xvfb screenshots/interactions | Eleven-frame palette/OCR/interaction check and SVG surface/composer/queue comparison passed |
| Native tool clipboard check | Edit/bash drag selection, keyboard copy, context-menu copy, and clean exit passed (`artifacts/tool-selection-check.log`) |
| Real Pi metadata handshake and isolated native resume | Both passed; `hello` hydrated with two owned processes and clean exit, no prompts/model calls |
| Native macOS/Windows rendering and packaging | **Not run here**; native CI jobs are configured |

### Regression coverage

- Each session retains its own draft, attached command, editor expansion, and transcript state. Switching while a slash-button draft is stashed cannot restore it into another session.
- A rejected background submission recovers to its originating composer without overwriting that session's newer text or another session's draft.
- Bootstrap failure blocks submission, and an immutable requested session ID is checked when resuming. Unnamed saved sessions retain their preview/first-message title.
- A 1,000-model picker keeps fixed row heights and constructs only visible choices. A 500-command slash menu scrolls keyboard selection into view without constructing all rows.
- A 1,000-message history creates documents only for visible rows and overdraw. Metadata-only updates do not resynchronize them.
- Stable-bounds typing, filtering, and picker hover do not rebuild the transcript. Background text deltas neither render the active transcript nor parse the inactive transcript until it is shown.
- Streaming keeps Markdown entity identities; scrolling away from the live edge preserves the reading position.
- Tool details start collapsed. Expanded edit/write/bash details support mouse-drag selection, keyboard copy, and right-click copy. Code containing Markdown fences remains literal, and copying the whole code source excludes the display wrapper.
- Existing core framing/correlation/deadline/process cleanup, Unicode/IME input, title, skill, Markdown language, sidebar, resize, queue/stop, and recovery checks remain covered.

Render counters verify scope/bounded work, not a hardware frame-rate claim. GPUI focus changes, changed bounds, themes, and running animations can legitimately render additional frames. A release build is recommended for interactive performance checks.

### Native validation

From the repository root, after building:

```sh
CARGO_TARGET_DIR=target/linux xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-first-screen.sh
python3 scripts/validate-screens.py
python3 scripts/compare-thread-study.py
CARGO_TARGET_DIR=target/linux xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-tool-selection.sh
```

The capture script produces eleven actual GPUI window frames, including both themes, collapsed tools, expanded edit/bash details, model/thinking/command menus, queue recovery, compact layout, and a new session. The palette/content check uses pixels and OCR; the comparison reads the supplied SVG and checks surfaces plus composer/queue geometry within 3px. It deliberately does not require the newer bordered messages or collapsed/selectable tool details to match the older study's static diff rows.

The separate selection probe checks the real X11 clipboard, not just selection state in a test dispatcher. It requires the real `/usr/bin/xclip` (`PI_DESKTOP_XCLIP` overrides the path); a sandbox host-clipboard stub earlier on PATH cannot validate an Xvfb clipboard.

For an **opt-in real RPC** check, use official Node 24+, the sibling Pi source with its npm dependencies installed, and the normal absolute `PI_DESKTOP_RPC_ENTRY` (or the source launcher default):

```sh
PI_DESKTOP_TEST_RPC_ENTRY="$PWD/scripts/pi-rpc.mjs" \
  CARGO_TARGET_DIR=target/linux cargo test -p pi_core local_pi_metadata_handshake_without_model_calls -- --ignored --nocapture

CARGO_TARGET_DIR=target/linux xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-live-startup.sh
```

The native startup/resume script creates temporary configuration and a saved `hello` fixture, runs Pi offline with extensions/skills/templates disabled, clicks that saved session in the real window, checks hydrated state and two owned processes, then quits. It sends **no prompts or model requests** and removes its temporary state. It does not inspect or alter the user's saved sessions.

Artifacts are local and ignored:

- `artifacts/refactor-{tests,clippy,build,capture,handshake}.log`
- `artifacts/thread-{evening,moonstone,queued,stopped,compact,new}.png`
- `artifacts/thread-{model-picker,thinking-picker,commands-picker}.png`
- `artifacts/thread-{edit-details,bash-details,edit-selected,bash-selected}.png`
- `artifacts/tool-{edit,bash,context}-copy.txt`, `tool-selection-check.log`
- `artifacts/screen-validation.json`, `study-comparison.{png,json}`
- `artifacts/thread-live.png`, `thread-live-resumed.{png,txt}`, `refactor-live-app.log`, `refactor-live-check.log`

Xvfb uses software Vulkan rendering, not a browser or SVG mockup. It can report missing physical input devices while synthetic mouse/keyboard checks still work.

## Earlier platform checks

Before this refactor, `pi_core` cross-checks passed for Apple Silicon macOS and Windows GNU, including Windows Job Object cleanup. These are historical compile checks, not native UI execution of the current refactor.

An earlier macOS host build reported `xcrun: unable to find utility "metal"`. The default `runtime-shaders` feature forwards through `gpui_platform/runtime_shaders` to the Apple backend. Metal source is compiled by the macOS runtime at launch rather than by a standalone build-time compiler. Feature forwarding was checked with `cargo tree --target aarch64-apple-darwin -e features -i gpui_apple`.

Xcode Command Line Tools and CMake remain required. The optional `--no-default-features` path needs full Xcode's Metal compiler. Linux cross-building the full macOS UI is not a substitute for testing it on a Mac.
