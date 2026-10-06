//! A session on the computer, as Pi's state, turned into what the phone shows:
//! each prompt with the stages its tools went through, the files Pi changed,
//! the last check it ran, and how the run ended. Nothing is made up: a stage
//! appears only once a tool of its kind ran, and Review shows Pi's own diffs.

use crate::model::*;
use pi_core::session::{Session as Pi, Tool};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// What the phone knows about a session besides Pi's state.
pub struct Facts<'a> {
    pub id: SessionId,
    /// The project folder on the computer, and as shown: `~/repos/pi`.
    pub cwd: &'a str,
    pub folder: String,
    /// Pi's open question, from an extension.
    pub question: Option<Question>,
    /// Prompts sent from the phone that Pi hasn't taken yet.
    pub outbox: &'a [String],
    pub key: &'a str,
}

pub fn project(pi: &Pi, facts: Facts) -> Session {
    let turns = turns(pi, facts.cwd);
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
    // The phone's prompt shows at once, before the computer has it.
    if let Some(prompt) = facts.outbox.first().filter(|_| !pi.busy()) {
        let mut turn = Turn::new(prompt.clone(), clock_now());
        turn.stages.clear();
        turns.push(turn);
    }
    if let Some(turn) = turns.last_mut() {
        settle(turn, state, pi, facts.cwd);
        // The stage Pi is in has gone on since the last message.
        if state.is_running()
            && let Some(live) = turn.live_stage()
            && let Some(since) = pi.messages.iter().rev().find_map(timestamp)
        {
            turn.add_time(live, now().saturating_sub(since));
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
    let mut queued: Vec<String> = pi.steering.iter().chain(&pi.follow_up).cloned().collect();
    queued.extend(facts.outbox.iter().skip(usize::from(!pi.busy())).cloned());
    let project = facts
        .cwd
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(facts.cwd)
        .to_owned();
    Session {
        id: facts.id,
        title: title(pi, facts.outbox),
        project,
        folder: facts.folder,
        state,
        activity: activity(pi),
        elapsed,
        finished_at: (!state.is_running()).then(|| ended.map(clock_at)).flatten(),
        turns,
        files: files(pi, facts.cwd),
        check: check(pi),
        question: facts.question,
        queued,
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
        turns: Vec::new(),
        files: Vec::new(),
        check: None,
        question: None,
        queued: Vec::new(),
        failure: (state == State::Failed)
            .then(|| listed.error.clone().unwrap_or_else(|| "failed".into())),
        details: Details {
            context_percent: 0,
            context_tokens: String::new(),
            cost: String::new(),
            turns: 0,
            tools: Vec::new(),
            snapshots: 0,
            session_file: format!("Durable session {}", &listed.key[..8.min(listed.key.len())]),
        },
        script: None,
    }
}

fn title(pi: &Pi, outbox: &[String]) -> String {
    let title = pi.title();
    if !title.is_empty() && title != "New session" {
        return title.to_owned();
    }
    outbox
        .first()
        .map(|prompt| crate::demo::title_for(prompt))
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

fn kind(tool: &str) -> StageKind {
    match tool {
        "edit" | "write" => StageKind::Change,
        "bash" => StageKind::Verify,
        _ => StageKind::Understand,
    }
}

fn turns(pi: &Pi, cwd: &str) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    let mut pages = crate::pages::Pages::default();
    // The run line's stretches: the time up to each message goes to what that
    // message did, and a tool's result to the tool's stage.
    let mut clock: Option<Duration> = None;
    let mut last_kind = StageKind::Understand;
    for message in &pi.messages {
        let at = timestamp(message);
        let spent = at
            .zip(clock)
            .map_or(Duration::ZERO, |(at, clock)| at.saturating_sub(clock));
        if at.is_some() {
            clock = at;
        }
        match message["role"].as_str() {
            Some("user") => {
                let mut turn = Turn::new(
                    text_of(message),
                    timestamp(message).map(clock_at).unwrap_or_default(),
                );
                turn.stages.clear();
                let images = message["content"].as_array().map_or(0, |blocks| {
                    blocks.iter().filter(|b| b["type"] == "image").count()
                });
                if images > 0 {
                    turn.attachments.push(match images {
                        1 => "1 image".into(),
                        count => format!("{count} images"),
                    });
                }
                turns.push(turn);
            }
            Some("toolResult") => {
                let tool = message["toolName"].as_str().map_or(last_kind, kind);
                if let Some(turn) = turns.last_mut() {
                    turn.add_time(tool, spent);
                }
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
                    .map(|block| kind(block["name"].as_str().unwrap_or("")));
                if let Some(first_tool) = first_tool {
                    last_kind = first_tool;
                }
                turn.add_time(first_tool.unwrap_or(StageKind::HandOff), spent);
                for block in message["content"].as_array().into_iter().flatten() {
                    if block["type"] != "toolCall" {
                        continue;
                    }
                    let id = block["id"].as_str().unwrap_or("");
                    let name = block["name"].as_str().unwrap_or("");
                    let observed = tool(pi, id);
                    if let Some(observed) = observed {
                        let path = observed.args["path"].as_str();
                        turn.images.extend(tool_images(&observed.images, path));
                    }
                    let args = observed.map_or(&block["arguments"], |tool| &tool.args);
                    add_tool(turn, id, name, args, observed, cwd);
                    if observed.is_some_and(|tool| tool.finished && !tool.is_error)
                        && let Some(path) = args["path"].as_str()
                        && let Some(page) = pages.follow(name, args, &relative(path, cwd))
                    {
                        crate::pages::show(&mut turn.pages, page);
                    }
                }
                let text = text_of(message);
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
    turns
}

/// The images in a tool's result, named by the file it read.
fn tool_images(images: &[Value], path: Option<&str>) -> Vec<crate::model::ToolImage> {
    let count = images.len();
    images
        .into_iter()
        .enumerate()
        .filter_map(|(index, image)| {
            let mime = image["mimeType"].as_str()?.to_owned();
            let data = image["data"].as_str().filter(|data| !data.is_empty());
            let key = match (image["imageId"].as_str(), data) {
                (Some(id), _) => id.to_owned(),
                (None, Some(data)) => {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    data.hash(&mut hasher);
                    format!("inline-{:016x}", hasher.finish())
                }
                (None, None) => return None,
            };
            let file = path
                .map(|path| path.rsplit('/').next().unwrap_or(path).to_owned())
                .unwrap_or_else(|| "Image".into());
            Some(crate::model::ToolImage {
                key,
                name: if count > 1 {
                    format!("{file} ({})", index + 1)
                } else {
                    file
                },
                mime,
                inline: data.map(str::to_owned),
            })
        })
        .collect()
}

fn add_tool(
    turn: &mut Turn,
    id: &str,
    name: &str,
    args: &Value,
    observed: Option<&Tool>,
    cwd: &str,
) {
    let kind = kind(name);
    let stage = turn.stage_mut(kind);
    stage.tools.push(crate::model::ToolActivity {
        id: id.to_owned(),
        name: name.to_owned(),
        target: args["path"]
            .as_str()
            .or_else(|| args["command"].as_str())
            .or_else(|| args["pattern"].as_str())
            .unwrap_or("")
            .to_owned(),
        output: observed.map_or_else(String::new, |tool| tool.output.clone()),
        finished: observed.is_some_and(|tool| tool.finished),
        failed: observed.is_some_and(|tool| tool.is_error),
    });
    stage.status = StageStatus::Done;
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
            let read = match files {
                0 => None,
                1 => Some("Read 1 file".to_owned()),
                count => Some(format!("Read {count} files")),
            };
            let searched = match searches {
                0 => None,
                1 => Some("searched once".to_owned()),
                2 => Some("searched twice".to_owned()),
                count => Some(format!("searched {count} times")),
            };
            stage.what = match (read, searched) {
                (Some(read), Some(searched)) => format!("{read} · {searched}"),
                (Some(read), None) => read,
                (None, Some(searched)) => {
                    let mut searched = searched;
                    searched[..1].make_ascii_uppercase();
                    searched
                }
                (None, None) => "Looked around".into(),
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
            if let Some(tool) = observed {
                let lines = tool_lines(tool);
                stage.added += lines.iter().filter(|l| l.kind == LineKind::Added).count() as u32;
                stage.removed +=
                    lines.iter().filter(|l| l.kind == LineKind::Removed).count() as u32;
                if !lines.is_empty() {
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
fn settle(turn: &mut Turn, state: State, pi: &Pi, cwd: &str) {
    if state.is_running() {
        let running = pi.tools.iter().rev().find(|tool| !tool.finished);
        let live = running.map(|tool| kind(&tool.name)).or_else(|| {
            // Between tools, Pi is reading its results or writing.
            turn.stages.last().map(|stage| stage.kind)
        });
        match live {
            Some(live) => {
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
                    let stage = turn.stage_mut(live);
                    stage.status = StageStatus::Live;
                    stage.what = doing(tool);
                } else if let Some(stage) = turn.stages.iter_mut().find(|s| s.kind == live) {
                    stage.status = StageStatus::Live;
                }
            }
            None => {
                let stage = turn.stage_mut(StageKind::Understand);
                stage.status = StageStatus::Live;
                stage.what = "Thinking".into();
            }
        }
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

/// What a running tool is doing: "Editing app.rs".
fn doing(tool: &Tool) -> String {
    let path = tool.args["path"].as_str().map(name);
    match (tool.name.as_str(), path) {
        ("read", Some(path)) => format!("Reading {path}"),
        ("edit" | "write", Some(path)) => format!("Editing {path}"),
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

fn activity(pi: &Pi) -> String {
    if let Some(tool) = pi.tools.iter().rev().find(|tool| !tool.finished) {
        return doing(tool);
    }
    match pi.run {
        pi_core::session::RunState::Retrying => return "Retrying".into(),
        pi_core::session::RunState::Compacting => return "Compacting".into(),
        _ => {}
    }
    match pi
        .streaming_message_index()
        .and_then(|index| pi.messages.get(index))
    {
        Some(message) if !text_of(message).trim().is_empty() => "Writing".into(),
        Some(_) => "Thinking".into(),
        None => "Working".into(),
    }
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
            (LineKind::Context, line.trim_start())
        };
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

/// Commands that check work, as opposed to looking around.
fn is_check(command: &str) -> bool {
    const WORDS: [&str; 14] = [
        "test", "check", "clippy", "lint", "build", "tsc", "pytest", "jest", "vitest", "vet",
        "fmt", "make", "eslint", "mypy",
    ];
    command
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .any(|word| WORDS.contains(&word))
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
    Details {
        context_percent: usage
            .and_then(|usage| usage.percent)
            .map_or(0, |percent| percent.round().clamp(0., 100.) as u32),
        context_tokens: usage
            .map(|usage| {
                format!(
                    "{} of {} tokens",
                    thousands(usage.tokens.unwrap_or(0)),
                    thousands(usage.context_window)
                )
            })
            .unwrap_or_default(),
        cost: pi
            .stats
            .cost
            .map(|cost| format!("${cost:.2}"))
            .unwrap_or_default(),
        turns: pi.messages.iter().filter(|m| m["role"] == "user").count() as u32,
        tools: pi
            .state
            .active_tools
            .iter()
            .flatten()
            .map(|tool| tool.name.clone())
            .collect(),
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
        "select" => request["options"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .zip([Answer::AllowOnce, Answer::AllowSession, Answer::Deny])
            .map(|(label, answer)| Choice {
                answer,
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
            let index = match answer {
                Answer::AllowOnce => 0,
                Answer::AllowSession => 1,
                Answer::Deny => 2,
            };
            match request["options"].get(index).and_then(Value::as_str) {
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

    fn facts(outbox: &[String]) -> Facts<'_> {
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

    const DIFF: &str = "  210 const a = 1;\n-211 old();\n+211 new();\n+212 more();\n  213 end\n     ...\n  300 x\n+301 y";

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
    fn a_running_write_previews_its_content() {
        let tool = Tool {
            id: "w".into(),
            name: "write".into(),
            args: json!({"path":"src/new.rs","content":"first\nsecond"}),
            output: String::new(),
            diff: None,
            finished: false,
            is_error: false,
            images: Vec::new(),
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
            json!({"type":"tool_execution_update","toolCallId":"b","partialResult":{"content":[{"type":"text","text":"running 100 tests"}]}}),
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
        assert_eq!(tools[0].output, "running 100 tests");
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
    fn a_prompt_from_the_phone_shows_before_pi_takes_it() {
        let outbox = ["Explain this project".to_owned()];
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
        assert_eq!((images[0].key.as_str(), images[0].name.as_str()), (id.as_str(), "page.png"));
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
    fn only_checking_commands_count_as_checks() {
        assert!(is_check("cargo test -p app"));
        assert!(is_check("npm run lint"));
        assert!(!is_check("ls -la src"));
        assert!(!is_check("grep -rn testing ."));
    }
}
