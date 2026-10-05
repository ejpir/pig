//! Session-local file tabs over shared project buffers. Editors, pending opens,
//! dirty/conflict policy and file-tree filtering never belong to Desktop.
use super::panels::{input_box, note};
use super::*;
use gpui::{EventEmitter, Focusable};
use pi_editor::{Buffer, BufferEvent, Editor, EditorEvent, EditorProject, FileEntry, ProjectEvent};
use std::collections::HashSet;
mod actions;
mod card;
mod provenance;
mod remote;
mod turn_bars;
use actions::{CancelMutation, ConfirmMutation, Mutation, MutationKind};
use card::AskPiToFix;

gpui::actions!(file_tabs, [SaveFile]);
#[derive(Clone)]
pub enum FileEvent {
    Tabs,
    Selected,
    /// A confirmation in the file area needs it shown; the inspector stays.
    Confirming,
    Empty,
    /// Compare the open file's observed edits in Changes, by project path.
    Review(String),
    ShowTree,
    /// A prompt asking pi to fix a language-server problem.
    AskPi(String),
    /// A turn the ⌥-click card names: show its line in the thread, or its diff.
    ShowTurn(usize),
    DiffTurn(usize),
}
struct Tab {
    path: PathBuf,
    buffer: Entity<Buffer>,
    editor: Option<Entity<Editor>>,
    _subscription: gpui::Subscription,
    _editor_subscription: Option<gpui::Subscription>,
}
#[derive(Clone)]
enum Confirmation {
    Close(PathBuf),
    Reload(PathBuf),
    Trust,
}
pub struct FilesView {
    root: PathBuf,
    demo: bool,
    remote: Option<remote::Remote>,
    host: Option<EditorProject>,
    tabs: Vec<Tab>,
    active: Option<PathBuf>,
    opening: HashSet<PathBuf>,
    saving: bool,
    browser_loading: bool,
    focus_pending: bool,
    error: Option<String>,
    confirm: Option<Confirmation>,
    trusted: bool,
    entries: Vec<FileEntry>,
    rows: Vec<FileEntry>,
    collapsed: HashSet<PathBuf>,
    selected_entry: Option<PathBuf>,
    mutation: Option<Mutation>,
    mutation_focus: gpui::FocusHandle,
    browser_focus: gpui::FocusHandle,
    mutating: bool,
    name_input: Entity<TextInput>,
    filter: Entity<TextInput>,
    scroll: gpui::UniformListScrollHandle,
    _filter_subscription: gpui::Subscription,
    _project_subscription: Option<gpui::Subscription>,
    hover: card::HoverCard,
    bars: turn_bars::TurnBars,
    /// What the edited-lines shading was computed for: file, length, edit.
    shaded: Option<(PathBuf, usize, Option<String>)>,
    /// Open disclosures in the file details inspector.
    details_open: HashSet<&'static str>,
    /// `editor.fontSize` as the editors have it.
    font_size: f32,
    _prefs_subscription: gpui::Subscription,
}
impl EventEmitter<FileEvent> for FilesView {}
impl FilesView {
    pub fn new(root: PathBuf, demo: bool, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("secondary-s", SaveFile, Some("FileEditor")),
            KeyBinding::new("alt-enter", AskPiToFix, Some("FileEditor")),
            KeyBinding::new("enter", ConfirmMutation, Some("FileMutation")),
            KeyBinding::new("escape", CancelMutation, Some("FileMutation")),
        ]);
        let filter = cx.new(|cx| TextInput::new("Filter files…", cx).compact());
        let subscription = cx.observe(&filter, |this, _, cx| this.filter_rows(cx));
        let prefs_subscription =
            cx.observe_global::<crate::prefs::Prefs>(|this, cx| this.apply_font_size(cx));
        Self {
            root,
            demo,
            remote: None,
            host: None,
            tabs: vec![],
            active: None,
            opening: HashSet::new(),
            saving: false,
            browser_loading: false,
            focus_pending: false,
            error: None,
            confirm: None,
            trusted: false,
            entries: vec![],
            rows: vec![],
            collapsed: HashSet::new(),
            selected_entry: None,
            mutation: None,
            mutation_focus: cx.focus_handle(),
            browser_focus: cx.focus_handle(),
            mutating: false,
            name_input: cx.new(|cx| TextInput::new("Name…", cx).compact()),
            filter,
            scroll: gpui::UniformListScrollHandle::new(),
            _filter_subscription: subscription,
            _project_subscription: None,
            hover: Default::default(),
            bars: Default::default(),
            shaded: None,
            details_open: HashSet::new(),
            font_size: font_size(cx),
            _prefs_subscription: prefs_subscription,
        }
    }
    pub fn with_remote(mut self, target: pi_core::ssh::SshTarget) -> Self {
        self.remote = Some(remote::Remote::new(target));
        self.demo = false;
        self
    }
    /// `editor.fontSize` changed: open editors follow at once.
    fn apply_font_size(&mut self, cx: &mut Context<Self>) {
        let size = font_size(cx);
        if size == self.font_size {
            return;
        }
        self.font_size = size;
        for editor in self.tabs.iter().filter_map(|tab| tab.editor.as_ref()) {
            editor.update(cx, |editor, cx| {
                editor.set_text_style_refinement(text_style(size));
                cx.notify();
            });
        }
    }
    pub fn has_unsaved(&self, cx: &App) -> bool {
        self.tabs.iter().any(|tab| tab.buffer.read(cx).is_dirty())
    }
    pub fn close_blocked(&self) -> bool {
        self.saving || self.mutating || !self.opening.is_empty()
    }
    pub fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }
    /// The project's files and folders, once the browser has loaded them.
    pub fn file_entries(&self) -> &[FileEntry] {
        &self.entries
    }
    /// Status for consumers waiting on the first tree scan (e.g. the @ menu).
    pub fn browser_status(&self) -> Option<&str> {
        if self.browser_loading {
            Some(if self.remote.is_some() {
                "Loading remote files…"
            } else {
                "Loading files…"
            })
        } else if self.entries.is_empty() {
            self.error.as_deref()
        } else {
            None
        }
    }
    pub fn root(&self) -> &PathBuf {
        &self.root
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        if let Some(editor) = self.tab().and_then(|t| t.editor.as_ref()) {
            editor.focus_handle(cx).focus(window, cx);
        }
    }
    fn tab(&self) -> Option<&Tab> {
        self.tabs
            .iter()
            .find(|t| Some(&t.path) == self.active.as_ref())
    }
    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            self.open_remote(path, cx);
            return;
        }
        if self.tabs.iter().any(|t| t.path == path) {
            self.select(path, cx);
            return;
        }
        if !self.opening.insert(path.clone()) {
            return;
        }
        self.error = None;
        if self.demo {
            // A read-only in-memory preview, never a read/write against fixture cwd.
            let buffer =
                pi_editor::preview_buffer(include_str!("../../../../fixtures/editor.ts"), cx);
            self.opening.remove(&path);
            match buffer {
                Ok(buffer) => self.add_tab(path, buffer, cx),
                Err(e) => {
                    self.error = Some(format!("{e:#}"));
                    cx.notify();
                }
            }
            return;
        }
        let root = self.root.clone();
        let canonical = cx
            .background_executor()
            .spawn(async move { std::fs::canonicalize(root) });
        cx.spawn(async move |this, cx| {
            let result = async {
                let root = canonical.await?;
                let task = this.update(cx, |this, cx| -> anyhow::Result<_> {
                    this.attach_project(root, cx)?;
                    Ok(this.host.as_ref().unwrap().open(path.clone(), cx))
                })??;
                task.await
            }
            .await;
            this.update(cx, |this, cx| {
                this.opening.remove(&path);
                match result {
                    Ok(buffer) => this.add_tab(path, buffer, cx),
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    fn attach_project(&mut self, root: PathBuf, cx: &mut Context<Self>) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.remote.is_none(),
            "Remote paths cannot create a local editor project"
        );
        if self.host.is_some() {
            return Ok(());
        }
        let host = EditorProject::new(root, cx)?;
        self._project_subscription =
            Some(
                cx.subscribe(&host.project, |this, _, event, cx| match event {
                    ProjectEvent::WorktreeAdded(_)
                    | ProjectEvent::WorktreeRemoved(_)
                    | ProjectEvent::WorktreeUpdatedEntries(..) => this.refresh_entries(cx),
                    ProjectEvent::LanguageServerAdded(..)
                    | ProjectEvent::LanguageServerRemoved(_)
                    | ProjectEvent::LanguageServerBufferRegistered { .. } => cx.notify(),
                    ProjectEvent::EntryRenamed {
                        old_abs_path,
                        new_abs_path,
                        ..
                    } => this.entry_renamed(old_abs_path, new_abs_path, cx),
                    ProjectEvent::Toast { message, .. } => {
                        this.error = Some(message.clone());
                        cx.notify();
                    }
                    _ => {}
                }),
            );
        self.root = host.root.clone();
        self.host = Some(host);
        Ok(())
    }
    /// Reveal a directory reference without opening it as a file or reading its contents.
    pub fn reveal_directory(&mut self, relative: &str, cx: &mut Context<Self>) {
        let relative = relative.trim_end_matches('/');
        let path = std::path::Path::new(relative);
        if relative.is_empty()
            || path
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return;
        }
        let target = self.root.join(path);
        self.selected_entry = Some(target.clone());
        self.collapsed.retain(|path| !target.starts_with(path));
        self.filter
            .update(cx, |input, cx| input.set_content(relative.to_owned(), cx));
        self.load_browser(cx);
        self.filter_rows(cx);
    }
    pub fn focus_browser(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.browser_focus.focus(window, cx);
    }
    pub fn load_browser(&mut self, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            self.load_remote_browser(cx);
            return;
        }
        if self.demo {
            self.refresh_entries(cx);
            return;
        }
        if self.host.is_some() || self.browser_loading {
            return;
        }
        self.browser_loading = true;
        let root = self.root.clone();
        let canonical = cx
            .background_executor()
            .spawn(async move { std::fs::canonicalize(root) });
        cx.spawn(async move |this, cx| {
            let result = async {
                let root = canonical.await?;
                let task = this.update(cx, |this, cx| -> anyhow::Result<_> {
                    this.attach_project(root.clone(), cx)?;
                    Ok(this
                        .host
                        .as_ref()
                        .unwrap()
                        .project
                        .update(cx, |p, cx| p.find_or_create_worktree(root, true, cx)))
                })??;
                task.await.map(|_| ())
            }
            .await;
            this.update(cx, |this, cx| {
                this.browser_loading = false;
                if let Err(e) = result {
                    this.error = Some(format!("{e:#}"));
                }
                this.refresh_entries(cx);
            })
            .ok();
        })
        .detach();
    }
    fn add_tab(&mut self, path: PathBuf, buffer: Entity<Buffer>, cx: &mut Context<Self>) {
        // Project returns the same buffer for aliases or concurrent opens.
        if let Some(tab) = self.tabs.iter().find(|t| t.buffer == buffer) {
            let path = tab.path.clone();
            self.select(path, cx);
            return;
        }
        let subscription = cx.subscribe(&buffer, |_, _, event, cx| {
            if matches!(
                event,
                BufferEvent::DirtyChanged
                    | BufferEvent::Saved
                    | BufferEvent::FileHandleChanged
                    | BufferEvent::Reloaded
                    | BufferEvent::ReloadNeeded
                    | BufferEvent::LanguageChanged(_)
                    | BufferEvent::DiagnosticsUpdated
            ) {
                cx.emit(FileEvent::Tabs);
                cx.notify();
            }
        });
        self.tabs.push(Tab {
            path: path.clone(),
            buffer,
            editor: None,
            _subscription: subscription,
            _editor_subscription: None,
        });
        self.select(path, cx);
        self.refresh_entries(cx);
    }
    fn select(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.hide_card(cx);
        self.selected_entry = Some(path.clone());
        self.active = Some(path);
        self.focus_pending = true;
        cx.emit(FileEvent::Selected);
        cx.notify();
    }
    fn refresh_entries(&mut self, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            self.filter_rows(cx);
            return;
        }
        self.entries = if self.demo {
            [
                ("packages", true),
                ("packages/ai", true),
                ("packages/ai/src", true),
                ("packages/ai/src/providers", true),
                ("packages/ai/src/providers/openai-completions.ts", false),
            ]
            .into_iter()
            .map(|(relative, directory)| FileEntry {
                path: self.root.join(relative),
                relative: relative.into(),
                directory,
            })
            .collect()
        } else {
            self.host
                .as_ref()
                .map(|h| h.entries(cx))
                .unwrap_or_default()
        };
        self.filter_rows(cx);
    }
    fn filter_rows(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).content().to_lowercase();
        self.rows = self
            .entries
            .iter()
            .filter(|e| {
                if !query.is_empty() {
                    e.relative.to_lowercase().contains(&query)
                } else {
                    !self
                        .collapsed
                        .iter()
                        .any(|p| e.path != *p && e.path.starts_with(p))
                }
            })
            .cloned()
            .collect();
        cx.notify();
    }
    fn close(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self
            .tabs
            .iter()
            .any(|t| t.path == path && t.buffer.read(cx).is_dirty())
        {
            self.select(path.clone(), cx);
            self.confirm = Some(Confirmation::Close(path));
            cx.notify();
            return;
        }
        self.remove(&path, cx);
    }
    fn remove(&mut self, path: &PathBuf, cx: &mut Context<Self>) {
        self.tabs.retain(|t| &t.path != path);
        if self.active.as_ref() == Some(path) {
            self.active = self.tabs.last().map(|t| t.path.clone());
            self.focus_pending = true;
        }
        self.confirm = None;
        cx.emit(if self.tabs.is_empty() {
            FileEvent::Empty
        } else {
            FileEvent::Tabs
        });
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.demo {
            return;
        }
        if self.remote.is_some() {
            self.save_remote(cx);
            return;
        }
        let Some(buffer) = self.tab().map(|t| t.buffer.clone()) else {
            return;
        };
        let Some(host) = self.host.as_ref() else {
            return;
        };
        let task = host.save(buffer, cx);
        self.saving = true;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(()) => this.error = None,
                    Err(e) => this.error = Some(format!("Save failed: {e:#}")),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    fn confirm(&mut self, cx: &mut Context<Self>) {
        match self.confirm.take() {
            Some(Confirmation::Close(path)) => self.remove(&path, cx),
            Some(Confirmation::Reload(path)) => {
                if self.remote.is_some() {
                    self.reload_remote(path, cx);
                    return;
                }
                if let Some(tab) = self.tabs.iter().find(|t| t.path == path) {
                    let task = tab.buffer.update(cx, |b, cx| b.reload(cx));
                    cx.spawn(async move |this, cx| {
                        let result = task.await;
                        this.update(cx, |this, cx| {
                            this.error = if result.is_err() {
                                Some("Reload failed; the buffer has been kept.".into())
                            } else {
                                None
                            };
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
            }
            Some(Confirmation::Trust) => {
                if let Some(host) = &self.host {
                    host.enable_language_services(cx);
                    self.trusted = true;
                }
            }
            None => {}
        };
        cx.notify();
    }
    pub fn tabs(&self, shown: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        h_flex()
            .h_full()
            .gap(px(16.))
            .flex_shrink_0()
            .when(!self.tabs.is_empty(), |v| {
                v.child(div().w(px(1.)).h(px(24.)).bg(theme.line))
            })
            .children(self.tabs.iter().enumerate().map(|(i, tab)| {
                let path = tab.path.clone();
                let close = path.clone();
                let dirty = tab.buffer.read(cx).is_dirty();
                h_flex()
                    .id(("file-tab", i))
                    .debug_selector(move || format!("file-tab-{i}"))
                    .role(gpui::Role::Tab)
                    .aria_selected(shown && self.active.as_ref() == Some(&path))
                    .gap(px(8.))
                    .h_full()
                    .max_w(px(260.))
                    .border_b_2()
                    .border_color(if shown && self.active.as_ref() == Some(&path) {
                        theme.accent
                    } else {
                        gpui::transparent_black()
                    })
                    .cursor_pointer()
                    .child(icon("file", theme.muted))
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(11.5))
                            .truncate()
                            .child(format!(
                                "{}{}",
                                path.file_name().unwrap_or_default().to_string_lossy(),
                                if dirty { " •" } else { "" }
                            )),
                    )
                    .child(
                        icon_button(("close-file", i), "close", "Close file", theme)
                            .size(px(18.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close(close.clone(), cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select(path.clone(), cx)))
            }))
            .into_any_element()
    }
    pub fn inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let weak = cx.entity().downgrade();
        let tree = gpui::uniform_list("project-files", self.rows.len(), move |range, _, cx| {
            weak.update(cx, |this, cx| {
                range
                    .map(|i| this.file_row(i, cx, theme))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        })
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h(px(100.))
        .w_full();
        let servers = self
            .host
            .as_ref()
            .map(|h| h.language_servers(cx))
            .unwrap_or_default();
        v_flex().id("file-browser").debug_selector(|| "file-browser".into()).key_context("FileBrowser").track_focus(&self.browser_focus).h((window.viewport_size().height-px(76.)).max(px(400.))).p(px(20.)).gap(px(12.))
            .on_action(cx.listener(|this,_:&Stop,window,cx|{this.filter.update(cx,|input,cx|input.set_content(String::new(),cx));this.browser_focus.focus(window,cx);cx.stop_propagation();}))
            .child(div().font_family(MONO).text_size(px(18.)).child("Files"))
            .child(note(self.root.display().to_string(),theme))
            .child(self.file_toolbar(cx))
            .when(self.mutation.is_some(),|v|v.child(self.mutation_form(cx)))
            .when_some(self.error.clone(),|v,e|v.child(div().text_size(px(11.)).text_color(theme.coral).child(e)))
            .child(tree)
            .when(self.selected_entry.is_some() && !self.demo && self.remote.is_none(),|v|v.child(h_flex().gap(px(8.))
                .child(icon_button("rename-file","pencil","Rename selected item",theme).tooltip(ui::Tooltip::text("Rename selected item")).on_click(cx.listener(|this,_,window,cx|this.begin_mutation(MutationKind::Rename,None,window,cx))))
                .child(icon_button("delete-file","trash","Delete selected item…",theme).debug_selector(||"delete-file".into()).tooltip(ui::Tooltip::text("Move selected item to Trash…")).on_click(cx.listener(|this,_,window,cx|this.begin_mutation(MutationKind::Trash,None,window,cx))))))
            .child(note(if self.demo {"Offline preview — editing and file operations disabled."}else if self.remote.is_some(){"SSH · UTF-8 files up to 1 MiB. Symlinks and create/rename/delete are not supported yet."}else{"Right-click for file actions. Delete uses the system Trash."},theme))
            .child(divider(theme)).child(label("LANGUAGE SERVICES",theme))
            .child(note(if servers.is_empty(){if self.trusted{"No language server running. Check that this language's server and Node/npm are available."}else{"Off until you explicitly trust this project."}.into()}else{format!("Running: {}",servers.join(", "))},theme))
            .when(!self.trusted && !self.demo && self.remote.is_none(),|v|v.child(button("enable-lsp","Trust project and start services…",theme).on_click(cx.listener(|this,_,_,cx|{this.confirm=Some(Confirmation::Trust);cx.emit(FileEvent::Confirming);cx.notify();}))))
            .into_any_element()
    }
}
impl Render for FilesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        for tab in &mut self.tabs {
            if tab.editor.is_none() {
                let editor = if let Some(host) = &self.host {
                    host.editor(tab.buffer.clone(), window, cx)
                } else {
                    cx.new(|cx| Editor::for_buffer(tab.buffer.clone(), None, window, cx))
                };
                let size = self.font_size;
                editor.update(cx, |editor, _| {
                    editor.set_text_style_refinement(text_style(size));
                    if self.demo {
                        editor.set_read_only(true);
                    }
                });
                // The card belongs to a spot on screen; scrolling or editing moves it.
                tab._editor_subscription = Some(cx.subscribe(&editor, |this, _, event, cx| {
                    if matches!(
                        event,
                        EditorEvent::ScrollPositionChanged { .. }
                            | EditorEvent::Edited { .. }
                            | EditorEvent::BufferEdited
                    ) {
                        this.hide_card(cx);
                    }
                }));
                tab.editor = Some(editor);
            }
        }
        if self.focus_pending && !window.has_active_prompt() {
            self.focus_pending = false;
            self.focus(window, cx);
        }
        self.shade_edits(cx);
        let tab = self.tab();
        let dirty = tab.is_some_and(|t| t.buffer.read(cx).is_dirty());
        let conflict = tab.is_some_and(|t| t.buffer.read(cx).has_conflict());
        let editor = tab.and_then(|t| t.editor.clone());
        let relative = tab
            .map(|t| {
                t.path
                    .strip_prefix(&self.root)
                    .unwrap_or(&t.path)
                    .to_path_buf()
            })
            .unwrap_or_default();
        let language = tab
            .and_then(|t| t.buffer.read(cx).language())
            .map(|l| l.name().to_string())
            .unwrap_or_else(|| "Plain Text".into());
        let lines = tab.map_or(0, |t| t.buffer.read(cx).max_point().row + 1);
        let observed = !self.active_edits(cx).is_empty();
        // packages / ai / src / providers / openai-completions.ts
        let mut segments: Vec<String> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let file = segments.pop().unwrap_or_default();
        let breadcrumb = h_flex()
            .debug_selector(|| "file-breadcrumb".into())
            .min_w_0()
            .gap(px(6.))
            .font_family(MONO)
            .text_size(px(12.))
            .text_color(theme.muted)
            .children(segments.into_iter().flat_map(|segment| {
                [
                    div().flex_shrink_0().child(segment),
                    div().flex_shrink_0().text_color(theme.faint).child("/"),
                ]
            }))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.secondary)
                    .child(file),
            );
        let state = if self.demo {
            "Read-only preview"
        } else if conflict {
            "Changed on disk"
        } else if dirty {
            "Unsaved changes"
        } else if self.remote.is_some() {
            if self
                .remote
                .as_ref()
                .is_some_and(|remote| remote.is_connected())
            {
                "Saved · polling remote host"
            } else {
                "Saved · disconnected (last known)"
            }
        } else {
            "Saved · watching disk"
        };
        v_flex().id("file-editor").debug_selector(||"file-editor".into()).key_context("FileEditor").size_full().min_h_0()
            .on_action(cx.listener(|this,_:&SaveFile,_,cx|this.save(cx)))
            .on_action(cx.listener(|this,_:&Stop,_,cx|{this.hide_card(cx);cx.stop_propagation()}))
            .on_action(cx.listener(|this,_:&AskPiToFix,_,cx|this.ask_pi_at_cursor(cx)))
            .when(tab.is_some(), |v| v.child(v_flex().px(WORK_GUTTER).pt(px(18.)).pb(px(10.)).gap(px(8.))
                .child(breadcrumb)
                .child(h_flex().h(px(28.)).gap(px(8.))
                    .child(div().flex_shrink_0().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Current file"))
                    .child(div().flex_1())
                    .child(div().flex_shrink_0().text_size(px(12.)).text_color(theme.muted).child(state))
                    .when(!self.demo, |row| row
                        .child(primary_button("save-file",if self.saving{"Saving…"}else{"Save"},dirty && !conflict && !self.saving,theme).on_click(cx.listener(|this,_,_,cx|this.save(cx))))
                        .child(button("reload-file","Reload…",theme).on_click(cx.listener(|this,_,_,cx|{if let Some(path)=this.active.clone(){this.confirm=Some(Confirmation::Reload(path));cx.notify();}})))))))
            .when(conflict,|v|v.child(div().px(WORK_GUTTER).pb(px(10.)).text_size(px(12.)).text_color(theme.amber).child("Changed on disk while this buffer has unsaved edits. Copy your edits before reloading; saving is blocked.")))
            .when_some(self.error.clone(),|v,e|v.child(div().px(WORK_GUTTER).pb(px(10.)).text_size(px(12.)).text_color(theme.coral).child(e)))
            .when_some(self.confirm.clone(),|v,confirm|v.child(v_flex().mx(WORK_GUTTER).mb(px(10.)).p(px(12.)).gap(px(8.)).rounded(px(6.)).bg(theme.selected)
                .child(match confirm {Confirmation::Close(_)=>"Close this tab without saving? Other tabs sharing the buffer keep their edits.",Confirmation::Reload(_)=>"Discard unsaved edits and reload from disk? This affects every tab sharing this buffer.",Confirmation::Trust=>"Trust this project? Language services may execute project configuration and download server packages. Node must already be installed."})
                .child(h_flex().gap(px(8.)).child(button("confirm-file-action","Confirm",theme).on_click(cx.listener(|this,_,_,cx|this.confirm(cx))))
                    .child(button("cancel-file-action","Cancel",theme).on_click(cx.listener(|this,_,_,cx|{this.confirm=None;cx.notify();}))))))
            .when(editor.is_none(),|v|v.child(super::panels::empty("Files","Select a file in the browser to open it here.",theme).flex_1()))
            .when_some(editor,|v,e|v.child(v_flex().debug_selector(||"file-code".into()).flex_1().min_h_0().mx(WORK_GUTTER).rounded(px(6.)).border_1().border_color(theme.line).overflow_hidden()
                // What the file is, above it: the study's "TypeScript · UTF-8" strip.
                .child(h_flex().h(px(28.)).flex_shrink_0().px(px(12.)).gap(px(12.)).bg(theme.panel).border_b_1().border_color(theme.line).text_size(px(11.)).text_color(theme.muted)
                    .child(format!("{language} · {lines} line{}", if lines == 1 { "" } else { "s" }))
                    .child(div().flex_1())
                    .children(self.turn_hint(cx)))
                .child(div().id("file-editor-text").relative().flex_1().min_h_0().w_full()
                .on_modifiers_changed(cx.listener(|this,event:&gpui::ModifiersChangedEvent,_,cx|this.alt_changed(event.modifiers.alt,cx)))
                // Before the editor: ⌥-click shows the line's turn instead of adding a cursor.
                .capture_any_mouse_down(cx.listener(|this,event:&gpui::MouseDownEvent,window,cx|{
                    if event.modifiers.alt && this.click_turn(event.position,window,cx) { cx.stop_propagation(); }
                }))
                .on_mouse_move(cx.listener(|this,event:&gpui::MouseMoveEvent,window,cx|this.pointer_moved(event.position,window,cx)))
                .on_hover(cx.listener(|this,hovered:&bool,_,cx|if !*hovered {this.leave_card(cx)}))
                .child(e.cached(gpui::StyleRefinement::default().size_full()))
                .children(self.card(cx))
                .children(self.turn_card(cx)))))
            // Comparing is Changes' job; the file area says where, not how.
            .when(observed, |v| {
                let path = relative.to_string_lossy().into_owned();
                v.child(h_flex().debug_selector(||"file-compare".into()).px(WORK_GUTTER).pt(px(10.)).text_size(px(12.)).text_color(theme.muted)
                    .child("Use\u{a0}")
                    .child(div().id("file-compare-changes").debug_selector(||"file-compare-changes".into()).text_color(theme.accent).cursor_pointer().hover(move |link| link.text_color(theme.text)).child("Changes")
                        .on_click(cx.listener(move |_,_,_,cx| cx.emit(FileEvent::Review(path.clone())))))
                    .child("\u{a0}to compare this file with the observed edit."))
            })
    }
}

/// `editor.fontSize`, from 8 to 32 px.
fn font_size(cx: &App) -> f32 {
    (crate::prefs::get(cx, "editor.fontSize", None)
        .as_f64()
        .unwrap_or(11.5) as f32)
        .clamp(8., 32.)
}

/// File tabs use the app's mono font; lines keep the 20 px they have at 11.5 px.
fn text_style(size: f32) -> gpui::TextStyleRefinement {
    gpui::TextStyleRefinement {
        font_family: Some(MONO.into()),
        font_size: Some(px(size).into()),
        line_height: Some(px((size * 20. / 11.5).round()).into()),
        ..Default::default()
    }
}
