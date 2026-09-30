//! All Sessions (design study 02, screen 07): every saved session in every project,
//! from pi's `list_sessions`, with filters, a sort and the header's search. The
//! inspector shows the selected session and its actions. An action on a session
//! open in a tab goes to that tab's pi; otherwise a short-lived pi does it
//! (`pi_core::session_actions`), so no tab's process is repurposed.
use super::super::panels::note;
use super::super::workspace::WorkspaceController;
use super::*;
use gpui::{ClipboardItem, PromptLevel, Task, UniformListScrollHandle, uniform_list};
use pi_core::{
    clock::{date_time, parse_timestamp, when},
    protocol::{Command, SavedSession},
    session_actions,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Project,
    Named,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Recent,
    Name,
    Messages,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Name => "Name",
            Self::Messages => "Messages",
        }
    }
    fn caption(self) -> &'static str {
        match self {
            Self::Recent => "Sorted by last change",
            Self::Name => "Sorted by name",
            Self::Messages => "Sorted by messages",
        }
    }
}

pub struct SessionsView {
    workspace: Entity<WorkspaceController>,
    search: Entity<TextInput>,
    filter: Filter,
    sort: Sort,
    sort_open: bool,
    focus: gpui::FocusHandle,
    /// The selected session's file.
    selected: Option<String>,
    rows: Vec<SavedSession>,
    rename: Option<Entity<TextInput>>,
    busy: Option<&'static str>,
    /// The last action's outcome; `true` for an error.
    notice: Option<(bool, String)>,
    scroll: UniformListScrollHandle,
    _task: Option<Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl SessionsView {
    pub fn new(
        workspace: Entity<WorkspaceController>,
        search: Entity<TextInput>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe(&workspace, |this, _, _, cx| this.refresh(cx)),
            cx.observe(&search, |this, _, cx| this.refresh(cx)),
        ];
        let mut this = Self {
            workspace,
            search,
            filter: Filter::All,
            sort: Sort::Recent,
            sort_open: false,
            focus: cx.focus_handle(),
            selected: None,
            rows: Vec::new(),
            rename: None,
            busy: None,
            notice: None,
            scroll: UniformListScrollHandle::new(),
            _task: None,
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        this
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let query = self.search.read(cx).content().to_lowercase();
        let project = workspace.selected_project.clone();
        let mut rows: Vec<SavedSession> = workspace
            .saved
            .iter()
            .filter(|saved| match self.filter {
                Filter::All => true,
                Filter::Project => project.as_deref() == Some(Path::new(&saved.cwd)),
                Filter::Named => saved.name.as_deref().is_some_and(|n| !n.trim().is_empty()),
            })
            .filter(|saved| {
                query.is_empty()
                    || saved.title().to_lowercase().contains(&query)
                    || saved.first_message.to_lowercase().contains(&query)
                    || saved.cwd.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        let modified = |saved: &SavedSession| saved.modified.as_deref().and_then(parse_timestamp);
        match self.sort {
            Sort::Recent => rows.sort_by_key(|saved| std::cmp::Reverse(modified(saved))),
            Sort::Name => rows.sort_by_key(|saved| saved.title().to_lowercase()),
            Sort::Messages => rows.sort_by_key(|saved| std::cmp::Reverse(saved.message_count)),
        }
        if !self
            .selected
            .as_ref()
            .is_some_and(|path| rows.iter().any(|r| &r.path == path))
        {
            self.selected = rows.first().map(|row| row.path.clone());
            self.rename = None;
        }
        self.rows = rows;
        cx.notify();
    }

    fn counts(&self, cx: &App) -> (usize, usize, usize) {
        let workspace = self.workspace.read(cx);
        let project = workspace.selected_project.as_deref();
        let all = workspace.saved.len();
        let here = workspace
            .saved
            .iter()
            .filter(|s| Some(Path::new(&s.cwd)) == project)
            .count();
        let named = workspace
            .saved
            .iter()
            .filter(|s| s.name.as_deref().is_some_and(|n| !n.trim().is_empty()))
            .count();
        (all, here, named)
    }

    fn selected(&self) -> Option<&SavedSession> {
        let path = self.selected.as_ref()?;
        self.rows.iter().find(|row| &row.path == path)
    }

    /// Whether the session is open in a tab, and whether its pi is working.
    fn open_state(&self, path: &str, cx: &App) -> Option<bool> {
        let workspace = self.workspace.read(cx);
        workspace.tabs.iter().find_map(|tab| {
            let controller = tab.controller.read(cx);
            (controller.model().state.session_file.as_deref() == Some(path))
                .then(|| controller.working())
        })
    }

    fn select(&mut self, path: String, cx: &mut Context<Self>) {
        if self.selected.as_ref() != Some(&path) {
            self.selected = Some(path);
            self.rename = None;
            self.notice = None;
            cx.notify();
        }
    }

    fn resume(&mut self, saved: SavedSession, cx: &mut Context<Self>) {
        self.workspace.update(cx, |workspace, cx| {
            workspace.open(PathBuf::from(&saved.cwd), Some(saved), cx);
        });
    }

    fn fork(&mut self, saved: SavedSession, window: &mut Window, cx: &mut Context<Self>) {
        // Forking picks an entry, which the Tree page does.
        let id = self.workspace.update(cx, |workspace, cx| {
            workspace.open(PathBuf::from(&saved.cwd), Some(saved), cx)
        });
        if let Some(tab) = self.workspace.read(cx).tab(id) {
            let view = tab.view.clone();
            view.update(cx, |view, cx| {
                view.set_page(super::super::panels::SessionPage::Tree, cx);
                view.focus(window, cx);
            });
        }
    }

    fn start_rename(&mut self, saved: &SavedSession, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| TextInput::new("Session name…", cx).compact());
        input.update(cx, |input, cx| {
            input.set_content(saved.title().to_owned(), cx)
        });
        input.focus_handle(cx).focus(window, cx);
        self.rename = Some(input);
        cx.notify();
    }

    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let (Some(input), Some(saved)) = (self.rename.take(), self.selected().cloned()) else {
            return;
        };
        let name = input.read(cx).content().trim().to_owned();
        if name.is_empty() || Some(name.as_str()) == saved.name.as_deref() {
            cx.notify();
            return;
        }
        let command = Command::SetSessionName {
            name,
            session_path: Some(saved.path.clone()),
        };
        self.run(saved, "Renamed.", command, cx);
    }

    /// Sends a command about a session to a pi that can take it and reports the result.
    fn run(
        &mut self,
        saved: SavedSession,
        done: &'static str,
        command: Command,
        cx: &mut Context<Self>,
    ) {
        if self.demo(cx) {
            return;
        }
        // Rename and export can go to the session's own pi, or any pi for a path.
        let own = self.workspace.read(cx).tabs.iter().find_map(|tab| {
            let controller = tab.controller.read(cx);
            (controller.model().state.session_file.as_deref() == Some(saved.path.as_str())
                && controller.is_connected())
            .then(|| tab.controller.clone())
        });
        if let Some(controller) = own {
            let sent = controller.update(cx, |controller, cx| controller.command(command, cx));
            self.notice = Some((
                sent.is_none(),
                if sent.is_some() {
                    done.into()
                } else {
                    "pi did not take it.".into()
                },
            ));
            self.refresh_list(cx);
            cx.notify();
            return;
        }
        self.busy = Some(done);
        let task = session_actions::on_session(
            PathBuf::from(&saved.cwd),
            saved.path,
            saved.id,
            command,
            crate::prefs::backend(cx),
        );
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                this.notice = Some(match result {
                    Ok(_) => (false, done.into()),
                    Err(error) => (true, format!("{error:#}")),
                });
                this.refresh_list(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Asks an open pi for the session list again, so renames and deletions show.
    fn refresh_list(&mut self, cx: &mut Context<Self>) {
        let controller = self
            .workspace
            .read(cx)
            .tabs
            .iter()
            .map(|tab| tab.controller.clone())
            .find(|controller| controller.read(cx).is_connected());
        if let Some(controller) = controller {
            controller.update(cx, |controller, cx| {
                controller.command(
                    Command::ListSessions {
                        scope: "all".into(),
                    },
                    cx,
                );
            });
        }
    }

    fn demo(&mut self, cx: &mut Context<Self>) -> bool {
        let demo = self.workspace.read(cx).is_demo();
        if demo {
            self.notice = Some((true, "Not available in the offline demo.".into()));
            cx.notify();
        }
        demo
    }

    fn clone_session(&mut self, saved: SavedSession, cx: &mut Context<Self>) {
        if self.demo(cx) {
            return;
        }
        self.busy = Some("Cloning…");
        let cwd = PathBuf::from(&saved.cwd);
        let task =
            session_actions::clone(cwd.clone(), saved.path, saved.id, crate::prefs::backend(cx));
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(clone) => {
                        this.notice = None;
                        this.workspace
                            .update(cx, |workspace, cx| workspace.open(cwd, Some(clone), cx));
                    }
                    Err(error) => this.notice = Some((true, format!("{error:#}"))),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn export_html(&mut self, saved: SavedSession, cx: &mut Context<Self>) {
        let directory = PathBuf::from(&saved.cwd);
        let name = format!("{}.html", file_stem(saved.title()));
        let path = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            this.update(cx, |this, cx| {
                let command = Command::ExportHtml {
                    output_path: Some(path.display().to_string()),
                };
                this.run(saved, "Exported as HTML.", command, cx)
            })
            .ok();
        })
        .detach();
    }

    fn export_jsonl(&mut self, saved: SavedSession, cx: &mut Context<Self>) {
        let directory = PathBuf::from(&saved.cwd);
        let name = format!("{}.jsonl", file_stem(saved.title()));
        let path = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            let from = saved.path.clone();
            let copy = cx
                .background_executor()
                .spawn(async move { std::fs::copy(from, path).map(|_| ()) })
                .await;
            this.update(cx, |this, cx| {
                this.notice = Some(match copy {
                    Ok(()) => (false, "Exported the session file.".into()),
                    Err(error) => (true, error.to_string()),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn share(&mut self, saved: SavedSession, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo(cx) {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Upload this session?",
            Some("pi uploads it to a private GitHub gist, or to Radius when signed in. It can contain prompts, replies, command output and file contents."),
            &["Upload", "Cancel"],
            cx,
        );
        let backend = crate::prefs::backend(cx);
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let task = session_actions::on_session(
                PathBuf::from(&saved.cwd),
                saved.path,
                saved.id,
                Command::Share,
                backend,
            );
            this.update(cx, |this, cx| {
                this.busy = Some("Uploading…");
                cx.notify();
            })
            .ok();
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                this.notice = Some(
                    match result.map(|data| data["url"].as_str().map(str::to_owned)) {
                        Ok(Some(url)) => {
                            cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                            (false, format!("Link copied: {url}"))
                        }
                        Ok(None) => (true, "pi returned no link.".into()),
                        Err(error) => (true, format!("{error:#}")),
                    },
                );
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn delete(&mut self, saved: SavedSession, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo(cx) {
            return;
        }
        if self.open_state(&saved.path, cx).is_some() {
            self.notice = Some((true, "Close this session's tab before deleting it.".into()));
            cx.notify();
            return;
        }
        let answer = window.prompt(
            PromptLevel::Critical,
            &format!("Delete “{}”?", saved.title()),
            Some("pi moves the session file to the trash when a trash command is installed, and deletes it otherwise."),
            &["Delete", "Cancel"],
            cx,
        );
        let backend = crate::prefs::backend(cx);
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let path = saved.path.clone();
            let task = session_actions::delete(PathBuf::from(&saved.cwd), saved.path, backend);
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(data) => {
                        this.notice = Some((
                            false,
                            if data["method"] == "trash" {
                                "Moved to the trash.".into()
                            } else {
                                "Deleted.".into()
                            },
                        ));
                        this.workspace
                            .update(cx, |workspace, cx| workspace.forget_saved(&path, cx));
                    }
                    Err(error) => this.notice = Some((true, format!("{error:#}"))),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn row(&self, index: usize, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let saved = &self.rows[index];
        let selected = self.selected.as_ref() == Some(&saved.path);
        let state = self.open_state(&saved.path, cx);
        let named = saved.name.as_deref().is_some_and(|n| !n.trim().is_empty());
        let path = saved.path.clone();
        let project = Path::new(&saved.cwd)
            .file_name()
            .map_or_else(|| saved.cwd.clone(), |n| n.to_string_lossy().into_owned());
        let now = now_ms();
        h_flex()
            .id(("session-row", index))
            .debug_selector(move || format!("session-row-{index}"))
            .relative()
            .w_full()
            .h(px(34.))
            .overflow_hidden()
            .px(px(24.))
            .gap(px(10.))
            .cursor_pointer()
            .bg(if selected {
                theme.selected
            } else if index % 2 == 1 {
                theme.hover.opacity(0.5)
            } else {
                gpui::transparent_black()
            })
            .hover(move |row| row.bg(theme.hover))
            .when(selected, |row| {
                row.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(7.))
                        .w(px(2.))
                        .h(px(20.))
                        .bg(theme.accent),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| this.select(path.clone(), cx)))
            .child(icon("chat", if selected { theme.accent } else { theme.faint }).size(px(14.)))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(10.))
                    .child(
                        div()
                            .id(("session-title", index))
                            .debug_selector(move || format!("session-title-{index}"))
                            .min_w_0()
                            .h(px(20.))
                            .line_height(px(20.))
                            .truncate()
                            .text_size(px(13.))
                            .when(named, |title| {
                                title
                                    .text_color(if selected {
                                        theme.text
                                    } else {
                                        theme.secondary
                                    })
                                    .child(single_line(saved.title()))
                            })
                            .when(!named, |title| {
                                title
                                    .italic()
                                    .text_color(theme.muted)
                                    .child(format!("“{}”", single_line(&saved.first_message)))
                            }),
                    )
                    .when(state == Some(true), |row| {
                        row.child(spinner(("session-working", index), theme))
                    })
                    .when(state == Some(false), |row| {
                        row.child(div().size(px(6.)).rounded_full().bg(theme.green))
                    })
                    .when(saved.parent_session_path.is_some(), |row| {
                        row.child(icon("git_branch", theme.faint).size(px(12.)))
                    }),
            )
            .child(
                div()
                    .w(px(150.))
                    .truncate()
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .text_color(theme.muted)
                    .child(project),
            )
            .child(
                div()
                    .w(px(70.))
                    .text_right()
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .text_color(theme.secondary)
                    .child(
                        saved
                            .message_count
                            .map_or_else(|| "—".into(), |n| n.to_string()),
                    ),
            )
            .child(
                div()
                    .w(px(110.))
                    .pl(px(24.))
                    .text_size(px(12.))
                    .text_color(theme.muted)
                    .child(
                        saved
                            .modified
                            .as_deref()
                            .and_then(parse_timestamp)
                            .map_or_else(|| "—".into(), |then| when(then, now)),
                    ),
            )
            .into_any_element()
    }

    /// The inspector column for the selected session.
    pub fn inspector(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let Some(saved) = self.selected().cloned() else {
            return v_flex()
                .p(px(20.))
                .child(note("No saved sessions match.", theme))
                .into_any_element();
        };
        let state = self.open_state(&saved.path, cx);
        let project = Path::new(&saved.cwd)
            .file_name()
            .map_or_else(|| saved.cwd.clone(), |n| n.to_string_lossy().into_owned());
        let parent = saved.parent_session_path.as_ref().map(|parent| {
            self.workspace
                .read(cx)
                .saved
                .iter()
                .find(|s| &s.path == parent)
                .map_or_else(|| parent.clone(), |s| s.title().to_owned())
        });
        let stamp = |text: &Option<String>| {
            text.as_deref()
                .and_then(parse_timestamp)
                .map_or_else(|| "—".to_owned(), date_time)
        };
        let busy = self.busy;
        let action = |id: &'static str, label: &'static str| {
            button(id, label, theme)
                .flex_1()
                .justify_center()
                .when(busy.is_some(), |button| button.opacity(0.5))
        };
        v_flex()
            .id("sessions-inspector")
            .debug_selector(|| "sessions-inspector".into())
            .size_full()
            .overflow_y_scroll()
            .px(px(20.))
            .pt(px(16.))
            .pb(px(16.))
            .gap(px(2.))
            .text_size(px(12.))
            .child(match &self.rename {
                Some(input) => div()
                    .key_context("SessionRename")
                    .on_action(cx.listener(|this, _: &Submit, _, cx| this.finish_rename(cx)))
                    .on_action(cx.listener(|this, _: &Stop, _, cx| {
                        this.rename = None;
                        cx.notify();
                    }))
                    .h(px(30.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(theme.focus)
                    .child(input.clone())
                    .into_any_element(),
                None => inspector_title(saved.title().to_owned(), true).into_any_element(),
            })
            .child(
                h_flex()
                    .mt(px(4.))
                    .h(px(22.))
                    .gap(px(8.))
                    .text_color(theme.secondary)
                    .child(
                        div()
                            .size(px(7.))
                            .rounded_full()
                            .border_1()
                            .border_color(theme.muted)
                            .when(state.is_some(), |dot| {
                                dot.bg(theme.green).border_color(theme.green)
                            }),
                    )
                    .child(match state {
                        Some(true) => "Open · working",
                        Some(false) => "Open",
                        None => "Closed",
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .text_color(theme.faint)
                            .child(project),
                    ),
            )
            .child(divider(theme).my(px(10.)))
            .child(section("DETAILS", "", theme))
            .child(detail("Project", tilde(&saved.cwd), true, theme))
            .child(detail("Created", stamp(&saved.created), false, theme))
            .child(detail("Modified", stamp(&saved.modified), false, theme))
            .child(detail(
                "Messages",
                saved
                    .message_count
                    .map_or_else(|| "—".into(), |n| n.to_string()),
                true,
                theme,
            ))
            .child(detail(
                "Forked from",
                parent.unwrap_or_else(|| "—".into()),
                false,
                theme,
            ))
            .child(divider(theme).my(px(10.)))
            .child(section("FIRST MESSAGE", "", theme))
            .child(
                div()
                    .mt(px(4.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(5.))
                    .bg(theme.hover)
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(theme.text)
                    .child(truncated(&saved.first_message, 400)),
            )
            .child(section("FILE", "", theme).mt(px(12.)))
            .child(
                h_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(10.5))
                            .text_color(theme.secondary)
                            .child(tilde(&saved.path)),
                    )
                    .child({
                        let path = saved.path.clone();
                        icon_button("copy-session-path", "copy", "Copy path", theme).on_click(
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(path.clone()))
                            },
                        )
                    }),
            )
            .child(divider(theme).my(px(10.)))
            .child(section("ACTIONS", "", theme))
            .child(
                primary_button(
                    "resume-session",
                    if state.is_some() { "Show" } else { "Resume" },
                    busy.is_none(),
                    theme,
                )
                .debug_selector(|| "resume-session".into())
                .w_full()
                .justify_center()
                .mt(px(4.))
                .on_click({
                    let saved = saved.clone();
                    cx.listener(move |this, _, _, cx| this.resume(saved.clone(), cx))
                }),
            )
            .child(
                h_flex()
                    .mt(px(6.))
                    .gap(px(6.))
                    .child(action("rename-session", "Rename…").on_click({
                        let saved = saved.clone();
                        cx.listener(move |this, _, window, cx| {
                            this.start_rename(&saved, window, cx)
                        })
                    }))
                    .child(action("fork-session", "Fork…").on_click({
                        let saved = saved.clone();
                        cx.listener(move |this, _, window, cx| this.fork(saved.clone(), window, cx))
                    }))
                    .child(action("clone-session", "Clone").on_click({
                        let saved = saved.clone();
                        cx.listener(move |this, _, _, cx| this.clone_session(saved.clone(), cx))
                    })),
            )
            .child(
                h_flex()
                    .mt(px(6.))
                    .gap(px(6.))
                    .child(action("export-html", "Export HTML").on_click({
                        let saved = saved.clone();
                        cx.listener(move |this, _, _, cx| this.export_html(saved.clone(), cx))
                    }))
                    .child(action("export-jsonl", "Export JSONL").on_click({
                        let saved = saved.clone();
                        cx.listener(move |this, _, _, cx| this.export_jsonl(saved.clone(), cx))
                    })),
            )
            .child(
                h_flex()
                    .mt(px(6.))
                    .gap(px(6.))
                    .child(action("share-session", "Share link…").on_click({
                        let saved = saved.clone();
                        cx.listener(move |this, _, window, cx| {
                            this.share(saved.clone(), window, cx)
                        })
                    }))
                    .child(
                        action("delete-session", "Delete")
                            .debug_selector(|| "delete-session".into())
                            .border_color(theme.danger_line)
                            .bg(theme.danger)
                            .text_color(theme.coral)
                            .on_click({
                                let saved = saved.clone();
                                cx.listener(move |this, _, window, cx| {
                                    this.delete(saved.clone(), window, cx)
                                })
                            }),
                    ),
            )
            .when_some(busy, |panel, busy| {
                panel.child(note(busy, theme).mt(px(8.)))
            })
            .when_some(self.notice.clone(), |panel, (error, text)| {
                panel.child(
                    div()
                        .mt(px(8.))
                        .text_size(px(11.))
                        .text_color(if error { theme.coral } else { theme.green })
                        .child(text),
                )
            })
            .child(div().flex_1().min_h(px(16.)))
            .child(note(
                "Share uploads to a private gist, or to Radius when signed in. Review it first.",
                theme,
            ))
            .into_any_element()
    }
}

impl Render for SessionsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let (all, here, named) = self.counts(cx);
        let weak = cx.entity().downgrade();
        let count = self.rows.len();
        let list = uniform_list("sessions-list", count, move |range, _, cx| {
            weak.update(cx, |this, cx| {
                range.map(|index| this.row(index, cx, theme)).collect()
            })
            .unwrap_or_default()
        })
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();
        v_flex()
            .id("all-sessions")
            .debug_selector(|| "all-sessions".into())
            .track_focus(&self.focus)
            .when(self.sort_open, |view| {
                view.on_action(cx.listener(|this, _: &super::super::Stop, _, cx| {
                    this.sort_open = false;
                    cx.stop_propagation();
                    cx.notify();
                }))
            })
            .size_full()
            .bg(theme.canvas)
            .child(
                h_flex()
                    .relative()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px(px(20.))
                    .gap(px(12.))
                    .bg(theme.panel)
                    .border_b_1()
                    .border_color(theme.line)
                    .child(segments(
                        [
                            (Filter::All, format!("All  {all}")),
                            (Filter::Project, format!("This project  {here}")),
                            (Filter::Named, format!("Named  {named}")),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(i, (filter, label))| {
                            segment(("session-filter", i), label, self.filter == filter, theme)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.filter = filter;
                                    this.refresh(cx);
                                }))
                        }),
                        theme,
                    ))
                    .child(
                        chip("session-sort", "Sort sessions", theme)
                            .debug_selector(|| "session-sort".into())
                            .child(self.sort.label())
                            .child(icon("chevron_down", theme.faint).size(px(11.)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.sort_open = !this.sort_open;
                                if this.sort_open {
                                    this.focus.focus(window, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.faint)
                            .child(self.sort.caption()),
                    )
                    .when(self.sort_open, |bar| {
                        bar.child(
                            gpui::deferred(
                                v_flex()
                                    .id("session-sort-menu")
                                    .debug_selector(|| "session-sort-menu".into())
                                    .absolute()
                                    .top(px(40.))
                                    .left(px(300.))
                                    .w(px(140.))
                                    .p(px(4.))
                                    .rounded(px(7.))
                                    .border_1()
                                    .border_color(theme.chip_line)
                                    .bg(if theme.light { theme.chip } else { theme.bar })
                                    .shadow_lg()
                                    .occlude()
                                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                        this.sort_open = false;
                                        cx.notify();
                                    }))
                                    .children(
                                        [Sort::Recent, Sort::Name, Sort::Messages].into_iter().map(
                                            |sort| {
                                                h_flex()
                                                    .id(sort.label())
                                                    .debug_selector(move || {
                                                        format!("session-sort-{}", sort.label())
                                                    })
                                                    .h(px(26.))
                                                    .px(px(8.))
                                                    .rounded(px(4.))
                                                    .text_size(px(12.))
                                                    .cursor_pointer()
                                                    .when(self.sort == sort, |row| {
                                                        row.bg(theme.selected)
                                                    })
                                                    .hover(move |row| row.bg(theme.hover))
                                                    .child(sort.label())
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.sort = sort;
                                                        this.sort_open = false;
                                                        this.refresh(cx);
                                                    }))
                                            },
                                        ),
                                    ),
                            )
                            .with_priority(2),
                        )
                    }),
            )
            .child(
                h_flex()
                    .h(px(28.))
                    .flex_shrink_0()
                    .px(px(24.))
                    .gap(px(10.))
                    .bg(theme.hover.opacity(0.6))
                    .border_b_1()
                    .border_color(theme.line)
                    .text_size(px(11.))
                    .text_color(theme.muted)
                    .child(div().w(px(14.)))
                    .child(div().flex_1().child("Name"))
                    .child(div().w(px(150.)).child("Project"))
                    .child(div().w(px(70.)).text_right().child("Messages"))
                    .child(div().w(px(110.)).pl(px(24.)).child("Modified")),
            )
            .child(if count == 0 {
                div()
                    .flex_1()
                    .p(px(24.))
                    .child(note(
                        if all == 0 {
                            "No saved sessions yet. pi lists them once a session has messages."
                        } else {
                            "No sessions match."
                        },
                        theme,
                    ))
                    .into_any_element()
            } else {
                list.into_any_element()
            })
            .child(
                div()
                    .h(px(30.))
                    .flex_shrink_0()
                    .px(px(20.))
                    .pt(px(8.))
                    .border_t_1()
                    .border_color(theme.line)
                    .text_size(px(11.))
                    .text_color(theme.faint)
                    .child(format!(
                        "Showing {count} of {all}. Unnamed sessions show their first message."
                    )),
            )
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_millis() as u64)
}

fn tilde(path: &str) -> String {
    match dirs::home_dir().and_then(|home| {
        Path::new(path)
            .strip_prefix(home)
            .ok()
            .map(Path::to_path_buf)
    }) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.to_owned(),
    }
}

/// List-only projection: retain the original multiline message in session data.
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncated(text: &str, chars: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// A file name from a session title.
fn file_stem(title: &str) -> String {
    let stem: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem.trim_matches('-');
    if stem.is_empty() {
        "session".into()
    } else {
        stem.chars().take(60).collect()
    }
}
