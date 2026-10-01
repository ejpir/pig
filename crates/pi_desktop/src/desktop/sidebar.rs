//! Sidebar from design/desktop-projects-study: open sessions from every project, then
//! each project with its sessions.
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::{Div, ElementId, Role, Stateful};
use pi_core::clock::{parse_timestamp, short_age};

use super::session::Summary;
use super::workspace::{SessionId, WorkspaceController, WorkspaceEvent};
use super::*;

/// Sessions an expanded project lists before "Show N more".
const PROJECT_PREVIEW: usize = 4;
/// The sample data's clock, so demo ages stay stable: 2026-09-28T10:14:00Z.
const DEMO_NOW: u64 = 1_790_590_440_000;

enum Row<'a> {
    Open(SessionId),
    Saved(&'a SavedSession),
}

pub struct SidebarView {
    pub workspace: Entity<WorkspaceController>,
    pub search: Entity<TextInput>,
    expanded_projects: HashSet<PathBuf>,
    unabridged_projects: HashSet<PathBuf>,
    active_expanded: bool,
    projects_expanded: bool,
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
            unabridged_projects: HashSet::new(),
            active_expanded: true,
            projects_expanded: true,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
            #[cfg(test)]
            renders: 0,
        }
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

impl SidebarView {
    fn now_ms(&self, cx: &App) -> u64 {
        if self.workspace.read(cx).is_demo() {
            return DEMO_NOW;
        }
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64)
    }

    fn project_rows<'a>(
        &self,
        project: &Path,
        saved: &[&'a SavedSession],
        query: &str,
        cx: &App,
    ) -> Vec<Row<'a>> {
        let workspace = self.workspace.read(cx);
        let open = workspace
            .summaries
            .iter()
            .filter(|(_, summary)| {
                summary.cwd == project && summary.title.to_lowercase().contains(query)
            })
            .map(|(id, _)| Row::Open(*id));
        let closed = saved
            .iter()
            .filter(|saved| {
                Path::new(&saved.cwd) == project && saved.title().to_lowercase().contains(query)
            })
            .filter(|saved| {
                !workspace
                    .summaries
                    .values()
                    .any(|summary| summary.file.as_deref() == Some(&saved.path))
            })
            .map(|saved| Row::Saved(saved));
        open.chain(closed).collect()
    }

    pub(super) fn sidebar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        theme: Theme,
    ) -> impl IntoElement + use<> {
        let query = self.search.read(cx).content().to_lowercase();
        let workspace = self.workspace.read(cx);
        let saved: Vec<_> = workspace.saved.iter().collect();
        let now = self.now_ms(cx);
        let active: Vec<_> = workspace
            .summaries
            .iter()
            .filter(|(_, summary)| summary.title.to_lowercase().contains(&query))
            .collect();
        let mut list = v_flex()
            .id("sidebar-list")
            .debug_selector(|| "sidebar-list".into())
            .size_full()
            // A tracked handle makes the scrollbar draw only the thumb; the list scrolls itself.
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .justify_between()
                    .pl(px(7.))
                    .pr(px(8.))
                    .mt(px(5.))
                    .mb(px(8.))
                    .child(
                        h_flex()
                            .id("sidebar-active-toggle")
                            .debug_selector(|| "sidebar-active-toggle".into())
                            .role(Role::Button)
                            .aria_expanded(self.active_expanded)
                            .gap(px(5.))
                            .cursor_pointer()
                            .child(
                                icon(
                                    if self.active_expanded {
                                        "chevron_down"
                                    } else {
                                        "chevron_right"
                                    },
                                    theme.faint,
                                )
                                .size(px(10.)),
                            )
                            .child(label("ACTIVE", theme))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_expanded = !this.active_expanded;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(10.5))
                            .text_color(theme.faint)
                            .child(active.len().to_string()),
                    ),
            );
        if self.active_expanded || !query.is_empty() {
            for (index, open) in active {
                list = list.child(self.active_row(*index, open, cx, theme));
            }
        }
        list = list.child(
            h_flex()
                .flex_shrink_0()
                .justify_between()
                .pl(px(7.))
                .pr(px(4.))
                .mt(px(12.))
                .mb(px(8.))
                .child(
                    h_flex()
                        .id("sidebar-projects-toggle")
                        .debug_selector(|| "sidebar-projects-toggle".into())
                        .role(Role::Button)
                        .aria_expanded(self.projects_expanded)
                        .gap(px(5.))
                        .cursor_pointer()
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
                        .child(label("PROJECTS", theme))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.projects_expanded = !this.projects_expanded;
                            cx.notify();
                        })),
                )
                .child(
                    h_flex()
                        .id("sidebar-open-folder")
                        .role(Role::Button)
                        .aria_label("Open folder")
                        .size(px(18.))
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(move |button| button.bg(theme.hover))
                        .child(icon("plus", theme.muted).size(px(13.)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.workspace
                                .update(cx, |workspace, cx| workspace.open_folder(cx))
                        })),
                ),
        );
        if self.projects_expanded || !query.is_empty() {
            for (index, project) in workspace.projects.iter().cloned().enumerate() {
                let rows = self.project_rows(&project, &saved, &query, cx);
                if !query.is_empty() && rows.is_empty() {
                    continue;
                }
                let expanded = !query.is_empty() || self.expanded_projects.contains(&project);
                let running = workspace
                    .summaries
                    .values()
                    .any(|summary| summary.cwd == project && summary.busy);
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
                let unabridged = !query.is_empty() || self.unabridged_projects.contains(&project);
                let shown = if unabridged {
                    rows.len()
                } else {
                    rows.len().min(PROJECT_PREVIEW)
                };
                for row in &rows[..shown] {
                    list = list.child(self.session_row(row, now, cx, theme));
                }
                if query.is_empty() && rows.len() > PROJECT_PREVIEW {
                    list = list.child(
                        div()
                            .id(("show-more", index))
                            .debug_selector(move || format!("show-more-{index}"))
                            .role(Role::Button)
                            .flex_shrink_0()
                            .h(px(26.))
                            .pl(px(56.))
                            .pt(px(4.))
                            .text_size(px(11.))
                            .text_color(theme.faint)
                            .cursor_pointer()
                            .hover(move |row| row.text_color(theme.muted))
                            .child(if unabridged {
                                "Show fewer".to_owned()
                            } else {
                                format!("Show {} more", rows.len() - PROJECT_PREVIEW)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.unabridged_projects.remove(&project) {
                                    this.unabridged_projects.insert(project.clone());
                                }
                                cx.notify();
                            })),
                    );
                }
            }
            list = list.child(
                h_flex()
                    .id("open-folder")
                    .role(Role::Button)
                    .flex_shrink_0()
                    .h(px(28.))
                    .pl(px(4.))
                    .rounded(px(5.))
                    .text_size(px(12.))
                    .text_color(theme.muted)
                    .cursor_pointer()
                    .hover(move |row| row.bg(theme.hover))
                    .child(icon("plus", theme.faint).size(px(12.)))
                    .child(div().ml(px(24.)).child("Open Folder…"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.workspace
                            .update(cx, |workspace, cx| workspace.open_folder(cx))
                    })),
            );
        }
        // Rows run to the sidebar's edge, so the scrollbar gets a gutter instead of
        // covering their counts and ages.
        let list = div().flex_1().min_h_0().child(list).custom_scrollbars(
            super::scrollbar("sidebar-scrollbar", &self.scroll, Some(theme.panel)),
            window,
            cx,
        );
        v_flex()
            .relative()
            .w_full()
            .flex_shrink_0()
            .h_full()
            .bg(theme.panel)
            .border_r_1()
            .border_color(theme.edge)
            .p(px(10.))
            .text_size(px(14.))
            .child(list)
            .child(divider(theme).mx(px(6.)).mb(px(8.)))
            .children(
                [
                    (
                        "All Sessions",
                        "list_tree",
                        Some(super::app_views::AppView::Sessions),
                    ),
                    ("Models", "sparkle", Some(super::app_views::AppView::Models)),
                    (
                        "Resources",
                        "box",
                        Some(super::app_views::AppView::Resources),
                    ),
                    (
                        "Settings",
                        "settings",
                        Some(super::app_views::AppView::Settings),
                    ),
                ]
                .into_iter()
                .map(|(name, glyph, view)| {
                    let selected = view.is_some() && self.workspace.read(cx).view == view;
                    let color = match (selected, view) {
                        (true, _) => theme.text,
                        (false, Some(_)) => theme.secondary,
                        (false, None) => theme.faint,
                    };
                    h_flex()
                        .id(name)
                        .debug_selector(move || format!("nav-{name}"))
                        .relative()
                        .flex_shrink_0()
                        .h(px(28.))
                        .px(px(8.))
                        .gap(px(9.))
                        .rounded(px(5.))
                        .text_size(px(13.))
                        .text_color(color)
                        .when(selected, |row| row.bg(theme.selected))
                        .when_some(view, |row, view| {
                            row.cursor_pointer()
                                .hover(move |row| row.bg(theme.hover))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.workspace
                                        .update(cx, |workspace, cx| workspace.show_view(view, cx))
                                }))
                        })
                        .child(icon(glyph, color))
                        .child(name)
                        .when(name == "Settings", |row| row.mb(px(8.)))
                }),
            )
    }

    fn active_row(
        &self,
        id: SessionId,
        open: &Summary,
        cx: &Context<Self>,
        theme: Theme,
    ) -> impl IntoElement {
        let index = id.0 as usize;
        // An app view replaces the session, so no session row is selected then.
        let selected =
            id == self.workspace.read(cx).active && self.workspace.read(cx).view.is_none();
        let group = SharedString::from(format!("active-session-{index}"));
        let project = open
            .cwd
            .file_name()
            .unwrap_or(open.cwd.as_os_str())
            .to_string_lossy()
            .into_owned();
        selectable_row(("active-session", index), selected, theme)
            .group(group.clone())
            .debug_selector(move || format!("active-session-{index}"))
            .h(px(30.))
            .pl(px(9.))
            .pr(px(8.))
            .gap(px(11.))
            .child(state_mark(("active-state", index), open.busy, theme))
            .child(row_title(&open.title, selected, theme))
            .child(
                div()
                    .flex_shrink_0()
                    .font_family(MONO)
                    .text_size(px(10.5))
                    .text_color(theme.faint)
                    .group_hover(group.clone(), |style| style.invisible())
                    .child(project),
            )
            .child(self.close_button(
                group,
                workspace::CloseTarget::Session(id),
                "Close active session",
                cx,
                theme,
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.workspace
                    .update(cx, |workspace, cx| workspace.select(id, cx))
            }))
    }

    #[allow(clippy::too_many_arguments)]
    fn project_row(
        &self,
        index: usize,
        project: &Path,
        count: usize,
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
            .relative()
            .group(group.clone())
            .debug_selector(move || format!("project-{index}"))
            .role(Role::Button)
            .aria_label(format!("Project {name}"))
            .aria_expanded(expanded)
            .flex_shrink_0()
            .h(px(28.))
            .pl(px(2.))
            .pr(px(8.))
            .rounded(px(5.))
            .cursor_pointer()
            .hover(move |row| row.bg(theme.hover))
            .child(
                icon(
                    if expanded {
                        "chevron_down"
                    } else {
                        "chevron_right"
                    },
                    theme.faint,
                )
                .size(px(11.)),
            )
            .child(
                icon(
                    "folder",
                    if selected {
                        theme.accent
                    } else if expanded {
                        theme.secondary
                    } else {
                        theme.faint
                    },
                )
                .ml(px(5.))
                .size(px(14.)),
            )
            .child(
                div()
                    .ml(px(8.))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(if selected {
                        theme.accent
                    } else if expanded {
                        theme.text
                    } else {
                        theme.secondary
                    })
                    .when(expanded, |name| name.font_weight(FontWeight::SEMIBOLD))
                    .child(name),
            )
            .when(running, |row| {
                row.child(div().size(px(6.)).rounded_full().bg(theme.accent))
            })
            .child(
                div()
                    .w(px(16.))
                    .ml(px(7.))
                    .text_right()
                    .font_family(MONO)
                    .text_size(px(10.5))
                    .text_color(theme.faint)
                    .group_hover(group.clone(), |style| style.invisible())
                    .child(count.to_string()),
            )
            .child(self.close_button(
                group,
                workspace::CloseTarget::Project(project.to_owned()),
                "Remove project from sidebar",
                cx,
                theme,
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.workspace.update(cx, |workspace, cx| {
                    workspace.select_project(toggle.clone(), cx)
                });
                if !this.expanded_projects.remove(&toggle) {
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
            .size(px(24.))
            .bg(theme.panel)
            .invisible()
            .group_hover(group, |s| s.visible())
            .tooltip(ui::Tooltip::text(title))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.workspace.update(cx, |workspace, cx| {
                    workspace.request_close(target.clone(), window, cx)
                });
            }))
    }
    fn session_row(&self, row: &Row, now: u64, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let (id, selected, title, running, changed) = match row {
            Row::Open(index) => {
                let workspace = self.workspace.read(cx);
                let open = &workspace.summaries[index];
                let changed = open.modified.or_else(|| {
                    let path = open.file.as_deref()?;
                    let saved = workspace.saved.iter().find(|saved| saved.path == path)?;
                    parse_timestamp(saved.modified.as_deref()?)
                });
                (
                    ElementId::from(("open-session", index.0 as usize)),
                    *index == self.workspace.read(cx).active && workspace.view.is_none(),
                    open.title.clone(),
                    open.busy,
                    changed,
                )
            }
            Row::Saved(saved) => (
                ElementId::Name(format!("saved-session-{}", saved.id).into()),
                false,
                saved.title().to_owned(),
                false,
                saved.modified.as_deref().and_then(parse_timestamp),
            ),
        };
        let selector = match row {
            Row::Open(index) => format!("open-session-{}", index.0),
            Row::Saved(saved) => format!("saved-session-{}", saved.id),
        };
        let mark = SharedString::from(format!("state-{selector}"));
        let group = SharedString::from(selector.clone());
        let target = match row {
            Row::Open(id) => workspace::CloseTarget::Session(*id),
            Row::Saved(s) => workspace::CloseTarget::Saved(s.path.clone()),
        };
        selectable_row(id, selected, theme)
            .group(group.clone())
            .debug_selector(move || selector)
            .h(px(28.))
            .pl(px(36.))
            .pr(px(8.))
            .child(
                icon(
                    "thread",
                    if selected {
                        theme.secondary
                    } else {
                        theme.faint
                    },
                )
                .size(px(13.)),
            )
            .child(row_title(&title, selected, theme).ml(px(7.)))
            .child(
                div()
                    .group_hover(group.clone(), |style| style.invisible())
                    .child(if running {
                        state_mark(mark, true, theme).into_any_element()
                    } else {
                        div()
                            .flex_shrink_0()
                            .font_family(MONO)
                            .text_size(px(10.5))
                            .text_color(theme.faint)
                            .child(
                                changed.map_or_else(|| "now".into(), |then| short_age(then, now)),
                            )
                            .into_any_element()
                    }),
            )
            .child(self.close_button(
                group,
                target,
                if matches!(row, Row::Open(_)) {
                    "Close active session"
                } else {
                    "Hide saved session"
                },
                cx,
                theme,
            ))
            .on_click(cx.listener({
                let row = match row {
                    Row::Open(index) => Ok(*index),
                    Row::Saved(saved) => Err((*saved).clone()),
                };
                move |this, _, _, cx| match &row {
                    Ok(id) => this
                        .workspace
                        .update(cx, |workspace, cx| workspace.select(*id, cx)),
                    Err(saved) => {
                        this.workspace.update(cx, |workspace, cx| {
                            workspace.open(PathBuf::from(&saved.cwd), Some(saved.clone()), cx)
                        });
                    }
                }
            }))
            .into_any_element()
    }
}

fn selectable_row(id: impl Into<ElementId>, selected: bool, theme: Theme) -> Stateful<Div> {
    h_flex()
        .id(id)
        .role(Role::Button)
        .relative()
        .flex_shrink_0()
        .rounded(px(5.))
        .cursor_pointer()
        .hover(move |row| row.bg(theme.raised))
        .when(selected, |row| row.bg(theme.selected))
}

fn row_title(title: &str, selected: bool, theme: Theme) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(px(12.5))
        .text_color(if selected {
            theme.text
        } else {
            theme.secondary
        })
        .when(selected, |title| title.font_weight(FontWeight::SEMIBOLD))
        .child(title.to_owned())
}

/// Running sessions spin; idle ones show a hollow ring.
fn state_mark(id: impl Into<ElementId>, running: bool, theme: Theme) -> impl IntoElement {
    if running {
        return spinner(id, theme);
    }
    div()
        .flex_shrink_0()
        .size(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .size(px(7.))
                .rounded_full()
                .border_1()
                .border_color(theme.faint),
        )
}
