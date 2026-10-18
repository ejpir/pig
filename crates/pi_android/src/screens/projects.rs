//! Choosing where Pi works (design/android/projects): one browser for the
//! step after pairing (04) and for New session's project sheet (01, 02).
//! Search first (05), then recent projects, then the computer's folders as a
//! tree that rolls out in place, down to files inside projects.

use super::scroll_area;
use crate::{
    app::{PhoneApp, Route, Sheet},
    projects::Row,
    theme::{Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{
    AnyElement, Context, Div, ElementId, Focusable, FontWeight, HighlightStyle, SharedString,
    StyledText, Window, div, list, prelude::*, px,
};
use std::sync::Arc;

/// How far each level of the tree steps in.
const STEP: f32 = 20.;

impl PhoneApp {
    /// 04 First project: after pairing, where shall we work?
    pub(crate) fn projects_screen(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::Projects);
        let computer = self.computer_name();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                scroll_area("choose-project", &scroll).child(
                    div()
                        .px(px(20.))
                        .pt(px(32.))
                        .pb(px(24.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .text_size(px(12.5))
                                .line_height(px(16.))
                                .text_color(colors.muted)
                                .child(div().size(px(6.)).rounded(px(3.)).bg(colors.green))
                                .child(format!("Paired with {computer}")),
                        )
                        .child(
                            ui::serif("Where shall we work?", 31.)
                                .line_height(px(36.))
                                .mt(px(12.)),
                        )
                        .child(
                            div()
                                .mt(px(12.))
                                .line_height(px(22.))
                                .text_color(colors.secondary)
                                .child("Pick a project to start in. Nothing runs until you send a prompt."),
                        )
                        .child(self.folder_search(&colors, cx).mt(px(24.)))
                        .child(self.project_browser_body(true, &colors, cx)),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(20.))
                    .pt(px(12.))
                    .pb(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(self.use_folder_button(&colors, cx))
                    .child(
                        ui::button(
                            "view-sessions",
                            Button::Quiet,
                            None,
                            "Just look at sessions",
                            false,
                            &colors,
                        )
                        .w_full()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.back(window, cx);
                        })),
                    ),
            )
    }

    /// 01 Projects, from New session's project chip: the title stays, the
    /// tree scrolls, and the one action stays at the bottom.
    pub(crate) fn project_sheet(
        &mut self,
        sheet: Sheet,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let query = self.folder.read(cx).text().trim().to_owned();
        let browser: AnyElement = if query.is_empty() {
            self.project_browser_list(colors, cx).into_any_element()
        } else {
            crate::scroll::vertical("project-tree", &self.sheet_scroll)
                .flex_1()
                .min_h_0()
                .px(px(20.))
                .pb(px(8.))
                .child(self.found(&query, colors, cx))
                .into_any_element()
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(20.))
                    .debug_selector(|| "sheet-header".into())
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(22.))
                                    .line_height(px(28.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(match sheet {
                                        Sheet::EditorRoot => "Choose a folder",
                                        Sheet::EditorFiles => "Files",
                                        _ => "Projects",
                                    }),
                            )
                            .child(
                                div()
                                    .mt(px(4.))
                                    .text_size(px(12.5))
                                    .line_height(px(16.))
                                    .text_color(colors.muted)
                                    .truncate()
                                    .child(format!("on {}", self.computer_name())),
                            ),
                    )
                    .child(
                        ui::tap("close-sheet", "x", colors)
                            .mr(px(-12.))
                            .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(20.))
                    .mt(px(16.))
                    .child(self.folder_search(colors, cx)),
            )
            .child(browser)
            .child(
                div()
                    .flex_none()
                    .px(px(20.))
                    .pt(px(12.))
                    .border_t_1()
                    .border_color(colors.line)
                    .when(sheet == Sheet::EditorFiles, |footer| {
                        footer
                            .flex()
                            .gap(px(8.))
                            .child(
                                ui::button(
                                    "change-editor-folder",
                                    Button::Quiet,
                                    Some("folder"),
                                    "Change folder",
                                    false,
                                    colors,
                                )
                                .debug_selector(|| "change-editor-folder".into())
                                .flex_1()
                                .on_click(cx.listener(
                                    |this, _, _, cx| this.choose_another_editor_folder(cx),
                                )),
                            )
                            .child(
                                ui::button(
                                    "close-editor",
                                    Button::Quiet,
                                    Some("x"),
                                    "Close editor",
                                    false,
                                    colors,
                                )
                                .debug_selector(|| "close-editor".into())
                                .flex_1()
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.close_editor_workspace(window, cx),
                                )),
                            )
                    })
                    .when(sheet != Sheet::EditorFiles, |footer| {
                        footer.child(self.use_folder_button(colors, cx))
                    }),
            )
    }

    fn project_browser_list(&mut self, colors: &Theme, cx: &Context<Self>) -> AnyElement {
        let rows = self.project_browser.rows();
        self.sync_project_list(&rows);
        let state = self.project_list.clone();
        let offset = state.scroll_px_offset_for_scrollbar().y;
        crate::scroll::note_movement(offset - self.project_list_offset);
        self.project_list_offset = offset;

        let rows: Arc<Vec<Row>> = Arc::new(rows);
        let list_rows = rows.clone();
        let list_colors = *colors;
        let item_count = 1 + rows.len();
        let body = list(
            state.clone(),
            cx.processor(move |this, index: usize, _, cx| {
                let content = if index == 0 {
                    this.project_sheet_overview(&list_colors, cx)
                        .into_any_element()
                } else {
                    this.project_tree_row(list_rows[index - 1].clone(), &list_colors, cx)
                        .into_any_element()
                };
                div()
                    .w_full()
                    .min_w_0()
                    .px(px(20.))
                    .when(index + 1 == item_count, |row| row.pb(px(8.)))
                    .child(content)
                    .into_any_element()
            }),
        )
        .size_full();
        crate::scroll::virtual_list("project-tree", &state)
            .flex_1()
            .min_h_0()
            .child(body)
            .into_any_element()
    }

    fn sync_project_list(&mut self, rows: &[Row]) {
        let project_count = if self.project_browser_editor {
            0
        } else {
            self.store.as_ref().map_or(0, |store| store.projects.len())
        };
        let overview_shape = (project_count, self.project_browser.root.is_some());
        let state = self.project_list.clone();
        if self.project_overview_shape.is_none() {
            state.reset_with_uniform_height(1 + rows.len(), px(48.));
        } else {
            let old = &self.project_tree_rows;
            let mut prefix = 0;
            while prefix < old.len()
                && prefix < rows.len()
                && old[prefix].key() == rows[prefix].key()
            {
                if old[prefix] != rows[prefix] {
                    state.remeasure_items(prefix + 1..prefix + 2);
                }
                prefix += 1;
            }
            let mut suffix = 0;
            while suffix < old.len().saturating_sub(prefix)
                && suffix < rows.len().saturating_sub(prefix)
                && old[old.len() - 1 - suffix].key() == rows[rows.len() - 1 - suffix].key()
            {
                let old_index = old.len() - 1 - suffix;
                let new_index = rows.len() - 1 - suffix;
                if old[old_index] != rows[new_index] {
                    state.remeasure_items(new_index + 1..new_index + 2);
                }
                suffix += 1;
            }
            let old_end = old.len() - suffix;
            let new_end = rows.len() - suffix;
            if prefix != old_end || prefix != new_end {
                state.splice(prefix + 1..old_end + 1, new_end - prefix);
            }
            if self.project_overview_shape != Some(overview_shape) {
                state.remeasure_items(0..1);
            }
        }
        self.project_tree_rows = rows.to_vec();
        self.project_overview_shape = Some(overview_shape);
        debug_assert_eq!(state.item_count(), 1 + rows.len());
    }

    fn project_sheet_overview(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let browser = &self.project_browser;
        let computer = self.computer_name();
        let projects = self
            .store
            .as_ref()
            .map(|store| store.projects.clone())
            .unwrap_or_default();
        let sessions = |folder: &str| {
            self.store.as_ref().map_or(0, |store| {
                store
                    .sessions
                    .iter()
                    .filter(|session| session.folder == folder)
                    .count()
            })
        };
        let recent = projects.into_iter().enumerate().map(|(index, project)| {
            let current = self.project == index;
            let detail = match sessions(&project.folder) {
                0 => project.folder.clone(),
                1 => format!("{} · 1 session", project.folder),
                count => format!("{} · {count} sessions", project.folder),
            };
            div()
                .id(("project", index))
                .relative()
                .child(crate::testing::probe(format!(
                    "{:?}",
                    ElementId::from(("project", index))
                )))
                .debug_selector(move || format!("project-choice-{index}"))
                .min_h(px(64.))
                .py(px(12.))
                .flex()
                .items_center()
                .gap(px(12.))
                .when(index > 0, |row| row.border_t_1().border_color(colors.line))
                .active(|style| style.bg(colors.selected))
                .child(icon(
                    "folder",
                    20.,
                    if current { colors.accent } else { colors.muted },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(project.name.clone()),
                        )
                        .child(
                            div()
                                .mt(px(2.))
                                .text_size(px(12.5))
                                .line_height(px(16.))
                                .text_color(colors.muted)
                                .truncate()
                                .child(detail),
                        ),
                )
                .when(current, |row| row.child(icon("check", 20., colors.accent)))
                .on_click(cx.listener(move |this, _, _, cx| this.select_project(index, cx)))
        });
        let has_recent = !self.project_browser_editor
            && self
                .store
                .as_ref()
                .is_some_and(|store| !store.projects.is_empty());
        div()
            .flex()
            .flex_col()
            .when(has_recent, |body| {
                body.child(
                    ui::label("Recent", colors)
                        .text_color(colors.muted)
                        .mt(px(20.)),
                )
                .child(div().mt(px(4.)).flex().flex_col().children(recent))
            })
            .child(
                div()
                    .mt(px(20.))
                    .flex()
                    .items_center()
                    .child(
                        ui::label(format!("On {computer}"), colors)
                            .text_color(colors.muted)
                            .flex_1(),
                    )
                    .child(
                        ui::chip("hidden-toggle", Some("eye"), "Hidden", colors)
                            .h(px(28.))
                            .px(px(10.))
                            .text_size(px(12.5))
                            .when(browser.show_hidden, |chip| {
                                chip.bg(colors.tint(colors.accent))
                                    .border_color(gpui::transparent_black())
                                    .text_color(colors.accent)
                            })
                            .aria_label(if browser.show_hidden {
                                "Hide hidden files"
                            } else {
                                "Show hidden files"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.project_browser.show_hidden =
                                    !this.project_browser.show_hidden;
                                cx.notify();
                            })),
                    ),
            )
            .child(self.crumbs(colors, cx))
    }

    /// Starts a session or opens the selected folder as an editor workspace.
    fn use_folder_button(&self, colors: &Theme, cx: &Context<Self>) -> gpui::Stateful<Div> {
        let browser = &self.project_browser;
        let target = browser.picked.as_ref().or(browser.root.as_ref());
        let name = target.map(|path| {
            path.trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or("/")
                .to_owned()
        });
        let editor = self.sheet == Some(Sheet::EditorRoot);
        let button = ui::button(
            "use-folder",
            Button::Primary,
            Some(if editor { "folder" } else { "plus" }),
            name.map_or_else(
                || "Choose a folder".to_owned(),
                |name| {
                    if editor {
                        format!("Open {name}")
                    } else {
                        format!("New session in {name}")
                    }
                },
            ),
            false,
            colors,
        )
        .debug_selector(|| "use-folder".into())
        .w_full();
        if target.is_some() {
            button.on_click(cx.listener(move |this, _, window, cx| {
                if editor {
                    this.use_picked_editor_folder(window, cx);
                } else {
                    this.use_picked_folder(cx);
                }
            }))
        } else {
            ui::disabled(button, colors)
        }
    }

    /// The search pill: finds what is listed, or goes to a typed path.
    fn folder_search(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let typed = !self.folder.read(cx).text().is_empty();
        div().flex().child(
            div()
                .id("project-search")
                .relative()
                .child(crate::testing::probe("project-search"))
                .flex_1()
                .min_w_0()
                .h(px(48.))
                .pl(px(16.))
                .pr(px(4.))
                .flex()
                .items_center()
                .gap(px(12.))
                .rounded(px(24.))
                .border_1()
                .map(|search| {
                    if typed {
                        search.bg(colors.composer).border_color(colors.line_strong)
                    } else {
                        search
                            .bg(colors.panel)
                            .border_color(gpui::transparent_black())
                    }
                })
                .child(icon("search", 20., colors.muted))
                .child(div().flex_1().min_w_0().child(self.folder.clone()))
                .when(typed, |search| {
                    search.child(
                        ui::tap("clear-project-search", "x", colors)
                            .size(px(40.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.folder.update(cx, |area, cx| area.set_text("", cx));
                                cx.notify();
                            })),
                    )
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    window.focus(&this.folder.read(cx).focus_handle(cx), cx)
                })),
        )
    }

    /// What is under the search: what it found, or recent projects and the tree.
    fn project_browser_body(&self, first_run: bool, colors: &Theme, cx: &Context<Self>) -> Div {
        let query = self.folder.read(cx).text().trim().to_owned();
        if !query.is_empty() {
            return self.found(&query, colors, cx);
        }
        let browser = &self.project_browser;
        let computer = self.computer_name();
        let projects = self
            .store
            .as_ref()
            .map(|store| store.projects.clone())
            .unwrap_or_default();
        let sessions = |folder: &str| {
            self.store.as_ref().map_or(0, |store| {
                store
                    .sessions
                    .iter()
                    .filter(|session| session.folder == folder)
                    .count()
            })
        };
        let recent = projects.into_iter().enumerate().map(|(index, project)| {
            let current = self.project == index;
            let detail = match sessions(&project.folder) {
                0 => project.folder.clone(),
                1 => format!("{} · 1 session", project.folder),
                count => format!("{} · {count} sessions", project.folder),
            };
            div()
                .id(("project", index))
                .relative()
                .child(crate::testing::probe(format!(
                    "{:?}",
                    ElementId::from(("project", index))
                )))
                .debug_selector(move || format!("project-choice-{index}"))
                .min_h(px(64.))
                .py(px(12.))
                .flex()
                .items_center()
                .gap(px(12.))
                .when(index > 0, |row| row.border_t_1().border_color(colors.line))
                .active(|style| style.bg(colors.selected))
                .child(icon(
                    "folder",
                    20.,
                    if current { colors.accent } else { colors.muted },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(project.name.clone()),
                        )
                        .child(
                            div()
                                .mt(px(2.))
                                .text_size(px(12.5))
                                .line_height(px(16.))
                                .text_color(colors.muted)
                                .truncate()
                                .child(detail),
                        ),
                )
                .when(current, |row| row.child(icon("check", 20., colors.accent)))
                .on_click(cx.listener(move |this, _, _, cx| this.select_project(index, cx)))
        });
        let has_recent = !self.project_browser_editor
            && self
                .store
                .as_ref()
                .is_some_and(|store| !store.projects.is_empty());
        div()
            .flex()
            .flex_col()
            .when(has_recent, |body| {
                body.child(
                    ui::label(
                        if first_run {
                            "Projects Pi has worked in"
                        } else {
                            "Recent"
                        },
                        colors,
                    )
                    .text_color(colors.muted)
                    .mt(px(20.)),
                )
                .child(div().mt(px(4.)).flex().flex_col().children(recent))
            })
            .child(
                div()
                    .mt(px(20.))
                    .flex()
                    .items_center()
                    .child(
                        ui::label(format!("On {computer}"), colors)
                            .text_color(colors.muted)
                            .flex_1(),
                    )
                    .child(
                        ui::chip("hidden-toggle", Some("eye"), "Hidden", colors)
                            .h(px(28.))
                            .px(px(10.))
                            .text_size(px(12.5))
                            .when(browser.show_hidden, |chip| {
                                chip.bg(colors.tint(colors.accent))
                                    .border_color(gpui::transparent_black())
                                    .text_color(colors.accent)
                            })
                            .aria_label(if browser.show_hidden {
                                "Hide hidden files"
                            } else {
                                "Show hidden files"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.project_browser.show_hidden =
                                    !this.project_browser.show_hidden;
                                cx.notify();
                            })),
                    ),
            )
            .child(self.crumbs(colors, cx))
            .child(self.tree(colors, cx))
    }

    /// Each folder from home (or /) to where the tree starts; tapping one
    /// starts the tree there. Deep paths keep the first and the last two.
    fn crumbs(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let Some(root) = self.project_browser.root.clone() else {
            return div();
        };
        if self.sheet == Some(Sheet::EditorFiles) {
            return div()
                .min_h(px(40.))
                .flex()
                .items_center()
                .gap(px(8.))
                .overflow_hidden()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(icon("folder", 16., colors.accent))
                .child(div().truncate().child(self.short_path(&root)));
        }
        let short = self.short_path(&root);
        let mut crumbs: Vec<(String, String)> = Vec::new();
        let (first, rest) = if let Some(rest) = short.strip_prefix('~') {
            (("~".to_owned(), self.home_folder()), rest)
        } else {
            (("/".to_owned(), "/".to_owned()), short.as_str())
        };
        let mut path = first.1.trim_end_matches('/').to_owned();
        crumbs.push(first);
        for part in rest.split('/').filter(|part| !part.is_empty()) {
            path = format!("{path}/{part}");
            crumbs.push((part.to_owned(), path.clone()));
        }
        if crumbs.len() > 4 {
            let skipped = crumbs[crumbs.len() - 3].1.clone();
            let last = crumbs.split_off(crumbs.len() - 2);
            crumbs.truncate(1);
            crumbs.push(("…".into(), skipped));
            crumbs.extend(last);
        }
        let count = crumbs.len();
        div()
            .min_h(px(40.))
            .flex()
            .items_center()
            .gap(px(4.))
            .overflow_hidden()
            .text_size(px(13.))
            .text_color(colors.muted)
            .children(crumbs.into_iter().enumerate().map(|(index, (name, path))| {
                let here = index + 1 == count;
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .min_w_0()
                    .when(index > 0, |crumb| {
                        crumb.child(icon("chev_r", 14., colors.faint))
                    })
                    .child(
                        div()
                            .id(("crumb", index))
                            .relative()
                            .child(crate::testing::probe(format!(
                                "{:?}",
                                ElementId::from(("crumb", index))
                            )))
                            .h(px(40.))
                            .px(px(4.))
                            .flex()
                            .items_center()
                            .min_w_0()
                            .rounded(px(8.))
                            .whitespace_nowrap()
                            .when(here, |crumb| {
                                crumb
                                    .text_color(colors.text)
                                    .font_weight(FontWeight::SEMIBOLD)
                            })
                            .active(|style| style.bg(colors.selected))
                            .child(div().truncate().child(name))
                            .when(!here, |crumb| {
                                crumb.on_click(cx.listener(move |this, _, _, cx| {
                                    this.go_to_folder(path.clone(), cx)
                                }))
                            }),
                    )
            }))
    }

    fn tree(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        div().flex().flex_col().children(
            self.project_browser
                .rows()
                .into_iter()
                .map(|row| self.project_tree_row(row, colors, cx)),
        )
    }

    fn project_tree_row(
        &self,
        row: Row,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let picked = self.project_browser.picked.clone();
        let editor_files = self.sheet == Some(Sheet::EditorFiles);
        let editor_picker = self.sheet == Some(Sheet::EditorRoot);
        let guides = |depth: usize| {
            (0..depth).map(|level| {
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(16. + level as f32 * STEP))
                    .w(px(1.))
                    .bg(colors.line)
            })
        };
        let line = |id: ElementId, depth: usize| {
            div()
                .id(id.clone())
                .relative()
                .child(crate::testing::probe(format!("{id:?}")))
                .min_h(px(48.))
                .pl(px(depth as f32 * STEP))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(12.))
                .children(guides(depth))
        };
        match row {
            Row::Node { node, depth, open } => {
                let key = self.short_path(&node.path);
                let picked = picked.as_deref() == Some(node.path.as_str());
                let hue = if node.folder && node.project {
                    colors.accent
                } else if node.folder {
                    colors.muted
                } else {
                    colors.faint
                };
                let name = div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(node.project && node.root.is_none(), |name| {
                        name.font_weight(FontWeight::SEMIBOLD)
                    })
                    .text_color(if picked {
                        colors.accent
                    } else if node.folder {
                        colors.text
                    } else {
                        colors.secondary
                    })
                    .child(node.name.clone());
                let roll_key = format!("roll:{key}");
                let roll = div()
                    .id(ElementId::Name(format!("roll:{key}").into()))
                    .debug_selector(move || roll_key.clone())
                    .relative()
                    .child(crate::testing::probe(format!("roll:{key}")))
                    .w(px(32.))
                    .h(px(48.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.))
                    .when(node.folder, |roll| {
                        let node = node.clone();
                        roll.active(|style| style.bg(colors.selected))
                            .child(icon(
                                if open { "chev_d" } else { "chev_r" },
                                16.,
                                colors.muted,
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_folder(&node, cx)
                            }))
                    });
                let node_key = format!("node:{key}");
                line(ElementId::Name(format!("node:{key}").into()), depth)
                    .debug_selector(move || node_key.clone())
                    .when(picked, |row| row.bg(colors.tint(colors.accent)))
                    .when(!picked, |row| row.active(|style| style.bg(colors.selected)))
                    .aria_label(SharedString::from(node.name.clone()))
                    .child(roll)
                    .child(icon(if node.folder { "folder" } else { "file" }, 18., hue))
                    .child(name)
                    .children(node.tag.clone().map(|tag| {
                        div()
                            .flex_none()
                            .pr(px(12.))
                            .text_size(px(12.5))
                            .text_color(colors.faint)
                            .child(tag)
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if node.folder && editor_files {
                            this.toggle_folder(&node, cx);
                        } else if node.folder {
                            this.project_browser.picked = Some(node.path.clone());
                            cx.notify();
                        } else if !editor_picker {
                            this.open_file(&node, window, cx);
                        }
                    }))
            }
            Row::Loading { depth, path } => line(
                ElementId::Name(format!("tree-loading:{path}").into()),
                depth,
            )
            .pl(px(depth as f32 * STEP + 12.))
            .child(ui::working_indicator(colors))
            .child(ui::hint("Loading…", colors)),
            Row::Failed { depth, path, error } => {
                line(ElementId::Name(format!("tree-failed:{path}").into()), depth)
                    .pl(px(depth as f32 * STEP + 12.))
                    .py(px(8.))
                    .child(ui::hint(error, colors).flex_1().text_color(colors.coral))
                    .child(
                        ui::button(
                            ElementId::Name(format!("retry:{path}").into()),
                            Button::Plain,
                            None,
                            "Retry",
                            true,
                            colors,
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.retry_folder(path.clone(), cx)),
                        ),
                    )
            }
            Row::More { depth, path, count } => {
                line(ElementId::Name(format!("more:{path}").into()), depth)
                    .pl(px(depth as f32 * STEP + 40.))
                    .text_color(colors.accent)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.))
                    .active(|style| style.bg(colors.selected))
                    .child(format!("Show {count} more"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project_browser.all.insert(path.clone());
                        cx.notify();
                    }))
            }
            Row::Partial { depth, path } => line(
                ElementId::Name(format!("tree-partial:{path}").into()),
                depth,
            )
            .pl(px(depth as f32 * STEP + 40.))
            .child(ui::hint(
                "The computer listed only part of this folder. Search or type a path to go deeper.",
                colors,
            )),
        }
    }

    /// 05 What the search found among what is listed, or the path to go to.
    fn found(&self, query: &str, colors: &Theme, cx: &Context<Self>) -> Div {
        let editor_files = self.sheet == Some(Sheet::EditorFiles);
        let editor_picker = self.sheet == Some(Sheet::EditorRoot);
        if query.starts_with('~') || query.starts_with('/') {
            let path = self.full_path(query);
            let allowed = !editor_files
                || path.as_ref().is_some_and(|path| {
                    self.editor_root
                        .as_ref()
                        .is_some_and(|root| crate::projects::within(path, root))
                });
            if !allowed {
                return div().mt(px(12.)).child(
                    div()
                        .id("outside-editor-folder")
                        .debug_selector(|| "outside-editor-folder".into())
                        .min_h(px(56.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .text_color(colors.muted)
                        .child(icon("alert", 20., colors.muted))
                        .child("That path is outside the editor folder"),
                );
            }
            return div().mt(px(12.)).child(
                div()
                    .id("go-to-path")
                    .relative()
                    .child(crate::testing::probe("go-to-path"))
                    .min_h(px(56.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .rounded(px(12.))
                    .text_color(colors.accent)
                    .active(|style| style.bg(colors.selected))
                    .child(icon("chev_r", 20., colors.accent))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(format!("Go to {query}")),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if path.is_some() {
                            this.submit_folder_search(window, cx);
                        }
                    })),
            );
        }
        let browser = &self.project_browser;
        let found = browser.find(query);
        let root = browser.root.clone().unwrap_or_default();
        let place = root.rsplit('/').next().unwrap_or("/").to_owned();
        let rows = found.into_iter().enumerate().map(|(index, node)| {
            let folder = node.path[..node.path.len() - node.name.len()]
                .trim_end_matches('/')
                .strip_prefix(root.trim_end_matches('/'))
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_owned();
            let path = node.path.clone();
            div()
                .id(("found", index))
                .relative()
                .child(crate::testing::probe(format!(
                    "{:?}",
                    ElementId::from(("found", index))
                )))
                .min_h(px(56.))
                .py(px(8.))
                .flex()
                .items_center()
                .gap(px(12.))
                .when(index > 0, |row| row.border_t_1().border_color(colors.line))
                .active(|style| style.bg(colors.selected))
                .child(icon(
                    if node.folder { "folder" } else { "file" },
                    20.,
                    if node.folder {
                        colors.muted
                    } else {
                        colors.faint
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_color(if node.folder {
                                    colors.text
                                } else {
                                    colors.secondary
                                })
                                .child(marked(&node.name, query, colors)),
                        )
                        .child(
                            div()
                                .text_size(px(12.5))
                                .line_height(px(16.))
                                .text_color(colors.muted)
                                .truncate()
                                .child(if folder.is_empty() {
                                    place.clone()
                                } else {
                                    folder
                                }),
                        ),
                )
                .when(node.folder && !editor_files, |row| {
                    let path = path.clone();
                    row.child(
                        ui::button(
                            ("use-found", index),
                            Button::Quiet,
                            None,
                            if editor_picker { "Open" } else { "Use" },
                            true,
                            colors,
                        )
                        .px(px(8.))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.folder.update(cx, |area, cx| area.set_text("", cx));
                                if editor_picker {
                                    this.use_editor_folder(&path, window, cx);
                                } else {
                                    this.use_folder(&path, cx);
                                }
                            },
                        )),
                    )
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    window.dismiss_virtual_keyboard();
                    window.focus(&this.focus, cx);
                    if node.folder {
                        this.folder.update(cx, |area, cx| area.set_text("", cx));
                        this.reveal_folder(&node.path, cx);
                    } else if !editor_picker {
                        this.open_file(&node, window, cx);
                    }
                }))
        });
        let rows: Vec<_> = rows.collect();
        div()
            .flex()
            .flex_col()
            .child(
                ui::label(format!("In {place}"), colors)
                    .text_color(colors.muted)
                    .mt(px(20.)),
            )
            .when(rows.is_empty(), |body| {
                body.child(
                    ui::hint(
                        "Nothing listed so far has that in its name. Roll out a folder to look inside it.",
                        colors,
                    )
                    .mt(px(8.)),
                )
            })
            .child(div().mt(px(4.)).flex().flex_col().children(rows))
            .children((!editor_files).then(|| {
                div()
                    .id("type-a-path")
                    .relative()
                    .child(crate::testing::probe("type-a-path"))
                    .mt(px(8.))
                    .min_h(px(56.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .border_t_1()
                    .border_color(colors.line)
                    .text_color(colors.accent)
                    .active(|style| style.bg(colors.selected))
                    .child(icon("chev_r", 20., colors.accent))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Type ~ or / to go to a path"),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.folder.update(cx, |area, cx| area.set_text("~/", cx));
                        window.focus(&this.folder.read(cx).focus_handle(cx), cx);
                        cx.notify();
                    }))
            }))
    }
}

/// A found name with the part typed in the text's own colour, semibold.
fn marked(name: &str, query: &str, colors: &Theme) -> StyledText {
    let lower = name.to_lowercase();
    let query = query.to_lowercase();
    let highlights = (lower.len() == name.len())
        .then(|| lower.find(&query))
        .flatten()
        .map(|start| {
            vec![(
                start..start + query.len(),
                HighlightStyle {
                    color: Some(colors.text),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                },
            )]
        })
        .unwrap_or_default();
    StyledText::new(name.to_owned()).with_highlights(highlights)
}

#[cfg(all(test, feature = "ui-test"))]
mod tests {
    use super::*;

    #[gpui::test]
    fn project_sheet_only_mounts_visible_tree_rows(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_global(crate::testing::State::default()));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(px(384.), px(854.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.preview("project-tree", window, cx));
        });
        cx.run_until_parked();

        let (last, last_probe) = app.read_with(cx, |app, _| {
            let rows = app.project_browser.rows();
            let row = rows
                .iter()
                .rposition(|row| matches!(row, Row::Node { .. }))
                .expect("sample tree has nodes");
            let Row::Node { node, .. } = &rows[row] else {
                unreachable!()
            };
            (
                row + 1,
                format!("node:{}", app.short_path(&node.path)).leak(),
            )
        });
        assert!(last > 8);
        assert!(cx.debug_bounds(last_probe).is_none());

        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.project_list.scroll_to(gpui::ListOffset {
                    item_ix: last,
                    offset_in_item: px(0.),
                });
                cx.notify();
            });
        });
        cx.run_until_parked();
        let top = app.read_with(cx, |app, _| app.project_list.logical_scroll_top());
        assert!(top.item_ix > 0, "list did not move: {top:?}");
        assert!(cx.debug_bounds(last_probe).is_some());
    }
}
