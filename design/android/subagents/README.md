# Subagents

**A mock for work Pi hands off to other agents: scouts side by side, a chain that passes its result along, and what carries on after the computer restarts. A subagent always shows inside the session that started it.**

[Overview PNG](overview.png) · [Gallery](index.html) · Built on [next4](../next4/README.md)'s stylesheet and proportions.

## Screens

| Screen | What it shows |
| --- | --- |
| [01 Handed off](screens/01-working.png) | A stage on the run line, **Handed off**, with a card of the subagents it started: each one's task, what it is doing now, its time and its own small run line. Stages still ahead name the agent that will do them. The strip says what Pi is waiting on. |
| [02 A subagent](screens/02-subagent.png) | One subagent opened from the card: what Pi asked it, then its own run line. It is read-only: only Pi talks to a subagent. **Stop this scout** ends it, and Pi is told it was stopped; the others carry on. |
| [03 Done, with a chain](screens/03-done.png) | The finished run line with each subagent's stretch named, Pi's answer, the chain as numbered steps that each open, and the spend split between Pi and each agent. |
| [04 Picked up after a restart](screens/04-resumed.png) | The computer restarted mid-chain. After reconnecting, the worker carries on and is marked **Resumed**. A command the restart cut off is said plainly: it was not repeated by itself, and the worker was told. The reviewer waits its turn. |
| [05 Home](screens/05-home.png) | Working sessions show their subagents as small overlapping tiles in their agents' colours, live ones ringed, with one line of what they are doing. Subagents are never listed as sessions of their own. |
| [06 Agents on the computer](screens/06-agents.png) | Resources gains **Agents**: each agent's name, model, what it is for and its tools. A project's own agents are held back until Pi trusts the project. |
| [07 Evening](screens/07-evening.png) | Screen 01 in the dark theme. |

## How it behaves

- **A subagent belongs to its session.** It appears under the stage that started it, on Home in its session's row, and in notifications as its session. Stopping the session stops its subagents.
- **Only Pi talks to a subagent.** Its screen has no composer. You steer the session, and Pi decides what to pass on.
- **Each agent keeps one colour and icon:** scout reads, planner plans, worker edits, reviewer checks. A chain reads at a glance in the card, on the run line and in the spend.
- **Where the time and money went:** each subagent shows its time and cost, and the finished session splits the total by agent. Subagents can use other models, so the model is named wherever it differs from the session's.
- **Honest after a crash:** "Resumed" marks work that carried on after a restart. A command that was cut off is named, and it is never silently rerun: the agent is told and decides.

## What the computer provides

Subagents run in the durable backend: each is a conversation owned by the tool call that started it, so stopping the parent stops it, and after a restart it is found again instead of started twice. The runner's `subagent` tool takes the same `agent`/`task`, `tasks` and `chain` parameters as stock Pi's subagent extension, and reads agents from the same `~/.pi/agent/agents/*.md` files.

The card needs a little more than the session already sends: the tool's details carry each subagent's task, state, latest step, time, cost and model, and a new `get_subagent` request returns one subagent's messages when its screen opens, so whole transcripts never ride along with every update.

## Built

The durable runner's `subagent` tool, the card, the subagent screen with Stop, and the tiles on Home are in the app; see [the durable README](../../../backend/durable/README.md#subagents). `cargo run -p pi_android --example preview -- subagents` (also `subagents-done` and `subagent`) shows them with sample data. Not built yet: the Agents tab of Resources (screen 06) and the spend bar of screen 03; the run line keeps its four stages, and the stage Pi was in says who it is waiting on.

## Reproduce

```sh
swift design/android/subagents/render.swift            # every screen and overview.png
swift design/android/subagents/render.swift 03-done    # one screen
```

The renderer uses the system's WebKit on macOS, with no browser to install, and makes no network calls.
