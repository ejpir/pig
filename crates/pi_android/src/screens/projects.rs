//! Choosing where Pi works (design/android/projects): one browser for the
//! step after pairing (04) and for New session's project sheet (01, 02).
//! Search first (05), then recent projects, then the computer's folders as a
//! tree that rolls out in place, down to files inside projects.

use super::scroll_area;
use crate::{
    app::{PhoneApp, Route},
    projects::Row,
    theme::{Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{
    Context, Div, ElementId, Focusable, FontWeight, HighlightStyle, SharedString, StyledText,
    Window, div, prelude::*, px,
};

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
    pub(crate) fn project_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
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
                                    .child("Projects"),
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
            .child(
                crate::scroll::vertical("project-tree", &self.sheet_scroll)
                    .flex_1()
                    .min_h_0()
                    .px(px(20.))
                    .pb(px(8.))
                    .child(self.project_browser_body(false, colors, cx)),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(20.))
                    .pt(px(12.))
                    .border_t_1()
                    .border_color(colors.line)
                    .child(self.use_folder_button(colors, cx)),
            )
    }

    /// "New session in pi": the picked folder, or where the tree starts.
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
        let button = ui::button(
            "use-folder",
            Button::Primary,
            Some("plus"),
            name.map_or_else(
                || "Choose a folder".to_owned(),
                |name| format!("New session in {name}"),
            ),
            false,
            colors,
        )
        .debug_selector(|| "use-folder".into())
        .w_full();
        if target.is_some() {
            button.on_click(cx.listener(|this, _, _, cx| this.use_picked_folder(cx)))
        } else {
            ui::disabled(button, colors)
        }
    }

    /// The search pill: finds what is listed, or goes to a typed path.
    fn folder_search(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let typed = !self.folder.read(cx).text().is_empty();
        div()
            .flex()
            .child(
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
                            search.bg(colors.panel).border_color(gpui::transparent_black())
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
                .debug_selector(move || format!("project-choice-{index}").into())
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
        let has_recent = self
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
                    .when(index > 0, |crumb| crumb.child(icon("chev_r", 14., colors.faint)))
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
                                crumb.text_color(colors.text).font_weight(FontWeight::SEMIBOLD)
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
        let picked = self.project_browser.picked.clone();
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
        let rows = self.project_browser.rows().into_iter().map(|row| match row {
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
                            .child(icon(if open { "chev_d" } else { "chev_r" }, 16., colors.muted))
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
                        if node.folder {
                            this.project_browser.picked = Some(node.path.clone());
                            cx.notify();
                        } else {
                            this.open_file(&node, window, cx);
                        }
                    }))
            }
            Row::Loading { depth } => line(ElementId::Name("tree-loading".into()), depth)
                .pl(px(depth as f32 * STEP + 12.))
                .child(ui::working_indicator(colors))
                .child(ui::hint("Loading…", colors)),
            Row::Failed { depth, path, error } => line(
                ElementId::Name(format!("tree-failed:{path}").into()),
                depth,
            )
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
                .on_click(cx.listener(move |this, _, _, cx| this.retry_folder(path.clone(), cx))),
            ),
            Row::More { depth, path, count } => line(
                ElementId::Name(format!("more:{path}").into()),
                depth,
            )
            .pl(px(depth as f32 * STEP + 40.))
            .text_color(colors.accent)
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(14.))
            .active(|style| style.bg(colors.selected))
            .child(format!("Show {count} more"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.project_browser.all.insert(path.clone());
                cx.notify();
            })),
            Row::Partial { depth } => line(ElementId::Name("tree-partial".into()), depth)
                .pl(px(depth as f32 * STEP + 40.))
                .child(ui::hint("The computer listed only part of this folder. Search or type a path to go deeper.", colors)),
        });
        div().flex().flex_col().children(rows)
    }

    /// 05 What the search found among what is listed, or the path to go to.
    fn found(&self, query: &str, colors: &Theme, cx: &Context<Self>) -> Div {
        if query.starts_with('~') || query.starts_with('/') {
            let path = self.full_path(query);
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
                    if node.folder { colors.muted } else { colors.faint },
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
                                .child(if folder.is_empty() { place.clone() } else { folder }),
                        ),
                )
                .when(node.folder, |row| {
                    let path = path.clone();
                    row.child(
                        ui::button(("use-found", index), Button::Quiet, None, "Use", true, colors)
                            .px(px(8.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.folder.update(cx, |area, cx| area.set_text("", cx));
                                this.use_folder(&path, cx);
                            })),
                    )
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    window.dismiss_virtual_keyboard();
                    window.focus(&this.focus, cx);
                    if node.folder {
                        this.folder.update(cx, |area, cx| area.set_text("", cx));
                        this.reveal_folder(&node.path, cx);
                    } else {
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
            .child(
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
                    })),
            )
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
