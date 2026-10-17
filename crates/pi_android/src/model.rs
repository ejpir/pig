//! What the phone shows of Pi's sessions on a computer. The shapes follow Pi
//! Desktop's thread: a session is turns of a prompt, the stages Pi went
//! through, and a hand-off; its observed edits and checks are reviewed apart.

use std::{borrow::Cow, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u32);

/// Identity of a queued payload, independent of its text or visible position.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum QueueKey {
    Durable(String),
    Local(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Computer {
    /// The short name used everywhere: "studio-mac".
    pub name: String,
    /// What SSH connects to: "nick@studio-mac.local".
    pub address: String,
    pub pi_version: Option<String>,
    pub connected: bool,
}

impl Computer {
    pub fn from_address(address: &str) -> Self {
        let host = address.rsplit('@').next().unwrap_or(address);
        let host = host.split(':').next().unwrap_or(host);
        let name = host.strip_suffix(".local").unwrap_or(host);
        Self {
            name: name.to_owned(),
            address: address.to_owned(),
            pi_version: Some("Pi 1.0.0".into()),
            connected: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    NeedsYou,
    Working,
    Done,
    Stopped,
    Failed,
}

impl State {
    pub fn is_running(self) -> bool {
        matches!(self, Self::NeedsYou | Self::Working)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageKind {
    Understand,
    Change,
    Verify,
    HandOff,
}

impl StageKind {
    pub const ALL: [Self; 4] = [Self::Understand, Self::Change, Self::Verify, Self::HandOff];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Understand => "eye",
            Self::Change => "pencil",
            Self::Verify => "term",
            Self::HandOff => "send",
        }
    }

    /// Named as the run line names its stretches: looking, changing, checking.
    pub fn name(self, status: StageStatus) -> &'static str {
        match (self, status) {
            (Self::Understand, StageStatus::Done) => "Looked",
            (Self::Understand, StageStatus::Live) => "Looking",
            (Self::Understand, _) => "Look",
            (Self::Change, StageStatus::Done) => "Changed",
            (Self::Change, StageStatus::Live) => "Changing",
            (Self::Change, _) => "Change",
            (Self::Verify, StageStatus::Done) => "Checked",
            (Self::Verify, StageStatus::Live) => "Checking",
            (Self::Verify, _) => "Check",
            (Self::HandOff, StageStatus::Done) => "Handed off",
            (Self::HandOff, StageStatus::Live) => "Handing off",
            (Self::HandOff, _) => "Hand off",
        }
    }

    /// What a stage ahead will do.
    pub fn plan(self) -> &'static str {
        match self {
            Self::Understand => "Read and search",
            Self::Change => "Edit files",
            Self::Verify => "Run the checks",
            Self::HandOff => "Summary and changes",
        }
    }
}

/// What a stretch of a run went on: the run line's colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StretchKind {
    /// Reading, searching and looking around, and Pi's own words between calls.
    Look,
    /// Edits and writes.
    Change,
    /// Tests, builds, type checks and linters, until their result.
    Check,
    /// Waiting for an answer from you.
    Wait,
}

impl StretchKind {
    /// What a stage's calls count as. Pi's closing words are it looking the
    /// work over, so the hand-off is looking too.
    pub fn of(stage: StageKind) -> Self {
        match stage {
            StageKind::Change => Self::Change,
            StageKind::Verify => Self::Check,
            StageKind::Understand | StageKind::HandOff => Self::Look,
        }
    }

    /// "Looking", for a key.
    pub fn title(self) -> &'static str {
        match self {
            Self::Look => "Looking",
            Self::Change => "Changing",
            Self::Check => "Checking",
            Self::Wait => "Waiting for you",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Look => "eye",
            Self::Change => "pencil",
            Self::Check => "term",
            Self::Wait => "hand",
        }
    }
}

/// How a stretch ended, drawn where it ended: a check's result, or a
/// question you answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Passed,
    Failed,
    Answered,
}

/// A stretch of a run, in the order things happened: what it went on, for how
/// long, how it ended, and the calls made in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Stretch {
    pub kind: StretchKind,
    pub time: Duration,
    pub mark: Option<Mark>,
    /// Its calls, as `Flow::Tool` names them.
    pub calls: Vec<(StageKind, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageStatus {
    Done,
    Live,
    Planned,
    /// Passed over: a stopped run, or a check the user declined.
    Skipped,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reference {
    File(String),
    Search(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub kind: StageKind,
    pub status: StageStatus,
    /// "Read 1 file · 1 search", "Editing openai-completions.ts".
    pub what: String,
    /// What it read or searched for; for changes, the files it edited.
    pub references: Vec<Reference>,
    /// The file whose latest edit `diff` previews, relative to the project.
    pub diff_path: Option<String>,
    /// The latest edit, while changing.
    pub diff: Vec<DiffLine>,
    pub added: u32,
    pub removed: u32,
    /// Each observed tool call, including its full command and output.
    pub tools: Vec<ToolActivity>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolActivity {
    pub id: String,
    pub name: String,
    pub target: String,
    pub output: String,
    pub finished: bool,
    pub failed: bool,
    /// Lines this call added and removed, for an edit or a write.
    pub added: u32,
    pub removed: u32,
    /// This call's full diff, not the stage's truncated latest-edit preview.
    pub diff: Vec<DiffLine>,
    /// Derived from requested input rather than a reported applied diff.
    pub diff_is_preview: bool,
}

impl ToolActivity {
    pub fn output_for_display(&self) -> Cow<'_, str> {
        if self.name == "bash" {
            pi_core::command_output::for_display(&self.output)
        } else {
            Cow::Borrowed(&self.output)
        }
    }
}

impl Stage {
    pub fn new(kind: StageKind, status: StageStatus, what: impl Into<String>) -> Self {
        Self {
            kind,
            status,
            what: what.into(),
            references: Vec::new(),
            diff_path: None,
            diff: Vec::new(),
            added: 0,
            removed: 0,
            tools: Vec::new(),
        }
    }

    pub fn planned(kind: StageKind) -> Self {
        Self::new(kind, StageStatus::Planned, kind.plan())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub number: u32,
    pub text: String,
}

impl DiffLine {
    pub fn new(kind: LineKind, number: u32, text: &str) -> Self {
        Self {
            kind,
            number,
            text: text.to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hunk {
    /// "@@ 204,13 · readThinking"
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileChange {
    /// From the project's root.
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub hunks: Vec<Hunk>,
}

impl FileChange {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckResult {
    Passed,
    Failed,
    /// The user declined to run it.
    NotRun,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub command: String,
    pub result: CheckResult,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Pi's closing words, shown as a serif headline.
    pub headline: String,
    pub body: String,
    /// The original Markdown, kept intact for rendering and copying.
    pub source: Option<String>,
}

impl Summary {
    pub fn text(&self) -> String {
        self.source.clone().unwrap_or_else(|| {
            if self.body.is_empty() {
                self.headline.clone()
            } else {
                format!("{}\n\n{}", self.headline, self.body)
            }
        })
    }
}

/// One step of a turn's reply, in the order Pi made it.
#[derive(Clone, Debug, PartialEq)]
pub enum Flow {
    /// Markdown Pi wrote, before, between or after its tools.
    Text(String),
    /// `Turn::images[n]`.
    Image(usize),
    /// `Turn::pages[n]`, where it last changed.
    Page(usize),
    /// `Turn::handoffs[n]`: work Pi handed to subagents.
    Handoff(usize),
    /// Pi's context was summarized here, to make room: the summary. What came
    /// before stays in the thread.
    Compacted(String),
    /// The model's reasoning before what it said or called next, folded until
    /// tapped.
    Thinking(String),
    /// A tool Pi called: `stages[stage].tools[index]` of the stage of that kind.
    Tool(StageKind, usize),
}

impl Turn {
    /// Whether pictures or pages sit between Pi's words, so the reply reads in order.
    pub fn interleaved(&self) -> bool {
        self.flow
            .iter()
            .any(|step| !matches!(step, Flow::Text(_) | Flow::Tool(..) | Flow::Thinking(_)))
    }

    /// Whether the order of Pi's tool calls is known, so they can stream into
    /// the thread between its words.
    pub fn streams(&self) -> bool {
        self.flow.iter().any(|step| matches!(step, Flow::Tool(..)))
    }

    /// A tool call the flow names.
    pub fn tool(&self, kind: StageKind, index: usize) -> Option<&ToolActivity> {
        self.stages
            .iter()
            .find(|stage| stage.kind == kind)
            .and_then(|stage| stage.tools.get(index))
    }

    /// How many tools Pi called, and how many of those failed.
    pub fn call_counts(&self) -> (usize, usize) {
        self.stages
            .iter()
            .flat_map(|stage| &stage.tools)
            .fold((0, 0), |(calls, failed), tool| {
                (calls + 1, failed + usize::from(tool.failed))
            })
    }

    /// Notes a page that was written or changed: shown once, where it last changed.
    pub fn show_page(&mut self, page: pi_markdown::Page) {
        let n = match self.pages.iter().position(|shown| shown.path == page.path) {
            Some(n) => {
                self.pages[n] = page;
                n
            }
            None => {
                self.pages.push(page);
                self.pages.len() - 1
            }
        };
        self.flow.retain(|step| *step != Flow::Page(n));
        self.flow.push(Flow::Page(n));
    }
}

pub use pi_markdown::ToolImage;

#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    /// Local partial/failed turns stay visible but never enter future inference context.
    pub excluded: Option<String>,
    pub prompt: String,
    /// The local time it was sent: "09:41".
    pub at: String,
    /// What came with the prompt: "screenshot.png", "lines 211–212".
    pub attachments: Vec<String>,
    /// Images submitted with this prompt. They remain separate from pictures Pi
    /// later inspected so attachment chips can open the original image.
    pub prompt_images: Vec<ToolImage>,
    pub stages: Vec<Stage>,
    pub summary: Option<Summary>,
    /// HTML pages Pi wrote or changed, shown as cards that open them.
    pub pages: Vec<pi_markdown::Page>,
    /// Pictures Pi looked at, such as a screenshot it took and read.
    pub images: Vec<ToolImage>,
    /// Each `subagent` call: who Pi handed work to, and how it went.
    pub handoffs: Vec<pi_core::subagent::Handoff>,
    /// Its prompt is subagents reporting back, not something the user typed.
    pub reported: bool,
    /// Stable projected user-message identity, independent of turn position.
    pub visual_identity: Option<String>,
    /// Committed Durable user entry; UI association only, never execution authority.
    pub durable_entry_id: Option<u64>,
    /// What Pi said and showed, in order: words between tools, then a
    /// picture it looked at, more words, a page, the closing words.
    pub flow: Vec<Flow>,
    /// The run line: what the time went on, in the order it happened.
    pub run: Vec<Stretch>,
}

impl Turn {
    pub fn new(prompt: impl Into<String>, at: impl Into<String>) -> Self {
        Self {
            excluded: None,
            prompt: prompt.into(),
            at: at.into(),
            attachments: Vec::new(),
            prompt_images: Vec::new(),
            stages: StageKind::ALL.into_iter().map(Stage::planned).collect(),
            summary: None,
            pages: Vec::new(),
            images: Vec::new(),
            handoffs: Vec::new(),
            reported: false,
            visual_identity: None,
            durable_entry_id: None,
            flow: Vec::new(),
            run: Vec::new(),
        }
    }

    /// The time spent on what a stage's calls do, over the whole turn. The
    /// hand-off's closing words count as looking.
    pub fn time(&self, kind: StageKind) -> Duration {
        if kind == StageKind::HandOff {
            return Duration::ZERO;
        }
        self.spent(StretchKind::of(kind))
    }

    /// The time one kind of stretch took, in all.
    pub fn spent(&self, kind: StretchKind) -> Duration {
        self.run
            .iter()
            .filter(|stretch| stretch.kind == kind)
            .map(|stretch| stretch.time)
            .sum()
    }

    /// The whole run line's time, waits included.
    pub fn run_time(&self) -> Duration {
        self.run.iter().map(|stretch| stretch.time).sum()
    }

    /// Time on what Pi is doing now: the stretch under way grows, or the next
    /// one starts. A stretch that has ended, such as a check with its result,
    /// takes no more time.
    pub fn add_time(&mut self, kind: StretchKind, time: Duration) {
        self.stretch(kind).time += time;
    }

    /// Notes a call in the stretch under way of its kind, starting one if needed.
    pub fn add_call(&mut self, kind: StretchKind, call: (StageKind, usize)) {
        let stretch = self.stretch(kind);
        if !stretch.calls.contains(&call) {
            stretch.calls.push(call);
        }
    }

    /// Ends the stretch under way, if it is of this kind, with how it ended.
    pub fn end_stretch(&mut self, kind: StretchKind, mark: Mark) {
        if let Some(last) = self.run.last_mut()
            && last.kind == kind
            && last.mark.is_none()
        {
            last.mark = Some(mark);
        }
    }

    fn stretch(&mut self, kind: StretchKind) -> &mut Stretch {
        let open = self
            .run
            .last()
            .is_some_and(|last| last.kind == kind && last.mark.is_none());
        // A stretch with no time, call or result yet says nothing: what came
        // next takes its place instead of leaving a sliver on the line.
        let empty = self.run.last().is_some_and(|last| {
            last.time.is_zero() && last.mark.is_none() && last.calls.is_empty()
        });
        if !open && empty {
            self.run.last_mut().expect("a stretch").kind = kind;
        } else if !open {
            self.run.push(Stretch {
                kind,
                time: Duration::ZERO,
                mark: None,
                calls: Vec::new(),
            });
        }
        self.run.last_mut().expect("a stretch")
    }

    /// The stage Pi is in now, if the turn is running.
    pub fn live_stage(&self) -> Option<StageKind> {
        self.stages
            .iter()
            .find(|stage| stage.status == StageStatus::Live)
            .map(|stage| stage.kind)
    }

    pub fn stage_mut(&mut self, kind: StageKind) -> &mut Stage {
        let index = self
            .stages
            .iter()
            .position(|stage| stage.kind == kind)
            .unwrap_or_else(|| {
                self.stages.push(Stage::planned(kind));
                self.stages.len() - 1
            });
        &mut self.stages[index]
    }
}

/// One subagent anywhere in a session, with the coordinates that open it.
#[derive(Clone, Copy, Debug)]
pub struct SessionSubagent<'a> {
    pub turn: usize,
    pub handoff: usize,
    pub index: usize,
    pub subagent: &'a pi_core::subagent::Subagent,
}

/// Every subagent in a session: active first, then waiting, then terminal,
/// preserving call/task order within each group.
pub fn background_subagents_at_work(turns: &[Turn]) -> usize {
    turns
        .iter()
        .flat_map(|turn| &turn.handoffs)
        .filter(|handoff| handoff.background)
        .flat_map(|handoff| &handoff.subagents)
        .filter(|subagent| !subagent.status.finished())
        .count()
}

pub fn session_subagents(turns: &[Turn]) -> Vec<SessionSubagent<'_>> {
    let mut subagents: Vec<_> = turns
        .iter()
        .enumerate()
        .flat_map(|(turn, item)| {
            item.handoffs
                .iter()
                .enumerate()
                .flat_map(move |(handoff, item)| {
                    item.subagents
                        .iter()
                        .enumerate()
                        .map(move |(index, subagent)| SessionSubagent {
                            turn,
                            handoff,
                            index,
                            subagent,
                        })
                })
        })
        .collect();
    subagents.sort_by_key(|item| match item.subagent.status {
        pi_core::subagent::Status::Running => 0,
        pi_core::subagent::Status::Waiting => 1,
        pi_core::subagent::Status::Done
        | pi_core::subagent::Status::Stopped
        | pi_core::subagent::Status::Failed => 2,
    });
    subagents
}

/// A detailed live summary for the session-wide subagent directory.
pub fn subagent_summary(turns: &[Turn]) -> Option<String> {
    use pi_core::subagent::Status;

    let mut counts = [0usize; 5];
    for item in turns
        .iter()
        .flat_map(|turn| &turn.handoffs)
        .flat_map(|handoff| &handoff.subagents)
    {
        let index = match item.status {
            Status::Running => 0,
            Status::Waiting => 1,
            Status::Done => 2,
            Status::Stopped => 3,
            Status::Failed => 4,
        };
        counts[index] += 1;
    }
    let [working, waiting, done, stopped, failed] = counts;
    let total = counts.into_iter().sum::<usize>();
    if total == 0 {
        return None;
    }
    if working == 0 && waiting == 0 {
        let noun = if total == 1 { "subagent" } else { "subagents" };
        let mut parts = vec![format!("{total} {noun}")];
        if stopped > 0 {
            parts.push(format!("{stopped} stopped"));
        }
        if failed > 0 {
            parts.push(format!("{failed} failed"));
        }
        return Some(parts.join(" · "));
    }
    let mut parts = [(working, "working"), (waiting, "waiting")]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, label)| format!("{count} {label}"))
        .collect::<Vec<_>>();
    let terminal = done + stopped + failed;
    if terminal > 0 && stopped == 0 && failed == 0 {
        parts.push(format!("{done} done"));
    } else if terminal > 0 {
        let exceptions = [(stopped, "stopped"), (failed, "failed")]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, label)| format!("{count} {label}"))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("{terminal} finished ({exceptions})"));
    }
    Some(parts.join(" · "))
}

/// A short status-preserving summary for the constrained session Details row.
pub fn subagent_details_summary(turns: &[Turn]) -> Option<String> {
    use pi_core::subagent::Status;

    let mut total = 0;
    let mut working = 0;
    let mut waiting = 0;
    for subagent in turns
        .iter()
        .flat_map(|turn| &turn.handoffs)
        .flat_map(|handoff| &handoff.subagents)
    {
        total += 1;
        match subagent.status {
            Status::Running => working += 1,
            Status::Waiting => waiting += 1,
            Status::Done | Status::Stopped | Status::Failed => {}
        }
    }
    if total == 0 {
        return None;
    }
    if working == 0 && waiting == 0 {
        return Some(format!(
            "{total} {}",
            if total == 1 { "subagent" } else { "subagents" }
        ));
    }
    let mut parts = vec![format!("{total} total")];
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    if waiting > 0 {
        parts.push(format!("{waiting} waiting"));
    }
    Some(parts.join(" · "))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Yes to a `confirm`.
    AllowOnce,
    AllowSession,
    /// No to a `confirm`.
    Deny,
    /// One of a `select`'s options, by its place in the list.
    Pick(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub answer: Answer,
    pub label: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub title: String,
    pub body: String,
    pub command: String,
    pub choices: Vec<Choice>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Details {
    /// Current prompt-context use, when the backend reports it. Lifetime token
    /// totals are not a substitute for this value.
    pub context_percent: Option<u32>,
    pub context_tokens: String,
    /// Resolved backend threshold, not a design-time percentage.
    pub compaction_percent: Option<f64>,
    pub auto_compaction_enabled: bool,
    pub swarm_enabled: Option<bool>,
    pub background_compacting: bool,
    pub cost: String,
    pub turns: u32,
    pub tools: Vec<String>,
    pub snapshots: u32,
    pub session_file: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// Execution origin only. Feature availability comes from backend metadata.
    pub local: bool,
    /// Optional authoritative UI capabilities; absent retains legacy-backend behavior.
    pub backend_features: Option<Vec<String>>,
    pub id: SessionId,
    pub title: String,
    pub project: String,
    /// The project's folder on the computer: "~/repos/pi".
    pub folder: String,
    pub state: State,
    /// What Pi is doing right now: "Editing a file".
    pub activity: String,
    /// Time spent working, which stops while Pi waits for you.
    pub elapsed: Duration,
    /// When it finished, as local time.
    pub finished_at: Option<String>,
    /// Seconds since 1970 when it last started or ended; 0 if unknown. Home
    /// compares it with when you last looked.
    pub updated: u64,
    pub turns: Vec<Turn>,
    pub files: Vec<FileChange>,
    pub check: Option<Check>,
    pub question: Option<Question>,
    /// Prompts queued for this session, including steering and follow-ups.
    pub queued: Vec<String>,
    /// Parallel to `queued`, from this same snapshot. Missing helper identities
    /// are not guessed from text or indices and cannot be cancelled by the UI.
    pub queued_keys: Vec<Option<QueueKey>>,
    pub failure: Option<String>,
    pub details: Details,
    /// The sample run's progress; see `demo`.
    pub script: Option<crate::demo::Script>,
}

impl Session {
    pub fn supports(&self, feature: &str) -> bool {
        self.backend_features
            .as_ref()
            .is_none_or(|features| features.iter().any(|f| f == feature))
    }
    pub fn turn(&self) -> Option<&Turn> {
        self.turns.last()
    }

    pub fn turn_mut(&mut self) -> &mut Turn {
        if self.turns.is_empty() {
            self.turns.push(Turn::new("", ""));
        }
        let last = self.turns.len() - 1;
        &mut self.turns[last]
    }

    pub fn background_subagents_at_work(&self) -> usize {
        background_subagents_at_work(&self.turns)
    }

    /// A finished parent remains active while its background crew is running.
    pub fn is_working(&self) -> bool {
        self.state == State::Working
            || (self.state == State::Done && self.background_subagents_at_work() > 0)
    }

    /// The line under the title in lists: what it is doing, or how it ended.
    pub fn status_line(&self) -> String {
        if self.local {
            return self
                .failure
                .clone()
                .unwrap_or_else(|| self.activity.clone());
        }
        match self.state {
            State::NeedsYou => self.question.as_ref().map_or_else(
                || "Waiting for you".into(),
                |question| question.title.clone(),
            ),
            State::Working => self.activity.clone(),
            State::Done => {
                let background = self.background_subagents_at_work();
                if background > 0 {
                    return if background == 1 {
                        "1 subagent working".into()
                    } else {
                        format!("{background} subagents working")
                    };
                }
                let files = match self.files.len() {
                    0 => "no changes".to_owned(),
                    1 => "1 file changed".to_owned(),
                    count => format!("{count} files changed"),
                };
                match &self.check {
                    Some(check) if check.result == CheckResult::Passed => {
                        format!("{files} · checks passed")
                    }
                    Some(check) if check.result == CheckResult::Failed => {
                        format!("{files} · checks failed")
                    }
                    Some(_) => format!("{files} · checks not run"),
                    None => files,
                }
            }
            State::Stopped => "stopped by you".into(),
            State::Failed => self.failure.clone().unwrap_or_else(|| "failed".to_owned()),
        }
    }
}

/// "4:31", or "1:02:05" past an hour.
pub fn duration_label(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// The local time of day: "09:41".
pub fn clock_now() -> String {
    // SAFETY: `time` with a null pointer only returns the time.
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    clock_at(Duration::from_secs(now as u64))
}

/// The local time of day at a moment, given as time since 1970.
pub fn clock_at(since_epoch: Duration) -> String {
    let time = since_epoch.as_secs() as libc::time_t;
    // SAFETY: `localtime_r` reads `time` and writes only into `tm`.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&time, &mut tm).is_null() {
            return String::new();
        }
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_computer_is_named_by_its_host() {
        assert_eq!(
            Computer::from_address("nick@studio-mac.local").name,
            "studio-mac"
        );
        assert_eq!(Computer::from_address("dev@10.0.4.12").name, "10.0.4.12");
        assert_eq!(Computer::from_address("build-box:2222").name, "build-box");
    }

    #[test]
    fn durations_read_like_a_clock() {
        assert_eq!(duration_label(Duration::from_secs(72)), "1:12");
        assert_eq!(duration_label(Duration::from_secs(3725)), "1:02:05");
    }

    #[test]
    fn stages_are_named_by_their_status() {
        assert_eq!(StageKind::Change.name(StageStatus::Live), "Changing");
        assert_eq!(StageKind::Understand.name(StageStatus::Done), "Looked");
        assert_eq!(StageKind::Verify.name(StageStatus::Live), "Checking");
        assert_eq!(StageKind::HandOff.name(StageStatus::Planned), "Hand off");
    }

    #[test]
    fn a_run_keeps_its_order_and_each_checks_result() {
        let seconds = Duration::from_secs;
        let mut turn = Turn::new("", "");
        turn.add_time(StretchKind::Look, seconds(40));
        turn.add_time(StretchKind::Change, seconds(50));
        turn.add_time(StretchKind::Change, seconds(20));
        turn.add_time(StretchKind::Check, seconds(14));
        turn.end_stretch(StretchKind::Check, Mark::Failed);
        turn.add_time(StretchKind::Change, seconds(25));
        turn.add_time(StretchKind::Check, seconds(92));
        turn.end_stretch(StretchKind::Check, Mark::Passed);
        // A second check after a result starts a stretch of its own.
        turn.add_time(StretchKind::Check, seconds(5));
        turn.end_stretch(StretchKind::Look, Mark::Passed);
        let run: Vec<_> = turn
            .run
            .iter()
            .map(|stretch| (stretch.kind, stretch.time.as_secs(), stretch.mark))
            .collect();
        assert_eq!(
            run,
            [
                (StretchKind::Look, 40, None),
                (StretchKind::Change, 70, None),
                (StretchKind::Check, 14, Some(Mark::Failed)),
                (StretchKind::Change, 25, None),
                (StretchKind::Check, 92, Some(Mark::Passed)),
                (StretchKind::Check, 5, None),
            ]
        );
        assert_eq!(turn.spent(StretchKind::Check), seconds(111));
        assert_eq!(turn.time(StageKind::Change), seconds(95));
        assert_eq!(turn.time(StageKind::HandOff), Duration::ZERO);
        assert_eq!(turn.run_time(), seconds(246));
    }

    #[test]
    fn a_page_shows_once_per_turn() {
        let mut turn = Turn::new("", "");
        let page = |html: &str| pi_markdown::Page {
            path: "a.html".into(),
            html: Some(html.into()),
        };
        turn.show_page(page("one"));
        turn.flow.push(Flow::Text("Changing it.".into()));
        turn.show_page(page("two"));
        assert_eq!(turn.pages, vec![page("two")]);
        assert_eq!(
            turn.flow,
            [Flow::Text("Changing it.".into()), Flow::Page(0)],
            "where it last changed"
        );
    }

    fn subagent_turn(statuses: &[pi_core::subagent::Status]) -> Turn {
        let mut handoff = pi_core::subagent::Handoff::from_args(&serde_json::json!({
            "tasks": statuses
                .iter()
                .enumerate()
                .map(|(index, _)| serde_json::json!({
                    "agent": "scout",
                    "task": format!("task {index}")
                }))
                .collect::<Vec<_>>()
        }));
        for (subagent, status) in handoff.subagents.iter_mut().zip(statuses) {
            subagent.status = *status;
        }
        let mut turn = Turn::new("", "");
        turn.handoffs.push(handoff);
        turn
    }

    #[test]
    fn a_session_directory_orders_and_summarizes_every_status() {
        use pi_core::subagent::Status::*;

        let turns = vec![
            subagent_turn(&[Done, Running, Waiting]),
            subagent_turn(&[Stopped, Failed, Running]),
        ];
        assert_eq!(
            session_subagents(&turns)
                .into_iter()
                .map(|item| (item.turn, item.index, item.subagent.status))
                .collect::<Vec<_>>(),
            [
                (0, 1, Running),
                (1, 2, Running),
                (0, 2, Waiting),
                (0, 0, Done),
                (1, 0, Stopped),
                (1, 1, Failed),
            ]
        );
        assert_eq!(
            subagent_summary(&turns).as_deref(),
            Some("2 working · 1 waiting · 3 finished (1 stopped, 1 failed)")
        );
        assert_eq!(
            subagent_details_summary(&turns).as_deref(),
            Some("6 total · 2 working · 1 waiting")
        );
    }

    #[test]
    fn terminal_and_empty_subagent_summaries_stay_compact() {
        use pi_core::subagent::Status::*;

        assert_eq!(
            subagent_summary(&[subagent_turn(&[
                Running, Running, Running, Waiting, Waiting, Done, Done, Done, Done, Done,
            ])])
            .as_deref(),
            Some("3 working · 2 waiting · 5 done")
        );
        let terminal = [subagent_turn(&[Done; 4])];
        assert_eq!(subagent_summary(&terminal).as_deref(), Some("4 subagents"));
        assert_eq!(
            subagent_details_summary(&terminal).as_deref(),
            Some("4 subagents")
        );
        assert_eq!(
            subagent_summary(&[subagent_turn(&[Done, Stopped, Done])]).as_deref(),
            Some("3 subagents · 1 stopped")
        );
        assert_eq!(
            subagent_summary(&[subagent_turn(&[Done, Failed])]).as_deref(),
            Some("2 subagents · 1 failed")
        );
        assert_eq!(subagent_summary(&[]), None);
        assert_eq!(subagent_details_summary(&[]), None);
    }
}
