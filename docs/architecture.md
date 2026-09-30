# Architecture

Pi Desktop is a native GPUI application. It embeds Zed's editor, project, Markdown and terminal components without creating a Zed Workspace. Pi owns agent execution, conversation history, models, credentials and extension behavior; the desktop owns presentation, session lifecycle and local development tools.

## Components and ownership

| Component | Responsibility |
| --- | --- |
| `pi_desktop` | Window, session controllers/views, navigation, preferences and native dialogs |
| `pi_core` | GPUI-independent session reducer, typed RPC commands, transport and process ownership |
| `pi_editor` | Shared Zed projects/buffers, file operations and editor integration |
| `pi_lsp_bridge` | Local extension bridge for language-server feedback and desktop services |
| `pi_jj` | File-history operations through pinned `jj-lib`; no installed jj CLI required |
| `pi_terminal` | PTY shells and a standalone view over Zed's terminal emulator |
| `pi_settings` | Settings schemas, validation and atomic JSON file updates |
| `packages/pi-desktop-backend` | JSONL adapter over the published Pi SDK; Node or standalone Bun execution |

```text
Desktop
├── WorkspaceController: session identities, selection, saved-session catalog
│   └── SessionTab
│       ├── SessionController: Pi state, RPC lifecycle, submission policy, jj/LSP services
│       └── SessionView
│           ├── Transcript, composer, diagnostics
│           ├── Tree, Changes, Context, Files
│           └── Terminal drawer and inspector
├── Sidebar, header, status bar
└── App views: Sessions, Models, Resources, Settings
```

Each open session retains its own controller, draft, attachments, queues, disclosures, scroll state, editor tabs and terminals. Selecting another session does not repurpose its process or recover text into another composer. Views read the controller's state and issue typed commands; they cannot mutate the core model directly. Subscriptions and background tasks use weak receivers to avoid ownership cycles.

Project selection is independent of session selection. Selecting a project does not start a backend, and closing its last session does not remove the project.

## RPC and session lifecycle

Each session owns one backend subprocess. `pi_core` handles strict LF-framed UTF-8 JSONL, bounded queues, request correlation and deadlines. Records are limited to 16 MiB; stderr is diagnostic output, never protocol input. Responses must match both request ID and command. A timeout leaves the outcome unknown and never triggers an automatic prompt retry.

Submission stays disabled until state, messages, statistics and saved-session bootstrap complete. Resume checks the requested session identity against the backend's response. Optional metadata, including backend capabilities, is not an identity gate.

- Streaming blocks update by content index; tools are keyed by tool-call ID. History and live events use the same reducer.
- Prompt acknowledgements report acceptance, not completion. Only `agent_settled` completes a run.
- Stop clears queued work before aborting and returns recovered text to the originating composer.
- `!` and `!!` submit explicit SDK-owned shell work, including or excluding output from model context. They do not fabricate agent lifecycle events.
- Close rechecks running work, drafts, exclusive operations and shared dirty buffers. Closing a tab stops its owned process; it does not delete the project or saved conversation.

Unix subprocesses use isolated process groups; Windows subprocesses use kill-on-close Job Objects. Shutdown closes stdin, then kills and reaps an uncooperative child after a grace period. Descendants are stopped before joining pipe readers.

## Rendering and input

Controllers emit targeted events rather than invalidating the entire window. Cached session summaries drive navigation; background text changes do not parse or redraw the active transcript.

The transcript uses variable-height virtualization and retained Markdown entities. Content changes update affected documents in place, preserving selection and scroll anchors. Metadata updates do not rebuild them. Virtualization limits rendering work, not retained history or process count.

Presentation preserves original content and copy:

- Chat, prose and composer use the available width without a fixed reading cap.
- Live or unfinished activity opens automatically. Settled activity collapses, including failures, unless explicitly expanded. Success requires nonempty, settled, fully successful calls.
- Tool previews are bounded; full-value copy uses the original retained content. Diff line numbers are display-only.
- Composer and process-list scrollbars use stable gutters. Manual composer scrolling pauses caret following until editing or cursor navigation resumes it.
- Notifications float above content rather than changing transcript or table geometry.

`@` matching uses Zed's standalone `fuzzy` crate over existing owner snapshots. Background matches are cancellable and guarded by query, range and generation. Files and directories serialize as path references; directories never attach recursive contents. Other mention types serialize explicit text. Image attachments are separate prompt image blocks, with a 20 MB per-image limit.

## Pi SDK backend

The adapter creates one Pi `AgentSessionRuntime` per subprocess and rebinds services and subscriptions after session replacement. The npm version and dependency graph are pinned; private Pi imports and compatibility checks are isolated in `src/compat.mjs`.

Mutations and prompt preflight are serialized. Metadata reads, aborts and correlated extension-dialog replies remain responsive. Standard confirm/select requests use a bounded, session-local FIFO; timeout, abort or disconnect cancels them. Unsupported interactive requests cancel explicitly rather than inventing answers.

Tool inventories, resources, model capabilities and usage come from reported backend metadata. Unknown values remain distinct from empty, zero or false. Catalog views reuse existing controllers rather than starting inspection processes.

Sharing uses Pi's branch exporter and credential resolver. Uploads require explicit confirmation, are bounded and cancellable, and remove temporary exports. Authentication remains a terminal handoff; the desktop does not read credential files.

## Built-in backend

Release packaging compiles the adapter with Bun and embeds its executable and runtime assets as a compressed archive through the `bundled-backend` Cargo feature. Development builds without that feature remain usable with an external backend.

At startup, the app extracts the archive into a build-specific cache directory through temporary staging and rename. Later launches reuse it. Extraction failure is logged and leaves the external Pi fallback available.

Backend selection follows this precedence:

1. `PI_DESKTOP_RPC_ENTRY` or `PI_DESKTOP_PI` environment override.
2. Settings → General → Backend.
3. Embedded backend, when available.
4. `pi` from PATH.

JavaScript backend paths use Node; executables run directly. `PI_DESKTOP_NODE` overrides the configured Node program. The embedded backend needs neither Node nor Pi for agent execution, but terminal login, npm packages and Node-based language servers still require their respective external tools. Native Node add-ons may be incompatible with Bun.

Every backend starts with `NODE_USE_SYSTEM_CA=1` unless the environment sets it, so Bun and Node trust the system's certificates as native apps do. Without it, a company proxy that re-signs HTTPS makes every model request fail with "Connection error." (seen on macOS with the built-in backend).

Packages include application, Pi, Bun and dependency notices. Current macOS packages are ad-hoc signed, not Developer ID signed or notarized; Windows packages are unsigned. Native release targets are Linux amd64/arm64, macOS arm64 and Windows amd64.

## Editor, files and language services

A lazy registry shares Zed `Project` and buffer entities by canonical project root. Session-local Files views retain their own tabs over those shared buffers. The editor uses separate Pi Desktop data/database paths and does not start Zed authentication or telemetry.

File reads and mutations run off the foreground executor. Saves reject disk conflicts; rename and Trash operations recheck shared dirty buffers. Creation is atomic and refuses overwrite. Paths must remain within the project, with `.git` and `.jj` protected. Destructive actions require confirmation; deletion has no permanent-unlink fallback.

Language servers require explicit editor-project trust. They may execute project configuration or download packages; this trust is separate from Pi's process trust. Diagnostic cards combine reported messages, hover text and code actions. Quick fixes remain unsaved editor changes; “Ask Pi to fix” uses normal submission policy.

The LSP extension sends successful edit/write and run-end checks to a per-session bridge. Unix uses a private socket directory; Windows uses loopback TCP with a random session token. Answers include bounded primary errors, excluding unsaved buffers and already-reported diagnostics. Checks wait at most four seconds per file and twenty seconds at run end; slow servers can report later.

Terminals are session-owned PTY shells, independent of Pi context. Terminal and code text use the embedded Commit Mono, whose name table was fixed for Windows: its stray subfamily name `400 Regular` let DirectWrite's text layout miss the family `CommitMonoV143` and draw a proportional fallback font on the terminal's fixed grid, so letters bunched up and words drifted apart. The font now names its family explicitly (name IDs 21 and 22). Hiding the drawer does not stop them; closing the session does. “Open in terminal” stages a historical command without executing it.

## Conversation and file history

Conversation navigation and filesystem history are independent. Tree follows Pi's stable entry IDs and parent edges; navigating it does not rewind files.

Optional jj integration snapshots before and after a run. A run that changes files becomes a recorded turn; a read-only run creates no change. Changes displays immutable turn diffs, not a live Git diff. Successful tool-reported edits without a linked snapshot remain explicitly incomplete and cannot authorize rollback.

The SDK backend persists exact turn links as `pi-desktop-turn` custom entries outside model context. Reopening resolves their change/commit identities; it never infers associations from prompts, paths or transcript positions. Backends without these commands retain associations only while the session is open.

Undo, redo, file restoration and operation restoration run between agent runs and reject unsafe shared-buffer or repository states. Conflict-aware undo requires an explicit choice; a proposed fix remains a draft. Command snapshots are session-local and may cover a whole parallel tool batch. File snapshots cannot distinguish agent changes from concurrent edits in the same workspace.

Parallel-session workspaces, bringing turns into another workspace and forks with historical files are explicit operations. Forking with a different directory requires backend support. jj initialization is opt-in and colocated with Git; operations load current repository state and import/export Git refs.

## Settings and persistence

| Store | Contents |
| --- | --- |
| Pi user/project `settings.json` | Agent settings; owned by Pi |
| Pi Desktop config `settings.json` | Desktop preferences |
| Project `.pi/pi-desktop.json` | Supported desktop project overrides, including jj and language-server policy |
| Pi Desktop config `state.json` | Remembered sessions, selection and dismissed repository prompts |

Desktop settings resolve project override → user value → default. Invalid values are ignored; malformed files are shown as errors and never overwritten. Background writers serialize updates and write the newest pending snapshot atomically. Unknown JSON keys are preserved.

Theme and editor preferences apply immediately; backend choices apply to new sessions, and shell choices to new terminals. Pi settings do not silently reconfigure running sessions. Saved trust and effective process trust are shown separately; saving trust is not a reload.

## Safety and limitations

Confirmations default to Cancel and recheck the originating owner and readiness before execution. Reload and package operations may execute code. Pi trust is not an OS sandbox and does not provide per-extension filesystem or network permissions.

Session diagnostics retain at most 128 entries / 48 KiB, including an 8 KiB stderr tail. They omit request bodies and tool/model payloads, apply best-effort redaction and warn before copying. Diagnostics are local, in-memory and never uploaded automatically.

Remaining limitations include native authentication/setup UI, complete resource manifests, editor-selection mentions, terminal search and idle process/document eviction. Large-repository snapshot costs and cross-platform live behavior need broader validation. See [Validation](validation.md) for tested scope and release limitations.
