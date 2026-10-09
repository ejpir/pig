//! Sample sessions that move on their own, so every screen can be tried
//! before the phone connects to a computer: runs advance, ask a question,
//! finish, and take follow-ups. Nothing here runs anything.

use crate::{model::*, store::Event};
use std::time::Duration;

/// A sample run: beats applied in order while the session works.
#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    beats: Vec<Beat>,
    next: usize,
    /// Working time left before the next beat.
    wait: Duration,
    /// Runs for the follow-ups to come, in order; then the sample run.
    follow_ups: Vec<Vec<Beat>>,
}

impl Script {
    fn new(beats: Vec<Beat>) -> Self {
        Self {
            beats,
            next: 0,
            wait: Duration::ZERO,
            follow_ups: Vec::new(),
        }
    }

    /// Nothing to run now; these answer the next follow-ups.
    fn follow_ups(follow_ups: Vec<Vec<Beat>>) -> Self {
        Self {
            follow_ups,
            ..Self::new(Vec::new())
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Beat {
    Wait(u64),
    /// A shorter wait, in milliseconds.
    Pause(u64),
    Activity(&'static str),
    Stage(StageKind, StageStatus, &'static str),
    Changed(&'static str, u32, u32),
    Diff(Vec<DiffLine>),
    Files(Vec<FileChange>),
    /// Asks the question; declining replaces the rest of the run.
    Ask(Question, Vec<Beat>),
    Check(&'static str, CheckResult),
    Summary(&'static str, &'static str),
    /// The reply so far, in Markdown, as it streams in.
    Reply(String),
    Finish,
}

pub const PROJECTS: [(&str, &str); 3] = [
    ("pi", "~/repos/pi"),
    ("zed", "~/repos/zed"),
    ("minivm", "~/repos/minivm"),
];

/// The computers the sample knows: the one entered, and one not connected.
pub(crate) fn computers(computer: Computer) -> Vec<Computer> {
    let build_box = Computer {
        name: "build-box".into(),
        address: "dev@10.0.4.12".into(),
        pi_version: None,
        connected: false,
    };
    vec![computer, build_box]
}

pub(crate) fn sessions() -> Vec<Session> {
    vec![
        qwen(SessionId(1)),
        streaming_retry(SessionId(2)),
        gutter_blame(SessionId(3)),
        mistral(SessionId(4)),
        kimi(SessionId(5)),
        snapshot_restore(SessionId(6)),
        aurora(SessionId(7)),
    ]
}

/// Declining the question replaces the rest of the run with what it declined to.
pub(crate) fn decline(session: &mut Session) {
    if let Some(script) = &mut session.script
        && let Some(Beat::Ask(_, declined)) = script
            .next
            .checked_sub(1)
            .and_then(|index| script.beats.get(index))
            .cloned()
    {
        script.beats.truncate(script.next);
        script.beats.extend(declined);
    }
}

/// A new sample session in a project.
pub(crate) fn new_session(
    id: SessionId,
    project: usize,
    prompt: String,
    attachments: Vec<String>,
) -> Session {
    let (name, folder) = PROJECTS[project.min(PROJECTS.len() - 1)];
    let mut session = Session {
        id,
        title: title_for(&prompt),
        project: name.into(),
        folder: folder.into(),
        state: State::Working,
        activity: String::new(),
        elapsed: Duration::ZERO,
        finished_at: None,
        turns: Vec::new(),
        files: Vec::new(),
        check: None,
        question: None,
        queued: Vec::new(),
        failure: None,
        details: Details {
            context_percent: Some(2),
            context_tokens: "4k of 200k tokens".into(),
            cost: "$0.00".into(),
            turns: 0,
            tools: tools(),
            snapshots: 0,
            session_file: format!("~/.pi/agent/sessions/{}.jsonl", slug(&prompt)),
        },
        script: None,
    };
    begin_turn(&mut session, prompt, attachments);
    session
}

/// A session's name from its first words, as Pi names new sessions.
pub(crate) fn title_for(prompt: &str) -> String {
    let mut title = String::new();
    for word in prompt.split_whitespace() {
        if !title.is_empty() && title.len() + word.len() > 28 {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(word.trim_end_matches(['.', ',', ':', ';']));
    }
    if title.is_empty() {
        "New session".into()
    } else {
        title
    }
}

fn slug(prompt: &str) -> String {
    let slug: String = title_for(prompt)
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    slug.trim_matches('-').to_owned()
}

pub(crate) fn begin_turn(session: &mut Session, prompt: String, attachments: Vec<String>) {
    let mut turn = Turn::new(prompt, clock_now());
    turn.attachments = attachments;
    session.turns.push(turn);
    session.state = State::Working;
    session.finished_at = None;
    session.details.turns += 1;
    let mut follow_ups = session
        .script
        .take()
        .map(|script| script.follow_ups)
        .unwrap_or_default();
    let beats = if follow_ups.is_empty() {
        sample_run()
    } else {
        follow_ups.remove(0)
    };
    let mut script = Script::new(beats);
    script.follow_ups = follow_ups;
    session.script = Some(script);
    advance(session, Duration::ZERO);
}

/// Applies the beats that are due; returns what the user may want to know.
pub(crate) fn advance(session: &mut Session, elapsed: Duration) -> Option<Event> {
    let mut script = session.script.take()?;
    let mut left = elapsed;
    let mut event = None;
    loop {
        // Time left over from one wait goes on to the next.
        if script.wait > left {
            script.wait -= left;
            spend(session, left);
            break;
        }
        left -= script.wait;
        spend(session, script.wait);
        script.wait = Duration::ZERO;
        let Some(beat) = script.beats.get(script.next).cloned() else {
            break;
        };
        script.next += 1;
        match beat {
            Beat::Wait(seconds) => script.wait = Duration::from_secs(seconds),
            Beat::Pause(millis) => script.wait = Duration::from_millis(millis),
            Beat::Reply(text) => {
                session.turn_mut().summary = Some(Summary {
                    source: Some(text),
                    headline: String::new(),
                    body: String::new(),
                })
            }
            Beat::Activity(activity) => session.activity = activity.into(),
            Beat::Stage(kind, status, what) => {
                let stage = session.turn_mut().stage_mut(kind);
                stage.status = status;
                stage.what = what.into();
                if status != StageStatus::Live {
                    stage.diff.clear();
                }
            }
            Beat::Changed(path, added, removed) => {
                let stage = session.turn_mut().stage_mut(StageKind::Change);
                let reference = Reference::File(path.into());
                if !stage.references.contains(&reference) {
                    stage.references.push(reference);
                }
                stage.added += added;
                stage.removed += removed;
            }
            Beat::Diff(lines) => session.turn_mut().stage_mut(StageKind::Change).diff = lines,
            Beat::Files(files) => session.files = files,
            Beat::Ask(question, _) => {
                session.question = Some(question);
                session.state = State::NeedsYou;
                event = Some(Event::NeedsYou(session.id));
                // Waiting for an answer is not working time.
                break;
            }
            Beat::Check(command, result) => {
                session.check = Some(Check {
                    command: command.into(),
                    result,
                })
            }
            Beat::Summary(headline, body) => {
                session.turn_mut().summary = Some(Summary {
                    source: None,
                    headline: headline.into(),
                    body: body.into(),
                })
            }
            Beat::Finish => {
                session.state = State::Done;
                session.activity = "Done".into();
                session.finished_at = Some(clock_now());
                event = Some(Event::Finished(session.id));
                break;
            }
        }
    }
    if session.state.is_running() || !script.follow_ups.is_empty() {
        session.script = Some(script);
    }
    event
}

/// A reply that streams in a few words at a time, as a model writes it.
fn stream(markdown: &str) -> Vec<Beat> {
    let mut beats = Vec::new();
    let mut words = 0;
    for (index, char) in markdown.char_indices() {
        if char.is_whitespace() && !markdown[..index].ends_with(char::is_whitespace) {
            words += 1;
            if words % 3 == 0 {
                beats.push(Beat::Reply(markdown[..index].to_owned()));
                beats.push(Beat::Pause(110));
            }
        }
    }
    beats.push(Beat::Reply(markdown.to_owned()));
    beats
}

/// A follow-up Pi answers by reading, without changing anything.
fn answer(reading: &'static str, read: &'static str, reply: &str) -> Vec<Beat> {
    use Beat::*;
    use StageKind::*;
    use StageStatus::*;
    let mut beats = vec![
        Activity(reading),
        Stage(Understand, Live, reading),
        Wait(2),
        Stage(Understand, Done, read),
        Stage(Change, Skipped, "Nothing changed"),
        Stage(Verify, Skipped, "Nothing to check"),
        Activity("Writing the reply"),
        Stage(HandOff, Live, "Writing the reply"),
    ];
    beats.extend(stream(reply));
    beats.extend([Stage(HandOff, Done, "Answered"), Finish]);
    beats
}

/// Working time goes to the stage Pi is in.
fn spend(session: &mut Session, time: Duration) {
    let turn = session.turn_mut();
    if let Some(kind) = turn.live_stage() {
        turn.add_time(kind, time);
    }
}

/// Any new task or follow-up: the preview cannot run it, and says so.
fn sample_run() -> Vec<Beat> {
    use Beat::*;
    use StageKind::*;
    use StageStatus::*;
    vec![
        Activity("Reading the project"),
        Stage(Understand, Live, "Reading the project"),
        Wait(3),
        Stage(Understand, Done, "Read the project's README"),
        Stage(Change, Skipped, "Nothing changed"),
        Stage(Verify, Skipped, "Nothing to check"),
        Activity("Writing the summary"),
        Stage(HandOff, Live, "Writing the summary"),
        Wait(1),
    ]
    .into_iter()
    .chain(stream(
        "**This is a sample session, so nothing ran.** The preview does not connect to \
         the computer yet. Once it does, Pi works on this in the project's folder there, \
         and the phone follows along.",
    ))
    .chain([Stage(HandOff, Done, "Summary"), Finish])
    .collect()
}

fn tools() -> Vec<String> {
    ["read", "bash", "edit", "write", "grep", "find", "ls"]
        .map(String::from)
        .to_vec()
}

fn session(id: u32, title: &str, project: usize, state: State, turn: Turn) -> Session {
    let (name, folder) = PROJECTS[project];
    Session {
        id: SessionId(id),
        title: title.into(),
        project: name.into(),
        folder: folder.into(),
        state,
        activity: String::new(),
        elapsed: Duration::ZERO,
        finished_at: None,
        turns: vec![turn],
        files: Vec::new(),
        check: None,
        question: None,
        queued: Vec::new(),
        failure: None,
        details: Details {
            context_percent: Some(18),
            context_tokens: "36k of 200k tokens".into(),
            cost: "$0.22".into(),
            turns: 6,
            tools: tools(),
            snapshots: 1,
            session_file: format!(
                "~/.pi/agent/sessions/--repos-{name}--/{}.jsonl",
                title.to_lowercase().replace(' ', "-")
            ),
        },
        script: None,
    }
}

fn times(seconds: [u64; 4]) -> [Duration; 4] {
    seconds.map(Duration::from_secs)
}

fn line(kind: LineKind, number: u32, text: &str) -> DiffLine {
    DiffLine::new(kind, number, text)
}

fn stage(kind: StageKind, status: StageStatus, what: &str) -> Stage {
    Stage::new(kind, status, what)
}

fn qwen(id: SessionId) -> Session {
    use LineKind::*;
    use StageKind::*;
    use StageStatus::*;
    let mut turn = Turn::new(
        "qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the \
         response. Accept empty signatures there, but keep the check strict for Anthropic.",
        "09:41",
    );
    let mut understood = stage(Understand, Done, "Read 5 files · searched twice");
    understood.references = vec![
        Reference::File("openai-completions.ts".into()),
        Reference::File("anthropic.ts".into()),
        Reference::Search("“signature”".into()),
        Reference::File("thinking.ts".into()),
        Reference::Search("“isAnthropic”".into()),
        Reference::File("models.ts".into()),
        Reference::File("qwen.ts".into()),
    ];
    let mut changing = stage(Change, Live, "Editing openai-completions.ts");
    changing.references = vec![Reference::File("openai-completions.ts".into())];
    changing.diff_path = Some("packages/ai/src/providers/openai-completions.ts".into());
    changing.added = 3;
    changing.removed = 1;
    changing.diff = vec![
        line(Context, 210, "const signature = block.signature;"),
        line(Removed, 211, "if (!signature) {"),
        line(Added, 211, "if (!signature && isAnthropic(model)) {"),
    ];
    turn.stages = vec![
        understood,
        changing,
        Stage::planned(Verify),
        Stage::planned(HandOff),
    ];
    turn.times = times([21, 51, 0, 0]);
    let mut session = session(id.0, "Qwen signatures", 0, State::Working, turn);
    session.activity = "Editing a file".into();
    session.elapsed = Duration::from_secs(72);
    session.files = vec![qwen_completions()];
    session.details = Details {
        context_percent: Some(31),
        context_tokens: "62k of 200k tokens".into(),
        cost: "$0.41".into(),
        turns: 14,
        tools: tools(),
        snapshots: 3,
        session_file: "~/.pi/agent/sessions/--repos-pi--/qwen-signatures.jsonl".into(),
    };
    let question = Question {
        title: "Run the provider tests?".into(),
        body: "Pi wants to run a command on the computer, in ~/repos/pi.".into(),
        command: "pnpm test --filter @pi/ai -- qwen".into(),
        choices: vec![
            Choice {
                answer: Answer::AllowOnce,
                label: "Allow once".into(),
                detail: None,
            },
            Choice {
                answer: Answer::AllowSession,
                label: "Allow for this session".into(),
                detail: Some("Pi won't ask again for this command".into()),
            },
            Choice {
                answer: Answer::Deny,
                label: "Don't run it".into(),
                detail: Some("Tell Pi why in the next message".into()),
            },
        ],
    };
    let summary = "OpenCode models may now send an empty signature; Anthropic still must.";
    {
        use Beat::*;
        session.script = Some(Script::new(vec![
            Wait(4),
            Diff(vec![
                line(Context, 210, "const signature = block.signature;"),
                line(Removed, 211, "if (!signature) {"),
                line(Added, 211, "if (!signature && isAnthropic(model)) {"),
                line(Added, 212, "  signatures.push(signature);"),
            ]),
            Changed("openai-completions.ts", 1, 0),
            Wait(5),
            Changed("qwen.test.ts", 18, 0),
            Files(vec![qwen_completions(), qwen_test()]),
            Stage(Change, Done, "Edited 2 files"),
            Stage(Verify, Live, "Waiting for you"),
            Activity("Waiting for you"),
            Ask(
                question,
                vec![
                    Stage(Verify, Skipped, "You chose not to run the tests"),
                    Check("pnpm test --filter @pi/ai", CheckResult::NotRun),
                    Activity("Writing the summary"),
                    Stage(HandOff, Live, "Writing the summary"),
                    Wait(3),
                    Summary(
                        summary,
                        "The check in readThinking only rejects a missing signature for \
                         Anthropic models. A regression test covers both providers; the \
                         tests did not run.",
                    ),
                    Stage(HandOff, Done, "Summary and 2 changed files"),
                    Finish,
                ],
            ),
            Activity("Running the provider tests"),
            Stage(Verify, Live, "pnpm test --filter @pi/ai -- qwen"),
            Wait(6),
            Stage(Verify, Done, "Provider tests passed"),
            Check("pnpm test --filter @pi/ai", CheckResult::Passed),
            Activity("Writing the summary"),
            Stage(HandOff, Live, "Writing the summary"),
            Wait(3),
            Summary(
                summary,
                "The check in readThinking only rejects a missing signature for Anthropic \
                 models. A regression test covers both providers, and the provider tests pass.",
            ),
            Stage(HandOff, Done, "Summary and 2 changed files"),
            Finish,
        ]));
    }
    session
}

fn qwen_completions() -> FileChange {
    use LineKind::*;
    let lines = [
        (
            Context,
            204,
            "export function readThinking(blocks, model) {",
        ),
        (Context, 205, "  const signatures: string[] = [];"),
        (Context, 206, "  for (const block of blocks) {"),
        (Context, 207, "    if (block.type !== \"thinking\")"),
        (Context, 208, "      continue;"),
        (Context, 209, "    const signature ="),
        (Context, 210, "      block.signature;"),
        (Removed, 211, "    if (!signature) {"),
        (Added, 211, "    if (!signature &&"),
        (Added, 212, "        isAnthropic(model)) {"),
        (Context, 213, "      throw new Error("),
        (Context, 214, "        \"Missing signature\");"),
        (Context, 215, "    }"),
        (Added, 216, "    signatures.push(signature ?? \"\");"),
        (Context, 217, "  }"),
        (Context, 218, "  return signatures;"),
        (Context, 219, "}"),
    ];
    FileChange {
        path: "packages/ai/src/providers/openai-completions.ts".into(),
        added: 3,
        removed: 1,
        hunks: vec![Hunk {
            header: "@@ 204,13 · readThinking".into(),
            lines: lines
                .into_iter()
                .map(|(kind, number, text)| line(kind, number, text))
                .collect(),
        }],
    }
}

fn qwen_test() -> FileChange {
    let lines = [
        "import { describe, expect, it } from \"vitest\";",
        "import { readThinking } from \"../src/providers/openai-completions\";",
        "",
        "const block = { type: \"thinking\", signature: \"\" };",
        "",
        "describe(\"empty thinking signatures\", () => {",
        "  it(\"accepts them from OpenCode models\", () => {",
        "    const model = { provider: \"opencode\" };",
        "    expect(readThinking([block], model))",
        "      .toEqual([\"\"]);",
        "  });",
        "",
        "  it(\"rejects them from Anthropic models\", () => {",
        "    const model = { provider: \"anthropic\" };",
        "    expect(() => readThinking([block], model))",
        "      .toThrow(\"Missing signature\");",
        "  });",
        "});",
    ];
    FileChange {
        path: "packages/ai/test/qwen.test.ts".into(),
        added: 18,
        removed: 0,
        hunks: vec![Hunk {
            header: "@@ 1,18 · new file".into(),
            lines: lines
                .iter()
                .zip(1..)
                .map(|(text, number)| line(LineKind::Added, number, text))
                .collect(),
        }],
    }
}

fn streaming_retry(id: SessionId) -> Session {
    use LineKind::*;
    use StageKind::*;
    use StageStatus::*;
    let mut turn = Turn::new(
        "Retry a streaming request once when the connection drops before the first token.",
        "09:12",
    );
    let mut understood = stage(Understand, Done, "Read 4 files");
    understood.references = ["stream.ts", "retry.ts", "client.ts", "errors.ts"]
        .map(|file| Reference::File(file.into()))
        .to_vec();
    let mut changing = stage(Change, Live, "Editing stream.ts");
    changing.references = vec![
        Reference::File("retry.ts".into()),
        Reference::File("stream.ts".into()),
    ];
    changing.added = 21;
    changing.removed = 6;
    changing.diff = vec![
        line(Context, 88, "const response = await open(request);"),
        line(Removed, 89, "return read(response);"),
        line(
            Added,
            89,
            "return read(response, { retryBeforeFirstToken: 1 });",
        ),
    ];
    turn.stages = vec![
        understood,
        changing,
        Stage::planned(Verify),
        Stage::planned(HandOff),
    ];
    turn.times = times([24, 136, 0, 0]);
    let mut session = session(id.0, "Streaming retry", 0, State::Working, turn);
    session.activity = "Changing · 2 files so far".into();
    session.elapsed = Duration::from_secs(160);
    session.files = vec![
        small_change("packages/ai/src/stream.ts", 14, 4),
        small_change("packages/ai/src/retry.ts", 7, 2),
    ];
    {
        use Beat::*;
        session.script = Some(Script::new(vec![
            Wait(35),
            Activity("Changing · editing retry.ts"),
            Diff(vec![
                line(
                    Context,
                    12,
                    "export async function withRetry(run, attempts) {",
                ),
                line(Removed, 13, "  return run();"),
                line(Added, 13, "  for (let attempt = 0; ; attempt++) {"),
            ]),
            Wait(35),
            Stage(Change, Done, "Edited 2 files"),
            Activity("Verifying · pnpm test"),
            Stage(Verify, Live, "pnpm test --filter @pi/ai -- stream"),
            Wait(20),
            Stage(Verify, Done, "Stream tests passed"),
            Check("pnpm test --filter @pi/ai -- stream", CheckResult::Passed),
            Stage(HandOff, Live, "Writing the summary"),
            Wait(3),
            Summary(
                "A dropped stream now retries once if no token has arrived yet.",
                "After the first token, a drop still fails the request, so no output is \
                 ever repeated. The stream tests pass.",
            ),
            Stage(HandOff, Done, "Summary and 2 changed files"),
            Finish,
        ]));
    }
    session
}

fn gutter_blame(id: SessionId) -> Session {
    use StageKind::*;
    use StageStatus::*;
    let mut turn = Turn::new(
        "The blame gutter is too wide on small windows. Cap it at 30 columns.",
        "09:30",
    );
    let mut understood = stage(Understand, Done, "Read 3 files · 2 searches");
    understood.references = vec![
        Reference::File("blame.rs".into()),
        Reference::Search("“gutter_width” in crates/editor".into()),
    ];
    let mut changed = stage(Change, Done, "Edited blame.rs");
    changed.references = vec![Reference::File("blame.rs".into())];
    changed.added = 12;
    changed.removed = 4;
    turn.stages = vec![
        understood,
        changed,
        stage(Verify, Live, "cargo test -p editor gutter"),
        Stage::planned(HandOff),
    ];
    turn.times = times([12, 24, 16, 0]);
    let mut session = session(id.0, "Gutter blame width", 1, State::Working, turn);
    session.activity = "Verifying · cargo test".into();
    session.elapsed = Duration::from_secs(52);
    session.files = vec![small_change("crates/editor/src/blame.rs", 12, 4)];
    {
        use Beat::*;
        session.script = Some(Script::new(vec![
            Wait(40),
            Stage(Verify, Done, "Editor tests passed"),
            Check("cargo test -p editor gutter", CheckResult::Passed),
            Stage(HandOff, Live, "Writing the summary"),
            Wait(3),
            Summary(
                "The blame gutter stops at 30 columns and truncates long names.",
                "Narrow windows keep their text column; the full author is in the \
                 blame tooltip. The editor's gutter tests pass.",
            ),
            Stage(HandOff, Done, "Summary and 1 changed file"),
            Finish,
        ]));
    }
    session
}

fn small_change(path: &str, added: u32, removed: u32) -> FileChange {
    use LineKind::*;
    FileChange {
        path: path.into(),
        added,
        removed,
        hunks: vec![Hunk {
            header: "@@ 1,6".into(),
            lines: vec![
                line(Context, 1, "// Sample change: the preview shows"),
                line(Removed, 2, "// what Pi edited on the computer."),
                line(Added, 2, "// what Pi edited on the computer,"),
                line(Added, 3, "// one hunk at a time."),
            ],
        }],
    }
}

fn finished(mut session: Session, at: &str, summary: Summary) -> Session {
    session.finished_at = Some(at.into());
    let turn = session.turn_mut();
    for stage in &mut turn.stages {
        if stage.status == StageStatus::Planned {
            stage.status = StageStatus::Done;
        }
    }
    turn.summary = Some(summary);
    session
}

fn mistral(id: SessionId) -> Session {
    use StageKind::*;
    use StageStatus::*;
    let mut turn = Turn::new(
        "Mistral's reasoning models send thinking as plain text. Show it as thinking.",
        "13:31",
    );
    let mut understood = stage(Understand, Done, "Read 5 files");
    understood.references = vec![Reference::File("mistral.ts".into())];
    let mut changed = stage(Change, Done, "Edited 3 files");
    changed.references = ["mistral.ts", "thinking.ts", "mistral.test.ts"]
        .map(|file| Reference::File(file.into()))
        .to_vec();
    turn.stages = vec![
        understood,
        changed,
        stage(Verify, Done, "Provider tests passed"),
        stage(HandOff, Done, "Summary and 3 changed files"),
    ];
    turn.times = times([60, 200, 100, 28]);
    let mut session = session(id.0, "Mistral thinking", 0, State::Done, turn);
    session.elapsed = Duration::from_secs(388);
    session.files = vec![
        small_change("packages/ai/src/providers/mistral.ts", 22, 3),
        small_change("packages/ai/src/thinking.ts", 9, 0),
        small_change("packages/ai/test/mistral.test.ts", 31, 0),
    ];
    session.check = Some(Check {
        command: "pnpm test --filter @pi/ai".into(),
        result: CheckResult::Passed,
    });
    finished(
        session,
        "14:02",
        Summary {
            source: None,
            headline: "Mistral's thinking now shows as thinking, not as the answer.".into(),
            body: "Text inside its reasoning markers becomes a thinking block. Tests cover \
                   streamed and complete responses."
                .into(),
        },
    )
}

fn kimi(id: SessionId) -> Session {
    use StageKind::*;
    use StageStatus::*;
    let mut turn = Turn::new("Which Kimi model does Pi use by default, and why?", "11:36");
    let mut understood = stage(Understand, Done, "Read 2 files");
    understood.references = vec![Reference::File("models.ts".into())];
    turn.stages = vec![
        understood,
        stage(Change, Skipped, "Nothing changed"),
        stage(Verify, Skipped, "Nothing to check"),
        stage(HandOff, Done, "Answered"),
    ];
    turn.times = times([30, 0, 0, 11]);
    let mut session = session(id.0, "Kimi K3 default", 0, State::Done, turn);
    session.elapsed = Duration::from_secs(41);
    // A conversation to carry on: whatever is asked next, these answer it.
    session.script = Some(Script::follow_ups(vec![
        answer(
            "Reading the settings",
            "Read settings.ts",
            "Set it for the provider in your settings, so nothing else changes:\n\n\
             ```json\n{\n  \"providers\": {\n    \"moonshot\": { \"defaultModel\": \
             \"kimi-k2\" }\n  }\n}\n```\n\nPi reads this when a session starts, so **new \
             sessions** use K2 and running ones keep K3. For a single session, pick it from \
             the model menu instead.",
        ),
        answer(
            "Comparing the models",
            "Read models.ts",
            "## K2 next to K3\n\n- **Context:** 128k tokens instead of 256k.\n- **Speed:** \
             a little faster on short answers.\n- **Tools:** both call tools, but K2 slips \
             more often on long argument lists.\n\nFor big refactors I'd keep K3. For quick \
             questions, K2 is fine.",
        ),
    ]));
    finished(
        session,
        "11:40",
        Summary {
            source: Some(
                "## Kimi K3\n\nIt's the newest Kimi model that can **call tools**, and \
                 `models.ts` picks it like this:\n\n```ts\nconst kimi = models\n  \
                 .filter((m) => m.family === \"kimi\" && m.tools)\n  .sort(byRelease)\n  \
                 .at(-1);\n```\n\n```mermaid\ngraph LR\n  A[Provider models] --> B[Kimi \
                 with tools]\n  B --> C[Newest: K3]\n```\n\nThis only applies when a provider \
                 offers several Kimi models."
                    .into(),
            ),
            headline: "Kimi K3 is the default because it is the newest with tool calls.".into(),
            body: "The default lives in models.ts and only applies when a provider offers \
                   several Kimi models."
                .into(),
        },
    )
}

/// A page Pi made, which the thread shows as a card.
fn aurora(id: SessionId) -> Session {
    use StageKind::*;
    use StageStatus::*;
    let mut turn = Turn::new("Make me something cool to look at", "10:12");
    let mut changed = stage(Change, Done, "Wrote aurora.html");
    changed.references = vec![Reference::File("demo/aurora.html".into())];
    turn.stages = vec![
        stage(Understand, Skipped, "Nothing to read"),
        changed,
        stage(Verify, Skipped, "Nothing to check"),
        stage(HandOff, Done, "Summary and a page"),
    ];
    turn.pages = vec![pi_markdown::Page {
        path: "demo/aurora.html".into(),
        html: Some(include_str!("../assets/samples/aurora.html").into()),
    }];
    use base64::Engine as _;
    turn.images = vec![ToolImage {
        key: "sample-aurora".into(),
        name: "aurora.png".into(),
        mime: "image/png".into(),
        inline: Some(
            base64::engine::general_purpose::STANDARD
                .encode(include_bytes!("../assets/samples/aurora.png")),
        ),
    }];
    // Words, the screenshot Pi checked it with, then the page, in order.
    turn.flow = vec![
        Flow::Text("I'll draw it on a canvas, then take a screenshot to check it.".into()),
        Flow::Image(0),
        Flow::Text(
            "An aurora over a starfield. Touch it to stir the sky. It's one file with \
             nothing to download, so it works offline."
                .into(),
        ),
        Flow::Page(0),
    ];
    turn.times = times([0, 40, 0, 14]);
    let mut session = session(id.0, "Aurora", 0, State::Done, turn);
    session.elapsed = Duration::from_secs(54);
    finished(
        session,
        "10:13",
        Summary {
            source: None,
            headline: String::new(),
            body: "An aurora over a starfield, drawn on a canvas. Touch it to stir the sky. \
                   It's one file with nothing to download, so it works offline."
                .into(),
        },
    )
}

fn snapshot_restore(id: SessionId) -> Session {
    use StageKind::*;
    use StageStatus::*;
    let mut turn = Turn::new("Restore VM snapshots in the background.", "09:02");
    let mut understood = stage(Understand, Done, "Read 6 files");
    understood.references = vec![Reference::File("snapshot.rs".into())];
    turn.stages = vec![
        understood,
        stage(Change, Skipped, "Stopped before changing anything"),
        Stage::planned(Verify),
        Stage::planned(HandOff),
    ];
    turn.times = times([95, 0, 0, 0]);
    let mut session = session(id.0, "VM snapshot restore", 2, State::Failed, turn);
    session.elapsed = Duration::from_secs(95);
    session.finished_at = Some("09:15".into());
    session.failure = Some("stopped: provider error".into());
    session
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::sample(Computer::from_address("nick@studio-mac.local"))
    }

    fn qwen(store: &Store) -> &Session {
        store.session(SessionId(1)).unwrap()
    }

    #[test]
    fn a_run_asks_then_finishes_after_an_answer() {
        let mut store = store();
        assert_eq!(qwen(&store).state, State::Working);
        let events = store.tick(Duration::from_secs(10));
        assert!(events.contains(&Event::NeedsYou(SessionId(1))));
        assert_eq!(qwen(&store).state, State::NeedsYou);
        assert_eq!(qwen(&store).files.len(), 2);

        // Waiting does not count as working time.
        let elapsed = qwen(&store).elapsed;
        store.tick(Duration::from_secs(30));
        assert_eq!(qwen(&store).elapsed, elapsed);

        store.answer(SessionId(1), Answer::AllowOnce);
        let events = store.tick(Duration::from_secs(10));
        assert!(events.contains(&Event::Finished(SessionId(1))));
        let session = qwen(&store);
        assert_eq!(session.state, State::Done);
        assert_eq!(session.check.as_ref().unwrap().result, CheckResult::Passed);
        assert_eq!(session.status_line(), "2 files changed · checks passed");
        // Each stage's working time, for the run line.
        assert_eq!(
            session.turn().unwrap().times,
            [21, 60, 6, 3].map(Duration::from_secs)
        );
    }

    #[test]
    fn declining_skips_the_check_honestly() {
        let mut store = store();
        store.tick(Duration::from_secs(10));
        store.answer(SessionId(1), Answer::Deny);
        store.tick(Duration::from_secs(10));
        let session = qwen(&store);
        assert_eq!(session.state, State::Done);
        assert_eq!(session.check.as_ref().unwrap().result, CheckResult::NotRun);
        assert_eq!(session.status_line(), "2 files changed · checks not run");
        let verify = &session.turn().unwrap().stages[2];
        assert_eq!(verify.status, StageStatus::Skipped);
    }

    #[test]
    fn follow_ups_wait_for_the_run_then_start() {
        let mut store = store();
        store
            .send(SessionId(3), "Also cap the minimap".into(), Vec::new())
            .unwrap();
        assert_eq!(store.session(SessionId(3)).unwrap().queued.len(), 1);
        store.tick(Duration::from_secs(45));
        let session = store.session(SessionId(3)).unwrap();
        assert!(session.queued.is_empty());
        assert_eq!(session.turns.len(), 2);
        assert_eq!(session.state, State::Working);
    }

    #[test]
    fn stopping_ends_the_run() {
        let mut store = store();
        store.stop(SessionId(2)).unwrap();
        let session = store.session(SessionId(2)).unwrap();
        assert_eq!(session.state, State::Stopped);
        assert_eq!(session.status_line(), "stopped by you");
        store.tick(Duration::from_secs(100));
        assert_eq!(store.session(SessionId(2)).unwrap().state, State::Stopped);
    }

    #[test]
    fn a_new_session_says_it_is_a_sample() {
        let mut store = store();
        let id = store
            .start(
                1,
                "Make the tab bar scroll on small windows".into(),
                Vec::new(),
            )
            .unwrap();
        let session = store.session(id).unwrap();
        assert_eq!(session.title, "Make the tab bar scroll on");
        assert_eq!(session.project, "zed");
        store.tick(Duration::from_secs(6));
        let session = store.session(id).unwrap();
        assert_eq!(session.state, State::Done);
        assert!(
            session
                .turn()
                .unwrap()
                .summary
                .as_ref()
                .unwrap()
                .text()
                .contains("sample")
        );
    }

    #[test]
    fn a_conversation_carries_on_with_replies_that_stream_in() {
        let mut store = store();
        let kimi = SessionId(5);
        let reply = |store: &Store| {
            store
                .session(kimi)
                .unwrap()
                .turn()
                .unwrap()
                .summary
                .as_ref()
                .map(|s| s.text())
        };
        for (prompt, end) in [
            ("How do I switch it to K2?", "model menu instead."),
            ("What changes?", "K2 is fine."),
        ] {
            let prompt = crate::prompt::Prompt::new(prompt.into(), Vec::new());
            store.send(kimi, prompt, Vec::new()).unwrap();
            store.tick(Duration::from_millis(2_500));
            let partial = reply(&store).unwrap();
            assert!(!partial.is_empty() && !partial.ends_with(end), "{partial}");
            store.tick(Duration::from_secs(10));
            assert_eq!(store.session(kimi).unwrap().state, State::Done);
            assert!(reply(&store).unwrap().ends_with(end));
        }
        assert_eq!(store.session(kimi).unwrap().turns.len(), 3);
    }

    #[test]
    fn the_list_puts_what_needs_you_first() {
        let mut store = store();
        store.tick(Duration::from_secs(10));
        let [needs, working, finished] = store.grouped();
        assert_eq!(
            needs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
            ["Qwen signatures"]
        );
        assert_eq!(working.len(), 2);
        assert_eq!(finished.len(), 4);
    }
}
