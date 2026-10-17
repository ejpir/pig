//! A session on the computer, as Pi's state, turned into what the phone shows:
//! each prompt with the stages its tools went through, the files Pi changed,
//! the last check it ran, and how the run ended. Nothing is made up: a stage
//! appears only once a tool of its kind ran, and Review shows Pi's own diffs.

use crate::model::*;
use pi_core::session::{Session as Pi, Tool};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A prompt the phone still owns while the helper admits it.
pub struct PendingPrompt {
    pub text: String,
    pub key: Option<QueueKey>,
    /// This was admitted while the session was idle, so it starts a fresh turn
    /// even if `agent_start` reaches the phone before the user-message snapshot.
    pub starts_turn: bool,
}

/// What the phone knows about a session besides Pi's state.
pub struct Facts<'a> {
    pub id: SessionId,
    /// The project folder on the computer, and as shown: `~/repos/pi`.
    pub cwd: &'a str,
    pub folder: String,
    /// Pi's open question, from an extension.
    pub question: Option<Question>,
    /// Prompts sent from the phone that Pi hasn't taken yet.
    pub outbox: &'a [PendingPrompt],
    pub key: &'a str,
}

pub fn project(pi: &Pi, facts: Facts) -> Session {
    project_queue(pi, facts, |_| true)
}

/// Filter only the UI queue; the authoritative session/history stays untouched.
/// Indices are in steering-then-follow-up order, including hidden rows.
pub fn project_queue(pi: &Pi, facts: Facts, visible: impl Fn(usize) -> bool) -> Session {
    project_queue_keys(pi, facts, visible, |index| {
        pi.queued_submissions
            .get(index)
            .filter(|id| !id.is_empty())
            .cloned()
            .map(QueueKey::Durable)
    })
}

/// Live may preserve a local request key after its correlated durable receipt.
/// Both visibility and identity are read from the same authoritative snapshot.
pub fn project_queue_keys(
    pi: &Pi,
    facts: Facts,
    visible: impl Fn(usize) -> bool,
    key: impl Fn(usize) -> Option<QueueKey>,
) -> Session {
    let (turns, accounted) = turns(pi, facts.cwd);
    let working = pi.busy() || !facts.outbox.is_empty();
    let last = pi.messages.iter().rev().find(|m| m["role"] == "assistant");
    let stop = last.and_then(|m| m["stopReason"].as_str()).unwrap_or("");
    let state = if facts.question.is_some() {
        State::NeedsYou
    } else if working {
        State::Working
    } else if stop == "aborted" || stop == "toolUse" {
        // A run that ended on a tool call, with no answer after it, was stopped.
        State::Stopped
    } else if stop == "error" || pi.error.is_some() && pi.messages.is_empty() {
        State::Failed
    } else {
        State::Done
    };
    let failure = (state == State::Failed).then(|| {
        last.and_then(|m| m["errorMessage"].as_str())
            .map(str::to_owned)
            .or_else(|| pi.error.clone())
            .unwrap_or_else(|| "failed".into())
    });
    let mut turns = turns;
    for turn in &mut turns {
        turn.visual_identity = turn
            .visual_identity
            .take()
            .map(|identity| format!("{}:{identity}", facts.key));
    }
    // The phone's prompt shows at once, before the computer has it.
    let optimistic = facts.outbox.first().filter(|prompt| prompt.starts_turn);
    if let Some(prompt) = optimistic {
        let mut turn = Turn::new(prompt.text.clone(), clock_now());
        turn.stages.clear();
        turns.push(turn);
    }
    if let Some(turn) = turns.last_mut() {
        settle(turn, state, pi, facts.cwd, optimistic.is_none());
        // What Pi is doing has gone on since the last message; a question
        // has waited for you since then.
        let now_doing = if state == State::NeedsYou {
            Some(StretchKind::Wait)
        } else {
            turn.live_stage().map(StretchKind::of)
        };
        // Untimed messages still put what Pi is doing at the end of the line.
        if state.is_running()
            && let Some(kind) = now_doing
        {
            turn.add_time(
                kind,
                accounted.map_or(Duration::ZERO, |since| now().saturating_sub(since)),
            );
        }
    }
    let started = pi
        .messages
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(timestamp);
    let ended = last.and_then(timestamp);
    let elapsed = match (started, state.is_running(), ended) {
        (Some(start), true, _) => now().saturating_sub(start),
        (Some(start), false, Some(end)) => end.saturating_sub(start),
        _ => Duration::ZERO,
    };
    let mut queued_keys = Vec::new();
    let mut queued: Vec<String> = pi
        .steering
        .iter()
        .chain(&pi.follow_up)
        .enumerate()
        .filter(|(index, _)| visible(*index))
        .map(|(index, text)| {
            queued_keys.push(key(index));
            (text, index < pi.steering.len())
        })
        // Nobody typed an internal report. Say whether it is joining the live
        // run or waiting for a separate turn instead of making both look like
        // an ordinary queued follow-up.
        .map(|(text, steering)| match pi_core::subagent::report(text) {
            Some(_) if steering => "Subagents reported back · steering this run".to_owned(),
            Some(_) => "Subagents reported back · next turn".to_owned(),
            None => crate::visualize_prompt::label(text),
        })
        .collect();
    queued.extend(
        facts
            .outbox
            .iter()
            .skip(usize::from(optimistic.is_some()))
            .map(|prompt| {
                queued_keys.push(prompt.key.clone());
                prompt.text.clone()
            }),
    );
    let project = facts
        .cwd
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(facts.cwd)
        .to_owned();
    Session {
        local: false,
        backend_features: match &pi.backend {
            pi_core::session::BackendInfo::Found(info) => {
                serde_json::from_value(info["uiCapabilities"].clone()).ok()
            }
            _ => None,
        },
        id: facts.id,
        title: title(pi, facts.outbox),
        project,
        folder: facts.folder,
        state,
        activity: if optimistic.is_some() {
            "Working".into()
        } else {
            activity(pi)
        },
        elapsed,
        finished_at: (!state.is_running()).then(|| ended.map(clock_at)).flatten(),
        updated: pi
            .messages
            .iter()
            .rev()
            .find_map(timestamp)
            .map_or(0, |at| at.as_secs()),
        turns,
        files: files(pi, facts.cwd),
        check: check(pi),
        question: facts.question,
        queued,
        queued_keys,
        failure,
        details: details(pi, facts.key),
        script: None,
    }
}

/// A session the computer listed, before the phone attaches to it.
pub fn listed(id: SessionId, listed: &crate::remote::Listed, folder: String) -> Session {
    let state = match (listed.busy, listed.stop_reason.as_deref()) {
        (true, _) if listed.running => State::Working,
        // Its daemon stopped mid-run; attaching resumes a durable session.
        (true, _) => State::Stopped,
        (false, Some("aborted" | "toolUse")) => State::Stopped,
        (false, Some("error")) => State::Failed,
        _ => State::Done,
    };
    let cwd = listed.cwd.trim_end_matches('/');
    Session {
        local: false,
        backend_features: None,
        id,
        title: listed
            .title
            .clone()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Untitled session".into()),
        project: cwd.rsplit('/').next().unwrap_or(cwd).to_owned(),
        folder,
        state,
        activity: if state == State::Working {
            "Working".into()
        } else {
            String::new()
        },
        elapsed: Duration::ZERO,
        finished_at: (!state.is_running() && listed.updated > 0)
            .then(|| clock_at(Duration::from_secs(listed.updated))),
        updated: listed.updated,
        turns: Vec::new(),
        files: Vec::new(),
        check: None,
        question: None,
        queued: Vec::new(),
        queued_keys: Vec::new(),
        failure: (state == State::Failed)
            .then(|| listed.error.clone().unwrap_or_else(|| "failed".into())),
        details: Details {
            context_percent: None,
            context_tokens: "Not reported".into(),
            compaction_percent: None,
            auto_compaction_enabled: false,
            swarm_enabled: None,
            background_compacting: false,
            cost: String::new(),
            turns: 0,
            tools: Vec::new(),
            snapshots: 0,
            session_file: format!("Durable session {}", &listed.key[..8.min(listed.key.len())]),
        },
        script: None,
    }
}

fn title(pi: &Pi, outbox: &[PendingPrompt]) -> String {
    let title = pi.title();
    if !title.is_empty() && title != "New session" {
        return title.to_owned();
    }
    outbox
        .first()
        .map(|prompt| crate::demo::title_for(&prompt.text))
        .unwrap_or_else(|| title.to_owned())
}

fn now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

/// Pi stamps messages in milliseconds since 1970.
fn timestamp(message: &Value) -> Option<Duration> {
    message["timestamp"].as_u64().map(Duration::from_millis)
}

/// The text of a message's content, which is a string or a list of blocks.
fn text_of(message: &Value) -> String {
    pi_core::session::content_text(&message["content"])
}

fn tool<'a>(pi: &'a Pi, id: &str) -> Option<&'a Tool> {
    pi.tools.iter().find(|tool| tool.id == id)
}

/// An unfinished tool can remain in a durable snapshot after its run ended.
/// It belongs to the current turn only if no earlier turn called that id.
fn current_running_tool(pi: &Pi) -> Option<&Tool> {
    let current_turn = pi
        .messages
        .iter()
        .rposition(|message| message["role"] == "user")
        .unwrap_or(0);
    pi.tools.iter().rev().find(|tool| {
        tool.is_running()
            && !pi.messages[..current_turn].iter().any(|message| {
                message["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|block| block["type"] == "toolCall" && block["id"] == tool.id)
            })
    })
}

/// A project-relative path where Pi gave an absolute one.
fn relative(path: &str, cwd: &str) -> String {
    let cwd = cwd.trim_end_matches('/');
    path.strip_prefix(cwd)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
        .to_owned()
}

fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// What a call is for: edits change; tests, builds and linters check; the
/// rest, commands that only look around included, is looking.
fn kind(tool: &str, args: &Value) -> StageKind {
    match tool {
        "edit" | "write" => StageKind::Change,
        "bash" if args["command"].as_str().is_some_and(is_check) => StageKind::Verify,
        _ => StageKind::Understand,
    }
}

/// Where a call is filed in a turn, by its id.
fn call_of(turn: &Turn, id: &str) -> Option<(StageKind, usize)> {
    turn.stages.iter().find_map(|stage| {
        stage
            .tools
            .iter()
            .position(|tool| tool.id == id)
            .map(|index| (stage.kind, index))
    })
}

/// What the run line's time goes to, between the moments Pi stamps: its
/// messages, and when each call ended.
#[derive(Default)]
struct RunClock {
    /// The last moment accounted for.
    at: Option<Duration>,
    /// The last call whose end wasn't stamped: the time until the next
    /// message is its, then its result.
    pending: Option<(StretchKind, Option<Mark>)>,
    /// Work handed to subagents, until the next message: not Pi's own time.
    handed_off: bool,
}

impl RunClock {
    /// Ends the call still pending with its result, with no more time.
    fn settle(&mut self, turn: &mut Turn) {
        if let Some((kind, Some(mark))) = self.pending.take() {
            turn.end_stretch(kind, mark);
        }
    }
}

/// The turns, and the last moment the run line accounts for, from which a
/// running session's live stretch goes on.
fn turns(pi: &Pi, cwd: &str) -> (Vec<Turn>, Option<Duration>) {
    let mut turns: Vec<Turn> = Vec::new();
    let mut pages = pi_markdown::Pages::default();
    // The run line, in order: the time before a message goes to what it is
    // about to do, and a call's time up to its stamped end to what the call
    // was for. A check's result ends its stretch.
    let mut clock = RunClock::default();
    for message in &pi.messages {
        let at = timestamp(message);
        match message["role"].as_str() {
            Some("user") => {
                if let Some(turn) = turns.last_mut() {
                    clock.settle(turn);
                }
                clock = RunClock {
                    at,
                    ..RunClock::default()
                };
                let text = crate::visualize_prompt::label(&text_of(message));
                let reported = pi_core::subagent::report(&text).map(str::to_owned);
                // A `/skill:` prompt arrives expanded; it reads as it was typed.
                let text = match pi_core::skill::parse_skill_block(&text) {
                    Some(block) => match block.user_message {
                        Some(said) => format!("/skill:{} {said}", block.name),
                        None => format!("/skill:{}", block.name),
                    },
                    None => text,
                };
                let text = match &reported {
                    Some(said) => format!("Subagents reported back\n\n{said}"),
                    None => text,
                };
                let mut turn =
                    Turn::new(text, timestamp(message).map(clock_at).unwrap_or_default());
                turn.reported = reported.is_some();
                turn.durable_entry_id = message["__piDurableEntryId"].as_u64();
                {
                    use sha2::{Digest, Sha256};
                    // User messages are immutable; unlike turn indices this
                    // identity survives compaction/history rebasing.
                    if message.get("id").is_some_and(|v| !v.is_null())
                        || message.get("timestamp").is_some_and(|v| !v.is_null())
                    {
                        let identity = serde_json::json!({"id":message.get("id"), "timestamp":message.get("timestamp"), "prompt":turn.prompt});
                        turn.visual_identity = Some(format!(
                            "{:x}",
                            Sha256::digest(serde_json::to_vec(&identity).unwrap_or_default())
                        ));
                    }
                    // Legacy messages without an identity/timestamp cannot be
                    // safely distinguished across compaction; keep apps inert.
                }
                turn.stages.clear();
                let images: Vec<Value> = message["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|block| block["type"] == "image")
                    .cloned()
                    .collect();
                turn.prompt_images = pi_markdown::tool_images(&images, None);
                if !images.is_empty() {
                    turn.attachments.push(match images.len() {
                        1 => "1 image".into(),
                        count => format!("{count} images"),
                    });
                }
                turns.push(turn);
            }
            Some("compactionSummary") => {
                if turns.is_empty() {
                    let mut turn = Turn::new("", "");
                    turn.stages.clear();
                    turns.push(turn);
                }
                let summary = message["summary"].as_str().unwrap_or("").to_owned();
                let turn = turns.last_mut().expect("a turn");
                turn.flow.push(crate::model::Flow::Compacted(summary));
            }
            Some("assistant") => {
                if turns.is_empty() {
                    let mut turn = Turn::new("", "");
                    turn.stages.clear();
                    turns.push(turn);
                }
                let turn = turns.last_mut().expect("a turn");
                let first_tool = message["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|block| block["type"] == "toolCall")
                    .map(|block| kind(block["name"].as_str().unwrap_or(""), &block["arguments"]));
                let gap = at
                    .zip(clock.at)
                    .map_or(Duration::ZERO, |(at, since)| at.saturating_sub(since));
                if std::mem::take(&mut clock.handed_off) {
                    // Subagents' time is theirs.
                } else if let Some((kind, mark)) = clock.pending.take() {
                    // A call whose end wasn't stamped ran until this message.
                    turn.add_time(kind, gap);
                    if let Some(mark) = mark {
                        turn.end_stretch(kind, mark);
                    }
                } else {
                    // Thinking before a call goes to what the call is for;
                    // words with no call are Pi looking the work over.
                    turn.add_time(first_tool.map_or(StretchKind::Look, StretchKind::of), gap);
                }
                if at.is_some() {
                    clock.at = at;
                }
                // Redacted reasoning carries no text, so it shows nothing.
                let thinking = message["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|block| block["type"] == "thinking")
                    .filter_map(|block| block["thinking"].as_str())
                    .filter(|text| !text.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if !thinking.is_empty() {
                    turn.flow.push(crate::model::Flow::Thinking(thinking));
                }
                let text = text_of(message);
                if !text.trim().is_empty() {
                    turn.flow.push(crate::model::Flow::Text(text.clone()));
                }
                for block in message["content"].as_array().into_iter().flatten() {
                    if block["type"] != "toolCall" {
                        continue;
                    }
                    let id = block["id"].as_str().unwrap_or("");
                    let name = block["name"].as_str().unwrap_or("");
                    let observed = tool(pi, id);
                    if name == pi_core::subagent::TOOL {
                        let handoff = observed
                            .and_then(pi_core::subagent::Handoff::of)
                            .unwrap_or_else(|| {
                                let mut handoff =
                                    pi_core::subagent::Handoff::from_args(&block["arguments"]);
                                // Answered, though nothing said how: none is at work.
                                if let Some(result) = pi.messages.iter().find(|message| {
                                    message["role"] == "toolResult" && message["toolCallId"] == id
                                }) {
                                    handoff.ended(result["isError"] == true);
                                }
                                handoff
                            });
                        turn.flow
                            .push(crate::model::Flow::Handoff(turn.handoffs.len()));
                        turn.handoffs.push(handoff);
                        // Time handed off isn't Pi's own.
                        clock.settle(turn);
                        match observed.and_then(|tool| tool.ended_at) {
                            Some(end) => clock.at = Some(Duration::from_millis(end)),
                            None => clock.handed_off = true,
                        }
                        continue;
                    }
                    let args = observed.map_or(&block["arguments"], |tool| &tool.args);
                    add_tool(turn, id, name, args, observed, cwd);
                    // The call's own time, up to its stamped end, and a
                    // check's result.
                    if let Some(&Flow::Tool(stage, index)) = turn.flow.last() {
                        let stretch = StretchKind::of(stage);
                        let result = observed
                            .filter(|tool| stage == StageKind::Verify && tool.finished)
                            .map(|tool| {
                                if tool.is_error {
                                    Mark::Failed
                                } else {
                                    Mark::Passed
                                }
                            });
                        clock.settle(turn);
                        turn.add_call(stretch, (stage, index));
                        match observed.and_then(|tool| tool.ended_at) {
                            Some(end) => {
                                let end = Duration::from_millis(end);
                                let time = clock
                                    .at
                                    .map_or(Duration::ZERO, |since| end.saturating_sub(since));
                                turn.add_time(stretch, time);
                                if let Some(mark) = result {
                                    turn.end_stretch(stretch, mark);
                                }
                                clock.at = Some(end);
                            }
                            None => clock.pending = Some((stretch, result)),
                        }
                    }
                    // What a call returned follows the call.
                    if let Some(observed) = observed {
                        let path = observed.args["path"].as_str();
                        for image in pi_markdown::tool_images(&observed.images, path) {
                            turn.flow.push(crate::model::Flow::Image(turn.images.len()));
                            turn.images.push(image);
                        }
                    }
                    if observed.is_some_and(|tool| tool.finished && !tool.is_error)
                        && let Some(path) = args["path"].as_str()
                        && let Some(page) = pages.follow(name, args, &relative(path, cwd))
                    {
                        turn.show_page(page);
                    }
                }
                if !text.trim().is_empty() {
                    // Commentary before/between tools is visible progress too.
                    // Each message occurs once in Pi's authoritative history.
                    let source = turn.summary.as_ref().map_or_else(
                        || text.clone(),
                        |previous| format!("{}\n\n{text}", previous.text()),
                    );
                    turn.summary = Some(summary(&source));
                }
            }
            _ => {}
        }
    }
    if let Some(turn) = turns.last_mut() {
        clock.settle(turn);
    }
    (turns, clock.at)
}

fn add_tool(
    turn: &mut Turn,
    id: &str,
    name: &str,
    args: &Value,
    observed: Option<&Tool>,
    cwd: &str,
) {
    let kind = kind(name, args);
    let lines = observed.map(tool_lines).unwrap_or_default();
    let added = lines.iter().filter(|l| l.kind == LineKind::Added).count() as u32;
    let removed = lines.iter().filter(|l| l.kind == LineKind::Removed).count() as u32;
    let stage = turn.stage_mut(kind);
    let step = crate::model::Flow::Tool(kind, stage.tools.len());
    stage.tools.push(crate::model::ToolActivity {
        id: id.to_owned(),
        name: name.to_owned(),
        target: args["path"]
            .as_str()
            .or_else(|| args["command"].as_str())
            .or_else(|| args["pattern"].as_str())
            .or_else(|| args["file"].as_str())
            .unwrap_or("")
            .to_owned(),
        output: observed.map_or_else(String::new, |tool| tool.output.clone()),
        finished: observed.is_some_and(|tool| tool.finished),
        failed: observed.is_some_and(|tool| tool.is_error),
        added: if kind == StageKind::Change { added } else { 0 },
        removed: if kind == StageKind::Change {
            removed
        } else {
            0
        },
        diff: lines.clone(),
        diff_is_preview: observed.is_some_and(|tool| {
            tool.diff.is_none() && (tool.name == "edit" || !tool.finished || tool.is_error)
        }),
    });
    turn.flow.push(step);
    let stage = turn.stage_mut(kind);
    if observed.is_some_and(|tool| tool.finished || tool.is_running()) {
        stage.status = StageStatus::Done;
    }
    let path = args["path"].as_str().map(|path| relative(path, cwd));
    match kind {
        StageKind::Understand => {
            let reference = match (name, &path, args["pattern"].as_str()) {
                ("grep" | "find", _, Some(pattern)) => Some(Reference::Search(pattern.to_owned())),
                (_, Some(path), _) => Some(Reference::File(path.clone())),
                _ => None,
            };
            if let Some(reference) = reference
                && !stage.references.contains(&reference)
            {
                stage.references.push(reference);
            }
            let (files, searches) = stage.references.iter().fold((0, 0), |(f, s), r| match r {
                Reference::File(_) => (f + 1, s),
                Reference::Search(_) => (f, s + 1),
            });
            // Commands that only look around, such as `rg` or `git log`.
            let commands = stage
                .tools
                .iter()
                .filter(|tool| matches!(tool.name.as_str(), "bash" | "javascript"))
                .count();
            let read = match files {
                0 => None,
                1 => Some("read 1 file".to_owned()),
                count => Some(format!("read {count} files")),
            };
            let searched = match searches {
                0 => None,
                1 => Some("searched once".to_owned()),
                2 => Some("searched twice".to_owned()),
                count => Some(format!("searched {count} times")),
            };
            let ran = match commands {
                0 => None,
                1 => Some("ran 1 command".to_owned()),
                count => Some(format!("ran {count} commands")),
            };
            let parts: Vec<String> = [read, searched, ran].into_iter().flatten().collect();
            stage.what = if parts.is_empty() {
                "Looked around".into()
            } else {
                let mut what = parts.join(" · ");
                what[..1].make_ascii_uppercase();
                what
            };
        }
        StageKind::Change => {
            if let Some(path) = &path {
                let reference = Reference::File(path.clone());
                if !stage.references.contains(&reference) {
                    stage.references.push(reference);
                }
                stage.what = format!("Edited {}", name_of_files(&stage.references));
            }
            if observed.is_some() {
                stage.added += added;
                stage.removed += removed;
                if !lines.is_empty() {
                    stage.diff_path = path.clone();
                    stage.diff = lines.into_iter().take(8).collect();
                }
            }
        }
        StageKind::Verify => {
            if let Some(command) = args["command"].as_str() {
                stage.what = format!("Ran {}", one_line(command, 48));
            }
        }
        StageKind::HandOff => {}
    }
}

fn name_of_files(references: &[Reference]) -> String {
    match references {
        [Reference::File(path)] => name(path).to_owned(),
        files => format!("{} files", files.len()),
    }
}

fn one_line(text: &str, limit: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > limit || text.lines().nth(1).is_some() {
        let short: String = line.chars().take(limit).collect();
        format!("{}…", short.trim_end())
    } else {
        line.to_owned()
    }
}

/// Marks the last turn's stages for how the session stands now.
fn settle(turn: &mut Turn, state: State, pi: &Pi, cwd: &str, inherit_running_tools: bool) {
    // An authoritative empty tool catalog is not a coding workflow waiting
    // for its first tool. Text/thinking use the existing message flow instead.
    if pi.state.active_tools.as_ref().is_some_and(Vec::is_empty) && turn.stages.is_empty() {
        return;
    }
    if state.is_running()
        && inherit_running_tools
        && let Some(tool) =
            current_running_tool(pi).filter(|tool| tool.name == pi_core::subagent::TOOL)
    {
        // Handed off: the stage Pi was in waits on its subagents.
        let stage = turn.stage_mut(
            turn.flow
                .iter()
                .rev()
                .find_map(|piece| match piece {
                    Flow::Tool(kind, _) => Some(*kind),
                    _ => None,
                })
                .unwrap_or(StageKind::Understand),
        );
        stage.status = StageStatus::Live;
        stage.what = doing(tool);
        turn.stage_mut(StageKind::HandOff).status = StageStatus::Planned;
    } else if state.is_running() {
        let running = inherit_running_tools
            .then(|| current_running_tool(pi))
            .flatten();
        // Finished edits/checks are history, not current execution. Inference
        // between tool calls can revisit any stage; never infer it from the
        // first-seen/sorted order of the stage buckets.
        let live = running.map_or(StageKind::Understand, |tool| kind(&tool.name, &tool.args));
        if let Some(tool) = running {
            // Execution events can precede the assistant's toolCall.
            if !turn
                .stages
                .iter()
                .flat_map(|s| &s.tools)
                .any(|t| t.id == tool.id)
            {
                add_tool(turn, &tool.id, &tool.name, &tool.args, Some(tool), cwd);
            }
            if let Some(call) = call_of(turn, &tool.id) {
                turn.add_call(StretchKind::of(live), call);
            }
        }
        let stage = turn.stage_mut(live);
        stage.status = StageStatus::Live;
        stage.what = running.map_or_else(|| inference_activity(pi).into(), doing);
        turn.stage_mut(StageKind::HandOff).status = StageStatus::Planned;
    } else {
        let stopped = matches!(state, State::Stopped | State::Failed);
        // A turn where no tool ran is just the answer, with no stages to show.
        if !turn.stages.is_empty() && (turn.summary.is_some() || !stopped) {
            let hand_off = turn.stage_mut(StageKind::HandOff);
            hand_off.status = StageStatus::Done;
            hand_off.what = "Summary and changes".into();
        }
        if stopped {
            for stage in &mut turn.stages {
                if stage.status != StageStatus::Done {
                    stage.status = StageStatus::Skipped;
                }
            }
        }
    }
    turn.stages
        .sort_by_key(|stage| StageKind::ALL.iter().position(|kind| *kind == stage.kind));
}

/// What a running tool is doing: "Editing app.rs", "Waiting on 2 scouts".
fn doing(tool: &Tool) -> String {
    if let Some(handoff) = pi_core::subagent::Handoff::of(tool) {
        return waiting_on(&handoff);
    }
    let path = tool.args["path"].as_str().map(name);
    match (tool.name.as_str(), path) {
        ("read", Some(path)) => format!("Reading {path}"),
        ("edit" | "write", Some(path)) => format!("Editing {path}"),
        ("javascript", _) => format!(
            "Running {}",
            tool.args["file"].as_str().map(name).unwrap_or("JavaScript")
        ),
        ("bash", _) => format!(
            "Running {}",
            one_line(tool.args["command"].as_str().unwrap_or(""), 40)
        ),
        ("grep" | "find", _) => format!(
            "Searching for {}",
            one_line(tool.args["pattern"].as_str().unwrap_or(""), 32)
        ),
        ("ls", Some(path)) => format!("Looking in {path}"),
        (name, _) => format!("Using {name}"),
    }
}

/// "Waiting on 2 scouts", "Waiting on the planner", "Waiting on 3 subagents".
pub fn waiting_on(handoff: &pi_core::subagent::Handoff) -> String {
    let busy: Vec<&str> = handoff
        .subagents
        .iter()
        .filter(|subagent| !subagent.status.finished())
        .map(|subagent| subagent.agent.as_str())
        .collect();
    let same = busy.windows(2).all(|pair| pair[0] == pair[1]);
    match (busy.as_slice(), same) {
        ([], _) => "Hearing back".into(),
        ([agent], _) => format!("Waiting on the {agent}"),
        ([agent, ..], true) => format!("Waiting on {} {agent}s", busy.len()),
        (_, false) => format!("Waiting on {} subagents", busy.len()),
    }
}

/// "1 scout at work", "8 subagents at work": those running, else those to come.
pub fn at_work(handoff: &pi_core::subagent::Handoff) -> String {
    let running = handoff.running() > 0;
    let busy: Vec<&str> = handoff
        .subagents
        .iter()
        .filter(|subagent| {
            if running {
                subagent.status == pi_core::subagent::Status::Running
            } else {
                !subagent.status.finished()
            }
        })
        .map(|subagent| subagent.agent.as_str())
        .collect();
    let same = busy.windows(2).all(|pair| pair[0] == pair[1]);
    match (busy.as_slice(), same) {
        ([], _) => "hearing back".into(),
        ([agent], _) => format!("1 {agent} at work"),
        ([agent, ..], true) => format!("{} {agent}s at work", busy.len()),
        (_, false) => format!("{} subagents at work", busy.len()),
    }
}

/// A live assistant message is inference, not evidence that an old edit or
/// check is still running. Follow the last block, including text -> thinking.
fn inference_activity(pi: &Pi) -> &'static str {
    let Some(message) = pi
        .streaming_message_index()
        .and_then(|index| pi.messages.get(index))
    else {
        return "Working";
    };
    if let Some(blocks) = message["content"].as_array() {
        match blocks.last().and_then(|block| block["type"].as_str()) {
            Some("text") => "Writing",
            Some("toolCall") => "Preparing tool",
            _ => "Thinking",
        }
    } else if !text_of(message).trim().is_empty() {
        "Writing"
    } else {
        "Thinking"
    }
}

fn activity(pi: &Pi) -> String {
    // Control phases take priority over any lingering tool preview.
    if pi.run == pi_core::session::RunState::Compacting {
        return "Compacting".into();
    }
    if pi.run == pi_core::session::RunState::Retrying {
        return "Retrying".into();
    }
    if let Some(tool) = current_running_tool(pi) {
        return doing(tool);
    }
    inference_activity(pi).into()
}

/// Pi's closing words: the first sentence as the headline, the rest under it.
fn summary(text: &str) -> Summary {
    let text = text.trim();
    let first_line = text.lines().next().unwrap_or("");
    let cut = first_line
        .char_indices()
        .find(|(index, c)| matches!(c, '.' | '!' | '?') && first_line[index + 1..].starts_with(' '))
        .map_or(first_line.len(), |(index, _)| index + 1);
    // An entire paragraph or a code fence is body text, not an enormous
    // decorative heading. Never discard the rest of an answer.
    if first_line[..cut].chars().count() > 160 || first_line.starts_with("```") {
        return Summary {
            headline: "Pi".into(),
            body: text.to_owned(),
            source: Some(text.to_owned()),
        };
    }
    let headline = first_line[..cut]
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_owned();
    let body = text[cut..].trim();
    let body = body.to_owned();
    Summary {
        headline,
        body,
        source: Some(text.to_owned()),
    }
}

/// Pi's numbered diff lines: "+212 text", "-211 text", " 210 text"; an
/// unnumbered line such as "..." separates hunks.
pub fn parse_diff(diff: &str) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut fresh = true;
    for line in diff.lines() {
        let (kind, rest) = if let Some(rest) = line.strip_prefix('+') {
            (LineKind::Added, rest)
        } else if let Some(rest) = line.strip_prefix('-') {
            (LineKind::Removed, rest)
        } else {
            (LineKind::Context, line)
        };
        // Pi pads every line number to the file's width, including changed
        // lines after their +/- marker. Scan past that padding uniformly.
        let rest = rest.trim_start();
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let numbered = digits > 0 && (rest.len() == digits || rest[digits..].starts_with(' '));
        if !numbered {
            fresh = true;
            continue;
        }
        let number: u32 = rest[..digits].parse().unwrap_or(0);
        let text = rest.get(digits + 1..).unwrap_or("");
        if fresh {
            hunks.push(Hunk {
                header: format!("@@ {number}"),
                lines: Vec::new(),
            });
            fresh = false;
        }
        hunks
            .last_mut()
            .expect("a hunk")
            .lines
            .push(DiffLine::new(kind, number, text));
    }
    hunks
}

fn tool_lines(tool: &Tool) -> Vec<DiffLine> {
    if let Some(diff) = &tool.diff {
        return parse_diff(diff)
            .into_iter()
            .flat_map(|hunk| hunk.lines)
            .collect();
    }
    match tool.name.as_str() {
        "write" => written(tool),
        // The result diff often arrives only when the tool finishes. Its
        // requested replacements are still authoritative live input, so show
        // them in the expanded activity rail while execution is in progress.
        "edit" => requested_edits(tool),
        _ => Vec::new(),
    }
}

/// A file Pi wrote in full: every line is new.
fn written(tool: &Tool) -> Vec<DiffLine> {
    tool.args["content"]
        .as_str()
        .unwrap_or("")
        .lines()
        .enumerate()
        .map(|(index, line)| DiffLine::new(LineKind::Added, index as u32 + 1, line))
        .collect()
}

/// A live edit preview before Pi reports the applied diff. Line numbers are
/// intentionally unknown (zero); the row renderer leaves that gutter blank.
fn requested_edits(tool: &Tool) -> Vec<DiffLine> {
    let parsed;
    let edits = match &tool.args["edits"] {
        Value::String(source) => {
            parsed = serde_json::from_str::<Value>(source).unwrap_or(Value::Null);
            &parsed
        }
        Value::Null => &tool.args,
        edits => edits,
    };
    let edits: Vec<&Value> = match edits {
        Value::Array(edits) => edits.iter().collect(),
        Value::Object(_) => vec![edits],
        _ => Vec::new(),
    };
    edits
        .into_iter()
        .flat_map(|edit| {
            let removed = edit["oldText"]
                .as_str()
                .unwrap_or("")
                .lines()
                .map(|line| DiffLine::new(LineKind::Removed, 0, line));
            let added = edit["newText"]
                .as_str()
                .unwrap_or("")
                .lines()
                .map(|line| DiffLine::new(LineKind::Added, 0, line));
            removed.chain(added).collect::<Vec<_>>()
        })
        .collect()
}

/// Every file Pi changed in the session, in the order it first touched them.
fn files(pi: &Pi, cwd: &str) -> Vec<FileChange> {
    let mut files: Vec<FileChange> = Vec::new();
    for tool in &pi.tools {
        if !tool.finished || tool.is_error || !matches!(tool.name.as_str(), "edit" | "write") {
            continue;
        }
        let Some(path) = tool.args["path"].as_str().map(|path| relative(path, cwd)) else {
            continue;
        };
        let hunks = match &tool.diff {
            Some(diff) => parse_diff(diff),
            None if tool.name == "write" => vec![Hunk {
                header: "Written in full".into(),
                lines: written(tool),
            }],
            None if tool.name == "edit" => vec![Hunk {
                header: "Requested replacements".into(),
                lines: requested_edits(tool),
            }],
            None => Vec::new(),
        };
        let index = files
            .iter()
            .position(|file| file.path == path)
            .unwrap_or_else(|| {
                files.push(FileChange {
                    path,
                    added: 0,
                    removed: 0,
                    hunks: Vec::new(),
                });
                files.len() - 1
            });
        let file = &mut files[index];
        for hunk in hunks {
            file.added += hunk
                .lines
                .iter()
                .filter(|l| l.kind == LineKind::Added)
                .count() as u32;
            file.removed += hunk
                .lines
                .iter()
                .filter(|l| l.kind == LineKind::Removed)
                .count() as u32;
            file.hunks.push(hunk);
        }
    }
    files
}

/// Commands that check work, as opposed to looking around: some part of the
/// command runs a test, build or linter. `rg test` or `git commit -m "test"`
/// only mention one.
pub(crate) fn is_check(command: &str) -> bool {
    const WORDS: [&str; 14] = [
        "test", "check", "clippy", "lint", "build", "tsc", "pytest", "jest", "vitest", "vet",
        "fmt", "make", "eslint", "mypy",
    ];
    /// Programs that read, search or describe, whatever their arguments say.
    const LOOKS: [&str; 26] = [
        "rg", "grep", "egrep", "fgrep", "ag", "cat", "less", "head", "tail", "ls", "find", "fd",
        "tree", "wc", "sed", "awk", "echo", "printf", "git", "cd", "pwd", "which", "file", "stat",
        "diff", "jq",
    ];
    command.split(['&', ';', '|', '\n']).any(|part| {
        let mut words = part
            .split_whitespace()
            // Environment assignments come before the program: `CI=1 pnpm test`.
            .skip_while(|word| word.contains('=') && !word.starts_with('-'));
        let Some(program) = words.next() else {
            return false;
        };
        let program = program.rsplit('/').next().unwrap_or(program);
        !LOOKS.contains(&program)
            && std::iter::once(program)
                .chain(words)
                .flat_map(|word| word.split(|c: char| !c.is_ascii_alphanumeric() && c != '-'))
                .any(|word| WORDS.contains(&word))
    })
}

/// The last check of the last turn, and whether it passed.
fn check(pi: &Pi) -> Option<Check> {
    let turn_start = pi.messages.iter().rposition(|m| m["role"] == "user")?;
    let ids: Vec<&str> = pi.messages[turn_start..]
        .iter()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|block| block["type"] == "toolCall" && block["name"] == "bash")
        .filter_map(|block| block["id"].as_str())
        .collect();
    ids.iter().rev().find_map(|id| {
        let tool = tool(pi, id).filter(|tool| tool.finished)?;
        let command = tool.args["command"].as_str()?;
        is_check(command).then(|| Check {
            command: one_line(command, 60),
            result: if tool.is_error {
                CheckResult::Failed
            } else {
                CheckResult::Passed
            },
        })
    })
}

fn details(pi: &Pi, key: &str) -> Details {
    let usage = pi.stats.context_usage.as_ref();
    let thousands = |tokens: u64| format!("{}k", tokens.div_ceil(1000));
    let context_percent = usage.and_then(|usage| {
        usage.percent.or_else(|| {
            usage
                .tokens
                .filter(|_| usage.context_window > 0)
                .map(|tokens| tokens as f64 * 100. / usage.context_window as f64)
        })
    });
    let mut tools = Vec::new();
    for tool in &pi.tools {
        if !tools.contains(&tool.name) {
            tools.push(tool.name.clone());
        }
    }
    Details {
        context_percent: context_percent.map(|percent| percent.round().clamp(0., 100.) as u32),
        context_tokens: usage
            .and_then(|usage| {
                usage.tokens.map(|tokens| {
                    format!(
                        "{} of {} tokens",
                        thousands(tokens),
                        thousands(usage.context_window)
                    )
                })
            })
            .unwrap_or_else(|| "Not reported".into()),
        compaction_percent: usage
            .filter(|usage| usage.context_window > 0)
            .and_then(|usage| {
                pi.stats.compaction.as_ref().map(|policy| {
                    let threshold = usage
                        .context_window
                        .saturating_sub(policy.reserve_tokens)
                        .saturating_sub(policy.background_tokens);
                    threshold as f64 * 100. / usage.context_window as f64
                })
            }),
        auto_compaction_enabled: pi.state.auto_compaction_enabled,
        swarm_enabled: pi.state.swarm_enabled,
        background_compacting: pi.state.compactions.iter().any(|item| !item.blocking),
        cost: pi
            .stats
            .cost
            .map(|cost| format!("${cost:.2}"))
            .unwrap_or_default(),
        turns: pi.messages.iter().filter(|m| m["role"] == "user").count() as u32,
        tools,
        snapshots: 0,
        session_file: pi
            .state
            .session_file
            .clone()
            .unwrap_or_else(|| format!("Durable session {}", &key[..8.min(key.len())])),
    }
}

/// Pi's open question, from an extension's `confirm` or `select`.
pub fn question(request: &Value) -> Option<Question> {
    let title = request["title"]
        .as_str()
        .unwrap_or("Pi has a question")
        .to_owned();
    let body = request["message"].as_str().unwrap_or("").to_owned();
    let choices = match request["method"].as_str()? {
        "confirm" => vec![
            Choice {
                answer: Answer::AllowOnce,
                label: "Yes".into(),
                detail: None,
            },
            Choice {
                answer: Answer::Deny,
                label: "No".into(),
                detail: None,
            },
        ],
        // Every option, by its place: the answer names the one picked.
        "select" => request["options"]
            .as_array()?
            .iter()
            .enumerate()
            .filter_map(|(index, option)| Some((index, option.as_str()?)))
            .map(|(index, label)| Choice {
                answer: Answer::Pick(index),
                label: label.to_owned(),
                detail: None,
            })
            .collect(),
        _ => return None,
    };
    Some(Question {
        title,
        body,
        command: String::new(),
        choices,
    })
}

/// The reply to Pi's question for the phone's answer.
pub fn answer(request: &Value, answer: Answer) -> Value {
    let mut response = serde_json::json!({"type": "extension_ui_response", "id": request["id"]});
    match request["method"].as_str() {
        Some("confirm") => response["confirmed"] = (answer != Answer::Deny).into(),
        _ => {
            // A select's choices name their option; the permission words are
            // its first and second option, and No picks none.
            let index = match answer {
                Answer::Pick(index) => Some(index),
                Answer::AllowOnce => Some(0),
                Answer::AllowSession => Some(1),
                Answer::Deny => None,
            };
            match index
                .and_then(|index| request["options"].get(index))
                .and_then(Value::as_str)
            {
                Some(value) => response["value"] = value.into(),
                None => response["cancelled"] = true.into(),
            }
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(records: &[Value]) -> Pi {
        let mut pi = Pi::new("/Users/nick/repos/pi".into());
        for record in records {
            pi.apply(record).unwrap();
        }
        pi
    }

    fn facts(outbox: &[PendingPrompt]) -> Facts<'_> {
        Facts {
            id: SessionId(1),
            cwd: "/Users/nick/repos/pi",
            folder: "~/repos/pi".into(),
            question: None,
            outbox,
            key: "0123456789abcdef0123456789abcdef",
        }
    }

    #[test]
    fn long_replies_keep_every_paragraph_and_final_character() {
        for count in [1, 20, 1000, 20_000] {
            let body = format!(
                "{}\n\nFINAL 中文 👩🏽‍💻",
                "A complete paragraph.\n\n".repeat(count)
            );
            let reply = summary(&format!("Result.\n\n{body}"));
            assert_eq!(reply.headline, "Result.");
            assert_eq!(reply.body, body);
        }
        let single_paragraph = "Long unbroken answer ".repeat(1000);
        assert_eq!(summary(&single_paragraph).body, single_paragraph.trim());
    }

    const DIFF: &str = "   210 const a = 1;\n-  211 old();\n+  211 new();\n+  212 more();\n   213 end\n     ...\n   300 x\n+  301 y";

    fn finished_run() -> Pi {
        session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Fix the flaky test","timestamp":1_759_600_000_000u64},
                {"role":"assistant","content":[
                    {"type":"toolCall","id":"r","name":"read","arguments":{"path":"/Users/nick/repos/pi/src/app.rs"}},
                    {"type":"toolCall","id":"g","name":"grep","arguments":{"pattern":"flaky"}}
                ],"stopReason":"toolUse"},
                {"role":"toolResult","toolCallId":"r","toolName":"read","content":[{"type":"text","text":"…"}],"isError":false},
                {"role":"toolResult","toolCallId":"g","toolName":"grep","content":[{"type":"text","text":"…"}],"isError":false},
                {"role":"assistant","content":[
                    {"type":"toolCall","id":"e","name":"edit","arguments":{"path":"src/app.rs"}},
                    {"type":"toolCall","id":"t","name":"bash","arguments":{"command":"cargo test -p app"}}
                ],"stopReason":"toolUse"},
                {"role":"toolResult","toolCallId":"e","toolName":"edit","content":[{"type":"text","text":"ok"}],"details":{"diff":DIFF},"isError":false},
                {"role":"toolResult","toolCallId":"t","toolName":"bash","content":[{"type":"text","text":"ok"}],"isError":false},
                {"role":"assistant","content":[{"type":"text","text":"Fixed the race. The timer now waits for the lock."}],"stopReason":"stop","timestamp":1_759_600_272_000u64}
            ]}}),
        ])
    }

    #[test]
    fn a_finished_run_reads_like_the_design() {
        let shown = project(&finished_run(), facts(&[]));
        assert_eq!(shown.title, "Fix the flaky test");
        assert_eq!(shown.project, "pi");
        assert_eq!(shown.state, State::Done);
        assert_eq!(shown.elapsed, Duration::from_secs(272));
        let turn = &shown.turns[0];
        let kinds: Vec<_> = turn.stages.iter().map(|s| (s.kind, s.status)).collect();
        assert_eq!(
            kinds,
            [
                (StageKind::Understand, StageStatus::Done),
                (StageKind::Change, StageStatus::Done),
                (StageKind::Verify, StageStatus::Done),
                (StageKind::HandOff, StageStatus::Done),
            ]
        );
        assert_eq!(turn.stages[0].what, "Read 1 file · searched once");
        assert_eq!(turn.stages[1].what, "Edited app.rs");
        assert_eq!(turn.stages[1].diff_path.as_deref(), Some("src/app.rs"));
        assert_eq!((turn.stages[1].added, turn.stages[1].removed), (3, 1));
        assert_eq!(turn.stages[2].what, "Ran cargo test -p app");
        let summary = turn.summary.as_ref().unwrap();
        assert_eq!(summary.headline, "Fixed the race.");
        assert_eq!(summary.body, "The timer now waits for the lock.");
        assert_eq!(shown.files.len(), 1);
        assert_eq!(shown.files[0].path, "src/app.rs");
        assert_eq!((shown.files[0].added, shown.files[0].removed), (3, 1));
        assert_eq!(shown.files[0].hunks.len(), 2);
        assert_eq!(
            shown.check,
            Some(Check {
                command: "cargo test -p app".into(),
                result: CheckResult::Passed
            })
        );
        assert_eq!(shown.status_line(), "1 file changed · checks passed");
        assert_eq!(shown.details.tools, ["read", "grep", "edit", "bash"]);
        assert_eq!(shown.details.context_percent, None);
        assert_eq!(shown.details.context_tokens, "Not reported");
    }

    #[test]
    fn the_run_line_keeps_order_and_each_checks_result() {
        let at = |seconds: u64| 1_759_600_000_000u64 + seconds * 1000;
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Accept empty signatures","timestamp":at(0)},
                {"role":"assistant","content":[{"type":"toolCall","id":"r","name":"read","arguments":{"path":"src/a.ts"}}],"stopReason":"toolUse","timestamp":at(10)},
                {"role":"toolResult","toolCallId":"r","toolName":"read","content":[],"isError":false,"timestamp":at(12)},
                {"role":"assistant","content":[{"type":"toolCall","id":"s","name":"bash","arguments":{"command":"rg test src"}}],"stopReason":"toolUse","timestamp":at(14)},
                {"role":"toolResult","toolCallId":"s","toolName":"bash","content":[],"isError":false,"timestamp":at(15)},
                {"role":"assistant","content":[{"type":"toolCall","id":"e","name":"edit","arguments":{"path":"src/a.ts"}}],"stopReason":"toolUse","timestamp":at(30)},
                {"role":"toolResult","toolCallId":"e","toolName":"edit","content":[],"isError":false,"timestamp":at(35)},
                {"role":"assistant","content":[{"type":"toolCall","id":"c","name":"bash","arguments":{"command":"pnpm tsc --noEmit"}}],"stopReason":"toolUse","timestamp":at(40)},
                {"role":"toolResult","toolCallId":"c","toolName":"bash","content":[{"type":"text","text":"Command exited with code 2"}],"isError":true,"timestamp":at(46)},
                {"role":"assistant","content":[{"type":"toolCall","id":"f","name":"edit","arguments":{"path":"src/a.test.ts"}}],"stopReason":"toolUse","timestamp":at(60)},
                {"role":"toolResult","toolCallId":"f","toolName":"edit","content":[],"isError":false,"timestamp":at(62)},
                {"role":"assistant","content":[{"type":"toolCall","id":"t","name":"bash","arguments":{"command":"CI=1 pnpm test --filter @pi/ai"}}],"stopReason":"toolUse","timestamp":at(70)},
                {"role":"toolResult","toolCallId":"t","toolName":"bash","content":[],"isError":false,"timestamp":at(100)},
                {"role":"assistant","content":[{"type":"text","text":"Done."}],"stopReason":"stop","timestamp":at(110)}
            ]}}),
        ]);
        let shown = project(&pi, facts(&[]));
        let turn = &shown.turns[0];
        let run: Vec<_> = turn
            .run
            .iter()
            .map(|stretch| (stretch.kind, stretch.time.as_secs(), stretch.mark))
            .collect();
        // What happened, in order: the type check that failed, the fix, then
        // the tests that passed and the closing words.
        assert_eq!(
            run,
            [
                (StretchKind::Look, 15, None),
                (StretchKind::Change, 20, None),
                (StretchKind::Check, 11, Some(Mark::Failed)),
                (StretchKind::Change, 16, None),
                (StretchKind::Check, 38, Some(Mark::Passed)),
                (StretchKind::Look, 10, None),
            ]
        );
        // `rg test` only looked: it is in the first stretch, with the read.
        assert_eq!(
            turn.run[0].calls,
            [(StageKind::Understand, 0), (StageKind::Understand, 1)]
        );
        assert_eq!(turn.run[2].calls, [(StageKind::Verify, 0)]);
        assert_eq!(turn.stages[0].what, "Read 1 file · ran 1 command");
        assert_eq!(shown.updated, at(110) / 1000);
    }

    #[test]
    fn checks_run_something_rather_than_mention_it() {
        for command in [
            "cargo test -p app",
            "pnpm --filter @pi/ai test",
            "cd packages/ai && pnpm test",
            "CI=1 npm run lint",
            "npx tsc --noEmit",
            "make",
            "go vet ./...",
        ] {
            assert!(is_check(command), "{command}");
        }
        for command in [
            "rg test src",
            "git commit -m 'add a test'",
            "printf test",
            "cat test.txt",
            "ls tests",
            "grep -rn check .",
        ] {
            assert!(!is_check(command), "{command}");
        }
    }

    #[test]
    fn a_select_offers_every_option_and_answers_the_one_picked() {
        let request = json!({"type":"extension_ui_request","id":"q","method":"select",
            "title":"Where should the jitter go?",
            "options":["In the shared backoff","Only for OpenCode","As a setting","Leave it alone","Something else"]});
        let question = question(&request).unwrap();
        // All five, not the first three.
        assert_eq!(question.choices.len(), 5);
        assert_eq!(question.choices[4].label, "Something else");
        let reply = answer(&request, question.choices[3].answer);
        assert_eq!(reply["value"], "Leave it alone");
        // No to a select picks none of its options.
        assert_eq!(answer(&request, Answer::Deny)["cancelled"], true);
    }

    #[test]
    fn context_use_comes_only_from_current_context_stats() {
        let mut pi = finished_run();
        pi.apply(&json!({
            "type":"response",
            "command":"get_session_stats",
            "success":true,
            "data":{
                "tokens":{"input":900000,"output":20000,"cacheRead":400000},
                "cost":17.62,
                "contextUsage":{"tokens":50000,"contextWindow":200000,"percent":null}
            }
        }))
        .unwrap();
        let shown = project(&pi, facts(&[]));
        assert_eq!(shown.details.context_percent, Some(25));
        assert_eq!(shown.details.context_tokens, "50k of 200k tokens");
        assert_eq!(shown.details.cost, "$17.62");
    }

    #[test]
    fn compaction_details_use_reported_policy_and_background_activity() {
        let mut pi = finished_run();
        pi.apply(&json!({"type":"state_update","state":{
            "autoCompactionEnabled":true,"compactions":[{"taskId":42,"blocking":false}]
        }}))
        .unwrap();
        pi.apply(&json!({"type":"stats_update","stats":{
            "contextUsage":{"tokens":46000,"contextWindow":272000,"percent":null},
            "compaction":{"enabled":true,"reserveTokens":16384,"backgroundTokens":32768}
        }}))
        .unwrap();
        let shown = project(&pi, facts(&[]));
        assert!((shown.details.compaction_percent.unwrap() - 81.9294).abs() < 0.001);
        assert!(shown.details.auto_compaction_enabled);
        assert!(shown.details.background_compacting);
        assert_eq!(shown.details.context_percent, Some(17));

        // Changing the window changes the mark; disabling auto hides it in the view.
        pi.stats.context_usage.as_mut().unwrap().context_window = 200000;
        pi.state.auto_compaction_enabled = false;
        pi.state.compactions.clear();
        let shown = project(&pi, facts(&[]));
        assert!((shown.details.compaction_percent.unwrap() - 75.424).abs() < 0.001);
        assert!(!shown.details.auto_compaction_enabled);
        assert!(!shown.details.background_compacting);
        pi.stats.compaction.as_mut().unwrap().background_tokens = 0;
        assert!(
            (project(&pi, facts(&[])).details.compaction_percent.unwrap() - 91.808).abs() < 0.001
        );
        pi.stats.compaction = None;
        assert_eq!(project(&pi, facts(&[])).details.compaction_percent, None);
    }

    #[test]
    fn a_running_tool_is_the_live_stage() {
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Fix it"}
            ]}}),
            json!({"type":"agent_start"}),
            json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
            json!({"type":"tool_execution_start","toolCallId":"e","toolName":"edit","args":{"path":"/Users/nick/repos/pi/src/lib.rs","edits":[{"oldText":"old();","newText":"new();\nmore();"}]}}),
        ]);
        let shown = project(&pi, facts(&[]));
        assert_eq!(shown.state, State::Working);
        assert_eq!(shown.activity, "Editing lib.rs");
        let stages: Vec<_> = shown.turns[0]
            .stages
            .iter()
            .map(|s| (s.kind, s.status))
            .collect();
        assert!(
            stages.contains(&(StageKind::Change, StageStatus::Live)),
            "{stages:?}"
        );
        assert_eq!(
            stages.last(),
            Some(&(StageKind::HandOff, StageStatus::Planned))
        );
        let tools = &shown.turns[0]
            .stages
            .iter()
            .find(|s| s.kind == StageKind::Change)
            .unwrap()
            .tools;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].target, "/Users/nick/repos/pi/src/lib.rs");
        assert!(!tools[0].finished);
        let change = shown.turns[0]
            .stages
            .iter()
            .find(|stage| stage.kind == StageKind::Change)
            .unwrap();
        assert_eq!((change.added, change.removed), (2, 1));
        assert_eq!(
            change
                .diff
                .iter()
                .map(|line| (line.kind, line.text.as_str()))
                .collect::<Vec<_>>(),
            [
                (LineKind::Removed, "old();"),
                (LineKind::Added, "new();"),
                (LineKind::Added, "more();"),
            ]
        );
    }

    #[test]
    fn rail_revisits_tools_and_returns_to_inference_instead_of_sticking_to_completed_edits() {
        let mut pi = session(&[
            json!({"type":"message_end","message":{"role":"user","content":"offline rail regression"}}),
            json!({"type":"agent_start"}),
        ]);
        for (id, name, args, expected) in [
            (
                "read-1",
                "read",
                json!({"path":"a.rs"}),
                StageKind::Understand,
            ),
            ("edit-1", "edit", json!({"path":"a.rs"}), StageKind::Change),
            (
                "run-1",
                "bash",
                json!({"command":"cargo test -p app"}),
                StageKind::Verify,
            ),
            (
                "read-2",
                "read",
                json!({"path":"b.rs"}),
                StageKind::Understand,
            ),
            (
                "write-2",
                "write",
                json!({"path":"b.rs","content":"test"}),
                StageKind::Change,
            ),
            (
                "js-1",
                "javascript",
                json!({"file":"inspect.cjs"}),
                StageKind::Understand,
            ),
        ] {
            pi.apply(&json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"toolCall","id":id,"name":name,"arguments":args}],"stopReason":"toolUse"}})).unwrap();
            pi.apply(
                &json!({"type":"tool_execution_start","toolCallId":id,"toolName":name,"args":args}),
            )
            .unwrap();
            let active = project(&pi, facts(&[]));
            assert_eq!(active.turns[0].live_stage(), Some(expected), "{name}");
            assert_eq!(
                active.turns[0]
                    .stages
                    .iter()
                    .filter(|stage| stage.status == StageStatus::Live)
                    .count(),
                1
            );
            if name == "javascript" {
                assert_eq!(active.activity, "Running inspect.cjs");
                assert!(
                    active.turns[0]
                        .stages
                        .iter()
                        .flat_map(|stage| &stage.tools)
                        .any(|tool| tool.name == "javascript" && tool.target == "inspect.cjs")
                );
            }
            pi.apply(&json!({"type":"tool_execution_end","toolCallId":id,"toolName":name,"result":{"content":[]},"isError":false})).unwrap();
            pi.apply(&json!({"type":"message_start","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Inspecting the result"}]}})).unwrap();
            let thinking = project(&pi, facts(&[]));
            assert_eq!(thinking.turns[0].live_stage(), Some(StageKind::Understand));
            assert_eq!(thinking.activity, "Thinking");
            assert!(
                thinking.turns[0]
                    .stages
                    .iter()
                    .filter(|stage| stage.kind != StageKind::Understand)
                    .all(|stage| stage.status != StageStatus::Live)
            );
            pi.apply(&json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Inspecting the result"}]}})).unwrap();
        }
    }

    #[test]
    fn inference_status_tracks_the_latest_stream_block_not_an_earlier_text_block() {
        let mut pi = session(&[
            json!({"type":"agent_start"}),
            json!({"type":"message_start","message":{"role":"assistant","content":[{"type":"text","text":"Some commentary"}]}}),
        ]);
        assert_eq!(project(&pi, facts(&[])).activity, "Writing");
        pi.apply(&json!({"type":"message_update","assistantMessageEvent":{"type":"thinking_start","contentIndex":1}})).unwrap();
        assert_eq!(project(&pi, facts(&[])).activity, "Thinking");
        pi.apply(&json!({"type":"message_update","assistantMessageEvent":{"type":"toolcall_start","contentIndex":2,"id":"next","toolName":"edit"}})).unwrap();
        let preparing = project(&pi, facts(&[]));
        assert_eq!(preparing.activity, "Preparing tool");
        assert_eq!(preparing.turns[0].live_stage(), Some(StageKind::Understand));
    }

    #[test]
    fn an_unfinished_tool_from_an_earlier_turn_does_not_leak_into_a_follow_up() {
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Fix it"},
                {"role":"assistant","content":[{"type":"toolCall","id":"old-edit","name":"edit","arguments":{"path":"/Users/nick/repos/pi/src/app.rs","oldText":"old","newText":"new"}}]}
            ]}}),
            json!({"type":"tool_execution_start","toolCallId":"old-edit","toolName":"edit","args":{"path":"/Users/nick/repos/pi/src/app.rs","oldText":"old","newText":"new"}}),
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Fix it"},
                {"role":"assistant","content":[{"type":"toolCall","id":"old-edit","name":"edit","arguments":{"path":"/Users/nick/repos/pi/src/app.rs","oldText":"old","newText":"new"}}]},
                {"role":"user","content":"One more thing"}
            ]}}),
            json!({"type":"agent_start"}),
        ]);
        let shown = project(&pi, facts(&[]));
        assert_eq!(shown.state, State::Working);
        assert_eq!(shown.activity, "Working");
        let latest = shown.turns.last().unwrap();
        assert!(
            latest
                .stages
                .iter()
                .all(|stage| stage.kind != StageKind::Change),
            "an old edit must not become the new turn's live stage: {:?}",
            latest
                .stages
                .iter()
                .map(|stage| (stage.kind, stage.status, stage.what.as_str()))
                .collect::<Vec<_>>()
        );
        assert_eq!(latest.live_stage(), Some(StageKind::Understand));
    }

    #[test]
    fn an_optimistic_phone_follow_up_does_not_inherit_a_stale_tool() {
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Fix it"},
                {"role":"assistant","content":[{"type":"toolCall","id":"old-edit","name":"edit","arguments":{"path":"/Users/nick/repos/pi/src/app.rs","oldText":"old","newText":"new"}}]}
            ]}}),
            json!({"type":"tool_execution_start","toolCallId":"old-edit","toolName":"edit","args":{"path":"/Users/nick/repos/pi/src/app.rs","oldText":"old","newText":"new"}}),
            json!({"type":"agent_settled"}),
            // The helper can announce the accepted run before its snapshot has
            // appended the new user message.
            json!({"type":"agent_start"}),
        ]);
        let outbox = vec![PendingPrompt {
            text: "One more thing".into(),
            key: Some(QueueKey::Local("one-more".into())),
            starts_turn: true,
        }];
        let shown = project(&pi, facts(&outbox));
        assert_eq!(shown.state, State::Working);
        assert_eq!(shown.activity, "Working");
        assert_eq!(shown.turns.last().unwrap().prompt, "One more thing");
        assert!(
            shown
                .turns
                .last()
                .unwrap()
                .stages
                .iter()
                .all(|stage| stage.kind != StageKind::Change)
        );
        assert_eq!(
            shown.turns.last().unwrap().live_stage(),
            Some(StageKind::Understand)
        );
    }

    #[test]
    fn compaction_is_named_even_when_an_old_tool_is_unfinished() {
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Fix it"},
                {"role":"assistant","content":[{"type":"toolCall","id":"old-edit","name":"edit","arguments":{"path":"/Users/nick/repos/pi/src/app.rs"}}]}
            ]}}),
            json!({"type":"tool_execution_start","toolCallId":"old-edit","toolName":"edit","args":{"path":"/Users/nick/repos/pi/src/app.rs"}}),
            json!({"type":"compaction_start"}),
        ]);
        let shown = project(&pi, facts(&[]));
        assert_eq!(shown.state, State::Working);
        assert_eq!(shown.activity, "Compacting");
    }

    #[test]
    fn each_call_keeps_its_full_live_diff_and_prefers_the_applied_result() {
        let replacement = (0..24)
            .map(|n| format!("new_{n}();"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut pi = session(&[
            json!({"type":"agent_start"}),
            json!({"type":"message_end","message":{"role":"user","content":"Edit two files"}}),
            json!({"type":"message_end","message":{"role":"assistant","content":[
                {"type":"toolCall","id":"first","name":"edit","arguments":{
                    "path":"first.rs", "edits":[{"oldText":"old();", "newText":replacement}]
                }}
            ]}}),
            json!({"type":"tool_execution_start","toolCallId":"first","toolName":"edit","args":{
                "path":"first.rs", "edits":[{"oldText":"old();", "newText":replacement}]
            }}),
        ]);
        let shown = project(&pi, facts(&[]));
        let first = shown.turns[0].tool(StageKind::Change, 0).unwrap();
        assert_eq!(
            first.diff.len(),
            25,
            "the call is not limited to the stage's eight-line preview"
        );
        assert!(first.diff_is_preview);
        assert!(!first.finished);
        assert_eq!(first.diff.last().unwrap().text, "new_23();");
        assert!(first.diff.iter().all(|line| line.number == 0));

        pi.apply(&json!({"type":"tool_execution_end","toolCallId":"first","toolName":"edit",
            "result":{"content":[{"type":"text","text":"Updated"}],"details":{"diff":DIFF}},"isError":false
        })).unwrap();
        pi.apply(
            &json!({"type":"message_end","message":{"role":"assistant","content":[
                {"type":"toolCall","id":"second","name":"edit","arguments":{
                    "path":"second.rs", "oldText":"before();", "newText":"after();"
                }}
            ]}}),
        )
        .unwrap();
        pi.apply(
            &json!({"type":"tool_execution_start","toolCallId":"second","toolName":"edit","args":{
                "path":"second.rs", "oldText":"before();", "newText":"after();"
            }}),
        )
        .unwrap();
        let shown = project(&pi, facts(&[]));
        let first = shown.turns[0].tool(StageKind::Change, 0).unwrap();
        assert!(first.finished);
        assert!(!first.diff_is_preview);
        assert_eq!(
            first.diff,
            parse_diff(DIFF)
                .into_iter()
                .flat_map(|h| h.lines)
                .collect::<Vec<_>>()
        );
        let second = shown.turns[0].tool(StageKind::Change, 1).unwrap();
        assert!(second.diff_is_preview);
        assert_eq!(second.diff[0].text, "before();");
        assert_eq!(second.diff[1].text, "after();");

        pi.apply(
            &json!({"type":"tool_execution_end","toolCallId":"second","toolName":"edit",
                "result":{"content":[{"type":"text","text":"Not found"}]},"isError":true
            }),
        )
        .unwrap();
        let shown = project(&pi, facts(&[]));
        let second = shown.turns[0].tool(StageKind::Change, 1).unwrap();
        assert!(second.failed);
        assert!(
            second.diff_is_preview,
            "failed replacements must not claim an applied diff"
        );
    }

    #[test]
    fn a_running_write_previews_its_content() {
        let tool = Tool {
            id: "w".into(),
            started: None,
            name: "write".into(),
            args: json!({"path":"src/new.rs","content":"first\nsecond"}),
            output: String::new(),
            diff: None,
            finished: false,
            is_error: false,
            ended_at: None,
            images: Vec::new(),
            details: Value::Null,
        };
        assert_eq!(
            tool_lines(&tool)
                .iter()
                .map(|line| (line.kind, line.number, line.text.as_str()))
                .collect::<Vec<_>>(),
            [
                (LineKind::Added, 1, "first"),
                (LineKind::Added, 2, "second"),
            ]
        );
    }

    #[test]
    fn commentary_and_streaming_tool_output_stay_visible() {
        let mut pi = session(&[
            json!({"type":"agent_start"}),
            json!({"type":"message_end","message":{"role":"user","content":"Check it"}}),
            json!({"type":"message_end","message":{"role":"assistant","content":[
                {"type":"text","text":"I am checking the tests."},
                {"type":"toolCall","id":"b","name":"bash","arguments":{"command":"cargo test"}}
            ],"stopReason":"toolUse"}}),
            json!({"type":"tool_execution_start","toolCallId":"b","toolName":"bash","args":{"command":"cargo test"}}),
            json!({"type":"tool_execution_update","toolCallId":"b","partialResult":{"content":[{"type":"text","text":"starting\r\nrunning 10%\rrunning 100%"}]}}),
        ]);
        let shown = project(&pi, facts(&[]));
        let turn = &shown.turns[0];
        assert_eq!(
            turn.summary.as_ref().unwrap().text(),
            "I am checking the tests."
        );
        let tools = &turn
            .stages
            .iter()
            .find(|s| s.kind == StageKind::Verify)
            .unwrap()
            .tools;
        assert_eq!(
            tools.len(),
            1,
            "execution events must not duplicate tool calls"
        );
        assert_eq!(tools[0].output, "starting\r\nrunning 10%\rrunning 100%");
        assert_eq!(tools[0].output_for_display(), "starting\nrunning 100%");
        pi.apply(&json!({"type":"message_end","message":{"role":"assistant","content":"All tests passed.","stopReason":"stop"}})).unwrap();
        assert_eq!(
            project(&pi, facts(&[])).turns[0]
                .summary
                .as_ref()
                .unwrap()
                .text(),
            "I am checking the tests.\n\nAll tests passed."
        );
    }

    #[test]
    fn reasoning_comes_before_what_pi_said_and_redacted_reasoning_is_skipped() {
        use crate::model::Flow;
        let mut pi = session(&[
            json!({"type":"agent_start"}),
            json!({"type":"message_end","message":{"role":"user","content":"Why?"}}),
            json!({"type":"message_end","message":{"role":"assistant","content":[
                {"type":"thinking","thinking":"First, the cache."},
                {"type":"redacted_thinking","data":"opaque"},
                {"type":"thinking","thinking":"Then the index."},
                {"type":"text","text":"Because of the cache."}
            ],"stopReason":"stop"}}),
        ]);
        let flow = project(&pi, facts(&[])).turns[0].flow.clone();
        assert_eq!(
            flow,
            vec![
                Flow::Thinking("First, the cache.\n\nThen the index.".into()),
                Flow::Text("Because of the cache.".into()),
            ]
        );
        // Reasoning alone does not make a reply read as interleaved media.
        assert!(!project(&pi, facts(&[])).turns[0].interleaved());
        // Streaming reasoning shows before any text arrives.
        pi.apply(&json!({"type":"message_end","message":{"role":"user","content":"And?"}}))
            .unwrap();
        pi.apply(
            &json!({"type":"message_start","message":{"role":"assistant","content":[
                {"type":"thinking","thinking":"Looking"}
            ]}}),
        )
        .unwrap();
        let shown = project(&pi, facts(&[]));
        assert_eq!(
            shown.turns.last().unwrap().flow,
            vec![Flow::Thinking("Looking".into())]
        );
    }

    #[test]
    fn a_prompt_from_the_phone_shows_before_pi_takes_it() {
        let outbox = [PendingPrompt {
            text: "Explain this project".into(),
            key: Some(QueueKey::Local("explain".into())),
            starts_turn: true,
        }];
        let shown = project(&Pi::new("/Users/nick/repos/pi".into()), facts(&outbox));
        assert_eq!(shown.state, State::Working);
        assert_eq!(shown.title, "Explain this project");
        assert!(
            shown.queued.is_empty(),
            "the first prompt is the run, not a queued one"
        );
        assert_eq!(shown.turns.len(), 1);
        assert_eq!(shown.turns[0].prompt, "Explain this project");
        assert_eq!(shown.turns[0].stages[0].status, StageStatus::Live);
    }

    #[test]
    fn stopped_and_failed_runs_say_so() {
        let ended = |stop: &str| {
            session(&[
                json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                    {"role":"user","content":"Go"},
                    {"role":"assistant","content":[],"stopReason":stop,"errorMessage":"429 rate limited"}
                ]}}),
            ])
        };
        assert_eq!(project(&ended("aborted"), facts(&[])).state, State::Stopped);
        assert_eq!(project(&ended("toolUse"), facts(&[])).state, State::Stopped);
        let answered = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Hi"},
                {"role":"assistant","content":[{"type":"text","text":"Hello."}],"stopReason":"stop"}
            ]}}),
        ]);
        let answered = project(&answered, facts(&[]));
        assert!(answered.turns[0].stages.is_empty(), "no tools, no stages");
        assert_eq!(
            answered.turns[0].summary.as_ref().unwrap().headline,
            "Hello."
        );
        let failed = project(&ended("error"), facts(&[]));
        assert_eq!(failed.state, State::Failed);
        assert_eq!(failed.failure.as_deref(), Some("429 rate limited"));
    }

    #[test]
    fn diffs_keep_pi_line_numbers() {
        let hunks = parse_diff(DIFF);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].header, "@@ 210");
        assert_eq!(
            hunks[0].lines[1],
            DiffLine::new(LineKind::Removed, 211, "old();")
        );
        assert_eq!(
            hunks[0].lines[2],
            DiffLine::new(LineKind::Added, 211, "new();")
        );
        assert_eq!(hunks[1].lines[1], DiffLine::new(LineKind::Added, 301, "y"));
    }

    #[test]
    fn questions_answer_as_pi_expects() {
        let confirm = json!({"type":"extension_ui_request","id":"q","method":"confirm","title":"Run the tests?","message":"cargo test"});
        let question = question(&confirm).unwrap();
        assert_eq!(question.choices.len(), 2);
        assert_eq!(answer(&confirm, Answer::AllowOnce)["confirmed"], true);
        assert_eq!(answer(&confirm, Answer::Deny)["confirmed"], false);
        let select = json!({"type":"extension_ui_request","id":"s","method":"select","title":"Which?","options":["Block","Allow"]});
        assert_eq!(super::question(&select).unwrap().choices[1].label, "Allow");
        assert_eq!(answer(&select, Answer::AllowSession)["value"], "Allow");
        assert_eq!(answer(&select, Answer::Deny)["cancelled"], true);
    }

    #[test]
    fn images_submitted_with_a_prompt_remain_available_to_open() {
        let id = "b".repeat(64);
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":[
                    {"type":"text","text":"Inspect this screenshot"},
                    {"type":"image","data":"","imageId":id,"mimeType":"image/png","bytes":10}
                ]},
                {"role":"assistant","content":[{"type":"text","text":"I can see it."}],"stopReason":"stop"}
            ]}}),
        ]);
        let shown = project(&pi, facts(&[]));
        let turn = &shown.turns[0];
        assert_eq!(turn.prompt, "Inspect this screenshot");
        assert_eq!(turn.attachments, ["1 image"]);
        assert_eq!(turn.prompt_images.len(), 1);
        assert_eq!(turn.prompt_images[0].key, id);
        assert_eq!(turn.prompt_images[0].mime, "image/png");
        assert_eq!(turn.prompt_images[0].inline, None);
        assert!(turn.images.is_empty());
    }

    #[test]
    fn images_tools_return_show_under_their_turn_named_by_the_file() {
        let id = "a".repeat(64);
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Take a screenshot of the page"},
                {"role":"assistant","content":[
                    {"type":"toolCall","id":"r","name":"read","arguments":{"path":"/tmp/shots/page.png"}},
                    {"type":"toolCall","id":"b","name":"bash","arguments":{"command":"chrome --screenshot"}}
                ],"stopReason":"toolUse"},
                {"role":"toolResult","toolCallId":"r","toolName":"read","content":[
                    {"type":"text","text":"Read image file [image/png]"},
                    {"type":"image","data":"","imageId":id,"mimeType":"image/png","bytes":10}
                ],"isError":false},
                {"role":"toolResult","toolCallId":"b","toolName":"bash","content":[
                    {"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"}
                ],"isError":false},
                {"role":"assistant","content":[{"type":"text","text":"Here it is."}],"stopReason":"stop"}
            ]}}),
        ]);
        let images = &project(&pi, facts(&[])).turns[0].images;
        assert_eq!(images.len(), 2);
        assert_eq!(
            (images[0].key.as_str(), images[0].name.as_str()),
            (id.as_str(), "page.png")
        );
        assert_eq!(images[0].inline, None);
        assert_eq!(images[1].name, "Image");
        assert_eq!(images[1].inline.as_deref(), Some("iVBORw0KGgo="));
    }

    #[test]
    fn pages_pi_wrote_show_under_their_turn() {
        let page = "<html><title>Aurora</title><body>blue</body></html>";
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Make me something cool"},
                {"role":"assistant","content":[
                    {"type":"toolCall","id":"w","name":"write","arguments":{"path":"/Users/nick/repos/pi/aurora.html","content":page}},
                    {"type":"toolCall","id":"x","name":"write","arguments":{"path":"broken.html","content":"<p>"}}
                ],"stopReason":"toolUse"},
                {"role":"toolResult","toolCallId":"w","toolName":"write","content":[],"isError":false},
                {"role":"toolResult","toolCallId":"x","toolName":"write","content":[],"isError":true},
                {"role":"assistant","content":[{"type":"text","text":"Done."}],"stopReason":"stop"},
                {"role":"user","content":"Make it green"},
                {"role":"assistant","content":[
                    {"type":"toolCall","id":"e","name":"edit","arguments":{"path":"aurora.html","edits":[{"oldText":"blue","newText":"green"}]}}
                ],"stopReason":"toolUse"},
                {"role":"toolResult","toolCallId":"e","toolName":"edit","content":[],"isError":false},
                {"role":"assistant","content":[{"type":"text","text":"Green now."}],"stopReason":"stop"}
            ]}}),
        ]);
        let shown = project(&pi, facts(&[]));
        let pages: Vec<Vec<(&str, Option<&str>)>> = shown
            .turns
            .iter()
            .map(|turn| {
                turn.pages
                    .iter()
                    .map(|page| (page.path.as_str(), page.html.as_deref()))
                    .collect()
            })
            .collect();
        assert_eq!(
            pages,
            [
                vec![("aurora.html", Some(page))],
                vec![(
                    "aurora.html",
                    Some("<html><title>Aurora</title><body>green</body></html>")
                )],
            ]
        );
        assert_eq!(shown.turns[0].pages[0].title(), "Aurora");
    }

    #[test]
    fn words_pictures_and_pages_read_in_the_order_pi_made_them() {
        use crate::model::Flow;
        let page = "<html><title>Aurora</title><body>blue</body></html>";
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Improve the page and check how it looks"},
                {"role":"assistant","content":[
                    {"type":"text","text":"Writing a first version."},
                    {"type":"toolCall","id":"w","name":"write","arguments":{"path":"aurora.html","content":page}}
                ],"stopReason":"toolUse"},
                {"role":"toolResult","toolCallId":"w","toolName":"write","content":[],"isError":false},
                {"role":"assistant","content":[
                    {"type":"text","text":"Taking a look."},
                    {"type":"toolCall","id":"r1","name":"read","arguments":{"path":"one.png"}}
                ],"stopReason":"toolUse"},
                {"role":"toolResult","toolCallId":"r1","toolName":"read","content":[
                    {"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"}
                ],"isError":false},
                {"role":"assistant","content":[
                    {"type":"text","text":"Too dark; brightening it."},
                    {"type":"toolCall","id":"e","name":"edit","arguments":{"path":"aurora.html","edits":[{"oldText":"blue","newText":"green"}]}},
                    {"type":"toolCall","id":"r2","name":"read","arguments":{"path":"two.png"}}
                ],"stopReason":"toolUse"},
                {"role":"toolResult","toolCallId":"e","toolName":"edit","content":[],"isError":false},
                {"role":"toolResult","toolCallId":"r2","toolName":"read","content":[
                    {"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"}
                ],"isError":false},
                {"role":"assistant","content":[{"type":"text","text":"Better now."}],"stopReason":"stop"}
            ]}}),
        ]);
        let turn = &project(&pi, facts(&[])).turns[0];
        assert!(turn.interleaved());
        assert!(turn.streams());
        use crate::model::StageKind::{Change, Understand};
        assert_eq!(
            turn.flow,
            [
                Flow::Text("Writing a first version.".into()),
                Flow::Tool(Change, 0),
                Flow::Text("Taking a look.".into()),
                Flow::Tool(Understand, 0),
                Flow::Image(0),
                Flow::Text("Too dark; brightening it.".into()),
                Flow::Tool(Change, 1),
                Flow::Page(0),
                Flow::Tool(Understand, 1),
                Flow::Image(1),
                Flow::Text("Better now.".into()),
            ]
        );
        // Each call keeps its own change size: the edit swapped one line.
        let edit = turn.tool(Change, 1).expect("the edit");
        assert_eq!(edit.name, "edit");
        assert_eq!(turn.call_counts(), (4, 0));
        assert_eq!(turn.images[1].name, "two.png");
    }

    #[test]
    fn only_checking_commands_count_as_checks() {
        assert!(is_check("cargo test -p app"));
        assert!(is_check("npm run lint"));
        assert!(!is_check("ls -la src"));
        assert!(!is_check("grep -rn testing ."));
    }

    #[test]
    fn work_handed_to_subagents_shows_as_a_handoff_not_a_stage() {
        let args = json!({"tasks":[{"agent":"scout","task":"find alpha"},{"agent":"scout","task":"find beta"}]});
        let details = json!({"version":1,"mode":"parallel","results":[
            {"index":0,"agent":"scout","task":"find alpha","status":"done","conversationId":"3","output":"found alpha"},
            {"index":1,"agent":"scout","task":"find beta","status":"running","conversationId":"4","now":"Reading beta.ts"}]});
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"look around"},
                {"role":"assistant","content":[{"type":"toolCall","id":"hand","name":"subagent","arguments":args}]}]}}),
            json!({"type":"agent_start"}),
            json!({"type":"tool_execution_start","toolCallId":"hand","toolName":"subagent","args":args}),
            json!({"type":"tool_execution_update","toolCallId":"hand","toolName":"subagent","partialResult":{"content":[],"details":details}}),
        ]);
        let shown = project(&pi, facts(&[]));
        let turn = &shown.turns[0];
        assert_eq!(turn.flow, [Flow::Handoff(0)]);
        let handoff = &turn.handoffs[0];
        assert_eq!(handoff.subagents.len(), 2);
        assert_eq!(handoff.subagents[1].now.as_deref(), Some("Reading beta.ts"));
        // The call itself isn't a tool Pi ran in a stage; the stage waits on it.
        assert!(turn.stages.iter().all(|stage| stage.tools.is_empty()));
        assert_eq!(turn.stages[0].status, StageStatus::Live);
        assert_eq!(turn.stages[0].what, "Waiting on the scout");
        assert_eq!(shown.activity, "Waiting on the scout");
    }

    #[test]
    fn a_session_wide_subagent_directory_survives_compaction_and_completion() {
        use pi_core::subagent::Status;

        let first_args = json!({"tasks":[
            {"agent":"scout","task":"find alpha"},
            {"agent":"reviewer","task":"check alpha"}
        ]});
        let second_args = json!({"chain":[
            {"agent":"worker","task":"fix beta"},
            {"agent":"reviewer","task":"review beta"}
        ]});
        let first_details = json!({"version":1,"mode":"parallel","background":true,"results":[
            {"index":0,"agent":"scout","task":"find alpha","status":"done","conversationId":"1","output":"found alpha"},
            {"index":1,"agent":"reviewer","task":"check alpha","status":"failed","conversationId":"2","now":"Could not run tests"}
        ]});
        let second_details = json!({"version":1,"mode":"chain","background":true,"results":[
            {"index":0,"agent":"worker","task":"fix beta","status":"running","conversationId":"3","now":"Editing beta.rs"},
            {"index":1,"agent":"reviewer","task":"review beta","status":"waiting"}
        ]});
        // This is the shape received after reconnecting: full durable history,
        // including the compaction marker, plus current tool summaries.
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"first turn"},
                {"role":"assistant","content":[{"type":"toolCall","id":"first","name":"subagent","arguments":first_args}]},
                {"role":"toolResult","toolCallId":"first","toolName":"subagent","content":[],"isError":false},
                {"role":"assistant","content":"First turn done."},
                {"role":"compactionSummary","summary":"The first turn was compacted."},
                {"role":"user","content":"second turn"},
                {"role":"assistant","content":[{"type":"toolCall","id":"second","name":"subagent","arguments":second_args}]},
                {"role":"toolResult","toolCallId":"second","toolName":"subagent","content":[],"isError":false},
                {"role":"assistant","content":"The background work can continue.","stopReason":"stop"}
            ]}}),
            json!({"type":"tool_execution_start","toolCallId":"first","toolName":"subagent","args":first_args}),
            json!({"type":"tool_execution_update","toolCallId":"first","toolName":"subagent","partialResult":{"content":[],"details":first_details}}),
            json!({"type":"tool_execution_start","toolCallId":"second","toolName":"subagent","args":second_args}),
            json!({"type":"tool_execution_update","toolCallId":"second","toolName":"subagent","partialResult":{"content":[],"details":second_details}}),
        ]);
        let shown = project(&pi, facts(&[]));
        assert_eq!(shown.state, State::Done, "the parent run has completed");
        assert!(
            shown.turns[0]
                .flow
                .iter()
                .any(|step| matches!(step, Flow::Compacted(_)))
        );
        assert_eq!(
            session_subagents(&shown.turns)
                .into_iter()
                .map(|item| (item.turn, item.handoff, item.index, item.subagent.status))
                .collect::<Vec<_>>(),
            [
                (1, 0, 0, Status::Running),
                (1, 0, 1, Status::Waiting),
                (0, 0, 0, Status::Done),
                (0, 0, 1, Status::Failed),
            ]
        );
        assert_eq!(
            subagent_summary(&shown.turns).as_deref(),
            Some("1 working · 1 waiting · 2 finished (1 failed)")
        );
    }

    #[test]
    fn a_call_without_its_details_yet_lists_what_it_was_asked() {
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":"plan it"},
            {"role":"assistant","content":[{"type":"toolCall","id":"hand","name":"subagent","arguments":{"chain":[
                {"agent":"scout","task":"look"},{"agent":"planner","task":"plan {previous}"}]}}]}]}}),
        ]);
        let shown = project(&pi, facts(&[]));
        let handoff = &shown.turns[0].handoffs[0];
        assert_eq!(handoff.mode, pi_core::subagent::Mode::Chain);
        assert_eq!(
            handoff
                .subagents
                .iter()
                .map(|s| s.agent.as_str())
                .collect::<Vec<_>>(),
            ["scout", "planner"]
        );
    }

    #[test]
    fn queued_subagent_reports_name_whether_they_steer_or_follow_up() {
        let report = "<subagent_report call=\"hand\">\nreport\n</subagent_report>".to_owned();
        let mut pi = Pi::default();
        pi.steering.push(report.clone());
        pi.follow_up.push(report);
        assert_eq!(
            project(&pi, facts(&[])).queued,
            [
                "Subagents reported back · steering this run",
                "Subagents reported back · next turn",
            ]
        );
    }

    #[test]
    fn a_skill_prompt_reads_as_it_was_typed() {
        let block = "<skill name=\"lint\" location=\"/home/me/.pi/agent/skills/lint/SKILL.md\">\nReferences are relative to /home/me/.pi/agent/skills/lint.\n\nRun the linter.\n</skill>";
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":format!("{block}\n\nonly src")},
            {"role":"user","content":block}]}}),
        ]);
        let shown = project(&pi, facts(&[]));
        assert_eq!(shown.turns[0].prompt, "/skill:lint only src");
        assert_eq!(shown.turns[1].prompt, "/skill:lint");
    }

    #[test]
    fn subagents_reporting_back_is_not_the_users_prompt() {
        let report = "<subagent_report call=\"hand\">\nThe subagents you started have finished. This is their report, not a message from the user.\n\n## scout: find alpha\n\nfound alpha\n</subagent_report>";
        let pi = session(&[
            json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":"look around"},
            {"role":"user","content":report}]}}),
        ]);
        let shown = project(&pi, facts(&[]));
        assert!(!shown.turns[0].reported);
        assert!(shown.turns[1].reported);
        assert_eq!(
            shown.turns[1].prompt,
            "Subagents reported back\n\n## scout: find alpha\n\nfound alpha"
        );
    }
}

#[cfg(test)]
mod visual_regressions {
    use super::*;
    fn facts() -> Facts<'static> {
        Facts {
            id: SessionId(1),
            cwd: "/repo",
            folder: "repo".into(),
            question: None,
            outbox: &[],
            key: "durable-session-key",
        }
    }
    #[test]
    fn queued_contracts_are_hidden_without_mutating_authoritative_payloads() {
        let mut pi = Pi::default();
        let wire = crate::visualize_prompt::expand("@Visualize make a counter");
        pi.steering = vec![wire.clone()];
        let projected = project(&pi, facts());
        assert_eq!(projected.queued, ["@Visualize make a counter"]);
        assert_eq!(pi.steering, [wire]);
    }
    #[test]
    fn visual_message_identity_survives_history_rebasing() {
        let mut pi = Pi::default();
        pi.messages = vec![
            serde_json::json!({"role":"user","timestamp":100,"content":"Earlier"}),
            serde_json::json!({"role":"user","timestamp":200,"content":"@Visualize make a counter"}),
        ];
        let original = project(&pi, facts()).turns[1].visual_identity.clone();
        pi.messages.remove(0);
        assert_eq!(project(&pi, facts()).turns[0].visual_identity, original);
        let mut other = facts();
        other.key = "other-session-key";
        assert_ne!(project(&pi, other).turns[0].visual_identity, original);
    }
}
