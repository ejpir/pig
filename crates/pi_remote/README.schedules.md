# Host schedules

Supported: Linux and macOS with a bundled durable helper, or an explicitly configured native `PI_DESKTOP_DURABLE_RUNNER`. **Windows durable scheduling is unsupported.** Stock Pi is never a fallback.

## Explicit local setup

```sh
pi-desktop-remote scheduler status
pi-desktop-remote scheduler start                 # detached, until stopped/logout/reboot
pi-desktop-remote scheduler stop                  # existing sessions continue
pi-desktop-remote scheduler install --confirm     # opt-in login autostart; does NOT start now
pi-desktop-remote scheduler start
```

Installation writes/enables a systemd **user** unit on Linux, or writes a launchd LaunchAgent on macOS. Installation does not use sudo, configure Linux lingering, activate/replace the helper binary, or start the service. To remove autostart, disable/delete the named user unit (`pi-desktop-scheduler.service`) or LaunchAgent (`org.pi-desktop.scheduler.plist`) locally. Services pin the executable used for installation; reinstall explicitly after moving/replacing it. The host must be awake; Linux pre-login operation additionally needs explicitly configured lingering. No phone command installs or starts anything.

The service deliberately leaves separately owned session daemons alive when the scheduler stops (`KillMode=process` / `AbandonProcessGroup`). Worker locks are not bypassed. The scheduler is an ordinary helper client, not a UI controller or a direct runner owner.

## Phone/desktop API

`schedules --stdio` reads **one** JSON request line (maximum 512 KiB), writes **one** version-1 `pi_core::schedules::Response` line, and exits. It does not wait for stdin EOF or subsequent lines. Paired SSH keys allow exactly this argument vector, never scheduler lifecycle commands. Unknown request fields, including correlation-envelope `id`, are rejected. Schedule identity is `job_id`; epoch timestamps are unsigned milliseconds.

Definitions and the occurrence ledger live in `~/.pi/desktop/schedules/schedules.sqlite3`, outside session endpoints and binary caches. `PI_DESKTOP_REMOTE_STATE_DIR` relocates all helper state, including this scheduler directory (used by isolated tests). The directory/files are private; SQLite uses WAL and FULL synchronous commits, serialized immediate claim transactions, unique job/UTC recurring identities and a separate confirmed manual-operation identity. History returns the newest 500 rows; the complete ledger is retained.

Enabled creation, enabling/configuration edits and manual runs require explicit consent. Consent is an acknowledgment of project trust and execution with host-account permissions, **not sandboxing or enforced read-only access**. Pause alone does not require renewed consent. Revisions protect mutations. Edits/deletion never change existing payload snapshots or remove sessions/project files. `Run now` reserves a Pending occurrence, including for paused jobs; this is **not** a durable admission receipt. Reusing its confirmed `request_id` and revision returns the original operation rather than a second occurrence.

## Execution policy

* Numeric POSIX five-field cron: numbers, `*`, lists, ranges and steps. Restricted day-of-month/day-of-week use standard OR semantics. A real IANA timezone is required. Previews allow 1–20 future instants, bounded to an eight-year calendar search.
* Nonexistent local minutes are skipped. Repeated minutes use only their first UTC instant, including interval schedules. UTC identities do not include job revision.
* Due times have a one-minute admission grace. Older times are missed. `skip` fast-forwards without replay. `once` admits at most one actual missed instant in a **24-hour** catchup window; older times are summarized as Missed. The ledger coalesces missed ranges rather than writing an unbounded row/backlog per minute.
* An active **or unresolved** predecessor causes same-job occurrences to be Skipped. Committed queue entries and still-running/waiting durable subagents also keep the occurrence active. This does not isolate different jobs sharing a project.
* Exact prompt payload, model, session key and durable request key are committed before dispatch. Model selection is session-local (`persist=false`). Admission and completion are separate; only committed receipt lookup and settled helper state can settle history. Completed means the agent settled, not verified task success or exactly-once external effects.
* Restart reopens unresolved sessions through normal helper attachment/ownership and looks up their original receipt. Original prompts are **never resubmitted** after dispatch intent. A crash in the intent-before-send gap can therefore leave an Unknown occurrence without a receipt; it continues to block same-job overlap. Inspect the linked durable session before deciding on any new confirmed action. There is no automatic uncertain retry or administrative force-clear in this version.
* Timeout persists a best-effort Stop request once. A Stop acknowledgment does not imply completion; an unavailable worker or lost Stop acknowledgment can leave work unresolved. This is not a hard time/cost/security boundary. Interrupted unsafe tools remain the durable worker's responsibility and are not blindly repeated by the scheduler.

Health distinguishes missing login service, stopped scheduler, stale heartbeat, unavailable runner and unsupported platform. Running without autostart is valid but won't survive reboot. A stopped/missing service doesn't prevent reviewing and saving definitions; it does prevent execution. Cached phone state cannot authorize offline mutations.

## Offline checks

```sh
cargo test -p pi_remote --bin pi-desktop-remote
cargo test -p pi_remote --test schedules
cargo test -p pi_core pairing --lib
cargo build -p pi_remote
# Optional real-helper/faux-only runner smoke, using the explicitly built fixture:
PI_DESKTOP_TEST_DURABLE_RUNNER=/absolute/path/to/pi-desktop-durable-fixture \
  cargo test -p pi_remote --test schedules -- --ignored
```

Tests use temporary state/projects, fake clocks, an in-memory fake durable worker and isolated CLI HOME/config directories. No service is installed and no provider or credentials are used. Service-manager behavior is tested through escaped template generation, not by touching the user's launchd/systemd configuration.
