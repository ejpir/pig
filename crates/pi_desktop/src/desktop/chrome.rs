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
    _subscriptions: Vec<gpui::Subscription>,
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
            cx.observe(&search, |_, _, cx| cx.notify()),
        ];
        Self {
            workspace,
            search,
            layout,
            search_open: false,
            _subscriptions: subscriptions,
        }
    }
    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        if self.layout != layout {
            self.layout = layout;
            cx.notify();
        }
    }
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
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

    fn search_palette(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let query = self.search.read(cx).content().trim().to_lowercase();
        let matches = |label: &str| query.is_empty() || label.to_lowercase().contains(&query);
        let mut actions = v_flex().gap(px(2.));
        if matches("New session") {
            actions = actions.child(
                search_row(
                    "search-new-session",
                    "plus",
                    "New session",
                    Some(
                        if cfg!(target_os = "macos") {
                            "⌘ N"
                        } else {
                            "Ctrl+N"
                        }
                        .into(),
                    ),
                    theme,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.dismiss_search(cx);
                    super::new_session::show(this.workspace.clone(), window, cx);
                })),
            );
        }
        if matches("Open folder") {
            actions = actions.child(
                search_row(
                    "search-open-folder",
                    "folder",
                    "Open folder…",
                    Some(
                        if cfg!(target_os = "macos") {
                            "⌘ O"
                        } else {
                            "Ctrl+O"
                        }
                        .into(),
                    ),
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.dismiss_search(cx);
                    this.workspace
                        .update(cx, |workspace, cx| workspace.open_folder(cx));
                })),
            );
        }
        for (index, view) in [
            super::app_views::AppView::Sessions,
            super::app_views::AppView::Models,
            super::app_views::AppView::Resources,
            super::app_views::AppView::Settings,
        ]
        .into_iter()
        .enumerate()
        {
            if matches(view.title()) {
                actions = actions.child(
                    search_row(
                        ("search-view", index),
                        view.icon(),
                        view.title(),
                        None,
                        theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dismiss_search(cx);
                        this.workspace
                            .update(cx, |workspace, cx| workspace.show_view(view, cx));
                    })),
                );
            }
        }
        let mut sessions: Vec<_> = self
            .workspace
            .read(cx)
            .summaries
            .iter()
            .filter(|(_, summary)| {
                query.is_empty()
                    || summary.title.to_lowercase().contains(&query)
                    || summary
                        .cwd
                        .display()
                        .to_string()
                        .to_lowercase()
                        .contains(&query)
            })
            .map(|(id, summary)| (*id, summary.title.clone(), summary.cwd.clone()))
            .collect();
        sessions.sort_by_key(|entry| entry.1.to_lowercase());
        let has_sessions = !sessions.is_empty();
        let mut session_rows = v_flex().gap(px(2.));
        for (id, title, cwd) in sessions.into_iter().take(6) {
            session_rows = session_rows.child(
                search_row(
                    ("search-session", id.0 as usize),
                    "chat",
                    title,
                    cwd.file_name()
                        .map(|name| name.to_string_lossy().into_owned()),
                    theme,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dismiss_search(cx);
                    this.workspace
                        .update(cx, |workspace, cx| workspace.select(id, cx));
                })),
            );
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
                        .overflow_y_scroll()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(theme.line)
                        .bg(theme.panel)
                        .shadow_lg()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_search(cx)))
                        .child(
                            h_flex()
                                .h(px(44.))
                                .px(px(14.))
                                .gap(px(10.))
                                .border_b_1()
                                .border_color(theme.line)
                                .child(icon("magnifying_glass", theme.faint))
                                .child(div().flex_1().min_w_0().child(self.search.clone()))
                                .child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(px(10.))
                                        .text_color(theme.faint)
                                        .child("ESC"),
                                ),
                        )
                        .child(
                            label("QUICK ACTIONS", theme)
                                .mx(px(14.))
                                .mt(px(10.))
                                .mb(px(4.)),
                        )
                        .child(actions.mx(px(8.)))
                        .when(has_sessions, |palette| {
                            palette
                                .child(divider(theme).mx(px(14.)).mt(px(8.)))
                                .child(
                                    label("OPEN SESSIONS", theme)
                                        .mx(px(14.))
                                        .mt(px(7.))
                                        .mb(px(4.)),
                                )
                                .child(session_rows.mx(px(8.)).mb(px(8.)))
                        }),
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
    theme: Theme,
) -> Stateful<Div> {
    let title: SharedString = title.into();
    h_flex()
        .id(id)
        .role(Role::Button)
        .h(px(32.))
        .px(px(8.))
        .gap(px(10.))
        .rounded(px(5.))
        .cursor_pointer()
        .hover(move |row| row.bg(theme.hover))
        .child(icon(glyph, theme.muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(13.))
                .child(title),
        )
        .when_some(detail, |row, detail| {
            row.child(
                div()
                    .font_family(MONO)
                    .text_size(px(10.))
                    .text_color(theme.faint)
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
        let count = workspace
            .tabs
            .iter()
            .filter(|tab| {
                let controller = tab.controller.read(cx);
                controller.is_connected() && controller.pid().is_some()
            })
            .count();
        let connection = if open.is_demo() {
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
                    .when_some(model.turn_metrics.clone(), |row, metrics| {
                        row.tooltip(ui::Tooltip::text(format!(
                            "Pi extension · last reported run statistics\n{metrics}"
                        )))
                        .child(metrics)
                    }),
            )
            .child(div().min_w_0().truncate().child(format!(
                "{terminals}{} · {} observed edits · {cost} · {}",
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
                },
                model.changed_files().len(),
                model
                    .stats
                    .context_usage
                    .as_ref()
                    .and_then(|usage| usage.percent)
                    .map(|percent| format!("{percent:.0}% context"))
                    .unwrap_or_else(|| "context not reported".into())
            )))
            .when(self.processes_open, |bar| {
                bar.child(self.processes(window, cx, theme))
            })
            .into_any_element()
    }
}
