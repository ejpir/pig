# Schedules on the computer

**[Open the self-contained HTML study](index.html)** · [Overview PNG](overview.png)

A proposal for cron-like Pi jobs managed from Android. This is an interactive design study, **not an implemented scheduler**. It never invokes Pi, connects over SSH, installs a service, calls a provider or creates a real job. Changes live only in the page until reload.

## Look around

The gallery has six screens, in the existing Moonstone and Evening palettes, with 20 px gutters, 48 px primary touch targets, quiet surfaces and the bottom action dock. Fonts use local/system fallbacks; all CSS, JavaScript and SVG icons are inline. Opening `index.html` needs no server or network.

| Screen | What it shows |
| --- | --- |
| [01 · Schedules](index.html?screen=list&single=1) | Sessions / Schedules under one computer. The next occurrence, active and paused jobs, and the host-owned execution contract. |
| [02 · New schedule](index.html?screen=new&single=1) | Project, model and instruction; daily, weekdays or numeric five-field cron; explicit timezone and next-run previews. |
| [03 · Schedule](index.html?screen=detail&single=1) | The recurring instruction, enable switch, execution policy, recent occurrences, Edit and Run now. |
| [04 · Finished run](index.html?screen=result&single=1) | A scheduled occurrence as a normal session. Finished execution, passed checks and unresolved findings are different facts. |
| [05 · Unreachable](index.html?screen=offline&single=1) | Last-known schedules are marked cached and unknown. Editing and submission wait for reconnection. |
| [06 · Evening](index.html?screen=list&single=1&theme=evening) | The same hierarchy in the dark theme. |

Choose **Try the flow** for actual local interactions. Additional routes show [run history](index.html?screen=history&single=1), [admission, not completion](index.html?screen=admitted&single=1), and [host setup required](index.html?screen=setup&single=1). History includes completed, interrupted, skipped and missed occurrences.

The sample clock is **Monday 12 October 2026 at 08:42 CEST**. Dates, costs, findings and file paths are illustrative. Cron previews use that fixed clock and a 14-day lookahead; they are not a production scheduling engine.

## Interaction contract

- A schedule is not a session. Each occurrence gets a fresh durable session; no shared chat context by default.
- Creation has two steps: **Review schedule**, then explicit consent and **Enable schedule**. The confirmation names the project and host and shows upcoming dates.
- Edits are reviewed again. Editing a paused schedule keeps it paused. Edits apply to future occurrences, not to already admitted work.
- **Pause** stops future occurrences, not existing sessions. **Delete schedule** removes future scheduling, keeping existing sessions and project files.
- **Run now** confirms before starting a fresh occurrence, including when the schedule is paused. It does not change the next recurring time. The prototype shows an admission illustration, never pretends to execute work.
- No overlapping occurrences of the same job. An occurrence while its predecessor is active is recorded as skipped. This is not cross-job workspace isolation; parallel editing jobs would need an explicit worktree/isolation policy.
- Missed times are skipped by default. An optional **Run once when back** policy coalesces missed times into one catch-up; never an unbounded replay backlog.
- Timezone belongs to the schedule, not the phone. The backend should skip nonexistent DST times and fire once at the first instant of a repeated local time. Persist scheduled instants in UTC.
- The timeout requests Stop; it is not a hard execution, cost or security boundary.
- Project trust is required. An instruction such as “Don’t edit files” is not enforced read-only access. Pi runs with the host account’s capabilities, not in a sandbox.
- **Accepted** means committed admission. **Completed** means the agent settled; neither means verified task success or exactly-once external effects.
- Interrupted unsafe tools are not blindly repeated. Their session explains what stopped so the user/model can inspect its effects before deciding what to do next.
- Unreachable is not paused. The host may still be working, but cached state cannot authorize mutation. No hidden phone-side retry queue.
- A host lacking scheduler support or an installed scheduler service shows setup, not a phone-owned timer. Host setup is explicit and requires a helper capability separate from `durable`.

## Implementation direction (proposed)

Keep a per-user host scheduler alongside `pi_remote`, separate from individual session daemons. Desktop and Android are clients of the same host-level schedule API. Do not attach the scheduler as a session's UI controller or start the internal Bun runner directly; reuse the helper's ownership/attachment paths and writer locks.

Run the scheduler under a launchd user service on macOS or systemd user service on Linux. User services ordinarily start at login; unattended pre-login Linux operation requires explicitly configured lingering or equivalent service setup. The host must still be awake. A missing host service, a stopped service, an unsupported helper and an unreachable computer are distinct states. Windows needs separate durable ownership/service validation.

Persist definitions and an occurrence ledger in a separate scheduler database, outside session endpoint directories and immutable binary caches:

```text
job:        id, revision, name, cwd, prompt, model, cron, timezone,
            enabled, overlap_policy, missed_policy, timeout, next_due_utc
occurrence: id, job_id, scheduled_for_utc, job_revision, payload_snapshot,
            session_key, request_id, admission_receipt, status, outcome
```

Transactionally claim each due occurrence and save its exact prompt/settings, session key and stable request key before dispatch. Use a unique `(job_id, scheduled_for_utc)` occurrence identity; changing a job revision must not admit the same scheduled instant twice. Preserve a separate stable key for an explicitly confirmed Run now operation.

Dispatch through the existing durable admission path. Configure the saved model, submit the persisted request key, and record the receipt. `RpcClient::send(Command::Prompt)` generates a fresh key, so a scheduler must preserve its own key through `send_custom` or a new typed admission API. After an uncertain outcome, query `get_submission`; do not retry with a new identity or changed payload. Stable keys deduplicate admission, not arbitrary filesystem/network side effects.

Watch committed run state independently of admission, retain the session link, and update history on completion/interruption. Reopen unfinished durable sessions after a host restart without resubmitting their original prompts. Notifications can use the existing watcher path while connected; disconnected-phone push delivery is a separate feature, not assumed by this design.

Suggested host operations: list/create/update/pause/delete schedule, preview next occurrences, confirm a manual occurrence, read occurrence history and inspect scheduler health. Mutations require connectivity and a revision check. The real backend should use a maintained cron/timezone library, fake-clock tests and crash/recovery tests rather than this browser preview parser.

## Reproduce and check

Open `index.html` directly in a browser. To render the six 412 × 915 phone images and the gallery, use Node, an existing `puppeteer-core` installation and local Chromium:

```sh
node design/android/schedules/render.cjs
# Checks only; does not overwrite rendered PNGs:
node design/android/schedules/render.cjs --check
```

Set `CHROME` to your browser executable if it isn't at one of the paths detected by the script. The renderer does not install dependencies or start a server. On the development machine it uses the already installed Puppeteer Core and `/opt/google/chrome/chrome`.

Smoke checks cover navigation, pause/resume, explicit Run now confirmation, invalid cron rejection, timezone previews, enablement consent, escaped user text, schedule deletion, offline/setup states, skipped/interrupted history, unique IDs and page overflow at 360/412/768/1280 px. The checks assert no browser JavaScript errors and no HTTP(S) requests. They do not validate the future host scheduler.
