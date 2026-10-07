//! What the phone shows of Pi's sessions on a computer. The shapes follow Pi
//! Desktop's thread: a session is turns of a prompt, the stages Pi went
//! through, and a hand-off; its observed edits and checks are reviewed apart.

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u32);

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

    /// The stage's name in a key: "Understand".
    pub fn title(self) -> &'static str {
        self.name(StageStatus::Planned)
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Understand => "eye",
            Self::Change => "pencil",
            Self::Verify => "shield",
            Self::HandOff => "send",
        }
    }

    pub fn name(self, status: StageStatus) -> &'static str {
        match (self, status) {
            (Self::Understand, StageStatus::Done) => "Understood",
            (Self::Understand, StageStatus::Live) => "Understanding",
            (Self::Understand, _) => "Understand",
            (Self::Change, StageStatus::Done) => "Changed",
            (Self::Change, StageStatus::Live) => "Changing",
            (Self::Change, _) => "Change",
            (Self::Verify, StageStatus::Done) => "Checked",
            (Self::Verify, StageStatus::Live) => "Verifying",
            (Self::Verify, _) => "Verify",
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
}

impl Stage {
    pub fn new(kind: StageKind, status: StageStatus, what: impl Into<String>) -> Self {
        Self {
            kind,
            status,
            what: what.into(),
            references: Vec::new(),
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
}

impl Turn {
    /// Whether pictures or pages sit between Pi's words, so the reply reads in order.
    pub fn interleaved(&self) -> bool {
        self.flow.iter().any(|step| !matches!(step, Flow::Text(_)))
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
    pub prompt: String,
    /// The local time it was sent: "09:41".
    pub at: String,
    /// What came with the prompt: "screenshot.png", "lines 211–212".
    pub attachments: Vec<String>,
    pub stages: Vec<Stage>,
    pub summary: Option<Summary>,
    /// HTML pages Pi wrote or changed, shown as cards that open them.
    pub pages: Vec<pi_markdown::Page>,
    /// Pictures Pi looked at, such as a screenshot it took and read.
    pub images: Vec<ToolImage>,
    /// What Pi said and showed, in order: words between tools, then a
    /// picture it looked at, more words, a page, the closing words.
    pub flow: Vec<Flow>,
    /// Working time spent in each stage, in `StageKind::ALL` order: the run line's stretches.
    pub times: [Duration; 4],
}

impl Turn {
    pub fn new(prompt: impl Into<String>, at: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            at: at.into(),
            attachments: Vec::new(),
            stages: StageKind::ALL.into_iter().map(Stage::planned).collect(),
            summary: None,
            pages: Vec::new(),
            images: Vec::new(),
            flow: Vec::new(),
            times: [Duration::ZERO; 4],
        }
    }

    /// The working time spent in a stage.
    pub fn time(&self, kind: StageKind) -> Duration {
        self.times[kind.index()]
    }

    pub fn add_time(&mut self, kind: StageKind, time: Duration) {
        self.times[kind.index()] += time;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    AllowOnce,
    AllowSession,
    Deny,
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
    pub context_percent: u32,
    pub context_tokens: String,
    pub cost: String,
    pub turns: u32,
    pub tools: Vec<String>,
    pub snapshots: u32,
    pub session_file: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
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
    pub turns: Vec<Turn>,
    pub files: Vec<FileChange>,
    pub check: Option<Check>,
    pub question: Option<Question>,
    /// Follow-ups waiting for the current run to finish.
    pub queued: Vec<String>,
    pub failure: Option<String>,
    pub details: Details,
    /// The sample run's progress; see `demo`.
    pub script: Option<crate::demo::Script>,
}

impl Session {
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

    /// The line under the title in lists: what it is doing, or how it ended.
    pub fn status_line(&self) -> String {
        match self.state {
            State::NeedsYou => self.question.as_ref().map_or_else(
                || "Waiting for you".into(),
                |question| question.title.clone(),
            ),
            State::Working => self.activity.clone(),
            State::Done => {
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
        assert_eq!(StageKind::Understand.name(StageStatus::Done), "Understood");
        assert_eq!(StageKind::HandOff.name(StageStatus::Planned), "Hand off");
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
}
