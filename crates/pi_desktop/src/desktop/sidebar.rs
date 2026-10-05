//! Task-focused project/session navigation. Open sessions stay globally reachable;
//! project disclosure owns saved history and folder management.
use super::session::Summary;
use super::workspace::{SessionId, WorkspaceController, WorkspaceEvent};
use super::*;
use gpui::{Div, ElementId, Role, Stateful};
use std::path::Path;

const PROJECT_PREVIEW: usize = 2;
const TOOL_VIEWS: [(&str, &str, app_views::AppView); 4] = [
    ("All Sessions", "list_tree", app_views::AppView::Sessions),
    ("Models", "sparkle", app_views::AppView::Models),
    ("Resources", "box", app_views::AppView::Resources),
    ("Settings", "settings", app_views::AppView::Settings),
];
enum Row<'a> {
    Open(SessionId),
    Saved(&'a SavedSession),
}

pub struct SidebarView {
    pub workspace: Entity<WorkspaceController>,
    /// Local project/session filter, distinct from global commands/search.
    pub search: Entity<TextInput>,
    expanded_projects: HashSet<PathBuf>,
    collapsed_projects: HashSet<PathBuf>,
    unabridged_projects: HashSet<PathBuf>,
    open_expanded: bool,
    projects_expanded: bool,
    tools_open: bool,
    tools_index: usize,
    menu_focus: gpui::FocusHandle,
    previous_focus: Option<gpui::FocusHandle>,
    scroll: ScrollHandle,
    _subscriptions: Vec<gpui::Subscription>,
    #[cfg(test)]
    pub renders: usize,
}
impl SidebarView {
    pub fn new(workspace: Entity<WorkspaceController>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new("Search sessions", cx).compact());
        let expanded_projects = [workspace.read(cx).active_summary().cwd.clone()]
            .into_iter()
            .collect();
        let subscriptions = vec![
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.subscribe(&workspace, |this, workspace, event, cx| {
                if let WorkspaceEvent::Selection(_) = event
                    && let Some(summary) =
                        workspace.read(cx).summaries.get(&workspace.read(cx).active)
                    && !this.collapsed_projects.contains(&summary.cwd)
                {
                    this.expanded_projects.insert(summary.cwd.clone());
                }
                if !matches!(event, WorkspaceEvent::Status(_)) {
                    cx.notify();
                }
            }),
        ];
        Self {
            workspace,
            search,
            expanded_projects,
            collapsed_projects: HashSet::new(),
            unabridged_projects: HashSet::new(),
            open_expanded: true,
            projects_expanded: true,
            tools_open: false,
            tools_index: 0,
            menu_focus: cx.focus_handle(),
            previous_focus: None,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
            #[cfg(test)]
            renders: 0,
        }
    }
    fn project_rows<'a>(
        &self,
        project: &Path,
        saved: &[&'a SavedSession],
        query: &str,
        cx: &App,
    ) -> Vec<Row<'a>> {
        let ws = self.workspace.read(cx);
        saved
            .iter()
            .filter(|s| Path::new(&s.cwd) == project && s.title().to_lowercase().contains(query))
            .filter(|s| {
                !ws.summaries
                    .values()
                    .any(|open| open.file.as_deref() == Some(&s.path))
            })
            .map(|s| Row::Saved(s))
            .collect()
    }
    fn dismiss_tools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tools_open = false;
        if let Some(focus) = self.previous_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }
    fn select_tool(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_tools(window, cx);
        self.workspace
            .update(cx, |ws, cx| ws.show_view(TOOL_VIEWS[index].2, cx));
    }
    fn tools_menu(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        gpui::deferred(
            v_flex()
                .id("settings-tools-menu")
                .debug_selector(|| "settings-tools-menu".into())
                .absolute()
                .left(px(8.))
                .bottom(px(58.))
                .w(px(240.))
                .p(px(8.))
                .gap(px(2.))
                .occlude()
                .rounded(px(8.))
                .bg(theme.composer)
                .border_1()
                .border_color(theme.line)
                .shadow_lg()
                .on_mouse_down_out(
                    cx.listener(|this, _, window, cx| this.dismiss_tools(window, cx)),
                )
                .children(
                    TOOL_VIEWS
                        .into_iter()
                        .enumerate()
                        .map(|(index, (name, glyph, _))| {
                            work_menu_item(name, name, glyph, index == self.tools_index, theme)
                                .debug_selector(move || format!("nav-{name}"))
                                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                    if *hovered && this.tools_index != index {
                                        this.tools_index = index;
                                        cx.notify();
                                    }
                                }))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.select_tool(index, window, cx);
                                }))
                        }),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }
    pub(super) fn sidebar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        theme: Theme,
    ) -> impl IntoElement + use<> {
        let query = self.search.read(cx).content().to_lowercase();
        let ws = self.workspace.read(cx);
        let saved: Vec<_> = ws.saved.iter().collect();
        let mut open_rows = Vec::with_capacity(ws.summaries.len());
        if ws
            .summaries
            .get(&ws.active)
            .is_some_and(|summary| summary.title.to_lowercase().contains(&query))
        {
            open_rows.push(Row::Open(ws.active));
        }
        open_rows.extend(
            ws.summaries
                .iter()
                .filter(|(id, summary)| {
                    **id != ws.active && summary.title.to_lowercase().contains(&query)
                })
                .map(|(id, _)| Row::Open(*id)),
        );
        let mut list = v_flex()
            .id("sidebar-list")
            .debug_selector(|| "sidebar-list".into())
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                h_flex()
                    .id("sidebar-open-toggle")
                    .debug_selector(|| "sidebar-open-toggle".into())
                    .role(Role::Button)
                    .aria_expanded(self.open_expanded)
                    .h(px(28.))
                    .flex_shrink_0()
                    .px(px(8.))
                    .gap(px(7.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(theme.muted)
                    .child(
                        icon(
                            if self.open_expanded {
                                "chevron_down"
                            } else {
                                "chevron_right"
                            },
                            theme.faint,
                        )
                        .size(px(10.)),
                    )
                    .child("Open")
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(10.))
                            .text_color(theme.faint)
                            .child(ws.summaries.len().to_string()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_expanded = !this.open_expanded;
                        cx.notify();
                    })),
            );
        if self.open_expanded || !query.is_empty() {
            for row in &open_rows {
                list = list.child(self.session_row(row, true, cx, theme));
            }
        }
        list = list.child(
            h_flex()
                .justify_between()
                .flex_shrink_0()
                .mt(px(8.))
                .mb(px(8.))
                .child(
                    h_flex()
                        .id("sidebar-projects-toggle")
                        .debug_selector(|| "sidebar-projects-toggle".into())
                        .role(Role::Button)
                        .aria_expanded(self.projects_expanded)
                        .h(px(28.))
                        .px(px(8.))
                        .gap(px(7.))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(theme.muted)
                        .child(
                            icon(
                                if self.projects_expanded {
                                    "chevron_down"
                                } else {
                                    "chevron_right"
                                },
                                theme.faint,
                            )
                            .size(px(10.)),
                        )
                        .child("Projects")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.projects_expanded = !this.projects_expanded;
                            cx.notify();
                        })),
                )
                .child(
                    icon_button("sidebar-open-folder", "plus", "Open folder", theme).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.workspace.update(cx, |ws, cx| ws.open_folder(cx))
                        }),
                    ),
                ),
        );
        if self.projects_expanded || !query.is_empty() {
            for (index, project) in ws.projects.iter().cloned().enumerate() {
                let rows = self.project_rows(&project, &saved, &query, cx);
                if !query.is_empty() && rows.is_empty() {
                    continue;
                }
                let expanded = !query.is_empty() || self.expanded_projects.contains(&project);
                let running = ws.summaries.values().any(|s| s.cwd == project && s.busy);
                list = list.child(self.project_row(
                    index,
                    &project,
                    rows.len(),
                    running,
                    expanded,
                    cx,
                    theme,
                ));
                if !expanded {
                    continue;
                }
                let all = !query.is_empty() || self.unabridged_projects.contains(&project);
                let shown = if all {
                    rows.len()
                } else {
                    rows.len().min(PROJECT_PREVIEW)
                };
                for row in &rows[..shown] {
                    list = list.child(self.session_row(row, false, cx, theme));
                }
                if rows.len() > PROJECT_PREVIEW && query.is_empty() {
                    list = list.child(
                        div()
                            .id(("show-more", index))
                            .mt(px(7.))
                            .debug_selector(move || format!("show-more-{index}"))
                            .role(Role::Button)
                            .h(px(32.))
                            .pl(px(36.))
                            .text_size(px(12.))
                            .text_color(theme.muted)
                            .cursor_pointer()
                            .child(if all {
                                "Show fewer".into()
                            } else {
                                format!("{} saved sessions…", rows.len() - shown)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.unabridged_projects.remove(&project) {
                                    this.unabridged_projects.insert(project.clone());
                                }
                                cx.notify();
                            })),
                    );
                }
                list = list.child(div().h(px(12.)).flex_shrink_0());
            }
            list = list.child(
                h_flex()
                    .id("open-folder")
                    .mt(px(16.))
                    .role(Role::Button)
                    .h(px(36.))
                    .px(px(12.))
                    .gap(px(8.))
                    .text_size(px(13.))
                    .text_color(theme.muted)
                    .cursor_pointer()
                    .hover(move |v| v.bg(theme.hover))
                    .child(icon("folder_add", theme.muted))
                    .child("Open folder…")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.workspace.update(cx, |ws, cx| ws.open_folder(cx))
                    })),
            );
        }
        v_flex()
            .relative()
            .track_focus(&self.menu_focus)
            .size_full()
            .bg(theme.panel)
            .border_r_1()
            .border_color(theme.line)
            .p(px(8.))
            .on_action(cx.listener(|this, _: &PreviousChoice, _, cx| {
                if this.tools_open {
                    cx.stop_propagation();
                    this.tools_index = (this.tools_index + 3) % 4;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &NextChoice, _, cx| {
                if this.tools_open {
                    cx.stop_propagation();
                    this.tools_index = (this.tools_index + 1) % 4;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                if this.tools_open {
                    cx.stop_propagation();
                    this.select_tool(this.tools_index, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Stop, window, cx| {
                if this.tools_open {
                    cx.stop_propagation();
                    this.dismiss_tools(window, cx);
                }
            }))
            .child(div().flex_1().min_h_0().child(list).custom_scrollbars(
                super::scrollbar("sidebar-scrollbar", &self.scroll, Some(theme.panel)),
                window,
                cx,
            ))
            .child(divider(theme).mx(px(8.)).mb(px(8.)))
            .child(
                h_flex()
                    .id("settings-tools")
                    .debug_selector(|| "settings-tools".into())
                    .role(Role::Button)
                    .aria_expanded(self.tools_open)
                    .h(px(36.))
                    .px(px(12.))
                    .gap(px(8.))
                    .rounded(px(5.))
                    .text_size(px(13.))
                    .cursor_pointer()
                    .hover(move |v| v.bg(theme.hover))
                    .child(icon("settings", theme.muted))
                    .child("Settings & tools")
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.tools_open {
                            this.dismiss_tools(window, cx);
                        } else {
                            this.previous_focus = window.focused(cx);
                            this.tools_open = true;
                            this.menu_focus.focus(window, cx);
                            cx.notify();
                        }
                    })),
            )
            .when(self.tools_open, |v| v.child(self.tools_menu(cx, theme)))
    }
    #[allow(clippy::too_many_arguments)]
    fn project_row(
        &self,
        index: usize,
        project: &Path,
        _count: usize,
        running: bool,
        expanded: bool,
        cx: &Context<Self>,
        theme: Theme,
    ) -> impl IntoElement {
        let name = project
            .file_name()
            .unwrap_or(project.as_os_str())
            .to_string_lossy()
            .into_owned();
        let toggle = project.to_owned();
        let selected = self.workspace.read(cx).selected_project.as_deref() == Some(project);
        let group = SharedString::from(format!("project-{index}"));
        h_flex()
            .id(("project", index))
            .group(group.clone())
            .relative()
            .debug_selector(move || format!("project-{index}"))
            .role(Role::Button)
            .aria_label(format!("Project {name}"))
            .aria_expanded(expanded)
            .h(px(36.))
            .flex_shrink_0()
            .px(px(8.))
            .gap(px(8.))
            .rounded(px(5.))
            .cursor_pointer()
            .hover(move |v| v.bg(theme.hover))
            .child(
                icon(
                    if expanded {
                        "chevron_down"
                    } else {
                        "chevron_right"
                    },
                    theme.muted,
                )
                .size(px(10.)),
            )
            .child(icon("folder", theme.muted).size(px(14.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .when(selected, |v| v.font_weight(FontWeight::SEMIBOLD))
                    .child(name),
            )
            .when(running, |v| {
                v.child(spinner(("project-running", index), theme))
            })
            .child(self.close_button(
                group,
                workspace::CloseTarget::Project(project.to_owned()),
                "Remove project from sidebar",
                cx,
                theme,
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.workspace
                    .update(cx, |ws, cx| ws.select_project(toggle.clone(), cx));
                if this.expanded_projects.remove(&toggle) {
                    this.collapsed_projects.insert(toggle.clone());
                } else {
                    this.collapsed_projects.remove(&toggle);
                    this.expanded_projects.insert(toggle.clone());
                }
                cx.notify();
            }))
    }
    fn close_button(
        &self,
        group: SharedString,
        target: workspace::CloseTarget,
        title: &'static str,
        cx: &Context<Self>,
        theme: Theme,
    ) -> impl IntoElement {
        let selector = format!("close-{group}");
        icon_button(SharedString::from(selector.clone()), "close", title, theme)
            .debug_selector(move || selector)
            .absolute()
            .right(px(3.))
            .top(px(2.))
            .bg(theme.panel)
            .invisible()
            .group_hover(group, |v| v.visible())
            .tooltip(ui::Tooltip::text(title))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.workspace
                    .update(cx, |ws, cx| ws.request_close(target.clone(), window, cx));
            }))
    }
    fn session_row(
        &self,
        row: &Row,
        show_project: bool,
        cx: &Context<Self>,
        theme: Theme,
    ) -> AnyElement {
        let ws = self.workspace.read(cx);
        let (id, selected, title, project, busy, target) = match row {
            Row::Open(id) => {
                let s: &Summary = &ws.summaries[id];
                (
                    format!("open-session-{}", id.0),
                    *id == ws.active && ws.view.is_none(),
                    s.title.clone(),
                    s.cwd
                        .file_name()
                        .unwrap_or(s.cwd.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                    s.busy,
                    workspace::CloseTarget::Session(*id),
                )
            }
            Row::Saved(s) => (
                format!("saved-session-{}", s.id),
                false,
                s.title().to_owned(),
                Path::new(&s.cwd)
                    .file_name()
                    .unwrap_or_else(|| Path::new(&s.cwd).as_os_str())
                    .to_string_lossy()
                    .into_owned(),
                false,
                workspace::CloseTarget::Saved(s.path.clone()),
            ),
        };
        let group = SharedString::from(id.clone());
        let action = match row {
            Row::Open(id) => Ok(*id),
            Row::Saved(s) => Err((*s).clone()),
        };
        selectable_row(SharedString::from(id.clone()), selected, theme)
            .group(group.clone())
            .debug_selector(move || id)
            .h(px(32.))
            .pl(px(14.))
            .pr(px(8.))
            .gap(px(8.))
            .child(icon("thread", theme.muted).size(px(14.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .when(selected, |v| v.font_weight(FontWeight::SEMIBOLD))
                    .child(title),
            )
            .when(show_project, |v| {
                v.child(
                    div()
                        .max_w(px(64.))
                        .flex_shrink_0()
                        .truncate()
                        .font_family(MONO)
                        .text_size(px(9.5))
                        .text_color(theme.faint)
                        .child(project),
                )
            })
            .when(busy, |v| {
                v.child(div().mx(px(2.)).child(status_dot(
                    SharedString::from(format!("running-{group}")),
                    theme.steel,
                    true,
                )))
            })
            .child(self.close_button(
                group,
                target,
                if matches!(row, Row::Open(_)) {
                    "Close session"
                } else {
                    "Hide saved session"
                },
                cx,
                theme,
            ))
            .on_click(cx.listener(move |this, _, _, cx| match &action {
                Ok(id) => this.workspace.update(cx, |ws, cx| ws.select(*id, cx)),
                Err(s) => {
                    this.workspace.update(cx, |ws, cx| {
                        ws.open(s.cwd.clone().into(), Some(s.clone()), cx)
                    });
                }
            }))
            .into_any_element()
    }
}
impl Render for SidebarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        self.sidebar(window, cx, theme(cx))
    }
}
fn selectable_row(id: impl Into<ElementId>, selected: bool, theme: Theme) -> Stateful<Div> {
    h_flex()
        .id(id)
        .role(Role::Button)
        .aria_selected(selected)
        .relative()
        .flex_shrink_0()
        .rounded(px(5.))
        .cursor_pointer()
        .hover(move |v| v.bg(theme.hover))
        .when(selected, |v| v.bg(theme.selected))
}
