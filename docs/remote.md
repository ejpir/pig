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

A slow client is detached rather than blocking agent execution. Records and snapshots have the existing 16 MiB transport limit. IPC listens only on loopback and authenticates using a random token stored in the private per-user state directory. Unix state directories/files use 0700/0600; Windows relies on the user's profile ACLs. Startup diagnostics are stored beside endpoints in `.pi/desktop/run/1/<key>.log`.

**Stock Pi remains the default.** Its runs survive SSH/desktop disconnects, but a daemon/host crash does not automatically continue the interrupted run. Reopening can resume the saved conversation; it does not replay the prompt or tools. Idle daemons are not automatically evicted yet.

## Experimental durable backend

Choose **Durable · experimental** in the SSH form, or pass `--remote-backend durable` for a new SSH session. The helper must be built with `bundled-durable` (normal release packaging does not enable it yet), or have `PI_DESKTOP_DURABLE_RUNNER` configured on the SSH host. An older/unsupported helper is rejected before starting a stock agent. Reconnect never changes an existing session's backend or migrates its stock session file.

The same detached Rust daemon supervises a standalone Bun runner containing pinned pi-durable 1.0.2. No Node/Bun installation is needed on the SSH host. SQLite with WAL and `synchronous=FULL` is authoritative; desktop snapshots are disposable projections of committed durable state. Checkpoints live under `~/.pi/desktop/durable/<key>/`, independently of helper binary caches and daemon endpoints. Both the Rust storage owner and its runner hold the same Unix OS writer lock, including when the owner is killed.

Disconnect keeps work running. **Reconnect after an agent/daemon/host crash** opens storage and resumes unfinished work and the persisted inbox. This prototype has no unattended boot service or automatic daemon restart. Safe tools may rerun; interrupted unsafe tools report an error instead of automatically repeating their side effects. The model can still choose further actions; durable recovery is not a guarantee of task success or exactly-once external effects.

Prompt admission requires a stable `requestId` and acknowledges only after durable storage commits. The desktop creates a fresh key per send; it does not automatically replay an uncertain prompt and does not yet persist a crash-safe desktop outbox. A protocol client may query `get_submission` using that key and explicitly retry the same payload/key; different payloads with that key are rejected. The prototype retains up to 10,000 prompt receipts per conversation, refusing new admissions rather than expiring keys.

Currently supported: text prompts, read/write/edit/bash tools, subagents (stock Pi's `subagent` tool and agent files), streaming committed snapshots, steering/follow-ups, stop/clear queue, manual compaction, naming, model/thinking selection and reconnect. The runner reuses stock Pi's model/auth SDK on the SSH host: stored API keys and OAuth/subscription tokens in `~/.pi/agent/auth.json` (or the host's `PI_CODING_AGENT_DIR`), all built-in providers, global `models.json` configuration and cached catalogs, and the host's prompt templates and skills (see the [durable backend README](../backend/durable/README.md#prompt-templates-and-skills)). Token refresh uses stock Pi's file locking; auth/model metadata excludes secrets and the credential store is not copied to desktop snapshots or durable state. **Models → Refresh** reloads host configuration without restarting. Catalog discovery stays offline/cache-only; sign in/populate dynamic catalogs with stock Pi on the host. **Limits:** Unix SSH hosts only (Linux arm64 locally validated), no interactive durable login flow, stock extensions, Pi packages, forks or stock session-tree migration. Unsupported commands fail explicitly; nothing falls back to stock Pi or desktop services. Files remain independent and work with either engine.

See [`backend/durable/README.md`](../backend/durable/README.md) for native builds, no-provider crash tests and a Mac loopback walkthrough. This is the prototype before a default-backend switch; compatibility/authentication gates still need to pass.

## First-release scope

Remote chat, model/thinking controls, agent tools, steering, follow-ups, stop and reconnect are supported. The helper routes desktop-extension requests on the remote host, preserving stock Pi extensions.

Remote file browsing and text editing are supported through **Files**, and the composer's **@** menu supports remote files, directories and file previews. Terminals, LSP symbols/problems, jj, Tree/Changes/Context views, forks, saved-session file management, HTML export, and explicit `!`/`!!` shell submissions are not supported yet. Agent `bash` tools still run remotely. Remote file buffers have no local filesystem handle or local language-server project; terminal services remain disabled.

## Remote files

Open **Files**, select a file, edit it and use **Save** / ⌘S (Ctrl+S on Linux/Windows). The existing Zed editor supplies syntax highlighting and normal buffer/undo/dirty-state behavior. Reads and saves run through an independent `helper files --stdio` SSH channel, not through Pi or its controlling attachment. File browsing alone does not start an agent or call a model provider.

Type **@** in the composer to load/search the remote tree; you do not need to open Files first. **Enter** inserts a file/directory chip; **Tab** completes its path. Pi receives project-relative path references and reads them on the remote host, not desktop paths or attached file contents. File previews use the same bounded SSH read service, never a desktop filesystem fallback; directory mentions do not recursively attach contents. The menu shows initial scan/connection errors, and failed previews do not prevent inserting a path.

The helper accepts existing UTF-8 regular files up to 1 MiB, inside the selected project. Binary/special files, symlink paths and parent traversal are refused. The tree excludes `.git`, `.jj`, `.hg`, `node_modules` and `target`, and is bounded at 20,000 entries and the transport-size limit. Use **Refresh** to rescan/reconnect; folders do not have filesystem watchers yet.

Open files are polled every three seconds. Clean buffers reload when the remote contents change; dirty buffers keep edits and become conflicted. Saves compare the SHA-256 of the exact previously read bytes, then write through a same-directory temporary file and atomic replacement while preserving permissions. Cooperating desktop saves are locked; arbitrary tools on the host do not participate in that lock, so this is optimistic conflict detection, not a transaction or an OS sandbox.

On a failed/uncertain save the editor keeps its dirty buffer and blocks saving until an explicit reload/check. It never automatically repeats the write. **Reload** confirms discarding edits, and refuses to replace a buffer that was edited while the read was in flight. Remote create/rename/delete and remote LSP remain deferred. Buffers are currently session-local rather than shared across remote tabs.

## Development and packaging

Build a helper for the **remote** platform:

```sh
cargo build --locked -p pi_remote
```

Without `bundled-backend`, it runs Pi installed on the remote host. A manually copied Unix helper should be activated with `path/to/pi-desktop-remote activate`; activation accepts no destination argument and installs the running executable by SHA-256. To make the self-contained helper, fetch Pi's matching platform release (for example Linux arm64) and embed it:

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

Native packaging scripts and CI build both desktop and bundled helper, publishing four desktop archives and four standalone helper executables with one checksum manifest. The helper's `--licenses` prints embedded legal notices.

Tests use a deterministic fake Pi backend, never a model provider:

```sh
cargo test --locked -p pi_core -p pi_remote
cargo test --locked -p pi-desktop ssh_tests
python3 -m unittest discover -s scripts -p test_packaging.py
```

The helper integration tests detach during a streamed/tool-using run, reattach to its partial state, wait for completion, and confirm that a second attachment supersedes the first without duplicating messages. Desktop tests cover remote identity rejection, disconnected-versus-idle state, snapshot replacement, disabled local services, detached remote editor buffers and persisted reconnect targets. File-channel tests cover UTF-8/CRLF, bounds, traversal/symlink refusal, atomic saves, permissions, conflicts and reconnecting without starting Pi. Linux loopback OpenSSH installation and streaming reconnection have also been exercised with verified host keys, SFTP upload, a fish login shell and a project path containing spaces/quotes. Native SSH-server behavior on macOS/Windows still requires live validation.
