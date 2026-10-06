//! Choosing where Pi works, and looking at what is there: a folder tree that
//! rolls out in place. Folders come from the helper one level at a time;
//! inside a project, its file channel lists every folder and file at once.
//! Browsing is read-only, and replies that arrive after the computer changed
//! are dropped.

use crate::{
    app::{PhoneApp, Route, Sheet, size_label},
    remote,
};
use gpui::{Context, Window};
use std::collections::{HashMap, HashSet};

/// A rolled-out folder shows this many entries before "N more".
const SHOWN: usize = 100;

/// What a search lists at most.
const FOUND: usize = 40;

#[derive(Clone, Debug, PartialEq)]
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
pub(crate) enum Row {
    Node {
        node: Node,
        depth: usize,
        open: bool,
    },
    Loading {
        depth: usize,
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
    },
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
            None | Some(Children::Loading) => rows.push(Row::Loading { depth }),
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
                    rows.push(Row::Partial { depth });
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

/// A file opened read-only from the tree.
pub(crate) struct FileView {
    /// The project it is in, on the computer.
    pub root: String,
    /// From the project's root.
    pub path: String,
    pub size: Option<u64>,
    pub text: Load<String>,
    /// Back goes to the projects sheet it was opened from.
    pub from_sheet: bool,
}

impl FileView {
    pub fn name(&self) -> &str {
        file_name(&self.path)
    }

    /// The folder it is in: "packages/ai/src", or "" at the root.
    pub fn folder(&self) -> &str {
        parent(&self.path)
    }
}

impl PhoneApp {
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

    /// Starts the tree at `path`.
    pub(crate) fn go_to_folder(&mut self, path: String, cx: &mut Context<Self>) {
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

    /// Starts a session in the picked folder, or the one the tree starts at.
    pub(crate) fn use_picked_folder(&mut self, cx: &mut Context<Self>) {
        let browser = &self.project_browser;
        let Some(path) = browser.picked.clone().or_else(|| browser.root.clone()) else {
            return;
        };
        self.use_folder(&path, cx);
    }

    pub(crate) fn use_folder(&mut self, path: &str, cx: &mut Context<Self>) {
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
        if self.route() != Route::Start {
            self.routes = vec![Route::Sessions, Route::Start];
        }
    }

    pub(crate) fn open_file(&mut self, node: &Node, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = node.root.clone() else {
            return;
        };
        let path = node.path[root.len()..].trim_start_matches('/').to_owned();
        self.file_view = Some(FileView {
            root: root.clone(),
            path: path.clone(),
            size: node.size,
            text: Load::Loading,
            from_sheet: self.sheet == Some(Sheet::Project),
        });
        self.push(Route::File, window, cx);
        self.file_generation += 1;
        let generation = self.file_generation;
        let Some(store) = &self.store else { return };
        let Some(live) = &store.live else {
            if let Some(view) = &mut self.file_view {
                view.text = Load::Ready(sample_text(&path));
            }
            return;
        };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let host = store.computer.address.clone();
        cx.spawn(async move |this, cx| {
            let result = remote::read_file(&connection, &helper, &host, &root, &path).await;
            this.update(cx, |this, cx| {
                if this.file_generation != generation {
                    return;
                }
                if let Some(view) = &mut this.file_view {
                    view.text = match result {
                        Ok(text) => Load::Ready(text),
                        Err(error) => Load::Failed(format!("{error:#}")),
                    };
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn select_project(&mut self, index: usize, cx: &mut Context<Self>) {
        if self
            .store
            .as_ref()
            .is_none_or(|store| index >= store.projects.len())
        {
            return;
        }
        self.project = index;
        if let Some(store) = &self.store {
            let address = store.computer.address.clone();
            let path = store.projects[index].path.clone();
            if !store.is_sample() {
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
}
