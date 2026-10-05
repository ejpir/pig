use super::workspace::{WorkspaceController, WorkspaceEvent};
use super::*;
use gpui::EventEmitter;
use gpui::{Div, ElementId, Role, Stateful, WindowControlArea, rgb};

#[derive(Clone, Copy)]
pub enum ShellEvent {
    Sidebar,
    Inspector,
}
pub struct HeaderView {
    workspace: Entity<WorkspaceController>,
    search: Entity<TextInput>,
    layout: Layout,
    search_open: bool,
    /// The highlighted result; Enter opens it. Resets as the query changes.
    selected: usize,
    _subscriptions: Vec<gpui::Subscription>,
}

/// One result in global search: an action, a view or an open session.
#[derive(Clone, Copy, PartialEq)]
enum Entry {
    NewSession,
    OpenFolder,
    View(super::app_views::AppView),
    Page(super::panels::SessionPage),
    Session(super::workspace::SessionId),
}
impl EventEmitter<ShellEvent> for HeaderView {}
impl HeaderView {
    pub fn new(
        workspace: Entity<WorkspaceController>,
        search: Entity<TextInput>,
        layout: Layout,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe(&workspace, |_, workspace, event, cx| {
                if matches!(event, WorkspaceEvent::Selection(_) | WorkspaceEvent::View)
                    || matches!(event, WorkspaceEvent::Summary(id) if *id == workspace.read(cx).active)
                {
                    cx.notify();
                }
            }),
            cx.observe(&search, |this: &mut Self, _, cx| {
                this.selected = 0;
                cx.notify()
            }),
        ];
        Self {
            workspace,
            search,
            layout,
            search_open: false,
            selected: 0,
            _subscriptions: subscriptions,
        }
    }
    pub(super) fn search_focused(&self, window: &Window, cx: &App) -> bool {
        self.search_open && self.search.focus_handle(cx).is_focused(window)
    }
    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        if self.layout != layout {
            self.layout = layout;
            cx.notify();
        }
    }
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        self.selected = 0;
        self.search.focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    pub fn dismiss_search(&mut self, cx: &mut Context<Self>) {
        self.search_open = false;
        self.search
            .update(cx, |search, cx| search.set_content("", cx));
        cx.notify();
    }
}
impl Render for HeaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.header(
            self.layout.show_inspector(window.viewport_size().width),
            window.viewport_size().width < px(1150.),
            window.is_window_active(),
            cx,
            theme(cx),
        )
    }
}

impl HeaderView {
    pub(super) fn header(
        &self,
        show_inspector: bool,
        _compact: bool,
        window_active: bool,
        cx: &Context<Self>,
        theme: Theme,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .h(super::HEADER_HEIGHT)
            .flex_shrink_0()
            .bg(theme.bar)
            .border_b_1()
            .border_color(theme.edge)
            .debug_selector(|| "desktop-header".into())
            .px(px(10.))
            .gap(px(6.))
            .when(cfg!(target_os = "macos"), |header| {
                header.child(
                    h_flex()
                        .gap(px(8.))
                        .mr(px(10.))
                        .child(
                            mac_window_button(
                                "mac-close",
                                "Close window",
                                if window_active { 0xff5f57 } else { 0xb9b5b2 },
                            )
                            .on_click(|_, window, _| window.remove_window()),
                        )
                        .child(
                            mac_window_button(
                                "mac-minimize",
                                "Minimize window",
                                if window_active { 0xfebc2e } else { 0xb9b5b2 },
                            )
                            .on_click(|_, window, _| window.minimize_window()),
                        )
                        .child(
                            mac_window_button(
                                "mac-zoom",
                                "Zoom window",
                                if window_active { 0x28c840 } else { 0xb9b5b2 },
                            )
                            .on_click(|_, window, _| window.zoom_window()),
                        ),
                )
            })
            .child(
                icon_button(
                    "toggle-sidebar",
                    if self.layout.show_sidebar {
                        "threads_sidebar_left_open"
                    } else {
                        "threads_sidebar_left_closed"
                    },
                    "Toggle sidebar",
                    theme,
                )
                .debug_selector(|| "toggle-sidebar".into())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ShellEvent::Sidebar))),
            )
            .child(
                icon_button("toggle-search", "magnifying_glass", "Search", theme)
                    .debug_selector(|| "toggle-search".into())
                    .tooltip(|_, cx| ui::Tooltip::for_action("Search", &super::FocusSearch, cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.focus_search(window, cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                icon_button("new-session", "plus", "New session", theme)
                    .debug_selector(|| "new-session".into())
                    .tooltip(|_, cx| ui::Tooltip::for_action("New session", &super::NewSession, cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        super::new_session::show(this.workspace.clone(), window, cx)
                    })),
            )
            .child(
                icon_button(
                    "toggle-inspector",
                    if show_inspector {
                        "threads_sidebar_right_open"
                    } else {
                        "threads_sidebar_right_closed"
                    },
                    "Toggle inspector",
                    theme,
                )
                .debug_selector(|| "toggle-inspector".into())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ShellEvent::Inspector))),
            )
            .when(self.search_open, |header| {
                header.child(self.search_palette(cx, theme))
            })
    }

    /// What the query finds, in order: actions and views, then open sessions.
    fn entries(&self, cx: &App) -> Vec<(Entry, &'static str, SharedString, Option<String>)> {
        use super::{app_views::AppView, panels::SessionPage};
        let query = self.search.read(cx).content().trim().to_lowercase();
        let matches = |label: &str| query.is_empty() || label.to_lowercase().contains(&query);
        let shortcut = |mac: &str, other: &str| {
            Some(
                if cfg!(target_os = "macos") {
                    mac
                } else {
                    other
                }
                .to_owned(),
            )
        };
        let workspace = self.workspace.read(cx);
        let mut entries = vec![];
        if matches("New session") {
            entries.push((
                Entry::NewSession,
                "plus",
                "New session".into(),
                shortcut("⌘ N", "Ctrl+N"),
            ));
        }
        if matches("Open folder") {
            entries.push((
                Entry::OpenFolder,
                "folder",
                "Open folder…".into(),
                shortcut("⌘ O", "Ctrl+O"),
            ));
        }
        for view in [
            AppView::Sessions,
            AppView::Models,
            AppView::Resources,
            AppView::Settings,
        ] {
            if matches(view.title()) {
                entries.push((Entry::View(view), view.icon(), view.title().into(), None));
            }
        }
        if workspace.active_tab_opt().is_some() {
            for (page, glyph, title) in [
                (SessionPage::Tree, "list_tree", "Session tree"),
                (SessionPage::Context, "info", "Context & compaction"),
            ] {
                if matches(title) {
                    entries.push((Entry::Page(page), glyph, title.into(), None));
                }
            }
        }
        let mut sessions: Vec<_> = workspace
            .summaries
            .iter()
            .filter(|(_, summary)| {
                matches(&summary.title)
                    || summary
                        .cwd
                        .display()
                        .to_string()
                        .to_lowercase()
                        .contains(&query)
            })
            .map(|(id, summary)| {
                (
                    Entry::Session(*id),
                    "thread",
                    SharedString::from(summary.title.clone()),
                    summary
                        .cwd
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned()),
                )
            })
            .collect();
        sessions.sort_by_key(|entry| entry.2.to_lowercase());
        entries.extend(sessions.into_iter().take(6));
        entries
    }

    pub(super) fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.entries(cx).len();
        if count > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
            cx.notify();
        }
    }

    pub(super) fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((entry, ..)) = self.entries(cx).get(self.selected).cloned() {
            self.open(entry, window, cx);
        }
    }

    fn open(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_search(cx);
        match entry {
            Entry::NewSession => super::new_session::show(self.workspace.clone(), window, cx),
            Entry::OpenFolder => self
                .workspace
                .update(cx, |workspace, cx| workspace.open_folder(cx)),
            Entry::View(view) => self
                .workspace
                .update(cx, |workspace, cx| workspace.show_view(view, cx)),
            Entry::Page(page) => self.workspace.update(cx, |ws, cx| {
                ws.view = None;
                cx.emit(WorkspaceEvent::View);
                if let Some(tab) = ws.active_tab_opt() {
                    tab.view
                        .clone()
                        .update(cx, |view, cx| view.set_page(page, cx));
                }
            }),
            Entry::Session(id) => self
                .workspace
                .update(cx, |workspace, cx| workspace.select(id, cx)),
        }
    }

    fn search_palette(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let query = self.search.read(cx).content().trim().to_owned();
        let entries = self.entries(cx);
        let empty = entries.is_empty();
        let selected = self.selected.min(entries.len().saturating_sub(1));
        let first_session = entries
            .iter()
            .position(|(entry, ..)| matches!(entry, Entry::Session(_)));
        let row = |i: usize,
                   (entry, glyph, title, detail): (
            Entry,
            &'static str,
            SharedString,
            Option<String>,
        )| {
            let id: ElementId = match entry {
                Entry::NewSession => "search-new-session".into(),
                Entry::OpenFolder => "search-open-folder".into(),
                Entry::View(view) => ("search-view", view as usize).into(),
                Entry::Page(page) => ("search-session-tool", page as usize).into(),
                Entry::Session(id) => ("search-session", id.0 as usize).into(),
            };
            search_row(id, glyph, title, detail, i == selected, theme)
                .on_click(cx.listener(move |this, _, window, cx| this.open(entry, window, cx)))
        };
        let mut body = v_flex()
            .debug_selector(|| "global-search-results".into())
            .px(px(8.))
            .pb(px(8.));
        for (i, entry) in entries.into_iter().enumerate() {
            if i == 0 && first_session != Some(0) {
                body = body.child(
                    label("Actions & views", theme)
                        .px(px(8.))
                        .pt(px(12.))
                        .pb(px(6.)),
                );
            }
            if Some(i) == first_session {
                body = body
                    .when(i > 0, |body| {
                        body.child(div().h(px(1.)).mx(px(8.)).mt(px(8.)).bg(theme.line))
                    })
                    .child(
                        label("Open sessions", theme)
                            .px(px(8.))
                            .pt(px(12.))
                            .pb(px(6.)),
                    );
            }
            body = body.child(row(i, entry));
        }
        gpui::deferred(
            h_flex()
                .absolute()
                .top(px(46.))
                .left_0()
                .right_0()
                .justify_center()
                .child(
                    v_flex()
                        .id("global-search")
                        .debug_selector(|| "global-search".into())
                        .occlude()
                        .w(px(600.))
                        .max_h(px(520.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(theme.line)
                        .bg(theme.composer)
                        .shadow_lg()
                        .overflow_hidden()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_search(cx)))
                        // Before the input's own line movement: arrows move the selection.
                        .capture_action(cx.listener(|this, _: &crate::input::Up, _, cx| {
                            this.move_selection(-1, cx);
                            cx.stop_propagation();
                        }))
                        .capture_action(cx.listener(|this, _: &crate::input::Down, _, cx| {
                            this.move_selection(1, cx);
                            cx.stop_propagation();
                        }))
                        .child(
                            h_flex()
                                .h(px(48.))
                                .flex_shrink_0()
                                .px(px(16.))
                                .gap(px(10.))
                                .border_b_1()
                                .border_color(theme.line)
                                .child(icon("magnifying_glass", theme.muted))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(15.))
                                        .child(self.search.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme.muted)
                                        .child("Esc"),
                                ),
                        )
                        .child(if empty {
                            // A plain answer, at the palette's usual size.
                            v_flex()
                                .debug_selector(|| "global-search-empty".into())
                                .h(px(240.))
                                .items_center()
                                .justify_center()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .text_size(px(17.))
                                        .text_color(theme.text)
                                        .child(format!("No matches for “{query}”")),
                                )
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(theme.muted)
                                        .child("Try a session, project or action name."),
                                )
                                .child(
                                    button("search-clear", "Clear search", theme)
                                        .mt(px(6.))
                                        .debug_selector(|| "search-clear".into())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.search.update(cx, |search, cx| {
                                                search.set_content("", cx)
                                            });
                                            this.search.focus_handle(cx).focus(window, cx);
                                        })),
                                )
                                .into_any_element()
                        } else {
                            div()
                                .id("global-search-scroll")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .child(body)
                                .into_any_element()
                        })
                        .child(
                            h_flex()
                                .h(px(34.))
                                .flex_shrink_0()
                                .px(px(16.))
                                .gap(px(14.))
                                .border_t_1()
                                .border_color(theme.line)
                                .text_size(px(11.5))
                                .text_color(theme.muted)
                                .child("↑↓ Move")
                                .child("Enter Open")
                                .child("Esc Close")
                                .child(div().flex_1())
                                .child("Global search"),
                        ),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }
}

fn mac_window_button(id: &'static str, description: &'static str, color: u32) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(description)
        .size(px(10.))
        .rounded_full()
        .bg(rgb(color))
        .cursor_pointer()
}

fn search_row(
    id: impl Into<ElementId>,
    glyph: &'static str,
    title: impl Into<SharedString>,
    detail: Option<String>,
    selected: bool,
    theme: Theme,
) -> Stateful<Div> {
    let title: SharedString = title.into();
    h_flex()
        .id(id)
        .role(Role::Button)
        .aria_selected(selected)
        .h(px(34.))
        .px(px(8.))
        .gap(px(10.))
        .rounded(px(6.))
        .cursor_pointer()
        .when(selected, |row| row.bg(theme.selected))
        .hover(move |row| row.bg(theme.hover))
        .child(icon(glyph, theme.muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(13.5))
                .child(title),
        )
        .when_some(detail, |row, detail| {
            row.child(
                div()
                    .font_family(MONO)
                    .text_size(px(11.))
                    .text_color(theme.muted)
                    .child(detail),
            )
        })
}

#[cfg(target_os = "macos")]
const TOGGLE_TERMINAL: &str = "⌘J";
#[cfg(not(target_os = "macos"))]
const TOGGLE_TERMINAL: &str = "Ctrl+J";

pub struct StatusBarView {
    workspace: Entity<WorkspaceController>,
    /// The selected session's terminal drawer, whose terminal count is shown.
    terminals: Option<(gpui::EntityId, gpui::Subscription)>,
    processes_open: bool,
    process_focus: gpui::FocusHandle,
    pub(super) process_scroll: ScrollHandle,
    previous_focus: Option<gpui::FocusHandle>,
    _subscription: gpui::Subscription,
}
impl StatusBarView {
    pub fn new(workspace: Entity<WorkspaceController>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&workspace, |this, workspace, event, cx| {
            if matches!(event, WorkspaceEvent::Selection(_)) {
                this.watch_terminals(cx);
            }
            if matches!(event, WorkspaceEvent::Status(id) if *id == workspace.read(cx).active)
                || matches!(
                    event,
                    WorkspaceEvent::Selection(_)
                        | WorkspaceEvent::Navigation
                        | WorkspaceEvent::Summary(_)
                )
            {
                cx.notify();
            }
        });
        let mut this = Self {
            workspace,
            terminals: None,
            processes_open: false,
            process_focus: cx.focus_handle(),
            process_scroll: ScrollHandle::new(),
            previous_focus: None,
            _subscription: subscription,
        };
        this.watch_terminals(cx);
        this
    }
    fn close_processes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.processes_open = false;
        if let Some(focus) = self.previous_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }
    fn processes(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        theme: Theme,
    ) -> gpui::AnyElement {
        let (tabs, active) = {
            let workspace = self.workspace.read(cx);
            (
                workspace
                    .tabs
                    .iter()
                    .map(|tab| (tab.id, tab.controller.clone()))
                    .collect::<Vec<_>>(),
                workspace.active,
            )
        };
        let popup_width = (f32::from(window.viewport_size().width) - 28.).clamp(240., 520.);
        let list_height = px((tabs.len() as f32 * 54.)
            .min((f32::from(window.viewport_size().height) - 190.).max(54.)));
        gpui::deferred(v_flex().id("pi-processes-popup").debug_selector(|| "pi-processes-popup".into())
            .occlude().absolute().bottom(px(32.)).left(px(14.))
            .w(px(popup_width))
            .max_h(window.viewport_size().height - px(72.))
            .p(px(14.)).gap(px(10.)).rounded(px(8.)).bg(theme.panel).text_color(theme.text)
            .border_1().border_color(theme.line).shadow_lg()
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                if event.position.y < window.viewport_size().height - px(24.) { this.close_processes(window, cx); }
            }))
            .child(h_flex().child(label("PI PROCESSES", theme)).child(div().flex_1())
                .child(icon_button("close-processes", "close", "Close process list", theme)
                    .on_click(cx.listener(|this, _, window, cx| this.close_processes(window, cx)))))
            .child(div().h(list_height).min_h_0().custom_scrollbars(
                super::scrollbar("pi-processes-scrollbar", &self.process_scroll, Some(theme.panel)),
                window, cx,
            ).child(v_flex().id("pi-processes-list").debug_selector(|| "pi-processes-list".into())
                .h(list_height).w(px(popup_width - 44.)).overflow_y_scroll().track_scroll(&self.process_scroll)
                .children(tabs.iter().map(|(id, entity)| {
                let id = *id;
                let controller = entity.read(cx);
                let model = controller.model();
                let title = model.title().split_whitespace().collect::<Vec<_>>().join(" ");
                let pid = if controller.is_connected() { controller.pid() } else { None };
                let state = if controller.is_demo() { "Demo" } else if !controller.is_connected() { "Disconnected" }
                    else if controller.connecting() { "Connecting" } else if controller.bootstrap_failed() { "Setup failed" }
                    else if controller.working() { "Working" } else { "Ready" };
                h_flex().id(("pi-process", id.0)).debug_selector(move || format!("pi-process-{}", id.0))
                    .w_full().h(px(54.)).flex_shrink_0().gap(px(10.)).p(px(8.)).rounded(px(5.)).cursor_pointer()
                    .when(id == active, |row| row.bg(theme.selected))
                    .hover(move |row| row.bg(theme.hover))
                    .tooltip(ui::Tooltip::text(format!("{}\n{}\n{}", model.title(), model.cwd.display(), state)))
                    .child(div().size(px(5.)).rounded_full().bg(if pid.is_some() { theme.green } else { theme.faint }))
                    .child(v_flex().flex_1().min_w_0().gap(px(3.))
                        .child(div().truncate().font_family(SANS).text_size(px(12.)).text_color(theme.text).child(title))
                        .child(div().truncate().text_size(px(10.)).text_color(theme.faint).child(model.cwd.display().to_string())))
                    .child(v_flex().items_end().gap(px(3.)).text_size(px(10.))
                        .child(state).child(pid.map(|pid| format!("PID {pid}")).unwrap_or_else(|| "No subprocess".into())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.processes_open = false; this.previous_focus = None;
                        this.workspace.update(cx, |workspace, cx| workspace.select(id, cx)); cx.notify();
                    }))
            }))))
            .child(div().text_size(px(10.)).line_height(px(16.)).text_color(theme.faint)
                .child("Memory is not reported. Idle processes stay open until their sessions are closed.")))
            .with_priority(2).into_any_element()
    }
    fn watch_terminals(&mut self, cx: &mut Context<Self>) {
        let drawer = self
            .workspace
            .read(cx)
            .active_tab_opt()
            .map(|tab| tab.view.read(cx).terminal.clone());
        let Some(drawer) = drawer else {
            self.terminals = None;
            return;
        };
        if self
            .terminals
            .as_ref()
            .is_some_and(|(id, _)| *id == drawer.entity_id())
        {
            return;
        }
        let subscription = cx.subscribe(&drawer, |_, _, event, cx| {
            if let super::terminal::DrawerEvent::Count = event {
                cx.notify();
            }
        });
        self.terminals = Some((drawer.entity_id(), subscription));
    }
}
impl Render for StatusBarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.workspace.read(cx);
        let theme = theme(cx);
        let Some(tab) = workspace.active_tab_opt() else {
            return h_flex()
                .h(px(24.))
                .w_full()
                .bg(theme.status)
                .border_t_1()
                .border_color(theme.edge)
                .px(px(14.))
                .font_family(MONO)
                .text_size(px(10.))
                .text_color(theme.faint)
                .child(if workspace.is_demo() {
                    "OFFLINE DEMO · no active sessions"
                } else {
                    "No active sessions · no Pi processes"
                })
                .into_any_element();
        };
        let open = tab.controller.read(cx);
        let model = open.model();
        let workbench_demo = model.state.session_id.as_deref() == Some("demo-workbench");
        let count = workspace
            .tabs
            .iter()
            .filter(|tab| {
                let controller = tab.controller.read(cx);
                controller.is_connected() && controller.pid().is_some()
            })
            .count();
        let connection = if workbench_demo {
            "Offline sample".to_owned()
        } else if open.is_demo() {
            "OFFLINE DEMO · sample data".to_owned()
        } else if !open.is_connected() {
            "Disconnected".into()
        } else if open.bootstrap_failed() {
            "Connection setup failed".into()
        } else if open.connecting() {
            "Connecting to pi…".into()
        } else {
            format!(
                "pi · rpc   ·   {count} process{}",
                if count == 1 { "" } else { "es" }
            )
        };
        let terminals = match tab.view.read(cx).terminal.read(cx).count() {
            0 => String::new(),
            1 => format!("1 terminal · {TOGGLE_TERMINAL} toggles · "),
            count => format!("{count} terminals · {TOGGLE_TERMINAL} toggles · "),
        };
        let cost = model
            .stats
            .cost
            .map(|cost| format!("${cost:.2}"))
            .unwrap_or_else(|| "—".into());
        let changed_files = model.changed_files().len();
        let changed = format!(
            "{changed_files} changed file{}",
            if changed_files == 1 { "" } else { "s" }
        );
        h_flex()
            .relative()
            .track_focus(&self.process_focus)
            .on_action(cx.listener(|this, _: &super::Stop, window, cx| {
                cx.stop_propagation();
                this.close_processes(window, cx);
            }))
            .h(px(24.))
            .w_full()
            .bg(theme.status)
            .border_t_1()
            .border_color(theme.edge)
            .px(px(14.))
            .gap(px(14.))
            .overflow_hidden()
            .font_family(MONO)
            .text_size(px(10.))
            .text_color(theme.faint)
            .child(
                h_flex()
                    .id("status-processes")
                    .debug_selector(|| "status-processes".into())
                    .cursor_pointer()
                    .aria_expanded(self.processes_open)
                    .tooltip(ui::Tooltip::text(
                        "Pi processes · click to inspect every open session",
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.processes_open {
                            this.close_processes(window, cx);
                        } else {
                            this.previous_focus = window.focused(cx);
                            this.processes_open = true;
                            this.process_focus.focus(window, cx);
                            cx.notify();
                        }
                    }))
                    .flex_shrink_0()
                    .gap(px(7.))
                    .child(
                        div()
                            .size(px(5.))
                            .rounded_full()
                            .bg(if open.is_connected() {
                                theme.green
                            } else {
                                theme.coral
                            }),
                    )
                    .child(connection),
            )
            .child(
                div()
                    .id("status-turn-metrics")
                    .debug_selector(|| "status-turn-metrics".into())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when_some(
                        model.turn_metrics.clone().filter(|_| !workbench_demo),
                        |row, metrics| {
                            row.tooltip(ui::Tooltip::text(format!(
                                "Pi extension · last reported run statistics\n{metrics}"
                            )))
                            .child(metrics)
                        },
                    ),
            )
            .child(div().min_w_0().truncate().child(if workbench_demo {
                "Ctrl K  ·  Search & commands".to_owned()
            } else {
                format!(
                    "{terminals}{}{changed} · {cost} · {}",
                    if workspace.view.is_none() && tab.view.read(cx).shows_work_status(cx) {
                        String::new()
                    } else {
                        format!(
                            "{} · ",
                            if !open.is_connected() {
                                "Disconnected"
                            } else if open.bootstrap_failed() {
                                "Setup incomplete"
                            } else if open.connecting() {
                                "Connecting"
                            } else if model.shell_running() {
                                "Shell running"
                            } else if open.working() && !model.busy() {
                                "Starting"
                            } else {
                                model.run_label()
                            }
                        )
                    },
                    model
                        .stats
                        .context_usage
                        .as_ref()
                        .and_then(|usage| usage.percent)
                        .map(|percent| format!("{percent:.0}% context"))
                        .unwrap_or_else(|| "context not reported".into())
                )
            }))
            .when(self.processes_open, |bar| {
                bar.child(self.processes(window, cx, theme))
            })
            .into_any_element()
    }
}
