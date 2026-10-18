# Remote Pi over SSH

Choose **New session → SSH**, enter an SSH host alias and a project directory on that host, then create a session. Or launch:

```sh
pi-desktop --ssh dev --project /srv/my-project
```

The directory is a remote path. It is never canonicalized or opened on the desktop machine. Windows remote paths can be passed as quoted `C:\\repos\\project` strings.

## Prerequisites and installation

The desktop needs OpenSSH `ssh` and `scp`, and `curl` for downloading release helpers. Authenticate with keys or an SSH agent; establish the host's identity with `ssh dev` in a terminal first. The app uses `BatchMode=yes` and `StrictHostKeyChecking=yes`: it does not silently accept host keys or prompt for passwords. SSH config (including ports, identities and jump hosts) is respected.

The app detects Linux amd64/arm64, macOS arm64, or Windows amd64. It downloads that platform's standalone `pi-desktop-remote` from the desktop's versioned GitHub release and verifies it against that release's HTTPS `SHA256SUMS`. It uploads over SCP, verifies the checksum on the remote host too, and installs atomically under the user's `.pi/desktop/bin/<sha256>/`. On Unix it then runs that exact hashed helper's `activate` command, which verifies the content object and atomically replaces `.pi/desktop/bin/pi-desktop-remote` with a relative symlink to it. No sudo, remote download access, Node installation or separately installed Pi is needed for release helpers. Different helper builds never overwrite a running helper. Previously verified helpers and their checksum manifest are cached locally, so reconnecting does not require GitHub access.

Helpers are built on the same native platforms as the desktop; unsupported architectures and older incompatible system libraries are not silently substituted. Windows setup expects standard OpenSSH with PowerShell available and is unchanged by Unix activation. Existing credentials and Pi settings **on the remote host** are used; desktop credentials are not copied. For interactive setup on Unix, run the stable helper's `pi` subcommand in an SSH terminal (`~/.pi/desktop/bin/pi-desktop-remote pi`, then `/login`). Release helpers use their bundled Pi for this too.

On Linux and macOS, pairing and helper activation update `authorized_keys` with checked atomic exchanges. New phone entries use a forced-command commit marker and cannot authenticate until the approval response is durably published; failed approval is removal-only and never republishes an older same-key entry. Each displaced or provisional generation remains as a private `.pi-authorized-keys-recovery-*` sibling so a late write through an old descriptor is still reachable. New authorizations stop at 64 records; required rollback and bootstrap removal may exceed that cap to fail closed. Inspect and explicitly remove obsolete records before retrying.

## Ownership and reconnection

```text
Desktop RpcClient → ssh → helper connect --stdio
                              ↓ authenticated loopback IPC
                          helper daemon → Pi RPC + desktop extension
```

There is one detached daemon per remote session. The bridge belongs to SSH; the daemon owns Pi and its descendants. Closing the app, closing the tab, or losing SSH detaches the bridge, not the agent. **Stop** explicitly clears queued inputs and aborts the agent.

Use **Reconnect** after a connection failure. **All Sessions** and New session's SSH form also list saved remote sessions, including detached ones. **Remove** forgets a saved shortcut without closing its open tab, stopping the agent, or deleting remote files. Removed shortcuts remain removed even if that open session continues receiving updates or the app restarts. Their stable random keys and remote session files are remembered separately from local sessions. **Start a new remote session** chooses a fresh key instead of attaching to a previous agent.

Only one desktop attachment controls a remote session at a time. A new attachment replaces the old one. Reconnection begins with a complete snapshot, including partial assistant content, running tools and queues, followed by ordered live events. Pending extension questions are delivered to the new attachment. Events and replies belonging to an obsolete attachment cannot complete requests in its replacement.

A disconnect leaves the last known agent state visible as disconnected, not falsely idle. Requests with unknown outcomes are never automatically retried. If text is restored to the composer, reconnect and inspect the conversation before submitting it again.

A slow client is detached rather than blocking agent execution. Protocol-2 records and snapshots have a 64 MiB transport limit. A still-running protocol-1 daemon remains safely attached through a compatibility bridge at its negotiated 16 MiB limit until that daemon exits; it is not automatically killed or restarted because another attached client could be submitting work. IPC listens only on loopback and authenticates using a random token stored in the private per-user state directory. Unix state directories/files use 0700/0600; Windows relies on the user's profile ACLs. Startup diagnostics are stored beside endpoints in `.pi/desktop/run/1/<key>.log`.

**Stock Pi remains the default.** Its runs survive SSH/desktop disconnects, but a daemon/host crash does not automatically continue the interrupted run. Reopening can resume the saved conversation; it does not replay the prompt or tools. Idle daemons are not automatically evicted yet.

## Experimental durable backend

Choose **Durable · experimental** in the SSH form, or pass `--remote-backend durable` for a new SSH session. Normal Unix release helpers bundle the durable runner; development helpers need the `bundled-durable` feature or `PI_DESKTOP_DURABLE_RUNNER` on the SSH host. Windows helpers retain stock Pi only. An older or unsupported helper is rejected before starting a stock agent. Reconnect never changes an existing session's backend or migrates its stock session file.

The same detached Rust daemon supervises a standalone Bun runner containing pinned pi-durable 1.3.0. No Node/Bun installation is needed on the SSH host. SQLite with WAL and `synchronous=FULL` is authoritative; desktop snapshots are disposable projections of committed durable state. Checkpoints live under `~/.pi/desktop/durable/<key>/`, independently of helper binary caches and daemon endpoints. Both the Rust storage owner and its runner hold the same Unix OS writer lock, including when the owner is killed.

Steering is admitted immediately while the agent is busy, but is applied at the next completed tool-round boundary (or after the current response finishes if it has no tools). It does not interrupt a provider request or a running tool; a parallel round waits for its slowest tool, and sequential rounds finish all calls. The pinned durable engine consumes one queued steer per boundary. Parent steering does not redirect background subagents. If a run fails, queued inputs can remain parked until another submission; reconnect does not silently start a fresh run just to drain them.

Android treats an acknowledged new turn as busy until its snapshot arrives, so another prompt is normally sent as steering during that gap. **Known ordering limit:** helper snapshots and admission acknowledgements have no shared causal watermark yet. An older idle snapshot arriving after an acknowledgement can erase this busy hint and cause a subsequent prompt to be rejected as busy; the rejected payload remains recoverable. A correlated state refresh is not a fix for this ordering race.

Disconnect keeps work running. **Reconnect after an agent/daemon/host crash** opens storage and resumes unfinished work and the persisted inbox. Ad-hoc sessions still have no automatic daemon restart. The separate, opt-in [host scheduler](../crates/pi_remote/README.schedules.md) owns only its scheduled occurrences and can reopen those after restart; it does not supervise every session. Safe tools may rerun; interrupted unsafe tools report an error instead of automatically repeating their side effects. The model can still choose further actions; durable recovery is not a guarantee of task success or exactly-once external effects.

Prompt admission requires a stable `requestId` and acknowledges only after durable storage commits. The desktop creates a fresh key per send; it does not automatically replay an uncertain prompt and does not yet persist a crash-safe desktop outbox. A protocol client may query `get_submission` using that key and explicitly retry the same payload/key; different payloads with that key are rejected. The prototype retains up to 10,000 prompt receipts per conversation, refusing new admissions rather than expiring keys.

Currently supported: text prompts, read/write/edit/bash tools, subagents (stock Pi's `subagent` tool and agent files), streaming committed snapshots, steering/follow-ups, stop/clear queue, manual compaction, naming, model/thinking selection and reconnect. The runner reuses stock Pi's model/auth SDK on the SSH host: stored API keys and OAuth/subscription tokens in `~/.pi/agent/auth.json` (or the host's `PI_CODING_AGENT_DIR`), all built-in providers, global `models.json` configuration and cached catalogs, and the host's prompt templates and skills (see the [durable backend README](../backend/durable/README.md#prompt-templates-and-skills)). Token refresh uses stock Pi's file locking; auth/model metadata excludes secrets and the credential store is not copied to desktop snapshots or durable state. **Models → Refresh** reloads host configuration without restarting. Catalog discovery stays offline/cache-only; sign in/populate dynamic catalogs with stock Pi on the host. **Limits:** Unix SSH hosts only (Linux arm64 locally validated), no interactive durable login flow, stock extensions, Pi packages, forks or stock session-tree migration. Unsupported commands fail explicitly; nothing falls back to stock Pi or desktop services. Files remain independent and work with either engine.

See [`backend/durable/README.md`](../backend/durable/README.md) for native builds, no-provider crash tests and a Mac loopback walkthrough. This is the prototype before a default-backend switch; compatibility/authentication gates still need to pass.

## First-release scope

Remote chat, model/thinking controls, agent tools, steering, follow-ups, stop and reconnect are supported. The helper routes desktop-extension requests on the remote host, preserving stock Pi extensions.

Remote file browsing and text editing are supported through **Files**, and the composer's **@** menu supports remote files, directories and file previews. Android also provides a helper-owned project terminal on Linux hosts and source history backed by the helper's jj service. Desktop remote LSP symbols/problems, Tree/Changes/Context views, forks, saved-session file management, HTML export, and explicit `!`/`!!` shell submissions remain unsupported. Agent `bash` tools still run remotely. Remote file buffers have no local filesystem handle or local language-server project.

## Remote files

Open **Files**, select a file, edit it and use **Save** / ⌘S (Ctrl+S on Linux/Windows). The existing Zed editor supplies syntax highlighting and normal buffer/undo/dirty-state behavior. Reads and saves run through an independent `helper files --stdio` SSH channel, not through Pi or its controlling attachment. File browsing alone does not start an agent or call a model provider.

Type **@** in the composer to load/search the remote tree; you do not need to open Files first. **Enter** inserts a file/directory chip; **Tab** completes its path. Pi receives project-relative path references and reads them on the remote host, not desktop paths or attached file contents. File previews use the same bounded SSH read service, never a desktop filesystem fallback; directory mentions do not recursively attach contents. The menu shows initial scan/connection errors, and failed previews do not prevent inserting a path.

The helper accepts existing UTF-8 regular files up to 1 MiB, inside the selected project. Binary/special files, symlink paths and parent traversal are refused. Unix traversal and reads are anchored to no-follow directory descriptors, including bounded directory scanning. The tree excludes `.git`, `.jj`, `.hg`, `node_modules` and `target`, and is bounded at 20,000 returned entries, 100,000 scanned entries and the transport-size limit. Use **Refresh** to rescan/reconnect; folders do not have filesystem watchers yet. Secure file editing is currently advertised only by Linux and macOS helpers; Windows helpers fail closed until they have an equivalent handle-relative backend.

Open files are polled every three seconds. Clean buffers reload when the remote contents change; dirty buffers keep edits and become conflicted. Revisions bind the exact inode, bytes, owner, group, mode, modification time, ACL and bounded extended attributes. Saves preserve and verify the ownership, mode, ACL and extended attributes before one descriptor-relative atomic exchange. Once the exchange succeeds, no error path rolls back or otherwise mutates the live pathname.

Every save that passes its initial revision and read-only checks reserves a durable, private slot in a project-specific `.pi-file-recovery-<hash>` sibling directory outside the project tree. Its `original`, `boundary`, `installed` and independent `draft` names keep both sides of boundary races and later writes through old open descriptors reachable without appearing in Git/JJ status or project indexing. Projects at a filesystem mount boundary fail closed because an atomic same-filesystem recovery store cannot be placed outside their tree. Slots are never automatically deleted. The helper admits at most 64 per project, then applies backpressure before touching the live file. Android's editor history button lists records, opens each bounded original/boundary/installed/draft artifact as selectable text, and requires a second tap to delete a reviewed record. The same ID is included in uncertain-save errors for manual host inspection. This deliberately trades space for no silent data loss.

On a failed/uncertain save the editor keeps its dirty buffer and blocks saving until an explicit reload/check. It never automatically repeats the write. **Reload** confirms discarding edits, and refuses to replace a buffer that was edited while the read was in flight. Remote create/rename/delete and remote LSP remain deferred. Buffers are currently session-local rather than shared across remote tabs.

## Development and packaging

Build a helper for the **remote** platform:

```sh
cargo build --locked -p pi_remote
```

Without `bundled-backend`, it runs Pi installed on the remote host. A manually copied Unix helper should be activated with `path/to/pi-desktop-remote activate`; activation accepts no destination argument and installs the running executable by SHA-256. Copying a new content-addressed binary alone does not select it for Android: activate it as the paired SSH account, then reconnect the phone to refresh helper metadata. Activation does not stop live session daemons or clear their data. To make the self-contained helper, fetch Pi's matching platform release (for example Linux arm64) and embed it:

```sh
python3 scripts/fetch_pi.py --platform linux-arm64 --notices
PI_DESKTOP_BACKEND_ARCHIVE="$PWD/artifacts/pi/pi-linux-arm64.tar.gz" \
  cargo build --locked -p pi_remote --features bundled-backend
```

Until the new helpers are published in a release, supply the artifact explicitly:

```sh
PI_DESKTOP_REMOTE_HELPER="$PWD/target/debug/pi-desktop-remote" \
  cargo run --locked -p pi-desktop -- --ssh dev --project /srv/my-project
```

The override is a local file to upload, not a remote path; its native binary header must match the detected host platform. It does not disable SSH host verification or uploaded-file checksum verification.

Native packaging scripts and CI publish four desktop archives, four standalone helpers, the Android APK, two Android Local runtime/relink archives, and one checksum manifest. Unix helpers bundle Durable; Windows keeps stock Pi. The helper's `--licenses` prints embedded legal notices.

Tests use a deterministic fake Pi backend, never a model provider:

```sh
cargo test --locked -p pi_core -p pi_remote
cargo test --locked -p pi-desktop ssh_tests
python3 -m unittest discover -s scripts -p test_packaging.py
```

The helper integration tests detach during a streamed/tool-using run, reattach to its partial state, wait for completion, and confirm that a second attachment supersedes the first without duplicating messages. Desktop tests cover remote identity rejection, disconnected-versus-idle state, snapshot replacement, disabled local services, detached remote editor buffers and persisted reconnect targets. File-channel tests cover UTF-8/CRLF, bounds, traversal/symlink refusal, atomic saves, permissions, conflicts and reconnecting without starting Pi. Linux loopback OpenSSH installation and streaming reconnection have also been exercised with verified host keys, SFTP upload, a fish login shell and a project path containing spaces/quotes. Native SSH-server behavior on macOS/Windows still requires live validation.
