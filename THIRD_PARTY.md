# Third-party provenance

## Zed / GPUI

Source: https://github.com/zed-industries/zed, revision
`5becf8b5910fd538ccc5489e8edd3fde32917fad`.

- `gpui` and `gpui_platform` (Apache-2.0, `licenses/ZED-APACHE-2.0`) are linked directly from the sibling checkout.
- Chat Markdown uses Zed's `markdown` crate with `ui`, `menu`, `settings`, `theme`, `theme_settings`, and `util`, which bring in `language` and tree-sitter. These are GPL-3.0-or-later (`licenses/ZED-GPL-3.0`), which is why this application is licensed GPL-3.0-or-later. `ui` also supplies the context menus and scrollbars. The Zed editor and workspace are not linked.
- `Cargo.lock` is seeded from Zed's lockfile at the pinned revision, and the workspace mirrors Zed's `[patch.crates-io]` entries for the crates it links (tree-sitter-language, async-process, async-task, calloop, notify, windows-capture), because Zed's code depends on those forks.
- `util` is built with its `debug-embed` feature so debug builds embed Zed's settings/theme assets instead of reading them from the Zed checkout at runtime.
- `crates/pi_desktop/src/input.rs` is adapted from `crates/gpui/examples/input.rs` (Apache-2.0). The example's runnable demo was removed; input was given a constructor/public content API, scoped Ctrl/Cmd bindings, app theme colors, focus-on-click, horizontal caret tracking, and tests. Local fixes correct relative UTF-16 IME selection, point hit-testing, and reversed-selection reset. Its `EntityInputHandler`, shaped-line custom element, selection, clipboard, grapheme, and composition implementation are reused rather than replaced by raw key capture.
- `crates/pi_terminal` links Zed's `terminal` crate (GPL-3.0-or-later), which uses Zed's fork of `alacritty_terminal` (Apache-2.0). `crates/pi_terminal/src/element.rs` is adapted from `crates/terminal_view/src/terminal_element.rs` (GPL-3.0-or-later): the grid layout, text batching, background/block-glyph rectangles, pointer and IME handling are kept; the Workspace, `editor` cursor/selection types, inline mode, blocks below the cursor, search and hover tooltips are removed. `crates/pi_terminal/src/scrollbar.rs` is adapted from `terminal_view/src/terminal_scrollbar.rs`. Key bindings follow Zed's default keymaps for the `Terminal` context.
- `crates/pi_lsp_bridge` links Zed's `project`, `language` and `lsp` crates (GPL-3.0-or-later); its tests use Zed's fake language server and FakeFs. Its pi extension (`extension/pi-desktop-lsp.ts`) is original code that uses only pi's public extension API.
- Architectural/pattern references, not vendored implementations: `crates/ui/src/components/stack.rs` and `button/button.rs` (flex helpers and builder-style components); `crates/gpui/examples/hello_world.rs` (native application/window bootstrap); `crates/gpui/examples/testing.rs` (entity/action/visual-test contexts). The app uses GPUI's entity ownership, weak async updates, task retention, action dispatch, and render traits.

The sibling Zed checkout is intentionally unchanged. A future dependency update must re-check both the input adapter and native platform feature wiring.

## Fonts

Copied, unmodified, from Zed's `assets/fonts/`:

- IBM Plex Sans Regular, SemiBold, Italic: SIL Open Font License, `licenses/IBM-PLEX-OFL.txt`.
- Lilex Regular: SIL Open Font License, `licenses/LILEX-OFL.txt` (retained reference asset, no longer loaded).

Commit Mono 400 Regular is embedded for code/labels, matching the study's named font. Source: `eigilnikolajsen/commit-mono`, revision `d407cd2bf8e01ca1db70544052fbbb9606406c3b`, `src/fonts/fontlab/CommitMonoV143-400Regular.otf`. Modified in its name table only: the subfamily name `400 Regular` (ID 17) is removed and the family is named explicitly for Windows (IDs 21 and 22: `CommitMonoV143`, `Regular`), and the empty `DSIG` table is dropped. Glyphs and metrics are unchanged. SIL Open Font License: `licenses/COMMIT-MONO-OFL.txt`.

Fonts are embedded in the executable, so launch does not depend on locally installed UI/code fonts. Georgia (macOS/Windows) and DejaVu Serif (Linux) are system title-font fallbacks; no proprietary Plantin font is distributed.

## Icons

Selected SVGs copied from Zed's `assets/icons/`, including those Zed's `ui` components load by name (`info`, `chat`, `warning`, `dash`, `text_wrap`, `text_unwrap`) the sidebar toggles (`threads_sidebar_left_*`, `threads_sidebar_right_*`), and `undo` and `git_commit` for jj turns (`redo` is `undo` mirrored). Lucide/Feather attribution and ISC terms are preserved in `licenses/ZED-ICONS-ISC.txt`. The thread, terminal, stop, attachment, slash, queue, and compact glyphs use the outline paths from the provided `design/pi_study_common.py` instead, to match the study. All are embedded through GPUI's `AssetSource`. The three-color pi mark is drawn from the same study's geometry, and `scripts/generate-icons.py` renders the app icon from it. `spinner_track.svg` and `spinner_arc.svg` follow the study's running mark. `image.svg` and `open.svg` redraw Pi for Android's icons of the same names (`crates/pi_android/assets/icons/`) on the desktop's 16-unit grid.

## Pi

Pi runs outside the Rust UI in a subprocess. Protocol definitions were checked against Pi's `rpc-types.ts` and `docs/{rpc,rpc-commands,json,message-types,rpc-extension-ui}.md`. The fallback `scripts/pi-rpc.mjs` launches a sibling source checkout using its own source resolver.

Release builds embed Pi's official 1.0.0 release binary (MIT), downloaded by `scripts/fetch_pi.py` and checked against the SHA-256 digests committed in `packaging/pi-release.sha256`. Packages carry Pi's license as `licenses/PI-MIT.txt` and, as `licenses/PI-NOTICES.txt`, the licenses of the npm packages compiled into Pi's executable, collected by installing Pi's release lockfile (`fetch_pi.py --notices`).

The desktop extension, `crates/pi_core/extension/pi-desktop.ts`, uses only Pi's public extension API and package exports. Its sharing flow follows Pi's `/share` (branch export with a `pi.share` entry, Radius upload, private gist fallback).

## Bun

Pi's release binary is a Bun standalone executable (Bun 1.3.14 for Pi 1.0.0), so it contains the Bun runtime: MIT-licensed, statically linking JavaScriptCore/WebKit (LGPL-2) and the other libraries listed in `licenses/BUN-LICENSE.md` (copied unmodified from Bun's `LICENSE.md` at tag `bun-v1.3.14`; update it when a new Pi release reports another `bunVersion`). That file also explains how to rebuild Bun with a modified JavaScriptCore, as the LGPL requires. The executable is embedded compressed in `pi-desktop` and written to the user's cache folder before it runs.

## Experimental durable runner

`backend/durable` pins `@earendil-works/pi-durable`, `pi-ai`, `chord` and `pi-coding-agent` 1.0.2 (MIT) through its npm lockfile. The coding-agent SDK supplies only the public model/auth runtime and credential locking; the runner does not start its agent or extension host. Its native standalone executable contains Bun 1.4.2; `licenses/DURABLE-BUN-LICENSE.md` is copied unmodified from `oven-sh/bun` tag `bun-v1.4.2`, and covers JavaScriptCore/WebKit relinking and runtime dependencies for this separate version. `backend/durable/build.ts` gathers full installed dependency notices plus the application and matching Bun notices into `artifacts/durable/NOTICES.txt`. `pi_remote` embeds/prints these with the optional `bundled-durable` feature. The faux-only test runner is not shipped. Normal release packaging still selects stock Pi pending the durable compatibility gates.

## Design

Colors, dimensions, terminology, and screen structure come from the provided `design/` studies. Fixture names, paths, and numbers are explicitly sample data. The SVG/PNG studies are references, not the rendered application.
