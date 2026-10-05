//! Embedded Zed Project/Editor adapter. No Zed Workspace or Pi session state.
mod file_actions;
mod lookup;
mod problems;
use anyhow::{Context as _, Result, bail};
pub use editor::Editor;
pub use editor::EditorEvent;
pub use file_actions::FileAction;
use gpui::{App, AppContext, Entity, Global, KeyBinding, Task, UpdateGlobal, WeakEntity, Window};
use language::LanguageRegistry;
pub use language::{Buffer, BufferEvent};
pub use lookup::{ProblemFile, SymbolHit, problem_files, problems_text, workspace_symbols};
use node_runtime::{NodeBinaryOptions, NodeRuntime};
pub use problems::{
    HoverText, Problem, QuickFix, below, cursor_point, point_at, problem_at, word_at,
};
pub use project::Event as ProjectEvent;
pub use project::Project;
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use zed_theme::{ActiveTheme, GlobalTheme};

struct Services {
    fs: Arc<dyn fs::Fs>,
    client: Arc<client::Client>,
    users: Entity<client::UserStore>,
    languages: Arc<LanguageRegistry>,
    node: NodeRuntime,
    projects: HashMap<PathBuf, WeakEntity<Project>>,
}
impl Global for Services {}

fn services(cx: &mut App) -> Result<()> {
    if cx.has_global::<Services>() {
        return Ok(());
    }
    let data = dirs::data_local_dir()
        .context("Cannot locate desktop data directory")?
        .join("pi-desktop");
    // Never open Zed's database or watch its user configuration.
    release_channel::init(
        release_channel::AppVersion::load(env!("CARGO_PKG_VERSION"), None, None),
        cx,
    );
    let connection = gpui::block_on(db::open_db::<db::AppMigrator>(
        &data.join("db"),
        db::GlobalDbScope,
    ));
    cx.set_global(db::AppDatabase(connection));
    gpui_tokio::init(cx);
    cx.set_http_client(Arc::new(reqwest_client::ReqwestClient::user_agent(
        "pi-desktop",
    )?));
    let fs: Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.background_executor().clone());
    <dyn fs::Fs>::set_global(fs.clone(), cx);
    let client = client::Client::production(cx);
    // No authenticate/connect/telemetry.start calls. Only local project handlers.
    client::Client::set_global(client.clone(), cx);
    Project::init(&client, cx);
    project::trusted_worktrees::init(Default::default(), cx);
    let users = cx.new(|cx| client::UserStore::new(client.clone(), cx));
    let mut registry = LanguageRegistry::new(cx.background_executor().clone());
    registry.set_language_server_download_dir(data.join("language-servers"));
    let languages = Arc::new(registry);
    let (_, options) = watch::channel(Some(NodeBinaryOptions {
        allow_path_lookup: true,
        allow_binary_download: false,
        use_paths: None,
    }));
    let node = NodeRuntime::new(cx.http_client(), None, options);
    languages::init(languages.clone(), fs.clone(), node.clone(), cx);
    languages.set_theme(cx.theme().clone());
    cx.observe_global::<GlobalTheme>(|cx| {
        if let Some(services) = cx.try_global::<Services>() {
            services.languages.set_theme(cx.theme().clone());
        }
    })
    .detach();
    editor::init(cx);
    apply_preferences(cx);
    // Editor alone leaves GlobalDiagnosticRenderer unset. Zed's diagnostics
    // crate supplies the hover Markdown and inline diagnostic blocks; its
    // Workspace observers do not create or require a Workspace instance.
    diagnostics::init(cx);
    use editor::actions as a;
    // Deliberately bind only embedded-editor actions, not Zed Workspace actions.
    cx.bind_keys([
        KeyBinding::new("left", a::MoveLeft, Some("Editor")),
        KeyBinding::new("right", a::MoveRight, Some("Editor")),
        KeyBinding::new("up", zed_actions::editor::MoveUp, Some("Editor")),
        KeyBinding::new("down", zed_actions::editor::MoveDown, Some("Editor")),
        KeyBinding::new("shift-left", a::SelectLeft, Some("Editor")),
        KeyBinding::new("shift-right", a::SelectRight, Some("Editor")),
        KeyBinding::new("shift-up", a::SelectUp, Some("Editor")),
        KeyBinding::new("shift-down", a::SelectDown, Some("Editor")),
        KeyBinding::new("backspace", a::Backspace, Some("Editor")),
        KeyBinding::new("delete", a::Delete, Some("Editor")),
        KeyBinding::new("enter", a::Newline, Some("Editor")),
        KeyBinding::new("tab", a::Tab, Some("Editor")),
        KeyBinding::new("secondary-a", a::SelectAll, Some("Editor")),
        KeyBinding::new("secondary-c", a::Copy, Some("Editor")),
        KeyBinding::new("secondary-v", a::Paste, Some("Editor")),
        KeyBinding::new("secondary-x", a::Cut, Some("Editor")),
        KeyBinding::new("secondary-z", a::Undo, Some("Editor")),
        KeyBinding::new("secondary-shift-z", a::Redo, Some("Editor")),
        KeyBinding::new(
            "secondary-.",
            a::ToggleCodeActions {
                deployed_from: None,
                quick_launch: false,
            },
            Some("Editor"),
        ),
    ]);
    cx.set_global(Services {
        fs,
        client,
        users,
        languages,
        node,
        projects: HashMap::new(),
    });
    Ok(())
}

/// The desktop's editor settings that go through Zed's user settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preferences {
    /// The desktop draws one card for diagnostics, hover text and quick fixes
    /// (`problems`); Zed's own hover popovers show only when this is off.
    pub problem_card: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self { problem_card: true }
    }
}
impl Global for Preferences {}

/// Applies at once to open editors, and when language services first start.
pub fn set_preferences(preferences: Preferences, cx: &mut App) {
    if cx.try_global::<Preferences>() == Some(&preferences) {
        return;
    }
    cx.set_global(preferences);
    if cx.has_global::<Services>() {
        apply_preferences(cx);
    }
}

fn apply_preferences(cx: &mut App) {
    let preferences = cx.try_global::<Preferences>().copied().unwrap_or_default();
    // A constant key: if Zed renamed it, its popovers would simply show again.
    let json = format!(
        r#"{{"hover_popover_enabled": {}}}"#,
        !preferences.problem_card
    );
    settings::SettingsStore::update_global(cx, |store, cx| {
        let _ = store.set_user_settings(&json, cx);
    });
}

/// Colors whole lines in an editor's gutter, one color per marker type `T`, as
/// Zed marks diff hunks. `rows` are 0-based buffer rows, the end excluded.
pub fn mark_rows<T: 'static>(
    editor: &Entity<Editor>,
    rows: &[std::ops::Range<u32>],
    color: fn(&App) -> gpui::Hsla,
    cx: &mut App,
) {
    editor.update(cx, |editor, cx| {
        let snapshot = editor.buffer().read(cx).snapshot(cx);
        let ranges: Vec<_> = rows
            .iter()
            .map(|rows| {
                let last = rows.end.saturating_sub(1).max(rows.start);
                snapshot.anchor_before(language::Point::new(rows.start, 0))
                    ..snapshot.anchor_before(language::Point::new(last, 0))
            })
            .collect();
        editor.highlight_gutter::<T>(ranges, color, cx);
    });
}

pub fn unmark_rows<T: 'static>(editor: &Entity<Editor>, cx: &mut App) {
    editor.update(cx, |editor, cx| {
        editor.clear_gutter_highlights::<T>(cx);
    });
}

/// Shades whole lines behind the text, one color per type `T`, replacing what
/// `T` shaded before. `rows` are 0-based buffer rows, the end excluded.
pub fn shade_rows<T: 'static>(
    editor: &Entity<Editor>,
    rows: &[std::ops::Range<u32>],
    color: fn(&App) -> gpui::Hsla,
    cx: &mut App,
) {
    editor.update(cx, |editor, cx| {
        editor.clear_row_highlights::<T>();
        let snapshot = editor.buffer().read(cx).snapshot(cx);
        for rows in rows.iter().filter(|rows| !rows.is_empty()) {
            // Zed ends a row highlight at column 0 of the row after it.
            let range = snapshot.anchor_before(language::Point::new(rows.start, 0))
                ..snapshot.anchor_before(language::Point::new(rows.end, 0));
            editor.highlight_rows::<T>(range, color, Default::default(), cx);
        }
        cx.notify();
    });
}

/// Conservative guard used before file-history operations. Projects are shared
/// between sessions, so this also catches a dirty buffer in another tab.
pub fn has_unsaved_buffers(cx: &App) -> bool {
    cx.try_global::<Services>().is_some_and(|s| {
        s.projects
            .values()
            .filter_map(WeakEntity::upgrade)
            .any(|p| p.read(cx).dirty_buffers(cx).next().is_some())
    })
}

/// The shared project for a canonical `root` once the user has trusted it, so its
/// language servers may run. Never creates a project or starts a server.
pub fn language_project(root: &Path, cx: &App) -> Option<Entity<Project>> {
    let project = cx.try_global::<Services>()?.projects.get(root)?.upgrade()?;
    let opened = project
        .read(cx)
        .visible_worktrees(cx)
        .any(|worktree| worktree.read(cx).abs_path().as_ref() == root);
    let store = project.read(cx).worktree_store();
    let trusted =
        !project::trusted_worktrees::TrustedWorktrees::has_restricted_worktrees(&store, cx);
    (opened && trusted).then_some(project)
}

/// Detached offline buffer; no project is opened and no language server runs.
pub fn preview_buffer(text: &str, cx: &mut App) -> Result<Entity<Buffer>> {
    detached_buffer(Path::new("preview.ts"), text, cx)
}

/// In-memory editor buffer with syntax highlighting only. No local file handle,
/// Project, watcher, formatter or language-server process is attached.
pub fn detached_buffer(path: &Path, text: &str, cx: &mut App) -> Result<Entity<Buffer>> {
    services(cx)?;
    let buffer = cx.new(|cx| Buffer::local(text, cx));
    let registry = cx.global::<Services>().languages.clone();
    buffer.update(cx, |buffer, _| {
        buffer.set_language_registry(registry.clone())
    });
    let registry_for_language = registry.clone();
    let path = path.to_owned();
    let entity = buffer.clone();
    cx.spawn(async move |cx| {
        if let Ok(language) = registry_for_language
            .load_language_for_file_path(&path)
            .await
        {
            entity.update(cx, |buffer, cx| buffer.set_language(Some(language), cx));
        }
    })
    .detach();
    Ok(buffer)
}

#[derive(Clone)]
pub struct EditorProject {
    pub project: Entity<Project>,
    pub root: PathBuf,
}
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub relative: String,
    pub directory: bool,
}
impl EditorProject {
    /// Roots are canonicalized by the caller off the foreground executor.
    pub fn new(root: PathBuf, cx: &mut App) -> Result<Self> {
        services(cx)?;
        let services = cx.global::<Services>();
        if let Some(project) = services.projects.get(&root).and_then(WeakEntity::upgrade) {
            return Ok(Self { project, root });
        }
        let (client, node, users, languages, fs) = (
            services.client.clone(),
            services.node.clone(),
            services.users.clone(),
            services.languages.clone(),
            services.fs.clone(),
        );
        let project = Project::local(
            client,
            node,
            users,
            languages,
            fs,
            None,
            project::LocalProjectFlags {
                init_worktree_trust: true,
                watch_global_configs: false,
            },
            cx,
        );
        Services::update_global(cx, |s, _| {
            s.projects.insert(root.clone(), project.downgrade());
        });
        Ok(Self { project, root })
    }
    pub fn open(&self, path: PathBuf, cx: &mut App) -> Task<Result<Entity<Buffer>>> {
        let root = self.root.clone();
        let project = self.project.clone();
        let validate = cx.background_executor().spawn(async move {
            validate_file(&path)?;
            Ok::<_, anyhow::Error>(path)
        });
        cx.spawn(async move |cx| {
            let path = validate.await?;
            // A visible root worktree supplies the file tree, ignores and file watching.
            project
                .update(cx, |p, cx| p.find_or_create_worktree(&root, true, cx))
                .await?;
            project
                .update(cx, |p, cx| p.open_local_buffer(path, cx))
                .await
        })
    }
    pub fn editor(
        &self,
        buffer: Entity<Buffer>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Editor> {
        cx.new(|cx| Editor::for_buffer(buffer, Some(self.project.clone()), window, cx))
    }
    pub fn save(&self, buffer: Entity<Buffer>, cx: &mut App) -> Task<Result<()>> {
        if buffer.read(cx).has_conflict() {
            return Task::ready(Err(anyhow::anyhow!(
                "File changed on disk. Reload or resolve the conflict before saving."
            )));
        }
        self.project.update(cx, |p, cx| p.save_buffer(buffer, cx))
    }
    /// Explicit opt-in: project configuration and language servers can execute code
    /// and download server packages. Node itself is never downloaded automatically.
    pub fn enable_language_services(&self, cx: &mut App) {
        let store = self.project.read(cx).worktree_store();
        let Some(trusted) = project::trusted_worktrees::TrustedWorktrees::try_get_global(cx) else {
            return;
        };
        trusted.update(cx, |trusted, cx| {
            trusted.trust(
                &store,
                [project::trusted_worktrees::PathTrust::AbsPath(
                    self.root.clone(),
                )]
                .into_iter()
                .collect(),
                cx,
            )
        });
    }
    pub fn language_servers(&self, cx: &App) -> Vec<String> {
        self.project
            .read(cx)
            .language_server_statuses(cx)
            .map(|(_, s)| s.name.to_string())
            .collect()
    }
    pub fn entries(&self, cx: &App) -> Vec<FileEntry> {
        self.project
            .read(cx)
            .visible_worktrees(cx)
            .flat_map(|worktree| {
                let tree = worktree.read(cx);
                let root = tree.abs_path();
                tree.snapshot()
                    .entries(false, 0)
                    .filter(|e| !e.path.is_empty() && !e.is_external)
                    .map(|e| {
                        let relative = e.path.as_unix_str().to_owned();
                        FileEntry {
                            path: root.join(&relative),
                            relative,
                            directory: e.is_dir(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

/// Run only off the UI thread. Refuse binary/huge/special files rather than
/// silently decoding lossily or waiting forever on a FIFO/device.
fn validate_file(path: &Path) -> Result<()> {
    let meta = std::fs::metadata(path).with_context(|| format!("Opening {}", path.display()))?;
    if !meta.is_file() {
        bail!("Not a regular file");
    }
    if meta.len() > 16 * 1024 * 1024 {
        bail!("Files over 16 MiB are not supported in the editor yet");
    }
    let mut sample = [0u8; 8192];
    let n = std::fs::File::open(path)?.read(&mut sample)?;
    if sample[..n].contains(&0) {
        bail!("Binary files are not supported in the text editor");
    }
    Ok(())
}
