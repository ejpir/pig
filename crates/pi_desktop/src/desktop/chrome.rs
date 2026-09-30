use super::workspace::{WorkspaceController, WorkspaceEvent};
use super::*;
use gpui::EventEmitter;
use gpui::{Role, WindowControlArea};

#[derive(Clone, Copy)]
pub enum ShellEvent {
    Sidebar,
    Inspector,
    Theme,
    ResourcesInstall,
    ResourcesRefresh,
}
pub struct HeaderView {
    workspace: Entity<WorkspaceController>,
    search: Entity<TextInput>,
    layout: Layout,
    _subscription: gpui::Subscription,
}
impl EventEmitter<ShellEvent> for HeaderView {}
impl HeaderView {
    pub fn new(
        workspace: Entity<WorkspaceController>,
        search: Entity<TextInput>,
        layout: Layout,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&workspace, |_, workspace, event, cx| {
            if matches!(event, WorkspaceEvent::Selection(_) | WorkspaceEvent::View)
                || matches!(event, WorkspaceEvent::Summary(id) if *id == workspace.read(cx).active)
            {
                cx.notify();
            }
        });
        Self {
            workspace,
            search,
            layout,
            _subscription: subscription,
        }
    }
    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        if self.layout != layout {
            self.layout = layout;
            cx.notify();
        }
    }
}
impl Render for HeaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.header(
            self.layout.show_inspector(window.viewport_size().width),
            window.viewport_size().width < px(1150.),
            cx,
            theme(cx),
        )
    }
}

impl HeaderView {
    pub(super) fn header(
        &self,
        show_inspector: bool,
        compact: bool,
        cx: &Context<Self>,
        theme: Theme,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .h(px(52.))
            .flex_shrink_0()
            .bg(theme.bar)
            .border_b_1()
            .border_color(theme.edge)
            .child(
                h_flex()
                    // Aligned with the sidebar when it shows; just the brand when hidden.
                    .when(self.layout.show_sidebar, |brand| {
                        brand.w(self.layout.sidebar_width)
                    })
                    .h_full()
                    .flex_shrink_0()
                    .pl(px(93.))
                    .pr(px(10.))
                    .gap(px(9.))
                    .window_control_area(WindowControlArea::Drag)
                    .child(brand_mark())
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("pi"),
                    )
                    .child(div().flex_1().h_full())
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
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .pl(px(24.))
                    .pr(px(16.))
                    .gap(px(16.))
                    .when_some(self.workspace.read(cx).view, |bar, view| {
                        bar.child(
                            h_flex()
                                .w(px(164.))
                                .flex_shrink_0()
                                .gap(px(9.))
                                .debug_selector(|| "app-view-title".into())
                                .child(icon(view.icon(), theme.muted).size(px(18.)))
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(view.title()),
                                ),
                        )
                    })
                    .when(self.workspace.read(cx).view.is_none(), |bar| {
                        bar.child(
                            h_flex()
                                .w(px(164.))
                                .flex_shrink_0()
                                .gap(px(9.))
                                .child(icon("thread", theme.muted).size(px(18.)))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(
                                            self.workspace
                                                .read(cx)
                                                .summaries
                                                .get(&self.workspace.read(cx).active)
                                                .map(|s| s.title.clone())
                                                .or_else(|| {
                                                    self.workspace
                                                        .read(cx)
                                                        .selected_project
                                                        .as_ref()
                                                        .map(|path| {
                                                            path.file_name()
                                                                .unwrap_or(path.as_os_str())
                                                                .to_string_lossy()
                                                                .into_owned()
                                                        })
                                                })
                                                .unwrap_or_else(|| "No project open".into()),
                                        ),
                                )
                                .child(icon("chevron_down", theme.faint).size(px(12.))),
                        )
                    })
                    .child(div().w(px(1.)).h(px(23.)).bg(theme.chip_line))
                    .child(
                        h_flex()
                            .id("new-session")
                            .when(
                                self.workspace.read(cx).view.is_some_and(|view| {
                                    !matches!(view, super::app_views::AppView::Sessions | super::app_views::AppView::Resources)
                                }),
                                |new| new.invisible(),
                            )
                            .role(Role::Button)
                            .aria_label(if self.workspace.read(cx).view == Some(super::app_views::AppView::Resources) { "Install package" } else { "New session" })
                            .tooltip(ui::Tooltip::text(
                                self.workspace
                                    .read(cx)
                                    .selected_project
                                    .as_ref()
                                    .map(|p| format!("New session in {}", p.display()))
                                    .unwrap_or_else(|| "Open a project to start a session".into()),
                            ))
                            .h(px(27.))
                            .px(px(9.))
                            .gap(px(6.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.chip_line)
                            .bg(theme.chip)
                            .cursor_pointer()
                            .hover(move |style| style.bg(theme.hover))
                            .text_size(px(12.))
                            .text_color(theme.secondary)
                            .child(icon("plus", theme.secondary))
                            .child(if self.workspace.read(cx).view == Some(super::app_views::AppView::Resources) { "Install…" } else { "New" })
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.workspace.read(cx).view == Some(super::app_views::AppView::Resources) { cx.emit(ShellEvent::ResourcesInstall); }
                                else { super::new_session::show(this.workspace.clone(), window, cx); }
                            })),
                    )
                    .when(self.workspace.read(cx).view == Some(super::app_views::AppView::Resources), |bar| bar.child(
                        icon_button("refresh-resources", "refresh", "Refresh resource metadata", theme)
                            .tooltip(ui::Tooltip::text("Refresh resource metadata · does not reload or execute extensions"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ShellEvent::ResourcesRefresh))),
                    ))
                    .child(
                        h_flex()
                            .gap(px(16.))
                            .text_color(theme.faint)
                            // Session controls; an app view has none.
                            .when(self.workspace.read(cx).view.is_some(), |icons| {
                                icons.invisible()
                            })
                            .child(icon("git_branch", theme.faint).size(px(18.)))
                            .child(icon("compact", theme.faint).size(px(18.)))
                            .child(
                                icon_button(
                                    "open-diagnostics",
                                    "ellipsis",
                                    "Session diagnostics",
                                    theme,
                                )
                                .tooltip(|_, cx| {
                                    ui::Tooltip::for_action(
                                        "Session diagnostics",
                                        &super::ShowDiagnostics,
                                        cx,
                                    )
                                })
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(super::ShowDiagnostics), cx)
                                }),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .window_control_area(WindowControlArea::Drag),
                    )
                    .child(
                        h_flex()
                            .w(px(268.))
                            .min_w(px(130.))
                            .h(px(28.))
                            .px(px(9.))
                            .gap(px(8.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.chip_line)
                            .bg(theme.canvas)
                            .child(icon("magnifying_glass", theme.faint))
                            .child(div().flex_1().min_w_0().child(self.search.clone()))
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(10.))
                                    .text_color(theme.faint)
                                    .child(if cfg!(target_os = "macos") {
                                        "⌘ K"
                                    } else {
                                        "⌃ K"
                                    }),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .w(if show_inspector && !compact {
                        self.layout.inspector_width
                    } else {
                        px(92.)
                    })
                    .flex_shrink_0()
                    .h_full()
                    .px(px(20.))
                    .gap(px(12.))
                    .when(show_inspector && !compact, |row| {
                        row.child(label("INSPECTOR", theme).text_color(theme.muted))
                    })
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .window_control_area(WindowControlArea::Drag),
                    )
                    .child(
                        h_flex()
                            .id("theme")
                            .role(Role::Button)
                            .aria_label("Toggle Evening and Moonstone")
                            .size(px(24.))
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(move |style| style.bg(theme.hover))
                            .child(
                                div()
                                    .relative()
                                    .size(px(13.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(theme.muted)
                                    .overflow_hidden()
                                    .child(div().h_full().w(px(6.)).bg(theme.muted)),
                            )
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ShellEvent::Theme))),
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
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(ShellEvent::Inspector);
                        })),
                    ),
            )
    }
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
