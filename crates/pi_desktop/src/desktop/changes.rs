//! Immutable per-turn review, with a separate incomplete projection for legacy
//! tool edits, and the jj operation log (design study 05, 11). No repository
//! reads, mutations, or current-file guesses in render: operations load off
//! the UI thread when their tab shows.
use super::*;
use super::{
    files::FilesView,
    panels::*,
    session::{Changes, SessionController, SessionEvent},
};
mod data;
mod view;
use data::File;

/// A turn-local, factual file summary. Only the last assistant row owns it;
/// stored snapshots take precedence over their associated tool calls.
pub(super) fn thread_files(controller: &SessionController, row: usize) -> Vec<File> {
    let model = controller.model();
    if model
        .messages
        .get(row)
        .is_none_or(|m| m["role"] != "assistant")
    {
        return vec![];
    }
    if model
        .messages
        .iter()
        .skip(row + 1)
        .take_while(|m| m["role"] != "user")
        .any(|m| m["role"] == "assistant")
    {
        return vec![];
    }
    let start = model.messages[..row]
        .iter()
        .rposition(|m| m["role"] == "user")
        .unwrap_or(0);
    let ids: HashSet<_> = model.messages[start..=row]
        .iter()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "toolCall")
        .filter_map(|b| b["id"].as_str())
        .collect();
    let records: Vec<_> = controller
        .jj()
        .records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.anchored && r.after_message >= start && r.after_message <= row)
        .collect();
    let recorded = records
        .iter()
        .flat_map(|(_, r)| r.tool_ids.iter().cloned())
        .collect();
    let mut files: Vec<_> = records
        .iter()
        .flat_map(|(index, r)| r.diff.iter().map(|f| File::recorded(f, *index, r)))
        .collect();
    files.extend(data::observed_tools(
        model.tools.iter().filter(|t| ids.contains(t.id.as_str())),
        &recorded,
        &model.cwd,
    ));
    files
}
use gpui::EventEmitter;

#[derive(Clone, Debug)]
pub(super) enum ChangesEvent {
    RequestRevision { path: String, source: String },
}
impl EventEmitter<ChangesEvent> for ChangesView {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Turns,
    Operations,
}

#[derive(Clone, PartialEq, Eq)]
enum Row {
    Turn(usize),
    File(usize),
    Observed,
}

pub struct ChangesView {
    controller: Entity<SessionController>,
    editors: Entity<FilesView>,
    composer: Entity<super::composer::ComposerView>,
    files: Vec<File>,
    rows: Vec<Row>,
    selected: Option<usize>,
    document: Entity<DocumentView>,
    diff: Entity<super::diff::DiffView>,
    wide: bool,
    picker_open: bool,
    focus: gpui::FocusHandle,
    file_list: gpui::ListState,
    scroll: ScrollHandle,
    file_scroll: gpui::UniformListScrollHandle,
    tab: Tab,
    operations: Vec<pi_jj::OperationInfo>,
    operation: Option<usize>,
    /// What restoring to the selected operation would change.
    operation_files: Option<(pi_jj::OperationId, Vec<pi_jj::FileChange>)>,
    _load: Option<gpui::Task<()>>,
    _subscription: gpui::Subscription,
}
impl ChangesView {
    pub fn new(
        controller: Entity<SessionController>,
        editors: Entity<FilesView>,
        composer: Entity<super::composer::ComposerView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&controller, |this,_,event,cx| {
            if matches!(event, SessionEvent::Content(super::session::ContentChange::Reset | super::session::ContentChange::Tool(_)))
                || matches!(event,SessionEvent::Changed(c) if c.intersects(Changes::JJ|Changes::METADATA|Changes::HISTORY)) {this.refresh(cx);}
            if this.tab == Tab::Operations && matches!(event,SessionEvent::Changed(c) if c.intersects(Changes::JJ)) && this.controller.read(cx).jj_idle() {this.load_operations(cx);}
        });
        let mut this = Self {
            controller,
            editors,
            composer,
            files: vec![],
            rows: vec![],
            selected: None,
            document: cx.new(|cx| DocumentView::new(cx).review().wrapped()),
            diff: cx.new(super::diff::DiffView::new),
            wide: true,
            picker_open: false,
            focus: cx.focus_handle(),
            file_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(100.)),
            scroll: ScrollHandle::new(),
            file_scroll: Default::default(),
            tab: Tab::Turns,
            operations: Vec::new(),
            operation: None,
            operation_files: None,
            _load: None,
            _subscription: subscription,
        };
        this.refresh(cx);
        this
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let key = self.file().map(|f| (f.turn, f.path.clone()));
        let controller = self.controller.read(cx);
        let records = &controller.jj().records;
        let previous_files = std::mem::take(&mut self.files);
        let previous_rows = std::mem::take(&mut self.rows);
        for (turn, record) in records.iter().enumerate().rev() {
            self.rows.push(Row::Turn(turn));
            for file in &record.diff {
                self.rows.push(Row::File(self.files.len()));
                self.files.push(File::recorded(file, turn, record));
            }
        }
        let recorded = records
            .iter()
            .flat_map(|r| r.tool_ids.iter().cloned())
            .collect();
        let fallback = data::observed(controller.model(), &recorded);
        if !fallback.is_empty() {
            if !self.rows.is_empty() {
                self.rows.push(Row::Observed);
            }
            for file in fallback {
                self.rows.push(Row::File(self.files.len()));
                self.files.push(file);
            }
        }
        if self.files == previous_files && self.rows == previous_rows {
            return;
        }
        self.selected = key
            .and_then(|key| {
                self.files
                    .iter()
                    .position(|f| (f.turn, f.path.clone()) == key)
            })
            .or_else(|| (!self.files.is_empty()).then_some(0));
        self.file_list.reset(self.rows.len());
        self.sync_document(cx);
        cx.notify();
    }
    pub fn shows_composer(&self) -> bool {
        self.tab == Tab::Turns
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }
    fn move_file(&mut self, direction: isize, cx: &mut Context<Self>) {
        if self.files.is_empty() {
            return;
        }
        let index = (self.selected.unwrap_or(0) as isize + direction)
            .rem_euclid(self.files.len() as isize) as usize;
        self.select(index, cx);
        if let Some(row) = self
            .rows
            .iter()
            .position(|r| matches!(r,Row::File(i) if *i==index))
        {
            self.file_list.scroll_to_reveal_item(row);
        }
    }
    pub fn set_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        let wide = width >= px(900.);
        if self.wide != wide {
            self.wide = wide;
            self.picker_open = false;
            cx.notify();
        }
        self.diff
            .update(cx, |diff, cx| diff.set_wide(width >= px(1040.), cx));
    }
    pub fn select_path(&mut self, path: &str, turn: Option<usize>, cx: &mut Context<Self>) {
        if let Some(index) = self
            .files
            .iter()
            .position(|file| file.path == path && file.turn == turn)
        {
            self.select(index, cx);
        }
    }
    fn file(&self) -> Option<&File> {
        self.selected.and_then(|i| self.files.get(i))
    }
    fn sync_document(&self, cx: &mut Context<Self>) {
        let text = self.file().map(|f| f.preview.clone()).unwrap_or_default();
        let sections = self.file().map(|f| f.sections.clone()).unwrap_or_default();
        self.diff
            .update(cx, |diff, cx| diff.set(text, &sections, cx));
    }
    pub fn select_turn(&mut self, turn: usize, cx: &mut Context<Self>) {
        if let Some(i) = self.files.iter().position(|f| f.turn == Some(turn)) {
            self.select(i, cx);
        }
    }
    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.files.len() {
            self.selected = Some(index);
            self.picker_open = false;
            self.sync_document(cx);
            self.scroll.set_offset(gpui::point(px(0.), px(0.)));
            cx.notify();
        }
    }
    fn request_revision(&self, cx: &mut Context<Self>) {
        if let Some(file) = self.file() {
            let source = self
                .diff
                .read(cx)
                .selected_source(cx)
                .filter(|selection| !selection.trim().is_empty())
                .map(|selection| format!("{}\n\nSelected lines:\n{selection}", file.source))
                .unwrap_or_else(|| file.source.clone());
            cx.emit(ChangesEvent::RequestRevision {
                path: file.path.clone(),
                source,
            });
        }
    }
    fn open(&mut self, cx: &mut Context<Self>) {
        if let Some(file) = self.file() {
            let c = self.controller.read(cx);
            let root = if file.turn.is_some() {
                c.jj().root.as_ref().unwrap_or(&c.model().cwd)
            } else {
                &c.model().cwd
            };
            let path = root.join(&file.path);
            self.editors.update(cx, |view, cx| view.open(path, cx));
        }
    }
    fn undo(&mut self, cx: &mut Context<Self>) {
        if !self.turn_action(cx).1 {
            return;
        }
        if let Some(turn) = self.file().and_then(|f| f.turn) {
            self.controller.update(cx, |c, cx| {
                if c.jj().records[turn].undone.is_some() {
                    c.redo_turn(turn, cx);
                } else {
                    c.undo_turn(turn, cx);
                }
            });
        }
    }
    /// Whether the selected file can go back to before its turn.
    fn can_restore(&self, cx: &App) -> bool {
        let c = self.controller.read(cx);
        self.file()
            .and_then(|f| f.turn)
            .and_then(|i| c.jj().records.get(i))
            .is_some_and(|r| r.undone.is_none())
            && c.jj().project.is_some()
            && c.jj_idle()
            && !pi_editor::has_unsaved_buffers(cx)
    }
    /// Restore one file (design study 05, 02), after a confirmation.
    fn restore_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_restore(cx) {
            return;
        }
        let Some((turn, path)) = self.file().and_then(|f| Some((f.turn?, f.path.clone()))) else {
            return;
        };
        let short = self.controller.read(cx).jj().records[turn].short.clone();
        let name = PathBuf::from(&path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let answer = window.prompt(
            gpui::PromptLevel::Info,
            "Restore this file?",
            Some(&format!(
                "{name} goes back to how it was before turn {short}. A later turn that edits it too blocks this. Operations can undo it."
            )),
            &[
                gpui::PromptButton::cancel("Cancel"),
                gpui::PromptButton::ok("Restore file"),
            ],
            cx,
        );
        let controller = self.controller.clone();
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(1) {
                controller.update(cx, |c, cx| c.restore_file(turn, path, cx));
            }
        })
        .detach();
    }
    /// Bring this workspace's turns into the main folder, after a confirmation.
    fn bring_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(main) = self
            .controller
            .read(cx)
            .main_folder()
            .map(|main| main.display().to_string())
        else {
            return;
        };
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Bring this session's turns into {main}?"),
            Some("They go on top of that folder's latest turn, and its files gain their edits; edits not yet in a turn there stay on top. Where both edit the same lines, jj keeps a conflict and the files get conflict markers. Operations in that folder can undo it."),
            &[
                gpui::PromptButton::cancel("Cancel"),
                gpui::PromptButton::ok("Bring turns in"),
            ],
            cx,
        );
        let controller = self.controller.clone();
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(1) {
                controller.update(cx, |c, cx| c.bring_in_main(cx));
            }
        })
        .detach();
    }
    fn show_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        match tab {
            Tab::Operations => {
                self.operation_files = None;
                self.load_operations(cx);
            }
            Tab::Turns => self.sync_document(cx),
        }
        cx.notify();
    }
    fn load_operations(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.controller.read(cx).operations(cx) else {
            self.operations.clear();
            cx.notify();
            return;
        };
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(operations) => {
                        let selected = this
                            .operation
                            .and_then(|i| this.operations.get(i))
                            .map(|op| op.id.clone());
                        this.operations = operations;
                        this.operation = selected
                            .and_then(|id| this.operations.iter().position(|op| op.id == id))
                            .or_else(|| (!this.operations.is_empty()).then_some(0));
                        this.load_operation_files(cx);
                    }
                    Err(error) => {
                        this.controller.update(cx, |c, cx| {
                            c.notice(format!("jj: the operation log did not load: {error:#}"), cx)
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }
    fn select_operation(&mut self, index: usize, cx: &mut Context<Self>) {
        self.operation = Some(index);
        self.load_operation_files(cx);
        cx.notify();
    }
    fn load_operation_files(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .operation
            .and_then(|i| self.operations.get(i))
            .map(|op| op.id.clone())
        else {
            return;
        };
        if self
            .operation_files
            .as_ref()
            .is_some_and(|(known, _)| *known == id)
        {
            return;
        }
        let Some(task) = self.controller.read(cx).operation_files(id.clone(), cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let files = task.await.unwrap_or_default();
            this.update(cx, |this, cx| {
                let text: String = files.iter().map(data::patch).collect();
                this.operation_files = Some((id, files));
                if this.tab == Tab::Operations {
                    this.document
                        .update(cx, |d, cx| d.set(text, Some("diff"), cx));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn restore_operation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(op) = self.operation.and_then(|i| self.operations.get(i)).cloned() else {
            return;
        };
        let when = pi_core::clock::date_time(op.time.max(0) as u64);
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Restore the project to {when}?"),
            Some("Puts the project's files, turns and commits back as they were after this operation. Your current files are recorded first, and restoring is an operation too, so you can restore back."),
            &[
                gpui::PromptButton::cancel("Cancel"),
                gpui::PromptButton::ok("Restore project"),
            ],
            cx,
        );
        let controller = self.controller.clone();
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(1) {
                controller.update(cx, |c, cx| c.restore_operation(op.id, cx));
            }
        })
        .detach();
    }
    fn operation_row(&self, i: usize, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let op = &self.operations[i];
        let selected = self.operation == Some(i);
        let (icon_name, who) = match op.kind {
            pi_jj::OperationKind::Pi => ("git_commit", "pi"),
            pi_jj::OperationKind::Snapshot => ("file", "files"),
            pi_jj::OperationKind::Git => ("branch", "git"),
            pi_jj::OperationKind::Other => ("history", "jj"),
        };
        let description = op
            .description
            .strip_prefix("pi: ")
            .unwrap_or(&op.description);
        let description = match op.kind {
            pi_jj::OperationKind::Snapshot => "Files recorded".to_owned(),
            _ => {
                let mut text = description.to_owned();
                if let Some(first) = text.get(..1) {
                    text.replace_range(..1, &first.to_uppercase());
                }
                text
            }
        };
        h_flex()
            .id(("operation", i))
            .debug_selector(move || format!("operation-{i}"))
            .h(px(28.))
            .w_full()
            .px(px(6.))
            .gap(px(6.))
            .rounded(px(4.))
            .cursor_pointer()
            .when(selected, |v| v.bg(theme.selected))
            .hover(move |s| s.bg(theme.hover))
            .tooltip(ui::Tooltip::text(format!(
                "{} · {}",
                pi_core::clock::date_time(op.time.max(0) as u64),
                op.description
            )))
            .child(icon(icon_name, theme.muted).size(px(12.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .text_color(if selected {
                        theme.text
                    } else {
                        theme.secondary
                    })
                    .child(description),
            )
            .child(
                div()
                    .font_family(MONO)
                    .text_size(px(9.5))
                    .text_color(theme.faint)
                    .child(who),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select_operation(i, cx)))
            .into_any_element()
    }
    fn operations_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let Some(op) = self.operation.and_then(|i| self.operations.get(i)).cloned() else {
            return empty(
                "Operations",
                "Every jj operation in this project: turns, recorded files, git commits made outside, restores.",
                theme,
            )
            .into_any_element();
        };
        let when = pi_core::clock::date_time(op.time.max(0) as u64);
        let files = self
            .operation_files
            .as_ref()
            .filter(|(id, _)| *id == op.id)
            .map(|(_, files)| files.clone());
        let can = {
            let c = self.controller.read(cx);
            c.jj().project.is_some() && c.jj_idle() && !pi_editor::has_unsaved_buffers(cx)
        };
        v_flex()
            .p(px(20.))
            .min_h((window.viewport_size().height - px(76.)).max(px(400.)))
            .child(
                div()
                    .font_family(MONO)
                    .text_size(px(15.))
                    .line_height(px(22.))
                    .child(op.description.clone()),
            )
            .child(
                div()
                    .h(px(24.))
                    .text_size(px(11.5))
                    .text_color(theme.secondary)
                    .child(format!("{when} · {}", &pi_jj::ObjectId::hex(&op.id)[..12])),
            )
            .child(divider(theme))
            .child(section("RESTORING TO HERE", "", theme).mt(px(12.)))
            .child(match &files {
                None => note("Reading…", theme).into_any_element(),
                Some(files) if files.is_empty() => {
                    note("The files would stay as they are.", theme).into_any_element()
                }
                Some(files) => v_flex()
                    .mt(px(4.))
                    .children(files.iter().take(12).map(|file| {
                        h_flex()
                            .h(px(22.))
                            .gap(px(6.))
                            .child(icon("file", theme.faint).size(px(12.)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(px(10.5))
                                    .text_color(theme.secondary)
                                    .child(file.path.clone()),
                            )
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(10.))
                                    .text_color(theme.green)
                                    .child(format!("+{}", file.added)),
                            )
                            .when(file.removed > 0, |v| {
                                v.child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(px(10.))
                                        .text_color(theme.coral)
                                        .child(format!("−{}", file.removed)),
                                )
                            })
                    }))
                    .when(files.len() > 12, |v| {
                        v.child(note(format!("and {} more", files.len() - 12), theme))
                    })
                    .into_any_element(),
            })
            .child(
                primary_button(
                    "restore-operation",
                    format!("Restore project to {when}"),
                    can,
                    theme,
                )
                .justify_center()
                .mt(px(12.))
                .debug_selector(|| "restore-operation".into())
                .on_click(cx.listener(|this, _, window, cx| this.restore_operation(window, cx))),
            )
            .child(div().flex_1().min_h(px(24.)))
            .child(note(
                "Same as jj op restore. Restoring is an operation too, so you can restore back.",
                theme,
            ))
            .into_any_element()
    }
    fn turn_action(&self, cx: &App) -> (String, bool) {
        let c = self.controller.read(cx);
        let record = self
            .file()
            .and_then(|f| f.turn)
            .and_then(|i| c.jj().records.get(i));
        let title = record
            .map(|r| {
                format!(
                    "{} turn {}",
                    if r.undone.is_some() { "Redo" } else { "Undo" },
                    r.short
                )
            })
            .unwrap_or_else(|| "Undo unavailable · no snapshot".into());
        (
            title,
            record.is_some()
                && c.jj().project.is_some()
                && c.jj_idle()
                && !pi_editor::has_unsaved_buffers(cx),
        )
    }
    pub fn inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.tab == Tab::Operations {
            return self.operations_inspector(window, cx);
        }
        let theme = theme(cx);
        let Some(file) = self.file().cloned() else {
            return empty(
                "Changes",
                "Select a file to review its diff. Earlier work may not have been recorded.",
                theme,
            )
            .into_any_element();
        };
        let patch = file.patch.clone();
        let path = PathBuf::from(&file.path);
        let (action, enabled) = self.turn_action(cx);
        let restorable = self.can_restore(cx);
        let associations: Vec<_> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.path == file.path && f.turn.is_some())
            .map(|(i, f)| (i, f.source.clone(), f.added, f.removed))
            .collect();
        v_flex().p(px(20.)).min_h((window.viewport_size().height-px(76.)).max(px(400.)))
            .child(div().font_family(MONO).text_size(px(19.)).truncate().child(path.file_name().unwrap_or_default().to_string_lossy().into_owned()))
            .child(h_flex().h(px(28.)).justify_between().text_size(px(11.5)).text_color(theme.secondary)
                .child(format!("{} · +{} −{}",file.status,file.added,file.removed)).child(note(if file.turn.is_some(){"jj"}else{"No snapshot"},theme)))
            .child(section("FILE","",theme).mt(px(20.)))
            .child(div().font_family(MONO).text_size(px(11.)).line_height(px(18.)).mt(px(6.)).child(path.parent().unwrap_or(std::path::Path::new(".")).display().to_string()))
            .child(pair("Lines",if file.turn.is_some(){"Not stored in snapshot"}else{"Not reported"}.into(),theme))
            .child(pair("Language",super::diff_preview::language(&file.path).into(),theme))
            .child(divider(theme)).child(section("CHANGED BY","",theme).mt(px(12.)))
            .children(associations.into_iter().map(|(i,source,added,removed)|h_flex().id(("file-turn",i)).h(px(28.)).gap(px(6.)).cursor_pointer()
                .child(icon("git_commit",theme.faint).size(px(12.)))
                .child(div().flex_1().min_w_0().truncate().text_size(px(10.5)).text_color(theme.muted).child(source))
                .child(div().font_family(MONO).text_size(px(10.)).text_color(theme.green).child(format!("+{added}")))
                .when(removed>0,|v|v.child(div().font_family(MONO).text_size(px(10.)).text_color(theme.coral).child(format!("−{removed}"))))
                .on_click(cx.listener(move|this,_,_,cx|this.select(i,cx)))))
            .when(file.turn.is_none(),|v|v.child(note("Successful edit/write calls",theme)))
            .children(file.touches.into_iter().enumerate().map(|(index,(id,name))|h_flex().id(("change-tool",index))
                .debug_selector(move||format!("change-tool-{index}"))
                .role(gpui::Role::Button).w_full().min_w_0().h(px(26.)).mt(px(4.)).px(px(8.))
                .rounded(px(5.)).border_1().border_color(theme.chip_line).bg(theme.chip)
                .cursor_pointer().hover(move|v|v.bg(theme.hover))
                .tooltip(ui::Tooltip::text(format!("{name} · {id}")))
                .child(div().flex_1().min_w_0().truncate().text_size(px(11.)).child(format!("{name} · {}",data::short_call_id(&id))))
                .on_click(cx.listener(move|this,_,_,cx|this.controller.update(cx,|c,cx|c.reveal_tool(id.clone(),cx))))))
            .child(divider(theme)).child(section("ACTIONS","",theme).mt(px(12.)))
            .child(h_flex().gap(px(8.)).mt(px(6.))
                .child(primary_button("changes-open","Open in editor",true,theme).flex_1().justify_center().debug_selector(||"changes-open".into()).on_click(cx.listener(|this,_,_,cx|this.open(cx))))
                .child(primary_button("changes-restore","Restore file",restorable,theme).flex_1().justify_center().debug_selector(||"changes-restore".into())
                    .tooltip(ui::Tooltip::text("Put this file back as it was before its turn."))
                    .on_click(cx.listener(|this,_,window,cx|this.restore_file(window,cx)))))
            .child(primary_button("changes-undo",action,enabled,theme).justify_center().mt(px(8.)).on_click(cx.listener(|this,_,_,cx|this.undo(cx))))
            .child(button("changes-copy","Copy diff",theme).justify_center().mt(px(8.)).debug_selector(||"changes-copy".into()).on_click(move|_,_,cx|cx.write_to_clipboard(ClipboardItem::new_string(patch.clone()))))
            // Session usage lives in the Thread inspector; this one is about the file.
            .child(div().flex_1().min_h(px(24.)))
            .when(file.turn.is_some(),|v|v.child(note("Changes are what jj recorded, including shell/external edits during the turn. Earlier work may be unrecorded.",theme)))
            .child(note("Restore file and undo/redo refuse when a later turn edits the same lines; save unsaved editor buffers first.",theme).mt(px(10.)))
            .into_any_element()
    }
    fn row(&self, i: usize, cx: &Context<Self>, theme: Theme) -> AnyElement {
        match &self.rows[i] {
            Row::Observed => div()
                .child("Observed edits")
                .text_size(px(12.))
                .text_color(theme.muted)
                .py(px(8.))
                .h(px(32.))
                .into_any_element(),
            Row::Turn(index) => {
                let r = &self.controller.read(cx).jj().records[*index];
                h_flex()
                    .h(px(32.))
                    .gap(px(7.))
                    .px(px(4.))
                    .child(
                        icon(
                            "git_commit",
                            if r.undone.is_some() {
                                theme.faint
                            } else {
                                theme.accent
                            },
                        )
                        .size(px(12.)),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(10.))
                            .text_color(theme.faint)
                            .child(r.short.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme.muted)
                            .child(if r.undone.is_some() {
                                format!("Undone · {}", r.description)
                            } else {
                                r.description.clone()
                            }),
                    )
                    .into_any_element()
            }
            Row::File(index) => {
                let index = *index;
                let file = &self.files[index];
                let selected = self.selected == Some(index);
                div()
                    .h(px(76.))
                    .child(
                        review_file_row(
                            ("changed-file", index),
                            &file.path,
                            file.added,
                            file.removed,
                            selected,
                            theme,
                        )
                        .debug_selector(move || format!("changed-file-{index}"))
                        .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx))),
                    )
                    .into_any_element()
            }
        }
    }
}
impl ChangesView {
    /// The operation log retains its existing recorded-history presentation.
    fn operations_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let weak = cx.entity().downgrade();
        let count = self.operations.len();
        let rows = gpui::uniform_list("turn-files", count, move |range, _, cx| {
            weak.update(cx, |this, cx| {
                range
                    .map(|i| this.operation_row(i, cx, theme))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        })
        .track_scroll(&self.file_scroll)
        .h(px((count as f32 * 28.).min(
            (f32::from(window.viewport_size().height) - 180.).max(28.),
        )))
        .flex_shrink_0()
        .w_full();
        let tabs = work_button("changes-tab-turns", "← Files", theme)
            .debug_selector(|| "changes-tab-turns".into())
            .on_click(cx.listener(|this, _, _, cx| this.show_tab(Tab::Turns, cx)));
        let operation = self.operation.and_then(|i| self.operations.get(i));
        let operation_content = v_flex()
            .id("operations-scroll")
            .size_full()
            .overflow_y_scroll()
            .p(px(12.))
            .when_some(operation, |v, op| {
                let files = self
                    .operation_files
                    .as_ref()
                    .filter(|(id, _)| *id == op.id)
                    .map(|(_, files)| files.len());
                v.child(
                    div()
                        .font_family(MONO)
                        .text_size(px(11.5))
                        .px(px(8.))
                        .truncate()
                        .child(op.description.clone()),
                )
                .child(
                    div()
                        .h(px(26.))
                        .px(px(8.))
                        .text_size(px(10.5))
                        .text_color(theme.faint)
                        .child(format!(
                            "{} · {}",
                            pi_core::clock::date_time(op.time.max(0) as u64),
                            match files {
                                None => "reading…",
                                Some(0) => "restoring to here leaves the files as they are",
                                Some(_) => "restoring to here changes these files",
                            }
                        )),
                )
                .when(files.is_some_and(|files| files > 0), |v| {
                    v.child(
                        div()
                            .py(px(4.))
                            .bg(theme.deep)
                            .rounded(px(6.))
                            .child(self.document.clone()),
                    )
                })
            })
            .when(self.operations.is_empty(), |v| {
                v.child(empty(
                    "No operations yet",
                    "jj lists every operation in this project here.",
                    theme,
                ))
            });
        h_flex().debug_selector(|| "changes-view".into()).size_full().items_stretch().min_h_0()
            .child(v_flex().w(px(240.)).flex_shrink_0().border_r_1().border_color(theme.line)
                .p(px(8.)).gap(px(8.)).child(tabs).child(rows)
                .child(note("This project, all sessions: recorded files, turns, external Git commits and restores.",theme).px(px(8.))))
            .child(div().relative().flex_1().min_w_0().h_full().child(operation_content))
    }
}
