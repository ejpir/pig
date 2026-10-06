use super::*;
use super::{
    composer::ComposerView,
    inspector::InspectorView,
    session::{Changes, SessionController, SessionEvent},
    transcript::TranscriptView,
};
use gpui::Role;

pub struct SessionView {
    pub controller: Entity<SessionController>,
    pub composer: Entity<ComposerView>,
    pub transcript: Entity<TranscriptView>,
    /// Follow mode's stage beside the thread; closed until asked for.
    pub follow: Entity<super::follow::FollowView>,
    _follow_subscription: gpui::Subscription,
    pub landing: Entity<super::landing::LandingView>,
    landing_visible: bool,
    _landing_subscription: gpui::Subscription,
    pub inspector: Entity<InspectorView>,
    diagnostics: Entity<super::diagnostics::DiagnosticsView>,
    pub page: super::panels::SessionPage,
    pub tree: Entity<super::tree::TreeView>,
    pub file_changes: Entity<super::changes::ChangesView>,
    revision_focus_pending: bool,
    _changes_subscription: gpui::Subscription,
    pub context: Entity<super::context::ContextView>,
    pub files: Entity<super::files::FilesView>,
    files_visible: bool,
    _files_subscription: gpui::Subscription,
    focus: gpui::FocusHandle,
    focus_pending: bool,
    tools_open: bool,
    tools_index: usize,
    tools_previous_focus: Option<gpui::FocusHandle>,
    cwd: String,
    changes: usize,
    error: Option<String>,
    notice: Option<String>,
    /// A git-only project that jj can be turned on for.
    jj_offer: bool,
    pub terminal: Entity<super::terminal::TerminalDrawer>,
    /// A command from a bash row to type in a new terminal; that needs the window.
    open_in_terminal: Option<String>,
    /// An undo refused because later changes build on the turn: asked in a prompt,
    /// which needs the window.
    undo_conflict: Option<(usize, pi_jj::UndoConflict)>,
    /// Questions that need the window: where to work, and which files a fork takes.
    parallel_question: Option<super::session::ParallelPrompt>,
    fork_question: Option<(String, usize, String)>,
    pub(super) extension_dialogs: std::collections::VecDeque<serde_json::Value>,
    pub(super) active_extension: Option<String>,
    pub(super) cancel_extension: bool,
    _terminal_subscription: gpui::Subscription,
    _subscription: gpui::Subscription,
}
impl SessionView {
    pub fn new(controller: Entity<SessionController>, draft: &str, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| ComposerView::new(controller.clone(), draft, cx));
        let transcript = cx.new(|cx| TranscriptView::new(controller.clone(), cx));
        let follow = cx.new(|cx| super::follow::FollowView::new(controller.clone(), cx));
        transcript.update(cx, |transcript, cx| {
            transcript.set_follow(follow.clone(), cx)
        });
        let follow_subscription = cx.observe(&follow, |_, _, cx| cx.notify());
        let landing =
            cx.new(|cx| super::landing::LandingView::new(controller.clone(), composer.clone(), cx));
        let landing_subscription = cx.subscribe(&landing, |this, _, event, cx| match event {
            super::landing::LandingEvent::UseCommand(command) => this
                .composer
                .update(cx, |composer, cx| composer.use_command(command.clone(), cx)),
            super::landing::LandingEvent::Draft(text) => this
                .composer
                .update(cx, |composer, cx| composer.offer_draft(text, cx)),
        });
        let files = cx.new(|cx| {
            let files = super::files::FilesView::new(
                controller.read(cx).model().cwd.clone(),
                controller.read(cx).is_demo(),
                cx,
            );
            if let Some(target) = controller.read(cx).remote_target() {
                files.with_remote(target.clone())
            } else {
                files
            }
        });
        let inspector = cx.new(|cx| InspectorView::new(controller.clone(), cx));
        let diagnostics =
            cx.new(|cx| super::diagnostics::DiagnosticsView::new(controller.clone(), cx));
        let tree = cx.new(|cx| super::tree::TreeView::new(controller.clone(), cx));
        let file_changes = cx.new(|cx| {
            super::changes::ChangesView::new(
                controller.clone(),
                files.clone(),
                composer.clone(),
                cx,
            )
        });
        let changes_subscription = cx.subscribe(&file_changes, |this, _, event, cx| {
            let super::changes::ChangesEvent::RequestRevision { path, source } = event;
            this.composer.update(cx, |composer, cx| {
                composer.set_revision(
                    super::composer::RevisionContext {
                        path: path.clone(),
                        source: source.clone(),
                    },
                    cx,
                )
            });
            this.revision_focus_pending = true;
            cx.notify();
        });
        let context = cx.new(|cx| super::context::ContextView::new(controller.clone(), cx));
        let subscription = cx.subscribe(&controller, |this, controller, event, cx| {
            let landing_visible = controller.read(cx).not_started();
            if landing_visible != this.landing_visible {
                this.landing_visible = landing_visible;
                cx.notify();
            }
            if let SessionEvent::CancelExtensionDialog(id) = event {
                this.extension_dialogs.retain(|request| request["id"].as_str() != Some(id));
                if this.active_extension.as_ref() == Some(id) { this.cancel_extension = true; }
                cx.notify();
            }
            if let SessionEvent::ExtensionDialog(request) = event {
                if this.extension_dialogs.len() < 16 {
                    this.extension_dialogs.push_back(request.clone());
                } else {
                    controller.update(cx, |controller, cx| controller.answer_extension(
                        serde_json::json!({"type":"extension_ui_response","id":request["id"],"cancelled":true}), cx));
                }
                cx.notify();
            }
            if let SessionEvent::Changed(changes) = event
                && changes.intersects(Changes::STATUS) && !controller.read(cx).is_connected()
            {
                this.extension_dialogs.clear();
                this.cancel_extension = true;
            }
            if let SessionEvent::OpenInTerminal(command) = event {
                this.open_in_terminal = Some(command.clone());
                cx.notify();
            }
            if let SessionEvent::UndoConflict(record, conflict) = event {
                this.undo_conflict = Some((*record, conflict.clone()));
                cx.notify();
            }
            if let SessionEvent::ParallelQuestion(prompt) = event {
                this.parallel_question = Some((**prompt).clone());
                cx.notify();
            }
            if let SessionEvent::ForkQuestion { entry_id, before, text } = event {
                this.fork_question = Some((entry_id.clone(), *before, text.clone()));
                cx.notify();
            }
            if let SessionEvent::OpenFile(path) = event {
                let path = if controller.read(cx).is_remote() { std::path::PathBuf::from(path) } else { controller.read(cx).model().cwd.join(path) };
                this.files.update(cx, |files, cx| files.open(path, cx));
            }
            if let SessionEvent::OpenDirectory(path) = event {
                this.files.update(cx, |files, cx| files.reveal_directory(path, cx));
                this.inspector.update(cx, |inspector, cx| inspector.show(super::inspector::InspectorPage::Files(this.files.clone()), cx));
                this.focus_pending = false;
                cx.notify();
            }
            if let SessionEvent::ReviewFile(path,turn) = event {
                this.file_changes.update(cx, |view,cx|view.select_path(path,*turn,cx));
                this.set_page(super::panels::SessionPage::Changes,cx);
            }
            if let SessionEvent::ReviewTurn(index) = event {
                this.file_changes.update(cx, |view, cx| view.select_turn(*index, cx));
                this.set_page(super::panels::SessionPage::Changes, cx);
            }
            if matches!(event, SessionEvent::BranchChanged | SessionEvent::RevealTool(_) | SessionEvent::RevealMessage(_)) { this.set_page(super::panels::SessionPage::Thread,cx); }
            if matches!(event, SessionEvent::Changed(changes) if changes.intersects(Changes::METADATA | Changes::STATUS | Changes::SUMMARY | Changes::RUN | Changes::JJ)) {
                let controller = controller.read(cx);
                let model = controller.model();
                this.changes = controller.change_count();
                this.error = model.error.clone(); this.notice = model.notice.clone();
                this.jj_offer = controller.jj_offer().is_some();
                cx.notify();
            }
        });
        let files_subscription = cx.subscribe(&files, |this, _, event, cx| {
            match event {
                // A file on stage; its details wait in the inspector until asked for.
                super::files::FileEvent::Selected => {
                    this.files_visible = true;
                    this.inspector.update(cx, |view, cx| {
                        view.show(
                            super::inspector::InspectorPage::FileDetails(this.files.clone()),
                            cx,
                        )
                    });
                }
                super::files::FileEvent::Confirming => {
                    this.files_visible = true;
                    cx.notify();
                }
                super::files::FileEvent::Review(path) => {
                    this.file_changes
                        .update(cx, |view, cx| view.select_path(path, None, cx));
                    this.set_page(super::panels::SessionPage::Changes, cx);
                }
                super::files::FileEvent::ShowTree => {
                    this.set_page(super::panels::SessionPage::Tree, cx)
                }
                super::files::FileEvent::Empty => this.set_page(this.page, cx),
                super::files::FileEvent::ShowTurn(index) => {
                    this.controller
                        .update(cx, |c, cx| c.reveal_turn(*index, cx));
                }
                super::files::FileEvent::DiffTurn(index) => {
                    this.controller
                        .update(cx, |c, cx| c.review_turn(*index, cx));
                }
                super::files::FileEvent::Tabs => {}
                // The thread shows the request and pi's answer; the file reloads
                // when pi edits it.
                super::files::FileEvent::AskPi(prompt) => {
                    let sent = this.controller.update(cx, |controller, cx| {
                        controller.submit(prompt.clone(), true, cx)
                    });
                    this.set_page(super::panels::SessionPage::Thread, cx);
                    if !sent {
                        // pi cannot take a prompt right now; keep it as the draft.
                        let input = this.composer.read(cx).input.clone();
                        input.update(cx, |input, cx| input.set_content(prompt.clone(), cx));
                    }
                }
            }
            cx.notify();
        });
        files.update(cx, |files, _| files.set_session(controller.downgrade()));
        let mut cwd = controller.read(cx).model().cwd.clone();
        // The demo's projects are made up; its shells start in a real folder.
        if controller.read(cx).is_demo() && !cwd.is_dir() {
            cwd = std::env::temp_dir().join("pi-desktop-demo");
            std::fs::create_dir_all(&cwd).ok();
        }
        let remote = controller.read(cx).is_remote();
        let terminal = cx.new(|_| {
            let terminal = super::terminal::TerminalDrawer::new(cwd);
            if remote {
                terminal.unavailable()
            } else {
                terminal
            }
        });
        // The composer's `@` menu lists this session's files and shells.
        composer.update(cx, |composer, cx| {
            composer.set_sources(
                super::mention_menu::Sources {
                    files: Some(files.downgrade()),
                    terminal: Some(terminal.downgrade()),
                },
                cx,
            )
        });
        inspector.update(cx, |inspector, cx| inspector.set_composer(&composer, cx));
        let terminal_subscription = cx.subscribe(&terminal, |this, _, event, cx| {
            if let super::terminal::DrawerEvent::Hidden = event {
                this.focus_pending = true;
                cx.notify();
            }
        });
        let jj_offer = controller.read(cx).jj_offer().is_some();
        let model = controller.read(cx).model();
        Self {
            terminal,
            open_in_terminal: None,
            undo_conflict: None,
            parallel_question: None,
            fork_question: None,
            extension_dialogs: Default::default(),
            active_extension: None,
            cancel_extension: false,
            _terminal_subscription: terminal_subscription,
            page: Default::default(),
            files,
            files_visible: false,
            _files_subscription: files_subscription,
            tree,
            file_changes,
            revision_focus_pending: false,
            _changes_subscription: changes_subscription,
            context,
            focus: cx.focus_handle(),
            focus_pending: false,
            tools_open: false,
            tools_index: 0,
            tools_previous_focus: None,
            jj_offer,
            cwd: model.cwd.display().to_string(),
            changes: controller.read(cx).change_count(),
            error: model.error.clone(),
            notice: model.notice.clone(),
            landing_visible: controller.read(cx).not_started(),
            landing,
            _landing_subscription: landing_subscription,
            controller,
            composer,
            transcript,
            follow,
            _follow_subscription: follow_subscription,
            inspector,
            diagnostics,
            _subscription: subscription,
        }
    }
}
/// Below this, the thread is too narrow to share with the follow stage.
const FOLLOW_MIN_WIDTH: Pixels = px(900.);

impl SessionView {
    pub fn set_page(&mut self, page: super::panels::SessionPage, cx: &mut Context<Self>) {
        use super::{inspector::InspectorPage, panels::SessionPage};
        if self.controller.read(cx).is_remote() && page != SessionPage::Thread {
            self.controller.update(cx, |controller, cx| {
                controller.notice("Remote project views are not supported yet.", cx)
            });
            return;
        }
        self.page = page;
        self.tools_open = false;
        self.files_visible = false;
        self.focus_pending = true;
        let content = match page {
            SessionPage::Thread => InspectorPage::Thread,
            SessionPage::Tree => InspectorPage::Tree(self.tree.clone()),
            SessionPage::Changes => InspectorPage::Changes(self.file_changes.clone()),
            SessionPage::Context => InspectorPage::Context(self.context.clone()),
        };
        self.inspector
            .update(cx, |inspector, cx| inspector.show(content, cx));
        if page != SessionPage::Thread {
            self.controller.update(cx, |c, cx| c.load_views(cx));
        }
        cx.notify();
    }
    pub fn show_diagnostics(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_pending = false;
        self.inspector.update(cx, |inspector, cx| {
            inspector.show(
                super::inspector::InspectorPage::Diagnostics(self.diagnostics.clone()),
                cx,
            )
        });
        self.diagnostics
            .update(cx, |view, cx| view.focus(window, cx));
    }
    pub fn show_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_pending = false;
        self.inspector.update(cx, |inspector, cx| {
            inspector.show(
                super::inspector::InspectorPage::Files(self.files.clone()),
                cx,
            )
        });
        self.files
            .update(cx, |files, cx| files.focus_browser(window, cx));
    }
    pub fn toggle_follow(&mut self, cx: &mut Context<Self>) {
        if self.page != super::panels::SessionPage::Thread || self.files_visible {
            self.set_page(super::panels::SessionPage::Thread, cx);
            if self.follow.read(cx).open {
                return;
            }
        }
        self.follow.update(cx, |follow, cx| follow.toggle(cx));
    }
    pub fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.toggle(window, cx));
    }
    pub fn shows_work_status(&self, cx: &App) -> bool {
        !self.terminal.read(cx).is_maximized()
            && (self.files_visible
                || match self.page {
                    super::panels::SessionPage::Thread => true,
                    super::panels::SessionPage::Changes => {
                        self.file_changes.read(cx).shows_composer()
                    }
                    _ => false,
                })
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        if self.files_visible {
            self.files.update(cx, |files, cx| files.focus(window, cx));
        } else if self.page == super::panels::SessionPage::Tree {
            self.tree.update(cx, |tree, cx| tree.focus(window, cx));
        } else if self.page == super::panels::SessionPage::Changes {
            self.file_changes
                .update(cx, |changes, cx| changes.focus(window, cx));
        } else if self.page == super::panels::SessionPage::Thread {
            self.composer
                .read(cx)
                .input
                .focus_handle(cx)
                .focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
    }
}
impl SessionView {
    /// Undo with conflicts (design study 05, 10): undo anyway and draft a fix,
    /// take the later turns along, or leave it.
    fn ask_about_conflict(
        &mut self,
        record: usize,
        conflict: pi_jj::UndoConflict,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let controller = self.controller.read(cx);
        let records = &controller.jj().records;
        let Some(short) = records.get(record).map(|r| r.short.clone()) else {
            return;
        };
        // The later changes, when all are this session's turns and not undone.
        let later: Option<Vec<usize>> = conflict
            .later
            .iter()
            .map(|change| {
                records
                    .iter()
                    .position(|r| &r.change == change && r.undone.is_none())
            })
            .collect();
        let along = later.filter(|later| !later.is_empty() && !conflict.working_copy);
        let detail = format!(
            "Undoing turn {short} alone leaves conflict markers where {conflict}. jj keeps the conflict, so nothing is lost; the app can draft a prompt asking pi to resolve it."
        );
        let mut buttons = vec![gpui::PromptButton::ok("Undo and draft a fix")];
        if let Some(later) = &along {
            buttons.push(gpui::PromptButton::new(format!(
                "Undo {} turns",
                later.len() + 1
            )));
        }
        buttons.push(gpui::PromptButton::cancel("Cancel"));
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Later changes build on turn {short}"),
            Some(&detail),
            &buttons,
            cx,
        );
        let controller = self.controller.clone();
        cx.spawn(async move |_, cx| {
            let answer = answer.await.ok();
            controller.update(cx, |controller, cx| match (answer, along) {
                (Some(0), _) => controller.undo_turn_keeping_conflicts(record, cx),
                (Some(1), Some(mut later)) => {
                    later.push(record);
                    controller.undo_turns(later, cx)
                }
                _ => {}
            });
        })
        .detach();
    }
}

impl SessionView {
    fn ask_where_to_work(
        &mut self,
        prompt: super::session::ParallelPrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let question = super::parallel::Question {
            other: prompt.other.clone(),
            root: prompt.root.clone(),
            automatic: super::parallel::automatic_folder(&prompt.root, &prompt.content),
        };
        let answer = super::parallel::ask(question, window, cx);
        let controller = self.controller.clone();
        cx.spawn(async move |_, cx| {
            let answer = answer.recv().await.ok().flatten();
            controller.update(cx, |c, cx| c.answer_parallel(answer, prompt, cx));
        })
        .detach();
    }

    /// Fork with the files (design study 05, 05): as at the entry, in a new jj
    /// workspace, or as they are now, here.
    fn ask_which_files(
        &mut self,
        entry_id: String,
        before: usize,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let controller = self.controller.read(cx);
        let Some(short) = controller.jj().records.get(before).map(|r| r.short.clone()) else {
            return;
        };
        let cwd = controller.model().cwd.display().to_string();
        let title = text.lines().next().unwrap_or_default();
        let title: String = title.chars().take(60).collect();
        let answer = window.prompt(
            gpui::PromptLevel::Info,
            &format!("Fork from \u{201c}{title}\u{201d}"),
            Some(&format!(
                "The files changed after this entry. Which files should the new session start with?\n\nAs at this entry: a new jj workspace with the files from before turn {short}. The main folder is not touched.\n\nAs they are now: the new session works in {cwd} with today's files."
            )),
            &[
                gpui::PromptButton::ok("As at this entry"),
                gpui::PromptButton::new("As they are now"),
                gpui::PromptButton::cancel("Cancel"),
            ],
            cx,
        );
        let controller = self.controller.clone();
        cx.spawn(async move |_, cx| {
            let answer = answer.await.ok();
            controller.update(cx, |c, cx| match answer {
                Some(0) => c.fork_into_workspace(entry_id, before, cx),
                Some(1) => c.fork_session(entry_id, cx),
                _ => {}
            });
        })
        .detach();
    }
}

impl Render for SessionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.show_extension_dialog(window, cx);
        let waiting = self.active_extension.is_some();
        let contextual_issue = {
            let controller = self.controller.read(cx);
            self.page == super::panels::SessionPage::Thread
                && !self.files_visible
                && controller.is_connected()
                && !controller.working()
                && !super::session::latest_failed_tools(controller.model()).is_empty()
        };
        self.composer.update(cx, |view, cx| {
            view.set_waiting(waiting, cx);
            view.set_contextual_issue(contextual_issue, cx);
        });
        if let Some(command) = self.open_in_terminal.take() {
            self.terminal.update(cx, |terminal, cx| {
                terminal.new_terminal(Some(command), window, cx)
            });
        }
        if !window.has_active_prompt()
            && let Some((record, conflict)) = self.undo_conflict.take()
        {
            self.ask_about_conflict(record, conflict, window, cx);
        }
        if !window.has_active_prompt()
            && let Some(prompt) = self.parallel_question.take()
        {
            self.ask_where_to_work(prompt, window, cx);
        }
        if !window.has_active_prompt()
            && let Some((entry_id, before, text)) = self.fork_question.take()
        {
            self.ask_which_files(entry_id, before, text, window, cx);
        }
        if !window.has_active_prompt() && std::mem::take(&mut self.focus_pending) {
            self.focus(window, cx);
        }
        if !window.has_active_prompt() && std::mem::take(&mut self.revision_focus_pending) {
            self.composer
                .update(cx, |composer, cx| composer.focus(window, cx));
        }
        let theme = theme(cx);
        let workbench_demo =
            self.controller.read(cx).model().state.session_id.as_deref() == Some("demo-workbench");
        let project_label = if let Some(remote) = self.controller.read(cx).remote_target() {
            format!("SSH · {} · {}", remote.host, remote.cwd)
        } else if workbench_demo {
            "pi / main".to_owned()
        } else {
            std::path::Path::new(&self.cwd)
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new(&self.cwd))
                .to_string_lossy()
                .into_owned()
        };
        let drawer = self.terminal.read(cx);
        let (drawer_open, drawer_maximized) = (drawer.is_open(), drawer.is_maximized());
        v_flex()
            .id("session-view")
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .relative()
            .on_action(cx.listener(|this, _: &PreviousChoice, _, cx| {
                if this.tools_open {
                    cx.stop_propagation();
                    this.tools_index = (this.tools_index + 1) % 2;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &NextChoice, _, cx| {
                if this.tools_open {
                    cx.stop_propagation();
                    this.tools_index = (this.tools_index + 1) % 2;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                if this.tools_open {
                    cx.stop_propagation();
                    let page = if this.tools_index == 0 {
                        super::panels::SessionPage::Tree
                    } else {
                        super::panels::SessionPage::Context
                    };
                    this.set_page(page, cx);
                    this.focus(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Stop, window, cx| {
                if this.tools_open {
                    cx.stop_propagation();
                    this.tools_open = false;
                    if let Some(focus) = this.tools_previous_focus.take() {
                        focus.focus(window, cx);
                    }
                    cx.notify();
                }
            }))
            .on_drag_move(cx.listener(
                |this, event: &gpui::DragMoveEvent<super::terminal::DrawerResize>, _, cx| {
                    this.terminal
                        .update(cx, |terminal, cx| terminal.drag(event, cx))
                },
            ))
            .child(
                h_flex()
                    .id("session-tabs")
                    .overflow_x_scroll()
                    .h(px(40.))
                    .flex_shrink_0()
                    .px(px(24.))
                    .gap(px(24.))
                    .border_b_1()
                    .border_color(theme.line)
                    .children(
                        super::panels::SessionPage::ALL
                            .into_iter()
                            .filter(|page| {
                                // Before the first prompt there is nothing to review.
                                let changes = *page == super::panels::SessionPage::Changes
                                    && !(self.landing_visible && self.changes == 0);
                                *page == super::panels::SessionPage::Thread
                                    || changes
                                    || *page == self.page
                            })
                            .enumerate()
                            .map(|(i, page)| {
                                let title = if page == super::panels::SessionPage::Changes {
                                    format!("Changes  {}", self.changes)
                                } else {
                                    match page {
                                        super::panels::SessionPage::Thread => "Thread",
                                        super::panels::SessionPage::Tree => "Tree",
                                        super::panels::SessionPage::Context => "Context",
                                        _ => unreachable!(),
                                    }
                                    .into()
                                };
                                work_tab(
                                    ("session-tab", i),
                                    title,
                                    !self.files_visible && self.page == page,
                                    theme,
                                )
                                .when(page == super::panels::SessionPage::Changes, |tab| {
                                    tab.ml(px(9.)).pr(px(13.))
                                })
                                .debug_selector(move || {
                                    format!("tab-{}", page.name().to_lowercase())
                                })
                                .aria_label(page.name())
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.set_page(page, cx);
                                        this.focus(window, cx);
                                    },
                                ))
                            }),
                    )
                    .child(
                        self.files
                            .update(cx, |files, cx| files.tabs(self.files_visible, cx)),
                    )
                    .child(div().flex_1())
                    .when(!self.files.read(cx).has_tabs(), |tabs| {
                        tabs.child(
                            h_flex()
                                .id("session-tools")
                                .debug_selector(|| "session-tools".into())
                                .role(Role::Button)
                                .aria_label(format!("Session views for {project_label}"))
                                .aria_expanded(self.tools_open)
                                .h(px(28.))
                                .max_w(px(230.))
                                .px(px(8.))
                                .gap(px(6.))
                                .rounded(px(5.))
                                .font_family(MONO)
                                .text_size(px(12.))
                                .text_color(theme.muted)
                                .mr(px(-14.))
                                .cursor_pointer()
                                .when(self.tools_open, |button| button.bg(theme.selected))
                                .hover(move |button| button.bg(theme.hover))
                                .child(div().flex_1().min_w_0().truncate().child(project_label))
                                .child(
                                    icon(
                                        if self.tools_open {
                                            "chevron_up"
                                        } else {
                                            "chevron_down"
                                        },
                                        theme.faint,
                                    )
                                    .size(px(10.)),
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if this.tools_open {
                                        this.tools_open = false;
                                        if let Some(focus) = this.tools_previous_focus.take() {
                                            focus.focus(window, cx);
                                        }
                                    } else {
                                        this.tools_previous_focus = window.focused(cx);
                                        this.tools_open = true;
                                        this.focus.focus(window, cx);
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        h_flex()
                            .gap(px(2.))
                            .child({
                                let following = !self.files_visible
                                    && self.page == super::panels::SessionPage::Thread
                                    && self.follow.read(cx).open;
                                icon_button("follow-toggle", "eye", "Follow", theme)
                                    .debug_selector(|| "follow-toggle".into())
                                    .aria_expanded(following)
                                    .when(following, |button| button.bg(theme.selected))
                                    .tooltip(|_, cx| {
                                        ui::Tooltip::for_action(
                                            "Follow the agent beside the thread",
                                            &super::ToggleFollow,
                                            cx,
                                        )
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_follow(cx)))
                            })
                            .child(
                                icon_button("show-files", "folder", "Files", theme)
                                    .debug_selector(|| "show-files".into())
                                    .tooltip(ui::Tooltip::text("Browse project files"))
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(super::ShowFiles), cx);
                                    }),
                            )
                            // Always reachable, whatever the keyboard layout makes of ⌃`.
                            .child(
                                icon_button("terminal-toggle", "terminal", "Terminal", theme)
                                    .debug_selector(|| "terminal-toggle".into())
                                    .aria_expanded(drawer_open)
                                    .when(drawer_open, |button| button.bg(theme.selected))
                                    .tooltip(|_, cx| {
                                        ui::Tooltip::for_action(
                                            "Terminal",
                                            &super::ToggleTerminal,
                                            cx,
                                        )
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.terminal.update(cx, |terminal, cx| {
                                            terminal.show_or_hide(window, cx)
                                        })
                                    })),
                            ),
                    ),
            )
            .when(self.tools_open, |v| {
                v.child(
                    gpui::deferred(
                        v_flex()
                            .id("session-tools-menu")
                            .debug_selector(|| "session-tools-menu".into())
                            .occlude()
                            .absolute()
                            .top(px(42.))
                            .right(px(92.))
                            .w(px(240.))
                            .p(px(8.))
                            .gap(px(2.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme.line)
                            .bg(theme.composer)
                            .shadow_lg()
                            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                                this.tools_open = false;
                                if let Some(focus) = this.tools_previous_focus.take() {
                                    focus.focus(window, cx);
                                }
                                cx.notify();
                            }))
                            .children(
                                [
                                    super::panels::SessionPage::Tree,
                                    super::panels::SessionPage::Context,
                                ]
                                .into_iter()
                                .enumerate()
                                .map(|(index, page)| {
                                    let (label, glyph) = if page == super::panels::SessionPage::Tree
                                    {
                                        ("Session tree", "list_tree")
                                    } else {
                                        ("Context & compaction", "compact")
                                    };
                                    work_menu_item(
                                        SharedString::from(format!("session-tool-{}", page.name())),
                                        label,
                                        glyph,
                                        self.tools_index == index,
                                        theme,
                                    )
                                    .debug_selector(move || {
                                        format!("tab-{}", page.name().to_lowercase())
                                    })
                                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                        if *hovered && this.tools_index != index {
                                            this.tools_index = index;
                                            cx.notify();
                                        }
                                    }))
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.set_page(page, cx);
                                            this.focus(window, cx);
                                        },
                                    ))
                                }),
                            ),
                    )
                    .with_priority(2),
                )
            })
            .when_some(self.error.clone(), |column, error| {
                column.child(
                    h_flex()
                        .debug_selector(|| "session-error".into())
                        .items_start()
                        .gap(px(8.))
                        .m(px(12.))
                        .p(px(10.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(theme.coral)
                        .text_color(theme.coral)
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(6.))
                                .child(div().truncate().child(
                                    error.lines().next().unwrap_or("Session error").to_owned(),
                                ))
                                .child(
                                    h_flex()
                                        .gap(px(8.))
                                        .child(
                                            button("view-error-details", "View details", theme)
                                                .debug_selector(|| "view-error-details".into())
                                                .on_click(|_, window, cx| {
                                                    window.dispatch_action(
                                                        Box::new(super::ShowDiagnostics),
                                                        cx,
                                                    )
                                                }),
                                        )
                                        .child(
                                            button("copy-error-details", "Copy diagnostics", theme)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(
                                                            this.controller.read(cx).diagnostics(),
                                                        ),
                                                    )
                                                })),
                                        ),
                                ),
                        )
                        .child(
                            icon_button(
                                "dismiss-error",
                                "close",
                                "Dismiss error",
                                Theme {
                                    muted: theme.coral,
                                    ..theme
                                },
                            )
                            .size(px(20.))
                            .debug_selector(|| "dismiss-error".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.controller
                                    .update(cx, |controller, cx| controller.dismiss_error(cx))
                            })),
                        ),
                )
            })
            .when_some(
                self.controller.read(cx).model().shell.as_ref(),
                |column, shell| {
                    column.child(
                        v_flex()
                            .debug_selector(|| "live-shell".into())
                            .mx(px(24.))
                            .my(px(8.))
                            .p(px(12.))
                            .gap(px(6.))
                            .rounded(px(7.))
                            .bg(theme.deep)
                            .child(
                                h_flex()
                                    .gap(px(8.))
                                    .child(icon("terminal", theme.amber))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .font_family(MONO)
                                            .text_size(px(12.))
                                            .child(format!("$ {}", shell.command)),
                                    )
                                    .child(div().text_size(px(11.)).text_color(theme.muted).child(
                                        if shell.finished {
                                            "Finished"
                                        } else {
                                            "Running"
                                        },
                                    ))
                                    .when(!shell.finished, |row| {
                                        row.child(button("stop-shell", "Stop", theme).on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.controller.update(cx, |controller, cx| {
                                                    controller.command(Command::AbortBash, cx);
                                                });
                                            }),
                                        ))
                                    })
                                    .child(
                                        div()
                                            .id("live-shell-output")
                                            .max_h(px(140.))
                                            .overflow_y_scroll()
                                            .font_family(MONO)
                                            .text_size(px(11.))
                                            .line_height(px(17.))
                                            .child(shell.output.clone()),
                                    ),
                            ),
                    )
                },
            )
            .when(self.jj_offer, |column| {
                let enabled = !self.controller.read(cx).working();
                column.child(
                    h_flex()
                        .debug_selector(|| "jj-offer".into())
                        .mx(px(24.))
                        .mt(px(12.))
                        .px(px(14.))
                        .py(px(10.))
                        .gap(px(12.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.line)
                        .bg(theme.panel)
                        .child(icon("undo", theme.accent))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Undo agent turns with jj"),
                                )
                                .child(div().text_size(px(12.)).text_color(theme.muted).child(
                                    "This project uses git only. jj records each turn that edits \
                                     files as a commit; git HEAD follows it (detached) and \
                                     branches stay as they are.",
                                )),
                        )
                        .child(button("jj-decline", "Not now", theme).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.controller
                                    .update(cx, |controller, cx| controller.decline_jj(cx))
                            },
                        )))
                        .child(
                            primary_button("jj-enable", "Turn on jj", enabled, theme).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.controller
                                        .update(cx, |controller, cx| controller.enable_jj(cx))
                                }),
                            ),
                        ),
                )
            })
            .child(if drawer_maximized {
                div().into_any_element()
            } else if self.files_visible {
                // The file takes the stage; the composer stays below it, so file
                // and revision requests remain in the work area.
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        self.files
                            .clone()
                            .cached(gpui::StyleRefinement::default().flex_1().min_h_0().w_full()),
                    )
                    .child(self.composer.clone())
                    .into_any_element()
            } else {
                match self.page {
                    super::panels::SessionPage::Thread => {
                        // Before the first prompt the landing leads with the composer;
                        // after it, the composer docks below the thread.
                        let thread = if self.landing_visible {
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(self.landing.clone())
                        } else {
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(self.transcript.clone().cached(
                                    gpui::StyleRefinement::default().flex_1().min_h_0().w_full(),
                                ))
                                .child(self.composer.clone())
                        };
                        if self.landing_visible || !self.follow.read(cx).open {
                            thread.into_any_element()
                        } else {
                            let follow = self.follow.clone();
                            // The stage needs room beside a readable thread; on small
                            // windows the thread keeps the whole width.
                            gpui::container_query(move |size, _, _| {
                                if size.width < FOLLOW_MIN_WIDTH {
                                    return thread.size_full().into_any_element();
                                }
                                h_flex()
                                    .size_full()
                                    .items_start()
                                    .child(thread.h_full().min_w_0())
                                    .child(
                                        div()
                                            .h_full()
                                            .flex_shrink_0()
                                            .w((size.width * 0.5).max(px(420.)))
                                            .border_l_1()
                                            .border_color(theme.line)
                                            .child(follow.cached(
                                                gpui::StyleRefinement::default().size_full(),
                                            )),
                                    )
                                    .into_any_element()
                            })
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .into_any_element()
                        }
                    }
                    super::panels::SessionPage::Tree => self
                        .tree
                        .clone()
                        .cached(gpui::StyleRefinement::default().flex_1().min_h_0().w_full())
                        .into_any_element(),
                    super::panels::SessionPage::Changes => self
                        .file_changes
                        .clone()
                        .cached(gpui::StyleRefinement::default().flex_1().min_h_0().w_full())
                        .into_any_element(),
                    super::panels::SessionPage::Context => self
                        .context
                        .clone()
                        .cached(gpui::StyleRefinement::default().flex_1().min_h_0().w_full())
                        .into_any_element(),
                }
            })
            .when(drawer_open, |column| column.child(self.terminal.clone()))
            .when_some(self.notice.clone(), |column, notice| {
                let displayed = notice.clone();
                let close = icon_button(
                    "session-notice-close",
                    "close",
                    "Dismiss notification",
                    theme,
                )
                .debug_selector(|| "session-notice-close".into())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.controller.update(cx, |controller, cx| {
                        if controller.model().notice.as_ref() == Some(&displayed) {
                            controller.dismiss_notice(cx);
                        }
                    });
                }))
                .into_any_element();
                column.child(crate::components::notification_overlay(
                    crate::components::notification_card(
                        "session-notice",
                        &notice,
                        false,
                        close,
                        theme,
                    )
                    .into_any_element(),
                    px(56.),
                ))
            })
    }
}
