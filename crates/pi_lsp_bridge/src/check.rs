//! Decides which language-server errors pi hears about after files change.
//!
//! A changed file reaches its servers the way an editor save would: the buffer
//! reloads from disk, then `didSave`. The save is what starts save-triggered
//! checks such as rust-analyzer's `cargo check`.
use anyhow::Result;
use gpui::{App, AsyncApp, Context, Entity, Subscription, Task, WeakEntity};
use language::Point;
use lsp::DiagnosticSeverity;
use project::{Event, PathChange, Project, ProjectPath, lsp_store::OpenLspBufferHandle};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fmt::Write as _,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// How long an `edit` or `write` result may wait for the servers.
pub const FILE_WAIT: Duration = Duration::from_secs(4);
/// How long the end of a run may wait, for example for `cargo check`.
pub const RUN_WAIT: Duration = Duration::from_secs(20);
/// Servers send nothing when a file's errors did not change, so a check that
/// notified them waits at least this long for an answer...
const MIN_WAIT: Duration = Duration::from_secs(1);
/// ...and until no server has reported anything for this long.
const QUIET: Duration = Duration::from_millis(300);
const POLL: Duration = Duration::from_millis(50);
/// Errors listed per section; the rest are counted.
const LISTED: usize = 10;
/// Changed files sent to the servers when a run ends.
const RUN_FILES: usize = 20;
/// Files kept open in the servers after they changed, oldest closed first.
const KEPT_OPEN: usize = 64;
/// Files with errors read per check.
const READ_FILES: usize = 200;
/// Changed files remembered between run ends.
const TRACKED: usize = 256;

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// pi's `edit` or `write` changed this file.
    File { path: PathBuf },
    /// A run ended: report errors from anything else that changed.
    RunEnd,
}

/// Errors by worktree-relative path.
type Errors = BTreeMap<String, Vec<Problem>>;
/// The session folder's project while its language services are on.
type Lookup = Box<dyn Fn(&App) -> Option<Entity<Project>>>;
/// Whether the user wants this kind of check.
type Allow = Box<dyn Fn(&Request, &App) -> bool>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Problem {
    line: u32,
    column: u32,
    message: String,
}

pub struct Checker {
    lookup: Lookup,
    allow: Allow,
    project: Option<WeakEntity<Project>>,
    /// Errors pi was told about or that existed before checking began, by file
    /// and message: lines move when a file changes, so they are not compared.
    known: HashSet<(String, String)>,
    started: bool,
    /// Files whose last report listed errors, to say when they are gone.
    had_errors: HashSet<String>,
    /// Files changed on disk since the last run end that no file check covered.
    changed: Vec<ProjectPath>,
    /// Whether this run sent any file to the servers.
    notified: bool,
    open: VecDeque<(ProjectPath, OpenLspBufferHandle)>,
    last_event: Option<Instant>,
    updated: HashMap<ProjectPath, Instant>,
    _subscription: Option<Subscription>,
}

impl Checker {
    /// `lookup` returns the session folder's project while its language services are on.
    pub fn new(lookup: impl Fn(&App) -> Option<Entity<Project>> + 'static) -> Self {
        Self {
            lookup: Box::new(lookup),
            allow: Box::new(|_, _| true),
            project: None,
            known: HashSet::new(),
            started: false,
            had_errors: HashSet::new(),
            changed: Vec::new(),
            notified: false,
            open: VecDeque::new(),
            last_event: None,
            updated: HashMap::new(),
            _subscription: None,
        }
    }

    /// Answers only the requests `allow` accepts; the others get an empty answer.
    pub fn allowing(mut self, allow: impl Fn(&Request, &App) -> bool + 'static) -> Self {
        self.allow = Box::new(allow);
        self
    }

    /// The text to give pi; empty when there is nothing to say, no project to ask
    /// or the check is turned off.
    pub fn handle(&mut self, request: Request, cx: &mut Context<Self>) -> Task<Result<String>> {
        let Some(project) = self.attach(cx) else {
            return Task::ready(Ok(String::new()));
        };
        // Attached either way, so a run end still sees files a skipped check changed.
        if !(self.allow)(&request, cx) {
            return Task::ready(Ok(String::new()));
        }
        cx.spawn(async move |this, cx| match request {
            Request::File { path } => check_file(this, project, path, cx).await,
            Request::RunEnd => check_run(this, project, cx).await,
        })
    }

    fn attach(&mut self, cx: &mut Context<Self>) -> Option<Entity<Project>> {
        let project = (self.lookup)(cx)?;
        if self.project.as_ref().and_then(WeakEntity::upgrade).as_ref() != Some(&project) {
            let lookup = std::mem::replace(&mut self.lookup, Box::new(|_| None));
            let allow = std::mem::replace(&mut self.allow, Box::new(|_, _| true));
            *self = Self::new(lookup).allowing(allow);
            self._subscription = Some(cx.subscribe(&project, Self::project_event));
            self.project = Some(project.downgrade());
        }
        Some(project)
    }

    fn project_event(&mut self, _: Entity<Project>, event: &Event, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        match event {
            Event::DiagnosticsUpdated { paths, .. } => {
                self.last_event = Some(now);
                for path in paths {
                    self.updated.insert(path.clone(), now);
                }
            }
            Event::DiskBasedDiagnosticsStarted { .. }
            | Event::DiskBasedDiagnosticsFinished { .. }
            | Event::LanguageServerAdded(..) => self.last_event = Some(now),
            Event::WorktreeUpdatedEntries(worktree_id, entries) => {
                for (path, _, change) in entries.iter() {
                    let path = ProjectPath {
                        worktree_id: *worktree_id,
                        path: path.clone(),
                    };
                    if matches!(
                        change,
                        PathChange::Added | PathChange::Updated | PathChange::AddedOrUpdated
                    ) && self.changed.len() < TRACKED
                        && !self.changed.contains(&path)
                    {
                        self.changed.push(path);
                    }
                }
            }
            _ => {}
        }
    }

    fn keep_open(&mut self, path: ProjectPath, handle: OpenLspBufferHandle) {
        self.open.retain(|(open, _)| *open != path);
        self.open.push_back((path, handle));
        if self.open.len() > KEPT_OPEN {
            self.open.pop_front();
        }
    }

    fn file_report(&mut self, file: &str, errors: Errors, busy: &[String]) -> String {
        let mut out = String::new();
        match errors.get(file) {
            Some(problems) => {
                self.had_errors.insert(file.to_owned());
                let _ = writeln!(out, "Language server errors in {file}:");
                list(&mut out, problems.iter().map(|p| (None, p)));
            }
            None if self.had_errors.remove(file) => {
                let _ = writeln!(out, "{file} has no language server errors now.");
            }
            None => {}
        }
        let new = self.new_errors(&errors, Some(file));
        if !new.is_empty() {
            let _ = writeln!(out, "New errors in other files:");
            list(&mut out, new.into_iter().map(|(file, p)| (Some(file), p)));
        }
        if !busy.is_empty() {
            let _ = writeln!(
                out,
                "Still checking ({}); later errors are reported when this run ends.",
                busy.join(", ")
            );
        }
        self.known = keys(&errors);
        out
    }

    fn run_report(&mut self, errors: Errors, busy: &[String]) -> String {
        let mut out = String::new();
        let new = self.new_errors(&errors, None);
        if !new.is_empty() {
            let _ = writeln!(out, "Language server errors that appeared during this run:");
            for (file, _) in &new {
                self.had_errors.insert((*file).to_owned());
            }
            list(&mut out, new.into_iter().map(|(file, p)| (Some(file), p)));
        }
        if std::mem::take(&mut self.notified) && !busy.is_empty() {
            let _ = writeln!(
                out,
                "Language servers were still checking ({}) when this run ended.",
                busy.join(", ")
            );
        }
        self.known = keys(&errors);
        out
    }

    fn new_errors<'a>(
        &self,
        errors: &'a Errors,
        skip: Option<&str>,
    ) -> Vec<(&'a str, &'a Problem)> {
        errors
            .iter()
            .filter(|(file, _)| Some(file.as_str()) != skip)
            .flat_map(|(file, problems)| problems.iter().map(move |p| (file.as_str(), p)))
            .filter(|(file, p)| !self.known.contains(&(file.to_string(), p.message.clone())))
            .collect()
    }
}

async fn check_file(
    this: WeakEntity<Checker>,
    project: Entity<Project>,
    path: PathBuf,
    cx: &mut AsyncApp,
) -> Result<String> {
    let Some(project_path) = project.read_with(cx, |p, cx| p.find_project_path(&path, cx)) else {
        return Ok(String::new());
    };
    if !has_server(&project, &path, cx).await {
        return Ok(String::new());
    }
    start(&this, &project, cx).await?;
    let since = cx.background_executor().now();
    if !notify(&this, &project, project_path.clone(), cx).await? {
        return Ok(String::new());
    }
    let busy = settle(
        &this,
        &project,
        since,
        true,
        Some(&project_path),
        FILE_WAIT,
        cx,
    )
    .await?;
    let errors = errors(&project, cx).await;
    this.update(cx, |this, _| {
        // This file's own change is covered; a later one is not.
        this.changed.retain(|changed| *changed != project_path);
        this.file_report(project_path.path.as_unix_str(), errors, &busy)
    })
}

async fn check_run(
    this: WeakEntity<Checker>,
    project: Entity<Project>,
    cx: &mut AsyncApp,
) -> Result<String> {
    start(&this, &project, cx).await?;
    let changed = this.update(cx, |this, _| std::mem::take(&mut this.changed))?;
    let since = cx.background_executor().now();
    let mut notified = false;
    for path in changed.into_iter().rev().take(RUN_FILES) {
        let file = project.read_with(cx, |p, cx| {
            p.entry_for_path(&path, cx)
                .filter(|entry| entry.is_file() && !entry.is_ignored)
                .and_then(|_| p.absolute_path(&path, cx))
        });
        if let Some(file) = file
            && has_server(&project, &file, cx).await
        {
            notified |= notify(&this, &project, path, cx).await?;
        }
    }
    let busy = settle(&this, &project, since, notified, None, RUN_WAIT, cx).await?;
    let errors = errors(&project, cx).await;
    this.update(cx, |this, _| this.run_report(errors, &busy))
}

/// Records the errors that predate the first check, so they are not reported as new.
async fn start(
    this: &WeakEntity<Checker>,
    project: &Entity<Project>,
    cx: &mut AsyncApp,
) -> Result<()> {
    if this.read_with(cx, |this, _| this.started)? {
        return Ok(());
    }
    let errors = errors(project, cx).await;
    this.update(cx, |this, _| {
        if !std::mem::replace(&mut this.started, true) {
            this.known = keys(&errors);
        }
    })
}

async fn has_server(project: &Entity<Project>, path: &Path, cx: &mut AsyncApp) -> bool {
    let languages = project.read_with(cx, |p, _| p.languages().clone());
    match languages.load_language_for_file_path(path).await {
        Ok(language) => !languages.lsp_adapters(&language.name()).is_empty(),
        Err(_) => false,
    }
}

/// Sends the file's disk contents to its servers as a save. False when the file
/// has unsaved edits in an editor: the servers see those, not the disk.
async fn notify(
    this: &WeakEntity<Checker>,
    project: &Entity<Project>,
    path: ProjectPath,
    cx: &mut AsyncApp,
) -> Result<bool> {
    let buffer = project
        .update(cx, |p, cx| p.open_buffer(path.clone(), cx))
        .await?;
    let handle = project.update(cx, |p, cx| {
        p.register_buffer_with_language_servers(&buffer, cx)
    });
    if buffer.read_with(cx, |buffer, _| buffer.is_dirty()) {
        return Ok(false);
    }
    buffer.update(cx, |buffer, cx| buffer.reload(cx)).await.ok();
    project.update(cx, |p, cx| {
        p.lsp_store().update(cx, |store, cx| {
            store.on_buffer_saved(buffer.clone(), cx);
            // Servers that are asked for diagnostics instead of sending them.
            store.pull_diagnostics_for_buffer(buffer, cx).detach();
        })
    });
    this.update(cx, |this, _| {
        this.notified = true;
        this.keep_open(path, handle);
    })?;
    Ok(true)
}

/// Waits until the servers have answered and returns the ones still busy at
/// the time limit. Answered: none is busy, none reported for `QUIET`, and
/// `target` got new diagnostics or `MIN_WAIT` passed since `since`.
async fn settle(
    this: &WeakEntity<Checker>,
    project: &Entity<Project>,
    since: Instant,
    notified: bool,
    target: Option<&ProjectPath>,
    limit: Duration,
    cx: &mut AsyncApp,
) -> Result<Vec<String>> {
    let executor = cx.background_executor().clone();
    loop {
        let now = executor.now();
        let busy = project.read_with(cx, |p, cx| {
            p.language_server_statuses(cx)
                .filter(|(_, s)| s.has_pending_diagnostic_updates || !s.pending_work.is_empty())
                .map(|(_, s)| s.name.to_string())
                .collect::<Vec<_>>()
        });
        let answered = this.read_with(cx, |this, _| {
            let quiet = this.last_event.is_none_or(|at| now >= at + QUIET);
            let heard = target.is_some_and(|t| this.updated.get(t).is_some_and(|at| *at >= since));
            quiet && (!notified || heard || now >= since + MIN_WAIT)
        })?;
        if busy.is_empty() && answered {
            return Ok(Vec::new());
        }
        if now >= since + limit {
            return Ok(busy);
        }
        executor.timer(POLL).await;
    }
}

/// Current errors of files without unsaved edits.
async fn errors(project: &Entity<Project>, cx: &mut AsyncApp) -> Errors {
    let paths = project.read_with(cx, |p, cx| {
        let mut seen = HashSet::new();
        p.diagnostic_summaries(false, cx)
            .filter(|(_, _, summary)| summary.error_count > 0)
            .map(|(path, _, _)| path)
            .filter(|path| seen.insert(path.clone()))
            .take(READ_FILES)
            .collect::<Vec<_>>()
    });
    let mut errors = Errors::new();
    for path in paths {
        let Ok(buffer) = project
            .update(cx, |p, cx| p.open_buffer(path.clone(), cx))
            .await
        else {
            continue;
        };
        let problems = buffer.read_with(cx, |buffer, _| {
            if buffer.is_dirty() {
                return Vec::new();
            }
            let snapshot = buffer.snapshot();
            let mut problems = snapshot
                .diagnostics_in_range::<_, Point>(0..snapshot.len(), false)
                .filter(|e| {
                    e.diagnostic.severity == DiagnosticSeverity::ERROR && e.diagnostic.is_primary
                })
                .map(|e| Problem {
                    line: e.range.start.row + 1,
                    column: e.range.start.column + 1,
                    message: first_line(e.diagnostic.message.as_ref()),
                })
                .collect::<Vec<_>>();
            problems.sort();
            problems.dedup();
            problems
        });
        if !problems.is_empty() {
            errors.insert(path.path.as_unix_str().to_owned(), problems);
        }
    }
    errors
}

fn keys(errors: &Errors) -> HashSet<(String, String)> {
    errors
        .iter()
        .flat_map(|(file, problems)| problems.iter().map(|p| (file.clone(), p.message.clone())))
        .collect()
}

fn first_line(message: &str) -> String {
    let line = message.lines().next().unwrap_or("").trim();
    match line.char_indices().nth(200) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_owned(),
    }
}

fn list<'a>(
    out: &mut String,
    problems: impl ExactSizeIterator<Item = (Option<&'a str>, &'a Problem)>,
) {
    let total = problems.len();
    for (file, p) in problems.take(LISTED) {
        let file = file.map(|f| format!("{f}:")).unwrap_or_default();
        let _ = writeln!(out, "  {file}{}:{} {}", p.line, p.column, p.message);
    }
    if total > LISTED {
        let _ = writeln!(out, "  … and {} more", total - LISTED);
    }
}
