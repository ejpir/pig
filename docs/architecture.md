# Architecture

Pi Desktop is a native GPUI application. It embeds Zed's editor, project, Markdown and terminal components without creating a Zed Workspace. Pi owns agent execution, conversation history, models, credentials and extension behavior; the desktop owns presentation, session lifecycle and local development tools.

## Components and ownership

| Component | Responsibility |
| --- | --- |
| `pi_desktop` | Window, session controllers/views, navigation, preferences and native dialogs |
| `pi_core` | GPUI-independent session reducer, typed commands and their routing, local/SSH transport, desktop channel, desktop extension and process ownership |
| `pi_remote` | Cross-platform headless SSH helper: installation target, disposable stdio bridge, detached per-session backend owner, reconnect snapshots and independent file RPC |
| `backend/durable` | Experimental standalone durable runner: committed execution state, SQLite recovery and a text/coding-tool adapter; not a stock extension host |
| `pi_editor` | Shared Zed projects/buffers, file operations and editor integration |
| `pi_lsp_bridge` | Socket answering the desktop extension's language-server and jj requests |
| `pi_jj` | File-history operations through pinned `jj-lib`; no installed jj CLI required |
| `pi_terminal` | PTY shells and a standalone view over Zed's terminal emulator |
| `pi_settings` | Settings schemas, validation and atomic JSON file updates |

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

Project selection is independent of session selection. Selecting a project does not start pi, and closing its last session does not remove the project.

## RPC and session lifecycle

Each local session owns one pi subprocess. SSH sessions own a local bridge; a detached remote helper owns Pi. Remote close/disconnect detaches without aborting, reconnect replaces the projection from a live snapshot, and remote filesystem identities never enter local file services. Remote editors use detached Zed buffers over a separate `files --stdio` helper channel; reads, optimistic revision-checked saves and polling happen on the host, without a local Zed Project/file handle or language server. See [Remote Pi](remote.md) for the protocol, bootstrap and current capability limits.

On Unix, remote helper releases are immutable SHA-256 content objects. The verified installer invokes the object’s pathless `activate` command even on cache hits; activation serializes with a private lock, verifies checksum/version, and atomically selects a relative `~/.pi/desktop/bin/pi-desktop-remote` symlink. QR pairing and discovery use that stable path, while existing running helpers and old content directories remain untouched.

`pi_core` handles strict LF-framed UTF-8 JSONL, bounded queues, request correlation and deadlines. Records are limited to 16 MiB; stderr is diagnostic output, never protocol input. Responses must match both request ID and command. A timeout leaves the outcome unknown and never triggers an automatic prompt retry.

Submission stays disabled until state, messages, statistics and saved-session bootstrap complete. Resume checks the requested session identity against pi's response. Optional metadata, including the extension's versions and features, is not an identity gate.

- Streaming blocks update by content index; tools are keyed by tool-call ID. History and live events use the same reducer.
- Prompt acknowledgements report acceptance, not completion. Stock Pi's `agent_settled` completes a run; the experimental durable backend derives run state from its committed `pi.live` document.
- Stop clears queued work before aborting and returns recovered text to the originating composer.
- `!` and `!!` submit explicit SDK-owned shell work, including or excluding output from model context. They do not fabricate agent lifecycle events.
- Close rechecks running work, drafts, exclusive operations and shared dirty buffers. Closing a local tab stops its owned process; closing an SSH tab detaches while the remote agent continues. Neither deletes the project or saved conversation.

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

`@` matching uses Zed's standalone `fuzzy` crate over existing owner snapshots. Background matches are cancellable and guarded by query, range and generation. Files and directories serialize as path references; directories never attach recursive contents. SSH sessions use the remote file index and preview RPC without entering desktop filesystem or local language-server services. Other mention types serialize explicit text. Image attachments are separate prompt image blocks, with a 20 MB per-image limit.

## Pi and the desktop extension

By default, sessions run stock `pi --mode rpc` with the desktop extension, `crates/pi_core/extension/pi-desktop.ts`, which `pi_core` installs into the cache folder and passes with `-e`. Pi performs its own startup: stdout protection, proxy settings, trust, built-in extensions and model scope. The desktop uses no private Pi modules.

Problem: Pi's RPC mode lacks part of what the desktop needs, such as custom entries, active tools, a settings snapshot, labels, a fork into another folder, session listing, sharing, trust details, auth providers and packages. The extension supplies these through Pi's public extension API and package exports.

`pi_core` routes every `Command` (`Command::route`):

| Route | Commands | Transport |
| --- | --- | --- |
| Pi | Commands Pi's RPC has, including `fork` without a folder and `set_model` without `persist` | stdin/stdout |
| Extension | `get_backend_info`, `get_active_tools`, custom entries, `get_settings`, labels, `fork` into a folder, `list_sessions`, renaming another session, auth providers, trust, packages, model cycle and per-model thinking, `set_model` with `persist`, share, tree navigation, reload | desktop channel |

The desktop channel is a socket that `pi_core` owns for each pi process: a private Unix socket directory, or loopback TCP with a random token on Windows (`PI_DESKTOP_CHANNEL`, `PI_DESKTOP_CHANNEL_TOKEN`). The extension connects when a session starts and says `hello`. Pi recreates extensions whenever it replaces its session (new, resume, fork, reload), so the new instance connects again. Requests go to the newest connection. Replies are accepted from any connection, because a request sent before a reload is answered on the connection that received it. Requests sent before the first `hello` wait for it, and their deadlines still run. Replies have the shape of Pi's RPC responses, so both transports share correlation, deadlines and the session reducer. The language-server bridge (`pi_lsp_bridge`) remains a separate socket for the extension's own requests to the desktop.

Tree navigation, reload and the trust details need Pi's command context. The extension runs them as its `/pi-desktop` command, which it dispatches itself with `sendUserMessage`. Pi executes an extension command at once and never sends it to the model; the extension refuses if its command is not loaded, so the text can never become a prompt. The desktop hides this command from the user's command lists.

Settings changes (default model with `persist`, model cycle, per-model thinking) use the extension's own Pi `SettingsManager`. Pi writes settings under its lock and rereads the file, writing back only fields it changed. Such changes apply to new sessions; the current model's thinking level also changes at once. The extension serializes mutations and answers reads at once. Pi's own RPC mode handles extension dialogs (`extension_ui_request`).

Sharing first asks the extension to upload the current branch to Radius, as Pi's `/share` does, with the token from Pi's public model registry. Without Radius, `session_actions::share` exports Pi's HTML with RPC `export_html` and creates a private gist with the GitHub CLI. Uploads require explicit confirmation and are bounded. Session deletion moves the file to the system trash from Rust after checking its header; there is no permanent-delete fallback. Authentication remains a terminal handoff; the desktop does not read credential files.

Tool inventories, resources, model capabilities and usage come from reported metadata. Unknown values remain distinct from empty, zero or false; a failed `get_active_tools` leaves tools unreported without raising a session error. Catalog views reuse existing controllers rather than starting inspection processes.

Accepted consequences of stock Pi:

- A broken user extension stops the session from starting, as in every Pi mode; the desktop shows Pi's stderr diagnostic.
- Resources lists only extensions that register tools or commands. Pi exposes no list of loaded extensions.
- The saved model cycle, and per-model thinking for models other than the current one, apply to new sessions.
- Radius sharing resolves its token without Pi's five-minute validity margin.

## Experimental durable SSH engine

A new SSH target can opt into `RemoteBackend::Durable`; legacy targets deserialize as stock Pi and an existing key cannot change backend/cwd. SSH setup probes helper capabilities before connecting a durable target. No stock Pi fallback or local-service fallback is allowed.

`pi_remote/src/durable.rs` launches an internal Rust storage owner supervising a standalone Bun runner. On Unix the runner inherits the owner's OS writer-lock descriptor; killing the owner cannot free storage while the runner still writes. All descendants remain in the RPC-owned process group. Independent file services do not change.

The runner reuses stock Pi's public `ModelRuntime` from the pinned 1.0.2 SDK for built-in providers, host-side `auth.json`/OAuth refresh under the existing credential lock, global `models.json` and cached catalogs. It never constructs a stock agent or extension runtime. Model/auth metadata excludes credentials and request headers; catalog refresh is offline/cache-only. Interactive login remains a terminal handoff on the SSH host.

The runner uses pinned pi-durable 1.0.2 and its SQLite adapter with WAL/`synchronous=FULL`. A serialized committed watch supplies transcript, live generation/tool state, inbox and usage; Rust reconstructs a disposable `Session` projection and sends authoritative snapshots. Runner read acknowledgements never overwrite that projection with empty state. Reconnect after process/host failure reopens the same storage and resumes checkpoints; there is no unattended reboot service yet.

Prompt request keys are independent of correlation counters. Admission is acknowledged only after durable commit, same-key explicit retries deduplicate and payload collisions fail. Uncertain submissions are never automatically replayed; the desktop outbox is not crash-persistent yet. Unsafe interrupted tools return errors instead of being automatically repeated. Authentication/resources/stock-extension parity, capabilities UI, migration and native runtime gates precede making this the default. See [the runner walkthrough](../backend/durable/README.md) and [remote scope](remote.md).

## Built-in pi

Release builds embed Pi's official release binary for one pinned version (`PI_VERSION` in `crates/pi_core/src/extension.rs`, which the extension's version must match). `scripts/fetch_pi.py` downloads the platform's release archive, checks it against `packaging/pi-release.sha256`, which is committed and reviewed with each version change, and repacks pi's executable and the files beside it as a gzip-compressed tar for the `bundled-backend` Cargo feature (`PI_DESKTOP_BACKEND_ARCHIVE`). On macOS, `PI_DESKTOP_CODESIGN_IDENTITY` re-signs pi for the hardened runtime with `packaging/macos/backend.entitlements`. Development builds without the feature run `pi` from PATH.

At startup, the app extracts the archive into a build-specific cache directory through temporary staging and rename. Later launches reuse it. Extraction failure is logged and leaves `pi` from PATH as the fallback.

Program selection follows this precedence:

1. For development, `PI_DESKTOP_PI` (an executable) or `PI_DESKTOP_RPC_ENTRY` (a JavaScript entry run with `PI_DESKTOP_NODE` or `node`, such as `scripts/pi-rpc.mjs` for a sibling `../pi` checkout).
2. The embedded pi, when available.
3. `pi` from PATH.

Settings shows which pi and extension versions answer, and notes a pi other than the pinned version. The embedded pi needs neither Node nor an installed pi for agent execution, but terminal login, npm packages and Node-based language servers still require their respective external tools. Native Node add-ons may be incompatible with Bun, which Pi's release binary is built with.

Every pi starts with `NODE_USE_SYSTEM_CA=1` unless the environment sets it, so it trusts the system's certificates as native apps do. Without it, a company proxy that re-signs HTTPS makes every model request fail with "Connection error." (seen on macOS with the built-in backend).

Packages include application, Pi, Bun and npm dependency notices (`fetch_pi.py --notices` installs Pi's release lockfile with `npm ci --ignore-scripts` and collects the licenses). Current macOS packages are ad-hoc signed, not Developer ID signed or notarized; Windows packages are unsigned. Native release targets are Linux amd64/arm64, macOS arm64 and Windows amd64.

## Editor, files and language services

A lazy registry shares Zed `Project` and buffer entities by canonical project root. Session-local Files views retain their own tabs over those shared buffers. The editor uses separate Pi Desktop data/database paths and does not start Zed authentication or telemetry.

File reads and mutations run off the foreground executor. Saves reject disk conflicts; rename and Trash operations recheck shared dirty buffers. Creation is atomic and refuses overwrite. Paths must remain within the project, with `.git` and `.jj` protected. Destructive actions require confirmation; deletion has no permanent-unlink fallback.

Language servers require explicit editor-project trust. They may execute project configuration or download packages; this trust is separate from Pi's process trust. Diagnostic cards combine reported messages, hover text and code actions. Quick fixes remain unsaved editor changes; “Ask Pi to fix” uses normal submission policy.

The LSP extension sends successful edit/write and run-end checks to a per-session bridge. Unix uses a private socket directory; Windows uses loopback TCP with a random session token. Answers include bounded primary errors, excluding unsaved buffers and already-reported diagnostics. Checks wait at most four seconds per file and twenty seconds at run end; slow servers can report later.

Terminals are session-owned PTY shells, independent of Pi context. Terminal and code text use the embedded Commit Mono, whose name table was fixed for Windows: its stray subfamily name `400 Regular` let DirectWrite's text layout miss the family `CommitMonoV143` and draw a proportional fallback font on the terminal's fixed grid, so letters bunched up and words drifted apart. The font now names its family explicitly (name IDs 21 and 22). Hiding the drawer does not stop them; closing the session does. “Open in terminal” stages a historical command without executing it.

## Conversation and file history

Conversation navigation and filesystem history are independent. Tree follows Pi's stable entry IDs and parent edges; navigating it does not rewind files.

Optional jj integration snapshots before and after a run. A run that changes files becomes a recorded turn; a read-only run creates no change. Changes displays immutable turn diffs, not a live Git diff. Successful tool-reported edits without a linked snapshot remain explicitly incomplete and cannot authorize rollback.

The desktop extension persists exact turn links as `pi-desktop-turn` custom entries outside model context. Reopening resolves their change/commit identities; it never infers associations from prompts, paths or transcript positions. Without the extension, associations last only while the session is open.

Undo, redo, file restoration and operation restoration run between agent runs and reject unsafe shared-buffer or repository states. Conflict-aware undo requires an explicit choice; a proposed fix remains a draft. Command snapshots are session-local and may cover a whole parallel tool batch. File snapshots cannot distinguish agent changes from concurrent edits in the same workspace.

Parallel-session workspaces, bringing turns into another workspace and forks with historical files are explicit operations. Forking with a different directory uses the desktop extension, which writes the new session file. jj initialization is opt-in and colocated with Git; operations load current repository state and import/export Git refs.

## Settings and persistence

| Store | Contents |
| --- | --- |
| Pi user/project `settings.json` | Agent settings; owned by Pi |
| Pi Desktop config `settings.json` | Desktop preferences |
| Project `.pi/pi-desktop.json` | Supported desktop project overrides, including jj and language-server policy |
| Pi Desktop config `state.json` | Remembered sessions, selection and dismissed repository prompts |

Desktop settings resolve project override → user value → default. Invalid values are ignored; malformed files are shown as errors and never overwritten. Background writers serialize updates and write the newest pending snapshot atomically. Unknown JSON keys are preserved.

Theme and editor preferences apply immediately; shell choices apply to new terminals. Pi settings do not silently reconfigure running sessions. Saved trust and effective process trust are shown separately; saving trust is not a reload.

## Safety and limitations

Confirmations default to Cancel and recheck the originating owner and readiness before execution. Reload and package operations may execute code. Pi trust is not an OS sandbox and does not provide per-extension filesystem or network permissions.

Session diagnostics retain at most 128 entries / 48 KiB, including an 8 KiB stderr tail. They omit request bodies and tool/model payloads, apply best-effort redaction and warn before copying. Diagnostics are local, in-memory and never uploaded automatically.

Remaining limitations include native authentication/setup UI, complete resource manifests, editor-selection mentions, terminal search and idle process/document eviction. Large-repository snapshot costs and cross-platform live behavior need broader validation. See [Validation](validation.md) for tested scope and release limitations.
