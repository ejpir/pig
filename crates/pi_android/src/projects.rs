//! Choosing where Pi works, and looking at what is there: a folder tree that
//! rolls out in place. Folders come from the helper one level at a time;
//! inside a project, its file channel lists every folder and file at once.
//! Browsing is read-only, and replies that arrive after the computer changed
//! are dropped.

use crate::{
    app::{PhoneApp, Route, Sheet, size_label},
    remote,
    text_area::{TextArea, TextAreaEvent},
    theme::theme,
};
use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};
use pi_core::remote_files::{Document, Recovery, RecoveryArtifact};
use pi_markdown::{SyntaxDocument, SyntaxEdit};
use std::collections::{HashMap, HashSet};

/// A rolled-out folder shows this many entries before "N more".
const SHOWN: usize = 100;

/// What a search lists at most.
const FOUND: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Node {
    pub name: String,
    /// On the computer.
    pub path: String,
    pub folder: bool,
    /// Named at the right in the tree's tint: a git repository, a package.
    pub project: bool,
    /// At the right: "git", "package", a file's size, how many are inside.
    pub tag: Option<String>,
    /// A file's size in bytes, when the helper says.
    pub size: Option<u64>,
    /// The project whose file channel listed it.
    pub root: Option<String>,
}

impl Node {
    pub fn hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

pub(crate) enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

pub(crate) enum FileRecoveryState {
    Idle,
    Loading,
    Loaded(Vec<Recovery>),
    Failed(String),
}

/// A project's folders and files, by the folder they are in.
pub(crate) struct Files {
    children: HashMap<String, Vec<Node>>,
    truncated: bool,
}

impl Files {
    fn new(root: &str, tree: pi_core::remote_files::Tree) -> Self {
        let mut children: HashMap<String, Vec<Node>> = HashMap::new();
        let packages: HashSet<String> = tree
            .entries
            .iter()
            .filter(|entry| {
                !entry.directory
                    && matches!(
                        file_name(&entry.path),
                        "package.json" | "Cargo.toml" | "pyproject.toml"
                    )
            })
            .map(|entry| parent(&entry.path).to_owned())
            .collect();
        for entry in &tree.entries {
            let relative = entry.path.trim_end_matches('/');
            let folder = parent(relative);
            let package = entry.directory && packages.contains(relative);
            children.entry(join(root, folder)).or_default().push(Node {
                name: file_name(relative).to_owned(),
                path: join(root, relative),
                folder: entry.directory,
                project: package,
                tag: if entry.directory {
                    package.then(|| "package".into())
                } else {
                    entry.size.map(size_label)
                },
                size: entry.size,
                root: Some(root.to_owned()),
            });
        }
        let counts: HashMap<String, usize> = children
            .iter()
            .map(|(folder, nodes)| (folder.clone(), nodes.len()))
            .collect();
        for nodes in children.values_mut() {
            for node in nodes
                .iter_mut()
                .filter(|node| node.folder && node.tag.is_none())
            {
                node.tag = counts.get(&node.path).map(usize::to_string);
            }
            sort(nodes);
        }
        Self {
            children,
            truncated: tree.truncated,
        }
    }
}

/// Folders first, then by name as people read it.
fn sort(nodes: &mut [Node]) {
    nodes.sort_by(|a, b| {
        b.folder
            .cmp(&a.folder)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub(crate) fn language_for_path(path: &str) -> Option<&'static str> {
    let name = file_name(path).to_ascii_lowercase();
    if name == "go.mod" {
        return Some("gomod");
    }
    if name == "go.work" {
        return Some("gowork");
    }
    if name == "commit_editmsg" {
        return Some("gitcommit");
    }
    Some(
        match name.rsplit_once('.').map_or("", |(_, extension)| extension) {
            "rs" => "rust",
            "ts" => "typescript",
            "tsx" => "tsx",
            "js" | "jsx" | "mjs" | "cjs" => "javascript",
            "py" => "python",
            "go" => "go",
            "c" | "h" => "c",
            "cc" | "cpp" | "cxx" | "hpp" => "cpp",
            "sh" | "bash" | "zsh" => "bash",
            "json" => "json",
            "jsonc" => "jsonc",
            "css" => "css",
            "md" | "markdown" => "markdown",
            "jsdoc" => "jsdoc",
            "regex" => "regex",
            "yaml" | "yml" => "yaml",
            "diff" | "patch" => "diff",
            _ => return None,
        },
    )
}

fn set_file_recoveries(app: &mut PhoneApp, result: anyhow::Result<Vec<Recovery>>) {
    app.file_recoveries = match result {
        Ok(records) => FileRecoveryState::Loaded(records),
        Err(error) => FileRecoveryState::Failed(format!("{error:#}")),
    };
}

fn join(root: &str, relative: &str) -> String {
    match (root.trim_end_matches('/'), relative) {
        ("", "") => "/".into(),
        (root, "") => root.into(),
        (root, relative) => format!("{root}/{relative}"),
    }
}

/// Whether `path` is `folder` or inside it.
pub(crate) fn within(path: &str, folder: &str) -> bool {
    path == folder
        || path
            .strip_prefix(folder.trim_end_matches('/'))
            .is_some_and(|rest| rest.starts_with('/'))
}

/// What a rolled-out folder shows under it.
pub(crate) enum Children<'a> {
    Loading,
    Failed(&'a str),
    Ready(&'a [Node], bool),
}

/// One line of the tree as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    Node {
        node: Node,
        depth: usize,
        open: bool,
    },
    Loading {
        depth: usize,
        path: String,
    },
    Failed {
        depth: usize,
        path: String,
        error: String,
    },
    More {
        depth: usize,
        path: String,
        count: usize,
    },
    /// The computer listed only part of the folder.
    Partial {
        depth: usize,
        path: String,
    },
}

impl Row {
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Node { node, .. } => format!("node:{}", node.path),
            Self::Loading { path, .. } => format!("loading:{path}"),
            Self::Failed { path, .. } => format!("failed:{path}"),
            Self::More { path, .. } => format!("more:{path}"),
            Self::Partial { path, .. } => format!("partial:{path}"),
        }
    }
}

#[derive(Default)]
pub(crate) struct ProjectBrowser {
    /// Where the tree starts, the last folder of the path above it.
    pub root: Option<String>,
    /// Folders inside a folder, from the helper.
    folders: HashMap<String, Load<(Vec<Node>, bool)>>,
    /// Every folder and file in a project.
    projects: HashMap<String, Load<Files>>,
    pub open: HashSet<String>,
    /// The folder a new session would start in.
    pub picked: Option<String>,
    pub show_hidden: bool,
    /// Rolled-out folders showing everything in them.
    pub all: HashSet<String>,
    generation: u64,
}

impl ProjectBrowser {
    pub fn clear(&mut self) {
        *self = Self {
            generation: self.generation + 1,
            ..Self::default()
        };
    }

    /// For the previews: a folder that could not be read.
    pub fn fail(&mut self, path: &str, error: &str) {
        self.folders.insert(path.into(), Load::Failed(error.into()));
    }

    /// For the previews: a folder still being read.
    pub fn wait(&mut self, path: &str) {
        self.folders.insert(path.into(), Load::Loading);
    }

    pub fn children(&self, path: &str) -> Option<Children<'_>> {
        let mut loading = false;
        for (root, files) in &self.projects {
            if !within(path, root) {
                continue;
            }
            match files {
                Load::Ready(files) => {
                    return Some(Children::Ready(
                        files.children.get(path).map_or(&[][..], Vec::as_slice),
                        files.truncated && path == root,
                    ));
                }
                Load::Loading => loading = true,
                Load::Failed(_) => {}
            }
        }
        match self.folders.get(path) {
            Some(Load::Ready((nodes, truncated))) => Some(Children::Ready(nodes, *truncated)),
            Some(Load::Loading) => Some(Children::Loading),
            _ if loading => Some(Children::Loading),
            Some(Load::Failed(error)) => Some(Children::Failed(error)),
            None => self.projects.iter().find_map(|(root, files)| match files {
                Load::Failed(error) if path == root => Some(Children::Failed(error.as_str())),
                _ => None,
            }),
        }
    }

    /// Whether a project's file channel already lists what is in `path`.
    fn listed(&self, path: &str) -> bool {
        self.projects
            .iter()
            .any(|(root, files)| within(path, root) && !matches!(files, Load::Failed(_)))
    }

    /// The tree as drawn: rolled-out folders and what is in them.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if let Some(root) = &self.root {
            self.push_rows(root, 0, &mut rows);
        }
        rows
    }

    fn push_rows(&self, path: &str, depth: usize, rows: &mut Vec<Row>) {
        match self.children(path) {
            None | Some(Children::Loading) => rows.push(Row::Loading {
                depth,
                path: path.to_owned(),
            }),
            Some(Children::Failed(error)) => rows.push(Row::Failed {
                depth,
                path: path.to_owned(),
                error: error.to_owned(),
            }),
            Some(Children::Ready(nodes, truncated)) => {
                let visible: Vec<&Node> = nodes
                    .iter()
                    .filter(|node| self.show_hidden || !node.hidden())
                    .collect();
                let shown = if self.all.contains(path) {
                    visible.len()
                } else {
                    visible.len().min(SHOWN)
                };
                for node in &visible[..shown] {
                    let open = node.folder && self.open.contains(&node.path);
                    rows.push(Row::Node {
                        node: (*node).clone(),
                        depth,
                        open,
                    });
                    if open {
                        self.push_rows(&node.path, depth + 1, rows);
                    }
                }
                if shown < visible.len() {
                    rows.push(Row::More {
                        depth,
                        path: path.to_owned(),
                        count: visible.len() - shown,
                    });
                }
                if truncated {
                    rows.push(Row::Partial {
                        depth,
                        path: path.to_owned(),
                    });
                }
            }
        }
    }

    /// Folders and files already listed under the root whose names have
    /// `query` in them: names starting with it first, then folders, then the
    /// shallowest.
    pub fn find(&self, query: &str) -> Vec<Node> {
        let query = query.trim().to_lowercase();
        let Some(root) = self.root.as_ref().filter(|_| !query.is_empty()) else {
            return Vec::new();
        };
        let listed = self
            .projects
            .values()
            .filter_map(|files| match files {
                Load::Ready(files) => Some(files.children.values()),
                _ => None,
            })
            .flatten()
            .chain(self.folders.values().filter_map(|folder| match folder {
                Load::Ready((nodes, _)) => Some(nodes),
                _ => None,
            }));
        let mut seen = HashSet::new();
        let mut found: Vec<(bool, &Node)> = listed
            .flatten()
            .filter(|node| within(&node.path, root) && node.path != *root)
            .filter(|node| {
                self.show_hidden || !node.path.split('/').any(|part| part.starts_with('.'))
            })
            .filter_map(|node| {
                let name = node.name.to_lowercase();
                name.contains(&query)
                    .then(|| (name.starts_with(&query), node))
            })
            .filter(|(_, node)| seen.insert(node.path.clone()))
            .collect();
        found.sort_by(|(a_start, a), (b_start, b)| {
            b_start
                .cmp(a_start)
                .then_with(|| b.folder.cmp(&a.folder))
                .then_with(|| {
                    a.path
                        .matches('/')
                        .count()
                        .cmp(&b.path.matches('/').count())
                })
                .then_with(|| a.path.cmp(&b.path))
        });
        found
            .into_iter()
            .take(FOUND)
            .map(|(_, node)| node.clone())
            .collect()
    }
}

const EDIT_LIMIT: usize = 128 * 1024;
const MAX_FILE_TABS: usize = 8;
const HELPER_READ_ONLY: &str =
    "Update the computer's Pi helper to edit files. Browsing remains available.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingSave {
    pub normalized: String,
    pub wire: String,
    pub revision: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum FileSaveState {
    #[default]
    Idle,
    Saving,
    Failed(String),
    /// The helper may or may not have committed the request. Saving again is
    /// deliberately disabled until a fresh read resolves it.
    Uncertain(String),
    /// A newer remote document is retained while the local draft stays intact.
    Conflict(String),
}

enum SaveAttempt {
    Saved(Document),
    Failed {
        message: String,
        current: Result<Document, String>,
    },
}

/// One retained source buffer. Moving it between active and parked tabs keeps
/// IME selection, scroll position, undo-adjacent composition state and draft.
pub(crate) struct FileView {
    /// The project it is in, on the computer.
    pub root: String,
    /// From the project's root.
    pub path: String,
    pub size: Option<u64>,
    pub text: Load<Document>,
    pub editor: Entity<TextArea>,
    /// Normalized LF text at the current optimistic revision.
    pub saved_text: String,
    pub line_ending: LineEnding,
    pub editable: bool,
    pub read_only_reason: Option<String>,
    pub syntax: Option<SyntaxDocument>,
    pub save: FileSaveState,
    pub pending: Option<PendingSave>,
    pub conflict: Option<Document>,
    pub request: u64,
    pub _subscription: Subscription,
}

impl FileView {
    pub fn name(&self) -> &str {
        file_name(&self.path)
    }

    pub fn dirty(&self, cx: &App) -> bool {
        matches!(self.text, Load::Ready(_)) && self.editor.read(cx).text() != self.saved_text
    }

    pub fn revision(&self) -> Option<&str> {
        match &self.text {
            Load::Ready(document) => Some(&document.revision),
            _ => None,
        }
    }

    pub fn wire_text(&self, normalized: &str) -> String {
        match self.line_ending {
            LineEnding::Lf => normalized.to_owned(),
            LineEnding::CrLf => normalized.replace('\n', "\r\n"),
        }
    }

    pub fn saving_blocked(&self) -> bool {
        matches!(
            self.save,
            FileSaveState::Saving | FileSaveState::Uncertain(_) | FileSaveState::Conflict(_)
        )
    }
}

fn normalized_document(text: &str) -> Result<(String, LineEnding), &'static str> {
    let bytes = text.as_bytes();
    let mut crlf = false;
    let mut bare_lf = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                crlf = true;
                index += 2;
            }
            b'\r' => {
                return Err(
                    "This file uses mixed or legacy line endings, so it is read-only on mobile.",
                );
            }
            b'\n' => {
                bare_lf = true;
                index += 1;
            }
            _ => index += 1,
        }
    }
    if crlf && bare_lf {
        return Err("This file uses mixed or legacy line endings, so it is read-only on mobile.");
    }
    if crlf {
        Ok((text.replace("\r\n", "\n"), LineEnding::CrLf))
    } else {
        Ok((text.to_owned(), LineEnding::Lf))
    }
}

fn finish_saved(view: &mut FileView, pending: PendingSave, document: Document) {
    // A success can arrive after more typing. Advance only the saved baseline
    // and revision; never replace the editor's newer contents.
    view.saved_text = pending.normalized;
    view.text = Load::Ready(document);
    view.save = FileSaveState::Idle;
    view.pending = None;
    view.conflict = None;
}

pub(crate) fn reconcile_save(
    view: &mut FileView,
    pending: PendingSave,
    current: Document,
    failure: String,
    cx: &App,
) {
    if current.text == pending.wire {
        finish_saved(view, pending, current);
        return;
    }
    if current.revision == pending.revision {
        let clean = if let Ok((remote_text, ending)) = normalized_document(&current.text) {
            let clean = view.editor.read(cx).text() == remote_text;
            view.saved_text = remote_text;
            view.line_ending = ending;
            clean
        } else {
            false
        };
        view.text = Load::Ready(current);
        view.save = if clean {
            FileSaveState::Idle
        } else {
            FileSaveState::Failed(failure)
        };
        view.pending = None;
        view.conflict = None;
        return;
    }
    view.save = FileSaveState::Conflict(format!(
        "The file changed on the computer while this draft was open. {failure}"
    ));
    view.conflict = Some(current);
    view.pending = None;
}

fn install_document(
    view: &mut FileView,
    document: Document,
    helper_allows_editing: bool,
    cx: &mut Context<PhoneApp>,
) {
    let actual_size = document.text.len();
    let normalized = normalized_document(&document.text);
    let (mut shown, line_ending, ending_error) = match normalized {
        Ok((text, ending)) => (text, ending, None),
        Err(error) => (
            document.text.replace("\r\n", "\n").replace('\r', "\n"),
            LineEnding::Lf,
            Some(error),
        ),
    };
    if actual_size > EDIT_LIMIT {
        let mut end = EDIT_LIMIT.min(shown.len());
        while !shown.is_char_boundary(end) {
            end -= 1;
        }
        shown.truncate(end);
    }
    let reason = ending_error
        .map(str::to_owned)
        .or_else(|| {
            (actual_size > EDIT_LIMIT).then(|| {
                format!(
                    "Files larger than {} are read-only on mobile; this shows the first {}.",
                    size_label(EDIT_LIMIT as u64),
                    size_label(EDIT_LIMIT as u64)
                )
            })
        })
        .or_else(|| (!helper_allows_editing).then(|| HELPER_READ_ONLY.into()));
    view.size = Some(actual_size as u64);
    view.saved_text = shown.clone();
    view.line_ending = line_ending;
    view.editable = reason.is_none();
    view.read_only_reason = reason;
    view.save = FileSaveState::Idle;
    view.pending = None;
    view.conflict = None;
    view.text = Load::Ready(document);
    let syntax = view
        .syntax
        .as_mut()
        .map(|syntax| {
            syntax.set_source(shown.clone());
            syntax
                .highlights(theme(cx).syntax_palette())
                .into_iter()
                .filter_map(|(range, style)| style.color.map(|color| (range, color)))
                .collect()
        })
        .unwrap_or_default();
    let editable = view.editable;
    let line_count = shown.bytes().filter(|byte| *byte == b'\n').count() + 1;
    view.editor.update(cx, |editor, cx| {
        editor.set_read_only(!editable, cx);
        editor.set_max_lines(line_count);
        editor.set_text_at_start(shown, cx);
        editor.set_syntax(syntax, cx);
    });
}

impl PhoneApp {
    /// A reconnect can replace the helper without replacing retained buffers.
    /// Refresh only the capability-derived restriction; size and line-ending
    /// restrictions remain attached to the document that was actually read.
    pub(crate) fn refresh_file_editing_capability(
        &mut self,
        helper_allows_editing: bool,
        cx: &mut Context<Self>,
    ) {
        for view in self.file_view.iter_mut().chain(self.file_tabs.iter_mut()) {
            if !matches!(view.text, Load::Ready(_)) {
                continue;
            }
            let helper_restricted = view.read_only_reason.as_deref() == Some(HELPER_READ_ONLY);
            let editable = if helper_allows_editing {
                if helper_restricted {
                    view.read_only_reason = None;
                }
                view.read_only_reason.is_none()
            } else {
                if view.read_only_reason.is_none() || helper_restricted {
                    view.read_only_reason = Some(HELPER_READ_ONLY.into());
                }
                false
            };
            if view.editable != editable {
                view.editable = editable;
                view.editor
                    .update(cx, |editor, cx| editor.set_read_only(!editable, cx));
            }
        }
    }

    pub(crate) fn home_folder(&self) -> String {
        self.store
            .as_ref()
            .and_then(|store| store.live.as_ref())
            .map_or_else(|| SAMPLE_HOME.into(), |live| live.helper.home.clone())
    }

    /// A path as people read it: `~/repos/pi`.
    pub(crate) fn short_path(&self, path: &str) -> String {
        let home = self.home_folder();
        if within(path, &home) {
            format!("~{}", &path[home.len()..])
        } else {
            path.to_owned()
        }
    }

    /// A typed `~/repos` or `/opt`, on the computer.
    pub(crate) fn full_path(&self, path: &str) -> Option<String> {
        let path = path.trim().trim_end_matches('/');
        let home = self.home_folder();
        if path == "~" {
            Some(home)
        } else if let Some(rest) = path.strip_prefix("~/") {
            Some(format!("{home}/{rest}"))
        } else if path.is_empty() {
            None
        } else {
            path.starts_with('/').then(|| path.to_owned())
        }
        .map(|path| if path.is_empty() { "/".into() } else { path })
    }

    /// The tree starts at the folder the current project is in, with the
    /// project rolled out and picked; after pairing it starts at home, with
    /// the folders down to the project rolled out.
    pub(crate) fn open_project_browser(&mut self, at_home: bool, cx: &mut Context<Self>) {
        if !self.backend_supports("workspace_browse") {
            return;
        }
        if self.project_browser.root.is_some() {
            return;
        }
        let current = self
            .store
            .as_ref()
            .and_then(|store| store.projects.get(self.project))
            .and_then(|project| self.full_path(&project.path));
        let home = self.home_folder();
        let root = match current.as_deref() {
            Some(current) if at_home && within(current, &home) => home,
            Some(current) if !parent(current).is_empty() => parent(current).to_owned(),
            _ => home,
        };
        self.project_browser.picked = current.clone();
        self.go_to_folder(root.clone(), cx);
        let Some(current) = current else { return };
        if at_home {
            let mut folder = parent(&current);
            while within(folder, &root) && folder != root {
                self.project_browser.open.insert(folder.to_owned());
                self.load_folder(folder.to_owned(), cx);
                folder = parent(folder);
            }
        } else {
            self.project_browser.open.insert(current.clone());
            self.load_project_tree(current, cx);
        }
    }

    fn editor_browser_allows(&self, path: &str) -> bool {
        self.sheet != Some(Sheet::EditorFiles)
            || self
                .editor_root
                .as_ref()
                .is_some_and(|root| within(path, root))
    }

    /// Starts the tree at `path`.
    pub(crate) fn go_to_folder(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.backend_supports("workspace_browse") {
            return;
        }
        if !self.editor_browser_allows(&path) {
            self.notify_user("That path is outside the editor folder", cx);
            return;
        }
        self.project_browser.root = Some(path.clone());
        if !self.project_browser.listed(&path) {
            self.load_folder(path.clone(), cx);
        }
        let known = self.store.as_ref().is_some_and(|store| {
            store
                .projects
                .iter()
                .any(|project| self.full_path(&project.path).as_deref() == Some(path.as_str()))
        });
        if known {
            self.load_project_tree(path, cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle_folder(&mut self, node: &Node, cx: &mut Context<Self>) {
        if !self.backend_supports("workspace_browse") || !self.editor_browser_allows(&node.path) {
            return;
        }
        let browser = &mut self.project_browser;
        if !browser.open.remove(&node.path) {
            browser.open.insert(node.path.clone());
            if node.project && node.root.is_none() {
                self.load_project_tree(node.path.clone(), cx);
            } else if !self.project_browser.listed(&node.path) {
                self.load_folder(node.path.clone(), cx);
            }
        }
        cx.notify();
    }

    /// Picks `path` and rolls out every folder down to it.
    pub(crate) fn reveal_folder(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.backend_supports("workspace_browse") || !self.editor_browser_allows(path) {
            return;
        }
        let browser = &mut self.project_browser;
        if let Some(root) = browser.root.clone() {
            let mut folder = parent(path);
            while within(folder, &root) && folder != root {
                browser.open.insert(folder.to_owned());
                folder = parent(folder);
            }
        }
        browser.picked = Some(path.to_owned());
        cx.notify();
    }

    pub(crate) fn retry_folder(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.backend_supports("workspace_browse") || !self.editor_browser_allows(&path) {
            return;
        }
        let browser = &mut self.project_browser;
        let project = matches!(browser.projects.get(&path), Some(Load::Failed(_)));
        browser
            .projects
            .retain(|_, files| !matches!(files, Load::Failed(_)));
        browser.folders.remove(&path);
        if project {
            self.load_project_tree(path, cx);
        } else {
            self.load_folder(path, cx);
        }
    }

    fn load_folder(&mut self, path: String, cx: &mut Context<Self>) {
        if matches!(
            self.project_browser.folders.get(&path),
            Some(Load::Loading | Load::Ready(_))
        ) {
            return;
        }
        let Some(store) = &self.store else { return };
        let Some(live) = &store.live else {
            let listing = sample_folders(&path);
            self.project_browser
                .folders
                .insert(path, Load::Ready((listing, false)));
            return;
        };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let generation = self.project_browser.generation;
        self.project_browser
            .folders
            .insert(path.clone(), Load::Loading);
        cx.spawn(async move |this, cx| {
            // Hidden folders come too, so showing them needs no new request.
            let result = remote::directories(&connection, &helper, &path, true).await;
            this.update(cx, |this, cx| {
                let browser = &mut this.project_browser;
                if browser.generation != generation {
                    return;
                }
                let load = match result {
                    Ok(directory) => {
                        let mut nodes: Vec<Node> = directory
                            .entries
                            .into_iter()
                            .map(|folder| Node {
                                tag: folder
                                    .kind
                                    .or_else(|| folder.project.then(|| "project".into())),
                                name: folder.name,
                                path: folder.path,
                                folder: true,
                                project: folder.project,
                                size: None,
                                root: None,
                            })
                            .collect();
                        sort(&mut nodes);
                        Load::Ready((nodes, directory.truncated))
                    }
                    Err(error) => Load::Failed(format!("{error:#}")),
                };
                browser.folders.insert(path, load);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_project_tree(&mut self, root: String, cx: &mut Context<Self>) {
        if matches!(
            self.project_browser.projects.get(&root),
            Some(Load::Loading | Load::Ready(_))
        ) {
            return;
        }
        let Some(store) = &self.store else { return };
        let Some(live) = &store.live else {
            if let Some(tree) = sample_tree(&root) {
                self.project_browser
                    .projects
                    .insert(root.clone(), Load::Ready(Files::new(&root, tree)));
            } else {
                self.load_folder(root, cx);
            }
            return;
        };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let host = store.computer.address.clone();
        let generation = self.project_browser.generation;
        self.project_browser
            .projects
            .insert(root.clone(), Load::Loading);
        cx.spawn(async move |this, cx| {
            let result = remote::project_tree(&connection, &helper, &host, &root).await;
            this.update(cx, |this, cx| {
                if this.project_browser.generation != generation {
                    return;
                }
                let failed = result.is_err();
                let load = match result {
                    Ok(tree) => Load::Ready(Files::new(&root, tree)),
                    Err(error) => Load::Failed(format!("{error:#}")),
                };
                this.project_browser.projects.insert(root.clone(), load);
                // Its folders can still be listed the plain way.
                if failed {
                    this.load_folder(root, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn load_editor_tree(&mut self, root: String, cx: &mut Context<Self>) {
        if self.editor_root.as_deref() != Some(root.as_str()) {
            return;
        }
        self.project_browser.root = Some(root.clone());
        self.load_project_tree(root, cx);
    }

    pub(crate) fn use_picked_editor_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let browser = &self.project_browser;
        let Some(path) = browser.picked.clone().or_else(|| browser.root.clone()) else {
            return;
        };
        self.use_editor_folder(&path, window, cx);
    }

    fn enter_file_route(
        &mut self,
        return_to_project: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route() == Route::File {
            self.close_sheet(cx);
        } else {
            self.file_return_to_project = return_to_project;
            self.push(Route::File, window, cx);
        }
    }

    /// Resume the retained workspace when one already exists. Choosing a root
    /// is a first-open action, not something chat navigation should repeat.
    pub(crate) fn open_editor_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_root.is_none() {
            self.editor_root = self.file_view.as_ref().map(|view| view.root.clone());
        }
        if self.editor_root.is_some() {
            self.enter_file_route(false, window, cx);
        } else {
            self.open_sheet(Sheet::EditorRoot, cx);
        }
    }

    pub(crate) fn editor_workspace_needs_attention(&self, cx: &App) -> bool {
        self.file_view
            .iter()
            .chain(self.file_tabs.iter())
            .any(|view| view.dirty(cx) || !matches!(view.save, FileSaveState::Idle))
    }

    /// Opens a cancellable root picker without disturbing the current workspace.
    pub(crate) fn choose_another_editor_folder(&mut self, cx: &mut Context<Self>) {
        if self.editor_workspace_needs_attention(cx) {
            self.notify_user(
                "Save or resolve every open file before changing the editor folder",
                cx,
            );
            return;
        }
        self.open_sheet(Sheet::EditorRoot, cx);
    }

    /// Forgets a clean workspace. Drafts and uncertain saves must be resolved first.
    pub(crate) fn close_editor_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_workspace_needs_attention(cx) {
            self.notify_user(
                "Save or resolve every open file before closing the editor",
                cx,
            );
            return;
        }

        self.file_generation += 1;
        self.editor_root = None;
        self.file_view = None;
        self.file_tabs.clear();
        self.project_browser.clear();
        self.project_browser_editor = false;
        self.folder.update(cx, |area, cx| area.set_text("", cx));
        self.file_exit_armed = false;

        self.close_sheet(cx);
        if self.route() == Route::File && !self.back(window, cx) {
            self.routes = vec![Route::Sessions];
            window.dismiss_virtual_keyboard();
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub(crate) fn use_editor_folder(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.backend_supports("workspace_browse") {
            return;
        }
        let changing = self.editor_root.as_deref() != Some(path);
        if changing && self.editor_workspace_needs_attention(cx) {
            self.notify_user(
                "Save or resolve every open file before changing the editor folder",
                cx,
            );
            return;
        }
        if changing {
            self.file_generation += 1;
            self.file_view = None;
            self.file_tabs.clear();
        }
        let path = path.to_owned();
        self.editor_root = Some(path.clone());
        self.project_browser.clear();
        self.project_browser_editor = true;
        self.project_browser.root = Some(path.clone());
        self.project_browser.picked = None;
        self.folder.update(cx, |area, cx| area.set_text("", cx));
        self.load_editor_tree(path, cx);
        self.enter_file_route(false, window, cx);
        cx.notify();
    }

    /// Starts a session in the picked folder, or the one the tree starts at.
    pub(crate) fn use_picked_folder(&mut self, cx: &mut Context<Self>) {
        if !self.backend_supports("workspace_browse") {
            return;
        }
        let browser = &self.project_browser;
        let Some(path) = browser.picked.clone().or_else(|| browser.root.clone()) else {
            return;
        };
        self.use_folder(&path, cx);
    }

    pub(crate) fn use_folder(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.backend_supports("workspace_browse") {
            return;
        }
        let existing = self.store.as_ref().and_then(|store| {
            store
                .projects
                .iter()
                .position(|project| self.full_path(&project.path).as_deref() == Some(path))
        });
        let index = match (existing, self.store.as_mut()) {
            (Some(index), _) => index,
            (None, Some(store)) => store.add_project(path),
            (None, None) => return,
        };
        self.select_project(index, cx);
        if !matches!(self.route(), Route::Start | Route::ScheduleEdit) {
            self.routes = vec![Route::Sessions, Route::Start];
        }
    }

    pub(crate) fn open_file(&mut self, node: &Node, window: &mut Window, cx: &mut Context<Self>) {
        if !self.backend_supports("workspace_browse") {
            return;
        }
        let editor_drawer = self.sheet == Some(Sheet::EditorFiles);
        let Some(root) = (if editor_drawer {
            self.editor_root.clone()
        } else {
            node.root.clone()
        }) else {
            return;
        };
        if !within(&node.path, &root) {
            self.notify_user("That file is outside the selected folder", cx);
            return;
        }
        let Some(relative) = node.path.strip_prefix(root.trim_end_matches('/')) else {
            return;
        };
        let path = relative.trim_start_matches('/').to_owned();
        if path.is_empty() {
            return;
        }
        if self.editor_root.as_deref() != Some(root.as_str()) {
            if self.editor_workspace_needs_attention(cx) {
                self.notify_user(
                    "Save or resolve every open file before changing folders",
                    cx,
                );
                return;
            }
            self.editor_root = Some(root.clone());
            self.file_view = None;
            self.file_tabs.clear();
            self.file_exit_armed = false;
        }
        let return_to_project = self.sheet == Some(Sheet::Project);

        // A tab is a retained editor entity, not a reconstructed string view.
        if self
            .file_view
            .as_ref()
            .is_some_and(|view| view.root == root && view.path == path)
        {
            self.enter_file_route(return_to_project, window, cx);
            return;
        }
        if let Some(index) = self
            .file_tabs
            .iter()
            .position(|view| view.root == root && view.path == path)
        {
            let selected = self.file_tabs.remove(index);
            if let Some(active) = self.file_view.replace(selected) {
                self.file_tabs.push(active);
            }
            self.file_exit_armed = false;
            self.enter_file_route(return_to_project, window, cx);
            cx.notify();
            return;
        }

        if self.file_tabs.len() + usize::from(self.file_view.is_some()) >= MAX_FILE_TABS {
            if let Some(index) = self
                .file_tabs
                .iter()
                .position(|view| !view.dirty(cx) && matches!(view.save, FileSaveState::Idle))
            {
                self.file_tabs.remove(index);
            } else if self
                .file_view
                .as_ref()
                .is_some_and(|view| !view.dirty(cx) && matches!(view.save, FileSaveState::Idle))
            {
                self.file_view = None;
            } else {
                self.notify_user(
                    "Save one of the eight open file drafts before opening another file.",
                    cx,
                );
                return;
            }
        }

        self.file_generation += 1;
        let request = self.file_generation;
        let editor = cx.new(|cx| {
            let mut editor = TextArea::code(10_000, cx);
            editor.set_read_only(true, cx);
            editor
        });
        let watched_root = root.clone();
        let watched_path = path.clone();
        let subscription = cx.subscribe(&editor, move |this, _, event: &TextAreaEvent, cx| {
            this.file_editor_event(&watched_root, &watched_path, event, cx)
        });
        let view = FileView {
            root: root.clone(),
            path: path.clone(),
            size: node.size,
            text: Load::Loading,
            editor,
            saved_text: String::new(),
            line_ending: LineEnding::Lf,
            editable: false,
            read_only_reason: None,
            syntax: language_for_path(&path).and_then(SyntaxDocument::new),
            save: FileSaveState::Idle,
            pending: None,
            conflict: None,
            request,
            _subscription: subscription,
        };
        if let Some(active) = self.file_view.replace(view) {
            self.file_tabs.push(active);
        }
        self.file_exit_armed = false;
        self.enter_file_route(return_to_project, window, cx);
        cx.notify();

        let Some(store) = &self.store else { return };
        let Some(live) = &store.live else {
            let document = Document {
                path: path.clone(),
                text: sample_text(&path),
                revision: "sample-1".into(),
            };
            if let Some(view) = self.file_buffer_mut(&root, &path, request) {
                install_document(view, document, true, cx);
            }
            cx.notify();
            return;
        };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let editable = helper.can_edit_files();
        let host = store.computer.address.clone();
        cx.spawn(async move |this, cx| {
            let result = remote::read_file(&connection, &helper, &host, &root, &path).await;
            this.update(cx, |this, cx| {
                let Some(view) = this.file_buffer_mut(&root, &path, request) else {
                    return;
                };
                match result {
                    Ok(document) => install_document(view, document, editable, cx),
                    Err(error) => view.text = Load::Failed(format!("{error:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn file_editor_event(
        &mut self,
        root: &str,
        path: &str,
        event: &TextAreaEvent,
        cx: &mut Context<Self>,
    ) {
        self.file_exit_armed = false;
        let view = self
            .file_view
            .as_mut()
            .filter(|view| view.root == root && view.path == path)
            .or_else(|| {
                self.file_tabs
                    .iter_mut()
                    .find(|view| view.root == root && view.path == path)
            });
        if let Some(view) = view {
            if matches!(event, TextAreaEvent::Changed(_))
                && matches!(view.save, FileSaveState::Failed(_))
            {
                view.save = FileSaveState::Idle;
            }
            if let TextAreaEvent::Changed(edit) = event {
                let source = view.editor.read(cx).text().to_owned();
                let syntax = view
                    .syntax
                    .as_mut()
                    .map(|syntax| {
                        syntax.update_source(
                            source,
                            SyntaxEdit::new(edit.old_range.clone(), edit.new_range.clone()),
                        );
                        syntax
                            .highlights(theme(cx).syntax_palette())
                            .into_iter()
                            .filter_map(|(range, style)| style.color.map(|color| (range, color)))
                            .collect()
                    })
                    .unwrap_or_default();
                view.editor
                    .update(cx, |editor, cx| editor.set_syntax(syntax, cx));
            }
        }
        cx.notify();
    }

    fn file_buffer_mut(&mut self, root: &str, path: &str, request: u64) -> Option<&mut FileView> {
        if self
            .file_view
            .as_ref()
            .is_some_and(|view| view.root == root && view.path == path && view.request == request)
        {
            return self.file_view.as_mut();
        }
        self.file_tabs
            .iter_mut()
            .find(|view| view.root == root && view.path == path && view.request == request)
    }

    pub(crate) fn recolor_file_editors(&mut self, cx: &mut Context<Self>) {
        let palette = theme(cx).syntax_palette();
        for view in self.file_view.iter_mut().chain(self.file_tabs.iter_mut()) {
            let spans = view
                .syntax
                .as_ref()
                .map(|syntax| {
                    syntax
                        .highlights(palette)
                        .into_iter()
                        .filter_map(|(range, style)| style.color.map(|color| (range, color)))
                        .collect()
                })
                .unwrap_or_default();
            view.editor
                .update(cx, |editor, cx| editor.set_syntax(spans, cx));
        }
    }

    pub(crate) fn activate_file_tab(&mut self, root: &str, path: &str, cx: &mut Context<Self>) {
        let Some(index) = self
            .file_tabs
            .iter()
            .position(|view| view.root == root && view.path == path)
        else {
            return;
        };
        let selected = self.file_tabs.remove(index);
        if let Some(active) = self.file_view.replace(selected) {
            self.file_tabs.push(active);
        }
        self.file_exit_armed = false;
        cx.notify();
    }

    pub(crate) fn retry_active_file(&mut self, cx: &mut Context<Self>) {
        let Some(view) = &mut self.file_view else {
            return;
        };
        if !matches!(view.text, Load::Failed(_)) {
            return;
        }
        self.file_generation += 1;
        view.request = self.file_generation;
        let (root, path, request) = (view.root.clone(), view.path.clone(), view.request);
        view.text = Load::Loading;
        view.read_only_reason = None;
        view.editor
            .update(cx, |editor, cx| editor.set_read_only(true, cx));

        let Some(store) = &self.store else { return };
        let Some(live) = &store.live else {
            install_document(
                view,
                Document {
                    path: path.clone(),
                    text: sample_text(&path),
                    revision: "sample-retry".into(),
                },
                true,
                cx,
            );
            cx.notify();
            return;
        };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let editable = helper.can_edit_files();
        let host = store.computer.address.clone();
        cx.spawn(async move |this, cx| {
            let result = remote::read_file(&connection, &helper, &host, &root, &path).await;
            this.update(cx, |this, cx| {
                let Some(view) = this.file_buffer_mut(&root, &path, request) else {
                    return;
                };
                match result {
                    Ok(document) => install_document(view, document, editable, cx),
                    Err(error) => view.text = Load::Failed(format!("{error:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn open_file_recoveries(&mut self, cx: &mut Context<Self>) {
        self.file_recovery_delete_armed = None;
        self.file_recoveries = FileRecoveryState::Loading;
        self.open_sheet(Sheet::FileRecoveries, cx);
        let Some(root) = self.editor_root.clone() else {
            self.file_recoveries =
                FileRecoveryState::Failed("Choose an editor folder first.".into());
            return;
        };
        let Some(store) = &self.store else {
            self.file_recoveries = FileRecoveryState::Failed(
                "Recovery records are available for remote projects.".into(),
            );
            return;
        };
        let Some(live) = &store.live else {
            self.file_recoveries = FileRecoveryState::Failed(
                "Recovery records are unavailable in this preview.".into(),
            );
            return;
        };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let host = store.computer.address.clone();
        cx.spawn(async move |this, cx| {
            let result = remote::file_recoveries(&connection, &helper, &host, &root).await;
            this.update(cx, |this, cx| {
                if this.editor_root.as_deref() != Some(root.as_str()) {
                    return;
                }
                self::set_file_recoveries(this, result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn read_file_recovery(
        &mut self,
        id: String,
        artifact: RecoveryArtifact,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.editor_root.clone() else {
            return;
        };
        let Some(store) = &self.store else { return };
        let Some(live) = &store.live else { return };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let host = store.computer.address.clone();
        self.notify_user("Reading recovery artifact…", cx);
        cx.spawn(async move |this, cx| {
            let result =
                remote::read_file_recovery(&connection, &helper, &host, &root, id, artifact).await;
            this.update(cx, |this, cx| {
                if this.editor_root.as_deref() != Some(root.as_str()) {
                    return;
                }
                match result {
                    Ok(document) => {
                        this.selectable
                            .update(cx, |area, cx| area.show_selected(document.text, cx));
                        this.open_sheet(Sheet::SelectText, cx);
                        cx.notify();
                    }
                    Err(error) => {
                        this.notify_user(format!("Could not read recovery: {error:#}"), cx)
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn delete_file_recovery(&mut self, id: String, cx: &mut Context<Self>) {
        if self.file_recovery_delete_armed.as_deref() != Some(&id) {
            self.file_recovery_delete_armed = Some(id);
            cx.notify();
            return;
        }
        self.file_recovery_delete_armed = None;
        let Some(root) = self.editor_root.clone() else {
            return;
        };
        let Some(store) = &self.store else { return };
        let Some(live) = &store.live else { return };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let host = store.computer.address.clone();
        self.file_recoveries = FileRecoveryState::Loading;
        cx.spawn(async move |this, cx| {
            let result = async {
                remote::delete_file_recovery(&connection, &helper, &host, &root, id).await?;
                remote::file_recoveries(&connection, &helper, &host, &root).await
            }
            .await;
            this.update(cx, |this, cx| {
                if this.editor_root.as_deref() != Some(root.as_str()) {
                    return;
                }
                set_file_recoveries(this, result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn save_active_file(&mut self, cx: &mut Context<Self>) {
        let Some(view) = &mut self.file_view else {
            return;
        };
        if !view.editable || view.saving_blocked() {
            return;
        }
        let normalized = view.editor.read(cx).text().to_owned();
        if normalized == view.saved_text {
            view.save = FileSaveState::Idle;
            cx.notify();
            return;
        }
        let Some(revision) = view.revision().map(str::to_owned) else {
            return;
        };
        let wire = view.wire_text(&normalized);
        if wire.len() > EDIT_LIMIT {
            view.save = FileSaveState::Failed(format!(
                "This draft is larger than the {} mobile editing limit. Shorten it before saving.",
                size_label(EDIT_LIMIT as u64)
            ));
            view.pending = None;
            cx.notify();
            return;
        }
        let pending = PendingSave {
            wire,
            normalized,
            revision,
        };
        let root = view.root.clone();
        let path = view.path.clone();
        let request = view.request;

        let Some(store) = &self.store else { return };
        let Some(live) = &store.live else {
            let revision = format!("sample-{}", self.file_generation.saturating_add(1));
            let document = Document {
                path: path.clone(),
                text: pending.wire.clone(),
                revision,
            };
            finish_saved(view, pending, document);
            cx.notify();
            return;
        };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let host = store.computer.address.clone();
        view.pending = Some(pending.clone());
        view.conflict = None;
        view.save = FileSaveState::Saving;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let saved = remote::save_file(
                &connection,
                &helper,
                &host,
                &root,
                &path,
                pending.wire.clone(),
                pending.revision.clone(),
            )
            .await;
            let attempt = match saved {
                Ok(document) => SaveAttempt::Saved(document),
                Err(error) => {
                    let message = format!("{error:#}");
                    let current = remote::read_file(&connection, &helper, &host, &root, &path)
                        .await
                        .map_err(|error| format!("{error:#}"));
                    SaveAttempt::Failed { message, current }
                }
            };
            this.update(cx, |this, cx| {
                let Some(view) = this.file_buffer_mut(&root, &path, request) else {
                    return;
                };
                match attempt {
                    SaveAttempt::Saved(document) => finish_saved(view, pending, document),
                    SaveAttempt::Failed { message, current } => match current {
                        Ok(current) => reconcile_save(view, pending, current, message, cx),
                        Err(check) => {
                            view.save = FileSaveState::Uncertain(format!(
                                "Pi could not confirm whether this save reached the computer: {message}. Check failed: {check}"
                            ));
                            view.pending = Some(pending);
                        }
                    },
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Explicitly checks an uncertain save. It only reads; it never replays the
    /// write that may already have reached the computer.
    pub(crate) fn check_active_file_save(&mut self, cx: &mut Context<Self>) {
        let Some(view) = &mut self.file_view else {
            return;
        };
        if !matches!(view.save, FileSaveState::Uncertain(_)) {
            return;
        }
        let Some(pending) = view.pending.clone() else {
            return;
        };
        let (root, path, request) = (view.root.clone(), view.path.clone(), view.request);
        let Some(store) = &self.store else { return };
        let Some(live) = &store.live else { return };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let host = store.computer.address.clone();
        view.save = FileSaveState::Saving;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let current = remote::read_file(&connection, &helper, &host, &root, &path)
                .await
                .map_err(|error| format!("{error:#}"));
            this.update(cx, |this, cx| {
                let Some(view) = this.file_buffer_mut(&root, &path, request) else {
                    return;
                };
                match current {
                    Ok(current) => reconcile_save(
                        view,
                        pending,
                        current,
                        "The original save did not complete cleanly".into(),
                        cx,
                    ),
                    Err(error) => {
                        view.save = FileSaveState::Uncertain(format!(
                            "Pi still cannot confirm the save: {error}"
                        ));
                        view.pending = Some(pending);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn reload_conflicted_file(&mut self, cx: &mut Context<Self>) {
        let Some(view) = &mut self.file_view else {
            return;
        };
        let Some(document) = view.conflict.take() else {
            return;
        };
        install_document(view, document, true, cx);
        self.file_exit_armed = false;
        cx.notify();
    }

    /// Keeps the local draft while adopting the newly read revision. The user
    /// must still tap Save, making the overwrite explicit.
    pub(crate) fn keep_local_file(&mut self, cx: &mut Context<Self>) {
        let Some(view) = &mut self.file_view else {
            return;
        };
        let Some(document) = view.conflict.take() else {
            return;
        };
        if let Ok((remote_text, ending)) = normalized_document(&document.text) {
            view.saved_text = remote_text;
            view.line_ending = ending;
        }
        view.text = Load::Ready(document);
        view.save = FileSaveState::Idle;
        view.pending = None;
        self.file_exit_armed = false;
        cx.notify();
    }

    pub(crate) fn expand_file_selection(&mut self, cx: &mut Context<Self>) {
        let Some(view) = &mut self.file_view else {
            return;
        };
        let Some(syntax) = &view.syntax else { return };
        let selected = view.editor.read(cx).selection();
        let expanded = syntax.expand_selection(selected);
        if let Some(expanded) = expanded {
            view.editor
                .update(cx, |editor, cx| editor.select_range(expanded, cx));
        }
    }

    pub(crate) fn select_project(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.backend_supports("workspace_browse") {
            return;
        }
        if self
            .store
            .as_ref()
            .is_none_or(|store| index >= store.projects.len())
        {
            return;
        }
        self.project = index;
        if self.route() == Route::ScheduleEdit
            && let Some(draft) = &mut self.schedules.draft
            && let Some(project) = self
                .store
                .as_ref()
                .and_then(|store| store.projects.get(index))
        {
            draft
                .cwd
                .update(cx, |area, cx| area.set_text(&project.path, cx));
        }
        if let Some((address, path, sample)) = self.store.as_ref().map(|store| {
            (
                store.computer.address.clone(),
                store.projects[index].path.clone(),
                store.is_sample(),
            )
        }) {
            if let Some(live) = self.store.as_mut().and_then(|store| store.live.as_mut()) {
                live.refresh_commands(Some(path.clone()));
            }
            if !sample {
                self.update_prefs(cx, |prefs| {
                    prefs.projects.insert(address, path);
                });
            }
        }
        self.load_project_files(index, cx);
        if self.route() == Route::Projects {
            self.routes = vec![Route::Sessions, Route::Start];
        }
        self.close_sheet(cx);
        cx.notify();
    }

    pub(crate) fn load_project_files(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.backend_supports("workspace_browse") {
            return;
        }
        self.project_files_generation += 1;
        let generation = self.project_files_generation;
        let Some(store) = &self.store else { return };
        if store.is_sample() {
            self.start.update(cx, |start, _| start.use_files(None));
            return;
        }
        let (Some(project), Some(live)) = (store.projects.get(index), store.live.as_ref()) else {
            self.start
                .update(cx, |start, _| start.use_files(Some(Vec::new())));
            return;
        };
        let path = project.path.clone();
        let host = store.computer.address.clone();
        let connection = live.connection.clone();
        let helper = live.helper.clone();
        self.start.update(cx, |start, _| start.load_files());
        cx.spawn(async move |this, cx| {
            let result = remote::project_files(&connection, &helper, &host, &path)
                .await
                .map_err(|error| format!("{error:#}"));
            this.update(cx, |this, cx| {
                if this.project_files_generation != generation
                    || this
                        .store
                        .as_ref()
                        .and_then(|store| store.projects.get(this.project))
                        .is_none_or(|project| project.path != path)
                {
                    return;
                }
                this.start.update(cx, |start, _| match result {
                    Ok(files) => start.use_files(Some(files)),
                    Err(error) => start.fail_files(error),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

// The sample computer: a home folder with a few projects, so the browser
// can be tried without one.

pub(crate) const SAMPLE_HOME: &str = "/Users/nick";

fn sample_folders(path: &str) -> Vec<Node> {
    let folders: &[(&str, Option<&str>)] = match path.strip_prefix(SAMPLE_HOME) {
        Some("") => &[
            (".config", None),
            ("Desktop", None),
            ("Documents", None),
            ("Downloads", None),
            ("repos", None),
        ],
        Some("/repos") => &[
            ("dotfiles", Some("git")),
            ("minivm", Some("git")),
            ("pi", Some("git")),
            ("scratch", None),
            ("zed", Some("git")),
        ],
        _ => &[],
    };
    let mut nodes: Vec<Node> = folders
        .iter()
        .map(|(name, kind)| Node {
            name: (*name).into(),
            path: join(path, name),
            folder: true,
            project: kind.is_some(),
            tag: kind.map(str::to_owned),
            size: None,
            root: None,
        })
        .collect();
    sort(&mut nodes);
    nodes
}

/// Paths in the sample projects; folders end in `/`, files carry a size.
fn sample_tree(root: &str) -> Option<pi_core::remote_files::Tree> {
    let paths: &[(&str, u64)] = match root.strip_prefix(SAMPLE_HOME)? {
        "/repos/pi" => &[
            (".gitignore", 120),
            ("README.md", 4_100),
            ("apps/", 0),
            ("apps/desktop/", 0),
            ("apps/desktop/main.ts", 3_200),
            ("apps/phone/", 0),
            ("apps/phone/App.tsx", 5_400),
            ("packages/", 0),
            ("packages/ai/", 0),
            ("packages/ai/package.json", 900),
            ("packages/ai/src/", 0),
            ("packages/ai/src/provider-registry.ts", 3_800),
            ("packages/ai/src/providers/", 0),
            ("packages/ai/src/providers/anthropic.ts", 7_900),
            ("packages/ai/src/providers/azure.ts", 2_100),
            ("packages/ai/src/providers/bedrock.ts", 4_400),
            ("packages/ai/src/providers/cerebras.ts", 1_300),
            ("packages/ai/src/providers/deepseek.ts", 1_500),
            ("packages/ai/src/providers/google.ts", 6_200),
            ("packages/ai/src/providers/groq.ts", 1_200),
            ("packages/ai/src/providers/mistral.ts", 1_800),
            ("packages/ai/src/providers/ollama.ts", 2_600),
            ("packages/ai/src/providers/openai.ts", 8_300),
            ("packages/ai/src/providers/openrouter.ts", 2_200),
            ("packages/ai/src/providers/qwen.ts", 1_700),
            ("packages/ai/src/providers/vertex.ts", 3_100),
            ("packages/ai/src/providers/xai.ts", 1_400),
            ("packages/ai/src/retry.ts", 2_000),
            ("packages/ai/src/stream.ts", 6_100),
            ("packages/ai/test/", 0),
            ("packages/ai/test/providers.test.ts", 4_800),
            ("packages/ai/test/retry.test.ts", 2_300),
            ("packages/coding-agent/", 0),
            ("packages/coding-agent/package.json", 1_100),
            ("packages/coding-agent/src/", 0),
            ("packages/coding-agent/src/agent.ts", 9_700),
        ],
        "/repos/zed" => &[
            ("Cargo.toml", 6_000),
            ("README.md", 3_300),
            ("crates/", 0),
            ("crates/editor/", 0),
            ("crates/editor/Cargo.toml", 1_900),
            ("crates/editor/src/", 0),
            ("crates/editor/src/editor.rs", 980_000),
        ],
        "/repos/minivm" => &[
            ("Cargo.toml", 400),
            ("README.md", 1_200),
            ("src/", 0),
            ("src/main.rs", 5_500),
            ("src/vm.rs", 12_000),
        ],
        "/repos/dotfiles" => &[(".zshrc", 2_400), ("README.md", 600)],
        _ => return None,
    };
    Some(pi_core::remote_files::Tree {
        entries: paths
            .iter()
            .map(|(path, size)| pi_core::remote_files::Entry {
                path: path.trim_end_matches('/').into(),
                directory: path.ends_with('/'),
                size: (!path.ends_with('/')).then_some(*size),
            })
            .collect(),
        truncated: false,
    })
}

fn sample_text(path: &str) -> String {
    if path.ends_with("retry.ts") {
        return SAMPLE_RETRY.into();
    }
    let comment = if path.ends_with(".md") { "#" } else { "//" };
    let mut text = format!("{comment} {path}\n\n");
    for line in 1..=40 {
        text.push_str(&format!("export const step{line} = () => {line};\n"));
    }
    text
}

const SAMPLE_RETRY: &str = r#"import { sleep } from "./time";

export interface RetryOptions {
  attempts: number;
  baseDelayMs: number;
  maxDelayMs: number;
}

export async function withRetry<T>(
  run: () => Promise<T>,
  { attempts, baseDelayMs, maxDelayMs }: RetryOptions,
) {
  for (let attempt = 0; ; attempt++) {
    try {
      return await run();
    } catch (error) {
      if (attempt + 1 >= attempts) throw error;
      const delay = Math.min(
        maxDelayMs,
        baseDelayMs * 2 ** attempt,
      );
      await sleep(delay);
    }
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn browser() -> ProjectBrowser {
        let mut browser = ProjectBrowser {
            root: Some("/Users/nick/repos".into()),
            ..ProjectBrowser::default()
        };
        browser.folders.insert(
            "/Users/nick/repos".into(),
            Load::Ready((sample_folders("/Users/nick/repos"), false)),
        );
        let root = "/Users/nick/repos/pi";
        browser.projects.insert(
            root.into(),
            Load::Ready(Files::new(root, sample_tree(root).unwrap())),
        );
        browser
    }

    fn names(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Node { node, depth, .. } => format!("{}{}", "  ".repeat(*depth), node.name),
                Row::Loading { .. } => "…".into(),
                Row::Failed { .. } => "!".into(),
                Row::More { count, .. } => format!("+{count}"),
                Row::Partial { .. } => "partial".into(),
            })
            .collect()
    }

    #[test]
    fn folders_roll_out_in_place_with_files_inside_projects() {
        let mut browser = browser();
        assert_eq!(
            names(&browser.rows()),
            ["dotfiles", "minivm", "pi", "scratch", "zed"]
        );
        browser.open.insert("/Users/nick/repos/pi".into());
        browser.open.insert("/Users/nick/repos/zed".into());
        assert_eq!(
            names(&browser.rows()),
            [
                "dotfiles",
                "minivm",
                "pi",
                "  apps",
                "  packages",
                "  README.md",
                "scratch",
                "zed",
                "…"
            ]
        );
        browser.show_hidden = true;
        assert!(names(&browser.rows()).contains(&"  .gitignore".into()));
    }

    #[test]
    fn tags_name_packages_sizes_and_counts() {
        let browser = browser();
        let Some(Children::Ready(nodes, _)) = browser.children("/Users/nick/repos/pi/packages")
        else {
            panic!("pi is listed");
        };
        assert_eq!(nodes[0].tag.as_deref(), Some("package"));
        let Some(Children::Ready(nodes, _)) =
            browser.children("/Users/nick/repos/pi/packages/ai/src")
        else {
            panic!("pi is listed");
        };
        let tags: Vec<_> = nodes.iter().map(|node| node.tag.clone().unwrap()).collect();
        assert_eq!(tags, ["14", "4 KB", "2 KB", "6 KB"]);
        assert_eq!(nodes[2].root.as_deref(), Some("/Users/nick/repos/pi"));
    }

    #[test]
    fn search_finds_listed_names_starting_with_the_query_first() {
        let browser = browser();
        let found: Vec<_> = browser
            .find("prov")
            .into_iter()
            .map(|node| node.name)
            .collect();
        assert_eq!(
            found[..3],
            ["providers", "provider-registry.ts", "providers.test.ts"]
        );
        assert!(browser.find("  ").is_empty());
        assert!(browser.find(".gitignore").is_empty());
    }

    #[test]
    fn replies_for_another_computer_are_dropped() {
        let mut browser = browser();
        let generation = browser.generation;
        browser.clear();
        assert_ne!(browser.generation, generation);
        assert!(browser.rows().is_empty());
    }

    #[test]
    fn paths_join_and_split() {
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/x", ""), "/x");
        assert!(within("/a/b", "/a"));
        assert!(!within("/ab", "/a"));
        assert_eq!(parent("a/b/c"), "a/b");
    }

    #[test]
    fn source_languages_use_real_bundled_grammars() {
        assert_eq!(language_for_path("src/main.rs"), Some("rust"));
        assert_eq!(language_for_path("web/view.tsx"), Some("tsx"));
        assert_eq!(language_for_path("go.mod"), Some("gomod"));
        assert_eq!(language_for_path("notes.txt"), None);
    }

    #[test]
    fn line_endings_are_preserved_or_safely_restricted() {
        assert_eq!(
            normalized_document("one\ntwo\n"),
            Ok(("one\ntwo\n".into(), LineEnding::Lf))
        );
        assert_eq!(
            normalized_document("one\r\ntwo\r\n"),
            Ok(("one\ntwo\n".into(), LineEnding::CrLf))
        );
        assert!(normalized_document("one\r\ntwo\rthree").is_err());
        assert!(normalized_document("one\r\ntwo\nthree").is_err());
    }
}
