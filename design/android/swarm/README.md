# Swarm: independent minds, shared direction

**[Open the interactive study](index.html)** · [Overview PNG](overview.png) · [Android designs](../README.md)

A proposed extension to Pi’s durable subagents: peers can exchange findings, ask questions, publish artifact references and declare dependencies **without sharing a conversation**. Android shows the resulting work and exchanges inside the originating session.

**Design only.** These HTML screens run local, in-memory sample interactions. No Pi execution, SSH, provider calls, persistent storage, scheduler or native UI is implemented here. “Simulate” controls are walkthrough aids, not proposed product buttons.

## The recommendation

Use **logical peer-to-peer communication through a durable broker on the computer**. Do not give agents network addresses, open sockets between them, copy everyone’s transcript into everyone else’s context, or use a shared writable file as a chat room.

```text
Scout     API      UI      Reviewer       independent conversations
  │        │        │        │
  └────────┴─────┬──┴────────┘
            Durable broker                computer / server
     membership · log · mailboxes
       questions · artifact references
                │
        Pi, the coordinator
                │ compact committed projection + on-demand events
          SSH remote helper
                │
           Android / desktop
```

The phone is an observer and control surface. Pi coordinates scope, integration and escalations. A peer message is contextual information from another agent, **not a user instruction, a permission grant, or a new system prompt**.

### What exists already

The durable runtime has background crews, parallel tasks and chains; each child has a durable conversation. It reports progress through `app.subagent-calls`, supplies `get_subagent` / `stop_subagent`, and now supports capability-gated `prompt_subagent` steering and follow-ups. The older [subagent design](../subagents/README.md) describes read-only child screens; that limitation is historical, not the current runtime contract.

Current crews allow up to **200 tasks per call**, run **8 workers per call**, prevent recursive child delegation, and return a consolidated report to Pi. This is not a host-wide concurrency limit or a peer messaging system. Existing chains pass shortened previous answers, not a shared artifact bus.

New here: a broker, scoped peer tools, durable question/dependency records, immutable artifact publication, enforced workspace isolation, bounded swarm projections, and the map/exchange UI. Keep these **opt-in** and capability-gated; do not silently turn every existing crew into a swarm.

## Eight screens

| Screen | Purpose |
| --- | --- |
| [01 · Live map](screens/01-live-map.png) | Four peers and Pi, stable positions, named Working / Waiting / Done states. Scout → API is a finding; API → UI is a question; Reviewer depends on a patch. A peer’s wait is not “Needs you”. |
| [02 · Exchanges](screens/02-exchanges.png) | A readable alternative to the graph. Filter questions or artifacts; inspect sender, recipient, message ID, acceptance and delivery timestamps, and correlated replies. Not a firehose of tool output or hidden reasoning. |
| [03 · One agent](screens/03-agent.png) | Brief, model, tools, isolated worktree, exact blocker, shared artifact and on-demand conversation. Stop one peer without stopping the others; steer Pi from the session. |
| [04 · Decision](screens/04-decision.png) | A separate later scenario: Pi escalates a compatibility decision from API. Selecting is not sending. Answer requires an explicit action; Later keeps the request open. |
| [05 · Twenty-four peers](screens/05-large-swarm.png) | A separate larger swarm grouped as API / UI / Tests / Review. Eight working, five waiting, eleven done. Open all group members, search groups and inspect the dependency affecting other peers. |
| [06 · Offline](screens/06-offline.png) | A frozen observation with a timestamp and **current state unknown**. No Stop, Answer or Send while disconnected. The computer may still be working. |
| [07 · Finished](screens/07-finished.png) | A later completion scenario: exact patch/candidate revisions, a named check with exit code, reviewer evidence and per-agent spend. Candidate work is not automatically integrated into the main workspace. |
| [08 · Evening](screens/08-evening.png) | The same map and labels in Pi’s dark palette. Status is never conveyed by color alone. |

Jumping between scenes explores independent snapshots, not an automatic simulation of a real run. The four-peer sample and 24-peer roster are different swarm IDs. The large roster’s member details and limit controls are illustrative, not fully interactive execution controls.

## Communication design

### 1. Identity and scope

A swarm is owned by **one root session and one crew task**. The authoritative roster contains `swarmId`, `rootConversationId`, lifecycle, revision, stable `agentId`, child conversation ID, assignment epoch, capabilities and allowed workspace. Display names are not addresses.

The broker derives the sender from the executing child’s identity. A child cannot supply another sender, enumerate another session’s roster, address a foreign child, grant tools, create nested swarms or escalate its own permission scope. Topics and artifact access are restricted to authorized members of this swarm.

### 2. Typed messages, separate contexts

Proposed child tools (names are illustrative):

| Tool | Contract |
| --- | --- |
| `swarm_members` | Read authorized peer IDs, roles, status and published interfaces, not their conversations. |
| `swarm_send` | Commit a direct `finding`, `question`, `reply`, `handoff` or `decision` to an authorized recipient. A reply references an open question. |
| `swarm_publish` | Publish a bounded fact or artifact revision to an authorized topic, e.g. `schema-ready`. Explicit subscriptions only. |
| `swarm_wait` | Declare a correlated question/artifact dependency and deadline; yield the execution slot instead of polling or blocking a worker. |
| `swarm_artifact` | Publish an immutable, access-scoped object from the agent’s allowed workspace. Returns an ID and revision; no arbitrary path references. |

Mailboxes are drained at explicit turn boundaries, never injected into a shell command or a tool call already running. Recipients get the sender, type, correlation and payload marked as **agent-originated data**. They retain their own brief and tools. No automatic “every message starts a new reply” rule: notifications coalesce; agents choose whether an answer is necessary.

Example broker record, after sender identity and scope were validated:

```json
{
  "swarmId": "s-84",
  "messageId": "m-13",
  "requestId": "api-03-cursor-question-1",
  "sequence": 13,
  "sender": { "agentId": "api-03", "assignmentEpoch": 1 },
  "recipient": { "agentId": "ui-04", "assignmentEpoch": 1 },
  "kind": "question",
  "correlationId": "cursor-contract",
  "text": "Can the client persist an opaque cursor?",
  "artifacts": [],
  "expiresAt": "2026-10-11T09:44:15Z"
}
```

`sequence` is a **broker commit order**, not a claim about the order in which parallel tools ran. Message IDs and request keys remain stable across retries. The wire record would include schema version and explicit timestamps; the example omits unrelated fields for readability.

### 3. Delivery is not completion

Track these separately:

```text
Message:  accepted → delivered-to-context
                  ↘ cancelled / expired / recipient-ended
Question: open → answered / declined / expired / cancelled / peer-stopped
Agent:    queued / working / waiting / stopping / stopped / done / failed
```

- **Accepted:** committed to the broker and addressed mailbox, not necessarily visible to the recipient yet.
- **Delivered:** atomically admitted to the recipient’s committed context. It does not mean read, understood, agreed with, answered, or that a command ran.
- **Answered:** a valid reply from the designated respondent closed the matching open question. A late/duplicate reply is recorded as such; it cannot unblock a later assignment.
- **Done:** this delegated assignment ended. A passing test is a separate recorded result, and a later follow-up is a different run/epoch.

Use an authoritative assignment epoch **at both admission and consumption**. Existing exact-child steering validates a run at admission; do not assume its current inbox alone prevents a queued input from surviving into a different run. Swarm delivery must verify recipient lifecycle and epoch again in the transaction that appends context and advances the mailbox receipt. A terminal child is not silently resurrected by a late peer message.

### 4. Durability and recovery

Proposed durable documents/tables:

- roster / lifecycle control (authority, not merely display progress),
- immutable bounded message records with per-swarm sequence,
- per-recipient mailboxes and delivery cursors/receipts,
- question/dependency records with respondent and deadline,
- authorized topic revisions/subscribers,
- immutable artifact manifest and scoped content store,
- a separate compact UI projection.

Persist the message and recipient delivery entries in one transaction. A repeated request key with the same canonical payload returns the original receipt; a changed payload is rejected. Retried delivery uses the same message ID. Context admission plus the delivery cursor must be atomic, giving **at-least-once transport, idempotent context insertion**, not an unsupported promise of exactly-once external effects.

After a restart, recover the same children and mailboxes; replay only undelivered entries still valid for the assignment. Reevaluate deadlines against the computer’s clock. The UI marks recovered work “Resumed” only when the computer reports actual recovery, not merely when the phone reconnects.

**Never repeat an interrupted unsafe command just because a message was replayed.** Record unknown/interrupted effects, inspect evidence, then let the coordinator decide on recovery. Transactions cannot make shell commands, network calls or external side effects exactly-once.

### 5. Questions, dependencies and resource limits

Waiting is durable scheduler state, not a long-running `bash` or a loop consuming model calls. A peer waiting on a reply releases its execution slot. Reserve bounded coordinator capacity so all peers waiting cannot starve Pi’s ability to resolve the wait.

Suggested initial policy (new, not existing defaults):

- 24 peers per swarm in the first product cut; expand after measurement.
- Up to 8 executing **peers** per swarm, plus a separately bounded coordinator lane; also enforce host/provider-wide quotas across swarms.
- Direct messages: up to 4 KiB text and four scoped artifact references. Larger content becomes an artifact.
- Bounded per-sender/per-recipient mailboxes and total event quota; explicit backpressure/refusal rather than silently dropping messages. Example starting envelope: 100 pending entries/member, 10,000 committed messages/swarm, rate limit 10 sends/minute/member. Tune with tests.
- Default 2-minute question deadline, bounded configurable maximum; stale questions and dependencies always reach a terminal outcome.
- Estimated token/cost admission budgets plus provider usage reconciliation. Already-running calls may overshoot estimates; a dollar cap is not exact final billing.

Topics store a latest revision plus bounded history. Coalesce redundant progress updates, but never erase an outstanding question or its terminal outcome. A topic update is not an implicit broadcast permission or automatic task spawning.

Detect dependency cycles and expire waits; report the cycle to Pi without claiming every wait is a deadlock. Peer failure/Stop closes its open dependencies with reasons. Pi can reassign with a **new epoch**, decline the work or ask the user. It cannot reuse a late reply as authority for another assignment.

### 6. Shared knowledge without shared writers

Prefer immutable findings, patch bundles and exact revision references over a shared mutable scratchpad. If a shared task board is useful, give it one broker-owned revision and compare-and-swap updates; conflicting writes become explicit events for Pi to reconcile.

Each editing peer should use a separate worktree/sandbox. Scout and Reviewer get read-only workspace access. Pi serializes integration, checks the base revision, and publishes an integration candidate. Reviewer checks that exact candidate—not whatever happens to be in a changing folder.

**Tool labels alone do not enforce read-only access.** `bash` can write. Enforcing workspace boundaries needs an actual tool broker/sandbox, command policy and controlled integration path. Existing agent tools are not a complete security boundary. Until isolation is implemented, the conservative first phase is read-only peer messaging with a single coordinator writer; do not advertise safe parallel editing merely because messages exist.

Published artifacts have swarm-scoped IDs, immutable bytes/digests, size limits and owner metadata. Receivers retrieve authorized bytes by ID; a peer cannot exploit `../../`, a symlink, or an invented path to read host secrets. A new version is a new artifact revision. Artifact references do not confer write permission or arbitrary computer-file access.

## Visualization & phone contract

- **Map is logical, not physical.** Solid arrows mean an explicit peer exchange; dashed lines show assignment or dependency. The separate architecture diagram explains the physical broker. No animated packets or force-directed layout implying live traffic.
- **Stable layout:** retain positions through status changes and scrolling. The four-peer layout is intentionally simple; beyond roughly six peers, group by role and show a selected peer’s one-hop neighborhood rather than a 24-node hairball.
- **Tap → inspect, never execute:** agent nodes open details; the question marker opens its exchange. Questions list and Agents list are full text alternatives to the SVG. Edge hit targets and all controls are at least 48 dp, keyboard reachable, and named.
- **Labels before hues:** every status has text. Working is not Waiting, Waiting is not Needs you. A stopped/failed peer remains visible with its causal impact instead of disappearing from the graph.
- **Exchanges, not reasoning:** explicit shared messages, published findings and observed tool activity only. Private reasoning is neither requested nor visualized. Render text as text; never interpret a message as executable HTML or a command to Android.
- **Compact projection:** counts, member IDs, status, latest step, outstanding blockers, event cursor, spend and artifact metadata. Refresh at most around once a second/coalesce. Fetch full exchange pages or one conversation on demand with pagination; do not ship all transcripts with every tick.
- **User input:** steer Pi by default, with a visible recipient. Existing exact-child steering may remain in the agent’s conversation where the runtime advertises it; a message to one agent must never silently broadcast. Scoped questions from Pi are the only “Needs you” notifications.
- **Stop:** confirmation names the scope. The sample explicitly separates requesting Stop from simulating the computer’s acknowledgement. Show Stopping until acknowledged; stopped peers’ pending mail/waits close, other peers continue. Session Stop stops Pi and all peers. Stop keeps observed edits and does not undo external effects.
- **Offline:** freeze the projection with last-seen time; current state is unknown. Local drafts may be retained, but no hidden offline actions are admitted. On reconnect, resume from the cursor, validate identity/lifecycle and refresh before offering control.
- **Review:** opening the candidate/diff is inspection. Integration into main would be a separate, confirmed operation with base-revision validation. Tests and finished assignments never imply user acceptance.
- **Accessibility:** real buttons, visible focus, text alternatives, 48 dp targets, scrollable long content, modal focus containment, Escape/Back close sheets first. No motion is required; honor reduced motion and Android text scaling in a native implementation.

## Integration seams and phased build

### Phase 1 — bounded, read-only messaging

Add an opt-in `swarm` tool/extension and a **new versioned crew task kind**. Reuse child creation/model/resource selection, but preserve persisted `app.subagent-crew` semantics. The first version supports direct findings/questions/replies and correlated waits, a roster, one coordinator writer, no topics or peer worktree integration yet.

Primary seams:

- `backend/durable/src/subagents.ts`: `subagentExtension`, `Crew` / `CrewRun.child`, child definitions and conversation creation. Extract shared helpers rather than changing existing crew recovery meaning.
- `backend/durable/src/run.ts`: root-owned child resolution, request admission, committed snapshot/subscription paths. Add swarm capabilities; do not treat `app.subagent-calls` display summaries as authoritative mailbox membership.
- Proposed RPCs: `get_swarm` (bounded projection), `get_swarm_events(swarmId, afterSequence, limit)`, and guarded coordinator decisions. Reuse session/child abort mechanisms through swarm lifecycle handling rather than creating a second competing Stop authority. Requests validate root ownership, swarm ID, expected epoch/version and stable request ID server-side.
- `crates/pi_remote/src/durable.rs`: translate new durable swarm projections/events without expanding ordinary tool-result transcripts.
- `crates/pi_core/src/subagent.rs`: extend shared models with feature-gated swarm/member/message types; retain ordinary handoff parsing.
- `crates/pi_android/src/projection.rs`, `live.rs`, `screens/subagents.rs`: session-owned entry point, cached map, paged exchanges and existing child conversation screen. Negotiate runtime capabilities. Old helpers keep the ordinary crew view, not a fabricated live graph.

Test scoped identity, forged senders, duplicate requests with changed payloads, concurrent replies, duplicate delivery, epoch changes before consumption, late reply after Stop, terminal child input, missed deadlines, restart between admission/delivery, all peers waiting, and parent Stop races.

### Phase 2 — artifacts and safe parallel writers

Implement scoped artifact storage, immutable revisions, real workspace/tool isolation, controlled integration and exact-revision review. Add authorization/path/symlink tests and verify peers cannot write across worktrees. Introduce explicit patch readiness/dependencies and conflict reporting.

### Phase 3 — larger swarms and observation

Authorized topics, grouped maps, filtered paginated history and aggregate budgets. Add bounded projection/backpressure/host-wide quota tests, old-helper capability tests, event-gap recovery and Android accessibility/device screenshots. No background-service claim: the current app’s background-connection limitations still apply; a foreground design study does not solve them.

## Try & reproduce

Open **`index.html`** directly, in Android’s Page preview, or serve it locally. It is a **self-contained document**: CSS, JavaScript and fonts are embedded; no neighboring files or network access are needed to render the eight screens or use the interactive flow. This matters because Android’s preview receives only the HTML, not the checkout. Documentation/overview links refer to separate repository files and are not required for the preview itself.

Edit `index.template` for the page shell, `swarm.css` for styling and `swarm.js` for interactions, then rebuild the generated `index.html`. Keep the generated file checked in. The script runs after the markup so the gallery also initializes when the HTML is loaded as a string, with no deferred external script.

**Try the flow** opens a navigable phone preview. Tap nodes, the question marker, filters and artifact cards; try an explicit decision, cancelled/confirmed Stop, draft preservation and the offline scene.

```sh
node design/android/swarm/build.cjs          # dependency-free HTML bundling
node design/android/swarm/build.cjs --check  # verify the generated document is current
node design/android/swarm/render.cjs --check  # local browser smoke checks only
node design/android/swarm/render.cjs         # checks + 8 screenshots + overview
```

Needs an existing `puppeteer-core` and Chromium (`CHROME=/path/to/chrome`), or an existing `puppeteer` installation with its browser. The renderer rebuilds the HTML first. It does not install packages or fetch a browser. It also loads only the HTML text into an isolated browser page with **all external requests blocked**, asserting that styles, embedded fonts, eight screens, themes and interactions still work. It checks navigation/receipts/filters/Stop/decisions/dialog focus/drafts, and checks widths 320–1380 plus landscape phone layouts. Native-runtime tests are future implementation work, not tests claimed by this prototype.

The visual direction uses Moonstone and Evening, embedded repository IBM Plex Sans / Commit Mono fonts, ordinary fills, borders, radii and simple vector connectors. A GPUI implementation should paint the stable connectors with paths; it does not need a webview or graph-layout library. Names, IDs, costs, timestamps, revisions, check results and code are sample content.
