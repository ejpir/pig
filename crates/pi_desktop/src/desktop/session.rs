use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::anyhow;
use gpui::{App, Context, EventEmitter, Task};
use pi_core::{
    protocol::{Command, ImageContent, SavedSession},
    session::{RunState, Session, Tool, text},
    transport::{Launch, RpcClient, TransportEvent},
};
use pi_jj::{ObjectId as _, Project, Vcs};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes(u16);
impl Changes {
    pub const RUN: Self = Self(1);
    pub const METADATA: Self = Self(2);
    pub const QUEUE: Self = Self(4);
    pub const CATALOG: Self = Self(8);
    pub const SAVED: Self = Self(16);
    pub const STATUS: Self = Self(32);
    pub const SUMMARY: Self = Self(64);
    pub const JJ: Self = Self(128);
    pub const HISTORY: Self = Self(256);
    pub const CONTEXT: Self = Self(512);
    pub const DIAGNOSTICS: Self = Self(1024);
    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}
impl std::ops::BitOr for Changes {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}
impl std::ops::BitOrAssign for Changes {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

#[derive(Clone, Debug)]
pub enum ContentChange {
    Reset,
    Message(usize),
    Append(usize),
    Tool(String),
}
/// Failed tools belonging to the latest user turn. Earlier failures remain in
/// their own activity groups and must not color the current result.
pub(super) fn latest_failed_tools(model: &Session) -> Vec<&Tool> {
    let start = model
        .messages
        .iter()
        .rposition(|message| message["role"] == "user")
        .unwrap_or(0);
    let ids: HashSet<&str> = model.messages[start..]
        .iter()
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .filter(|block| block["type"] == "toolCall")
        .filter_map(|block| block["id"].as_str())
        .collect();
    model
        .tools
        .iter()
        .filter(|tool| tool.is_error && ids.contains(tool.id.as_str()))
        .collect()
}

#[derive(Clone, Debug)]
pub enum SessionEvent {
    Changed(Changes),
    Content(ContentChange),
    RequestFinished(String),
    RecoverDraft(String),
    /// Images of a prompt pi did not take, back to the composer with its text.
    RecoverImages(Vec<ImageContent>),
    RevealTail,
    /// Scroll the thread to a message, such as a turn's last one.
    RevealMessage(usize),
    BranchChanged,
    RevealTool(String),
    ReviewTurn(usize),
    ReviewFile(String, Option<usize>),
    OpenFork(Box<pi_core::session_actions::Forked>),
    /// Another session is working in this jj workspace: where should this
    /// session's first run work? (design study 05, 04)
    ParallelQuestion(Box<ParallelPrompt>),
    /// Start again in a new jj workspace, with this prompt.
    MoveToWorkspace(Box<MoveToWorkspace>),
    /// Turns changed the files after a forked entry: fork with the files as they
    /// were there, or as they are now? (design study 05, 05)
    ForkQuestion {
        entry_id: String,
        /// The first record after the entry.
        before: usize,
        text: String,
    },
    /// Type a command in a new terminal, from a bash row.
    OpenInTerminal(String),
    /// A project-relative file to show in a file tab, from a mention chip.
    OpenFile(String),
    OpenDirectory(String),
    ExtensionDialog(Value),
    /// Undoing a turn was refused because later changes build on it; the view
    /// asks what to do instead.
    UndoConflict(usize, pi_jj::UndoConflict),
    CancelExtensionDialog(String),
}

/// A first prompt held while the user answers where to work.
#[derive(Clone, Debug)]
pub struct ParallelPrompt {
    /// The session working in the same folder.
    pub other: String,
    pub root: PathBuf,
    pub content: String,
    pub images: Vec<ImageContent>,
    pub follow_up: bool,
}

#[derive(Clone, Debug)]
pub struct MoveToWorkspace {
    pub cwd: PathBuf,
    /// The folder whose workspace it came from, for Bring turns in.
    pub main: PathBuf,
    pub content: String,
    pub images: Vec<ImageContent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub cwd: PathBuf,
    pub title: String,
    pub file: Option<String>,
    pub busy: bool,
    pub connected: bool,
    pub modified: Option<u64>,
}

pub struct SessionController {
    model: Session,
    summary: Summary,
    demo: bool,
    client: Option<RpcClient>,
    remote: Option<pi_core::ssh::SshTarget>,
    remote_starting: bool,
    task: Option<Task<()>>,
    bootstrap: HashSet<String>,
    bootstrap_failed: bool,
    submissions: HashMap<String, String>,
    /// The images sent with a pending submission, by request id.
    submitted_images: HashMap<String, Vec<ImageContent>>,
    clear_request: Option<(String, bool)>,
    connected: bool,
    expected_resume_id: Option<String>,
    initial_model: Option<(String, String)>,
    #[cfg(test)]
    pub(super) extension_responses: Vec<Value>,
    jj: super::jj::Jj,
    view_request: Option<String>,
    views_loaded: bool,
    diagnostics: super::diagnostics::DiagnosticLog,
    /// Lets pi hear language-server errors; `None` in the demo or without a socket.
    _lsp: Option<pi_lsp_bridge::Bridge>,
    /// What runs this session, for Settings; `None` in the demo.
    program: Option<String>,
    /// The jj workspace this session reports working in (`parallel`).
    working_in: Option<PathBuf>,
    /// The first run may start although another session works here.
    parallel_ok: bool,
    /// A prompt to send once this session is ready, after moving into a workspace.
    pending_prompt: Option<(String, Vec<ImageContent>)>,
    /// When this session works in a jj workspace of its own: the main folder its
    /// turns can be brought into.
    main_folder: Option<PathBuf>,
}
impl EventEmitter<SessionEvent> for SessionController {}

impl SessionController {
    pub fn diagnostics(&self) -> String {
        format!(
            "Pi Desktop {}\nProject: {}\nSession: {}\nSession file: {}\nConnected: {} · Bootstrap failed: {}\n\n{}",
            env!("CARGO_PKG_VERSION"),
            self.model.cwd.display(),
            self.model
                .state
                .session_id
                .as_deref()
                .unwrap_or("not reported"),
            self.model
                .state
                .session_file
                .as_deref()
                .unwrap_or("not reported"),
            self.connected,
            self.bootstrap_failed,
            self.diagnostics.text()
        )
    }
    pub fn clear_diagnostics(&mut self, cx: &mut Context<Self>) {
        self.diagnostics = Default::default();
        self.publish(Changes::DIAGNOSTICS, cx);
    }
    pub fn model(&self) -> &Session {
        &self.model
    }
    pub fn summary(&self) -> &Summary {
        &self.summary
    }
    pub fn is_demo(&self) -> bool {
        self.demo
    }
    pub fn pid(&self) -> Option<u32> {
        if self.is_remote() {
            None
        } else {
            self.client.as_ref().map(RpcClient::pid)
        }
    }
    pub fn is_remote(&self) -> bool {
        self.remote.is_some()
    }
    pub fn remote_target(&self) -> Option<&pi_core::ssh::SshTarget> {
        self.remote.as_ref()
    }
    pub fn answer_extension(&mut self, response: Value, cx: &mut Context<Self>) {
        #[cfg(test)]
        self.extension_responses.push(response.clone());
        if let Some(client) = &self.client
            && let Err(error) = client.send_record(response)
        {
            self.error(error.to_string(), cx);
        }
    }
    pub fn is_connected(&self) -> bool {
        self.connected
    }
    pub fn bootstrap_failed(&self) -> bool {
        self.bootstrap_failed
    }
    pub fn jj(&self) -> &super::jj::Jj {
        &self.jj
    }
    pub fn program(&self) -> Option<&str> {
        self.program.as_deref()
    }
    pub fn close_blocked(&self) -> bool {
        self.jj.busy || self.jj.starting || self.view_request.is_some()
    }
    /// Stop and reap this session's process tree before a saved file can be reopened.
    pub fn shutdown(&mut self) {
        self.task.take();
        self.client.take();
        self.connected = false;
    }
    pub fn change_count(&self) -> usize {
        self.jj
            .records
            .iter()
            .flat_map(|r| r.diff.iter().map(|f| f.path.as_str()))
            .chain(
                self.model
                    .tools
                    .iter()
                    .filter(|t| {
                        t.finished && !t.is_error && matches!(t.name.as_str(), "edit" | "write")
                    })
                    .filter_map(|t| t.args["path"].as_str())
                    .filter(|p| !p.is_empty()),
            )
            .collect::<HashSet<_>>()
            .len()
    }
    pub fn not_started(&self) -> bool {
        self.model.messages.is_empty() && !self.working()
    }
    pub fn open_in_terminal(&self, command: String, cx: &mut Context<Self>) {
        if self.is_remote() {
            return;
        }
        cx.emit(SessionEvent::OpenInTerminal(command));
    }
    /// Drops a deleted session from this pi's last session list.
    pub fn forget_saved(&mut self, path: &str) {
        self.model.saved.retain(|saved| saved.path != path);
    }
    pub fn open_file(&self, path: String, cx: &mut Context<Self>) {
        cx.emit(SessionEvent::OpenFile(path));
    }
    /// Shows a turn's line in the thread, when its message is in this conversation.
    pub fn reveal_turn(&self, index: usize, cx: &mut Context<Self>) {
        if let Some(record) = self.jj.records.get(index).filter(|r| r.anchored) {
            cx.emit(SessionEvent::RevealMessage(record.after_message));
        }
    }
    pub fn review_turn(&self, index: usize, cx: &mut Context<Self>) {
        if index < self.jj.records.len() {
            cx.emit(SessionEvent::ReviewTurn(index));
        }
    }

    pub fn new(
        cwd: PathBuf,
        saved: Option<SavedSession>,
        demo: bool,
        first: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_remote(cwd, saved, None, demo, first, cx)
    }
    pub fn new_remote(target: pi_core::ssh::SshTarget, cx: &mut Context<Self>) -> Self {
        Self::new_with_remote(target.identity(), None, Some(target), false, false, cx)
    }
    #[cfg(test)]
    pub fn remote_test(target: pi_core::ssh::SshTarget, cx: &mut Context<Self>) -> Self {
        let mut controller =
            Self::new_with_remote(target.identity(), None, Some(target), true, false, cx);
        controller.demo = false;
        controller
    }
    fn new_with_remote(
        cwd: PathBuf,
        saved: Option<SavedSession>,
        remote: Option<pi_core::ssh::SshTarget>,
        demo: bool,
        first: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut model = Session::new(cwd.clone());
        if let Some(saved) = &saved {
            model.preview_title = Some(saved.title().to_owned());
            model.state.session_name = saved.name.clone();
            model.state.session_file = Some(saved.path.clone());
            model.state.session_id = Some(saved.id.clone());
        }
        let mut diagnostics = super::diagnostics::DiagnosticLog::default();
        let mut lsp = None;
        let mut program = None;
        if !demo && remote.is_none() {
            crate::prefs::load_project(cx, &cwd);
        }
        let client = if remote.is_some() {
            None
        } else if demo {
            super::demo::load(&mut model, saved.as_ref(), first);
            None
        } else {
            let backend = crate::prefs::backend(cx);
            let mut launch = Launch::pi_with(
                cwd.clone(),
                saved.as_ref().map(|s| s.path.as_str()),
                &backend,
            );
            program = Some(
                std::iter::once(&launch.program)
                    .chain(launch.args.first())
                    .map(|part| part.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            if crate::prefs::flag(cx, "jj.tools", Some(&cwd)) {
                launch.env.push(("PI_DESKTOP_JJ_TOOLS".into(), "1".into()));
            }
            let weak = cx.weak_entity();
            let handler: pi_lsp_bridge::Handler =
                std::rc::Rc::new(move |request, cx| match weak.upgrade() {
                    Some(controller) => controller
                        .update(cx, |controller, cx| controller.bridge_request(request, cx)),
                    None => Task::ready(Ok(String::new())),
                });
            lsp = lsp_bridge(&cwd, &mut launch, handler, &mut diagnostics, cx);
            diagnostics.push(
                "launch",
                &format!(
                    "Program: {} (arguments and environment not logged)",
                    launch.program.to_string_lossy()
                ),
            );
            match RpcClient::spawn(launch) {
                Ok(client) => Some(client),
                Err(error) => {
                    diagnostics.push("spawn failed", &error.to_string());
                    model.error = Some(error.to_string());
                    None
                }
            }
        };
        let connected = demo || client.is_some();
        let summary = Self::summarize(&model, connected, remote.as_ref());
        let jj = if demo {
            super::demo::workspace_history(&model)
        } else {
            Default::default()
        };
        let main_folder = if remote.is_none() {
            main_folder_of(&cwd)
        } else {
            None
        };
        let mut this = Self {
            model,
            summary,
            client,
            remote,
            remote_starting: false,
            task: None,
            demo,
            connected,
            bootstrap: HashSet::new(),
            bootstrap_failed: false,
            submissions: HashMap::new(),
            submitted_images: HashMap::new(),
            clear_request: None,
            expected_resume_id: saved.map(|s| s.id),
            initial_model: None,
            #[cfg(test)]
            extension_responses: Vec::new(),
            jj,
            view_request: None,
            views_loaded: false,
            diagnostics,
            _lsp: lsp,
            program,
            working_in: None,
            parallel_ok: false,
            pending_prompt: None,
            main_folder,
        };
        let id = cx.entity_id();
        cx.on_release(move |_, cx| super::parallel::set_working(cx, id, None))
            .detach();
        if !demo && !this.is_remote() {
            match pi_jj::detect(&this.model.cwd) {
                Vcs::Jj { root } => this.open_jj(root, false, cx),
                Vcs::Git { root }
                    if crate::prefs::choice(cx, "jj.offer", Some(&this.model.cwd)) != "never"
                        && !crate::prefs::jj_declined(cx, &root) =>
                {
                    this.jj.offer = Some(root)
                }
                Vcs::Git { .. } | Vcs::None => {}
            }
        }
        if let Some(client) = &this.client {
            let events = client.events();
            for command in [
                Command::GetState,
                Command::GetMessages,
                Command::GetSessionStats,
                Command::ListSessions {
                    scope: "all".into(),
                },
            ] {
                let name = command.name();
                match client.send(command) {
                    Ok(id) => {
                        this.diagnostics
                            .push("setup request", &format!("{name} · {id}"));
                        this.bootstrap.insert(id);
                    }
                    Err(error) => {
                        this.bootstrap_failed = true;
                        this.model.error = Some(error.to_string());
                    }
                }
            }
            this.task = Some(cx.spawn(async move |this, cx| {
                while let Ok(event) = events.recv().await {
                    if this.update(cx, |this, cx| this.receive(event, cx)).is_err() {
                        break;
                    }
                }
            }));
            // Optional metadata: populate the landing cards without opening slash,
            // and Settings' backend versions. Failure does not invalidate the
            // identity-critical bootstrap.
            this.command(Command::GetCommands, cx);
            this.command(Command::GetBackendInfo, cx);
            this.jj.links_request = this.command(super::turn_links::request(), cx);
        }
        if this.is_remote() {
            this.connect_remote(cx);
        }
        this
    }

    /// Bootstrap and reconnect never resend a prompt or mutate the remote agent.
    pub fn connect_remote(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.remote.clone() else {
            return;
        };
        if self.remote_starting || self.connected {
            return;
        }
        self.remote_starting = true;
        self.bootstrap_failed = false;
        self.model.error = None;
        self.client.take();
        self.publish(Changes::STATUS, cx);
        cx.spawn(async move |this, cx| {
            let attached = target.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let client = RpcClient::spawn_forwarded(pi_core::ssh::install(&attached)?)?;
                    client.send_record(attached.attach_record())?;
                    Ok::<_, anyhow::Error>(client)
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(client) => {
                    this.program = Some(format!("SSH · {}", target.host));
                    let events = client.events();
                    this.client = Some(client);
                    this.task = Some(cx.spawn(async move |this, cx| {
                        while let Ok(event) = events.recv().await {
                            if this.update(cx, |this, cx| this.receive(event, cx)).is_err() {
                                break;
                            }
                        }
                    }));
                    this.publish(Changes::STATUS, cx);
                }
                Err(error) => {
                    this.remote_starting = false;
                    this.error(format!("SSH connection failed: {error:#}"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn summarize(
        model: &Session,
        connected: bool,
        remote: Option<&pi_core::ssh::SshTarget>,
    ) -> Summary {
        Summary {
            cwd: model.cwd.clone(),
            title: model.title().to_owned(),
            file: remote
                .map(|target| format!("ssh-session://{}/{}", target.host, target.key))
                .or_else(|| model.state.session_file.clone()),
            busy: model.busy(),
            connected,
            modified: model
                .messages
                .iter()
                .rev()
                .find_map(|message| message["timestamp"].as_u64()),
        }
    }
    pub fn ready(&self) -> bool {
        self.connected
            && self.bootstrap.is_empty()
            && !self.bootstrap_failed
            && self.initial_model.is_none()
    }
    pub fn connecting(&self) -> bool {
        self.remote_starting || !self.bootstrap.is_empty() || self.initial_model.is_some()
    }
    pub fn set_initial_model(&mut self, model: (String, String), cx: &mut Context<Self>) {
        self.initial_model = Some(model);
        self.apply_initial_model(cx);
        self.publish(Changes::STATUS, cx);
    }
    fn apply_initial_model(&mut self, cx: &mut Context<Self>) {
        if !self.connected || !self.bootstrap.is_empty() || self.bootstrap_failed {
            return;
        }
        if let Some((provider, model_id)) = self.initial_model.take() {
            if let Some(id) = self.command(
                Command::SetModel {
                    provider,
                    model_id,
                    persist: false,
                },
                cx,
            ) {
                self.bootstrap.insert(id);
            } else if !self.demo {
                self.bootstrap_failed = true;
            }
        }
    }
    pub fn working(&self) -> bool {
        self.model.busy()
            || !self.submissions.is_empty()
            || self.jj.starting
            || self.view_request.is_some()
    }
    fn publish(&mut self, mut changes: Changes, cx: &mut Context<Self>) {
        let working_in = self.working().then(|| self.jj.root.clone()).flatten();
        if working_in != self.working_in {
            self.working_in = working_in.clone();
            let title = self.model.title().to_owned();
            let id = cx.entity_id();
            super::parallel::set_working(cx, id, working_in.map(|root| (root, title)));
        }
        let summary = Self::summarize(&self.model, self.connected, self.remote.as_ref());
        if summary != self.summary {
            self.summary = summary;
            changes |= Changes::SUMMARY;
        }
        if changes != Changes::default() {
            cx.emit(SessionEvent::Changed(changes));
        }
    }
    pub fn notice(&mut self, notice: impl Into<String>, cx: &mut Context<Self>) {
        self.model.notice = Some(notice.into());
        self.publish(Changes::STATUS, cx);
    }
    pub fn error(&mut self, error: impl Into<String>, cx: &mut Context<Self>) {
        let error = error.into();
        self.diagnostics.push("error", &error);
        self.model.error = Some(error);
        self.publish(Changes::STATUS | Changes::DIAGNOSTICS, cx);
    }
    pub fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.model.notice = None;
        self.publish(Changes::STATUS, cx);
    }
    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.model.error = None;
        self.publish(Changes::STATUS, cx);
    }
    pub fn reveal_tool(&mut self, id: String, cx: &mut Context<Self>) {
        cx.emit(SessionEvent::RevealTool(id));
    }
    /// Whether this session's first run should ask where to work: another
    /// session is working in the same jj workspace.
    fn crowded(&self, cx: &Context<Self>) -> Option<(PathBuf, String)> {
        if self.parallel_ok || !self.not_started() || self.jj.project.is_none() {
            return None;
        }
        let root = self.jj.root.clone()?;
        let other = super::parallel::working_elsewhere(cx, cx.entity_id(), &root)?;
        Some((root, other))
    }

    /// The answer to [`SessionEvent::ParallelQuestion`]; `None` is Cancel.
    pub fn answer_parallel(
        &mut self,
        answer: Option<super::parallel::Answer>,
        prompt: ParallelPrompt,
        cx: &mut Context<Self>,
    ) {
        let Some(answer) = answer else {
            cx.emit(SessionEvent::RecoverDraft(prompt.content));
            if !prompt.images.is_empty() {
                cx.emit(SessionEvent::RecoverImages(prompt.images));
            }
            return;
        };
        if answer.remember {
            let value = match &answer.place {
                super::parallel::Place::Same => "same",
                super::parallel::Place::Workspace(_) => "workspace",
            };
            let cwd = self.model.cwd.clone();
            let saved = crate::prefs::set_project(cx, &cwd, "jj.parallelSessions", json!(value));
            cx.spawn(async move |this, cx| {
                if let Err(error) = saved.await {
                    this.update(cx, |this, cx| {
                        this.notice(format!("Not remembered: {error:#}"), cx)
                    })
                    .ok();
                }
            })
            .detach();
        }
        match answer.place {
            super::parallel::Place::Same => {
                self.parallel_ok = true;
                let ParallelPrompt {
                    content,
                    images,
                    follow_up,
                    ..
                } = prompt;
                if !self.submit_with(content.clone(), images.clone(), follow_up, cx) {
                    cx.emit(SessionEvent::RecoverDraft(content));
                    if !images.is_empty() {
                        cx.emit(SessionEvent::RecoverImages(images));
                    }
                }
            }
            super::parallel::Place::Workspace(folder) => {
                self.move_to_workspace(folder, prompt.content, prompt.images, cx)
            }
        }
    }

    /// Makes a jj workspace in `folder`; the workspace then starts a new session
    /// there with the prompt, in place of this one, which had not started.
    fn move_to_workspace(
        &mut self,
        folder: PathBuf,
        content: String,
        images: Vec<ImageContent>,
        cx: &mut Context<Self>,
    ) {
        let root = self.jj.root.clone();
        let add = {
            let folder = folder.clone();
            self.jj_call(
                move |project| project.add_workspace(&folder, None).map(|_| ()),
                cx,
            )
        };
        let (Some(root), Some(add)) = (root, add) else {
            cx.emit(SessionEvent::RecoverDraft(content));
            return;
        };
        self.view_request = Some("workspace".into());
        self.publish(Changes::RUN, cx);
        cx.spawn(async move |this, cx| {
            let result = add.await;
            this.update(cx, |this, cx| {
                this.view_request = None;
                match result {
                    Ok(()) => cx.emit(SessionEvent::MoveToWorkspace(Box::new(MoveToWorkspace {
                        cwd: folder,
                        main: root,
                        content,
                        images,
                    }))),
                    Err(error) => {
                        this.jj_error("the workspace was not made", error, cx);
                        cx.emit(SessionEvent::RecoverDraft(content));
                        if !images.is_empty() {
                            cx.emit(SessionEvent::RecoverImages(images));
                        }
                    }
                }
                this.publish(Changes::RUN, cx);
            })
            .ok();
        })
        .detach();
    }

    /// A session opened in a new workspace for [`SessionEvent::MoveToWorkspace`]:
    /// sends the prompt once it is ready and its jj project is open.
    pub fn start_in_workspace(
        &mut self,
        content: String,
        images: Vec<ImageContent>,
        main: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.main_folder = Some(main);
        self.parallel_ok = true;
        self.pending_prompt = Some((content, images));
        self.send_pending_prompt(cx);
    }

    fn send_pending_prompt(&mut self, cx: &mut Context<Self>) {
        // jj opens right after the session starts (`busy`); the turn must wait for it.
        if self.pending_prompt.is_none() || !self.ready() || self.jj.busy || self.working() {
            return;
        }
        let Some((content, images)) = self.pending_prompt.take() else {
            return;
        };
        if !self.submit_with(content.clone(), images.clone(), false, cx) {
            cx.emit(SessionEvent::RecoverDraft(content));
            if !images.is_empty() {
                cx.emit(SessionEvent::RecoverImages(images));
            }
        }
    }

    /// The main folder this session's workspace came from.
    pub fn main_folder(&self) -> Option<&Path> {
        self.main_folder.as_deref()
    }

    /// Brings this workspace's turns into the main folder (design study 05, 04),
    /// after the user confirmed it: its files gain their edits.
    pub fn bring_in_main(&mut self, cx: &mut Context<Self>) {
        let Some(main) = self.main_folder.clone() else {
            return;
        };
        if !self.files_may_change("bringing in", cx) {
            return;
        }
        if let Some(other) = super::parallel::working_elsewhere(cx, cx.entity_id(), &main) {
            self.notice(
                format!("Wait for {other} to finish in {} first.", main.display()),
                cx,
            );
            return;
        }
        self.jj.busy = true;
        let folder = main.clone();
        self.jj_run(
            move |project| {
                let name = project.workspace_name_text();
                Project::open(&folder)?.bring_in(&name)
            },
            move |this, result, cx| {
                this.jj.busy = false;
                match result {
                    Ok(_) => this.notice(
                        format!(
                            "This session's turns are in {} now. Operations there can undo it.",
                            main.display()
                        ),
                        cx,
                    ),
                    Err(error) => this.jj_error("the turns were not brought in", error, cx),
                }
                this.publish(Changes::JJ, cx);
            },
            cx,
        );
    }

    /// Whether the backend can fork into another folder (`fork_cwd`).
    fn forks_into_folders(&self) -> bool {
        matches!(&self.model.backend, pi_core::session::BackendInfo::Found(info)
            if info["features"].as_array().is_some_and(|features| features.iter().any(|f| f == "fork_cwd")))
    }

    /// Fork from a Tree entry. When turns changed the files after it, asks
    /// which files the fork starts with (design study 05, 05).
    pub fn request_fork(&mut self, entry_id: String, cx: &mut Context<Self>) {
        let entry = self
            .model
            .history
            .as_ref()
            .and_then(|history| history.entry(&entry_id).cloned());
        let at = entry
            .as_ref()
            .and_then(|entry| entry["timestamp"].as_str())
            .and_then(pi_core::clock::parse_timestamp);
        let later = at.and_then(|at| {
            self.jj
                .records
                .iter()
                .position(|r| r.undone.is_none() && r.after.is_some_and(|after| after > at))
        });
        match later {
            Some(before) if self.jj.project.is_some() && self.forks_into_folders() => {
                let text = entry
                    .as_ref()
                    .map(pi_core::history::entry_text)
                    .unwrap_or_default();
                cx.emit(SessionEvent::ForkQuestion {
                    entry_id,
                    before,
                    text,
                });
            }
            _ => self.fork_session(entry_id, cx),
        }
    }

    /// A fork that works in a new jj workspace, with the files as they were
    /// just before the turn `before`. The main folder is not touched.
    pub fn fork_into_workspace(&mut self, entry_id: String, before: usize, cx: &mut Context<Self>) {
        if !self.can_navigate() {
            return;
        }
        let (Some((path, id)), Some(root), Some(change)) = (
            self.model
                .state
                .session_file
                .clone()
                .zip(self.model.state.session_id.clone()),
            self.jj.root.clone(),
            self.jj.records.get(before).map(|r| r.change.clone()),
        ) else {
            return;
        };
        let text = self
            .model
            .history
            .as_ref()
            .and_then(|history| history.entry(&entry_id))
            .map(pi_core::history::entry_text)
            .unwrap_or_default();
        let folder = super::parallel::automatic_folder(&root, &text);
        let Some(workspace) = ({
            let folder = folder.clone();
            self.jj_call(
                move |project| project.add_workspace(&folder, Some(&change)).map(|_| ()),
                cx,
            )
        }) else {
            return;
        };
        let cwd = self.model.cwd.clone();
        let backend = crate::prefs::backend(cx);
        self.view_request = Some("fork-worker".into());
        self.publish(Changes::RUN, cx);
        cx.spawn(async move |this, cx| {
            let result = match workspace.await {
                Ok(()) => {
                    cx.background_executor()
                        .spawn(pi_core::session_actions::fork(
                            cwd,
                            path,
                            id,
                            entry_id,
                            Some(folder),
                            backend,
                        ))
                        .await
                }
                Err(error) => Err(error),
            };
            this.update(cx, |this, cx| {
                this.view_request = None;
                match result {
                    Ok(forked) => cx.emit(SessionEvent::OpenFork(Box::new(forked))),
                    Err(error) => this.error(format!("{error:#}"), cx),
                }
                this.publish(Changes::RUN, cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn fork_session(&mut self, entry_id: String, cx: &mut Context<Self>) {
        if self.is_remote() {
            self.notice("Remote forks are not supported yet.", cx);
            return;
        }
        if !self.can_navigate() {
            return;
        }
        if self.demo {
            self.notice(
                "Fork requires a connected Pi session; demo files are never written.",
                cx,
            );
            return;
        }
        let Some((path, id)) = self
            .model
            .state
            .session_file
            .clone()
            .zip(self.model.state.session_id.clone())
        else {
            self.notice("This session is not saved yet.", cx);
            return;
        };
        let cwd = self.model.cwd.clone();
        let backend = crate::prefs::backend(cx);
        self.view_request = Some("fork-worker".into());
        self.publish(Changes::RUN, cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(pi_core::session_actions::fork(
                    cwd, path, id, entry_id, None, backend,
                ))
                .await;
            this.update(cx, |this, cx| {
                this.view_request = None;
                match result {
                    Ok(forked) => cx.emit(SessionEvent::OpenFork(Box::new(forked))),
                    Err(error) => this.error(error.to_string(), cx),
                };
                this.publish(Changes::RUN, cx);
            })
            .ok();
        })
        .detach();
    }
    pub fn load_views(&mut self, cx: &mut Context<Self>) {
        if self.views_loaded {
            return;
        }
        self.views_loaded = true;
        self.command(Command::GetEntries, cx);
        self.command(Command::GetSettings, cx);
        self.command(Command::GetCommands, cx);
    }
    pub fn can_navigate(&self) -> bool {
        self.ready() && !self.working() && !self.jj.busy
    }
    pub fn command(&mut self, command: Command, cx: &mut Context<Self>) -> Option<String> {
        if self.is_remote()
            && matches!(
                command,
                Command::Bash { .. }
                    | Command::NavigateTree { .. }
                    | Command::Fork { .. }
                    | Command::Clone
                    | Command::ExportHtml { .. }
            )
        {
            self.notice("This operation is not supported for SSH sessions yet.", cx);
            return None;
        }
        let exclusive = matches!(
            command,
            Command::NavigateTree { .. }
                | Command::SetLabel { .. }
                | Command::Compact { .. }
                | Command::InstallPackage { .. }
                | Command::RemovePackage { .. }
                | Command::UpdatePackages { .. }
                | Command::SetProjectTrust { .. }
                | Command::SetScopedModels { .. }
                | Command::SetModelThinkingLevel { .. }
                | Command::Reload
                | Command::Bash { .. }
        );
        if exclusive && !self.can_navigate() {
            self.notice(
                "Wait for the current run and file recording to finish first.",
                cx,
            );
            return None;
        }
        if !self.ready()
            && !matches!(
                command,
                Command::GetState
                    | Command::GetActiveTools
                    | Command::GetMessages
                    | Command::GetEntries
                    | Command::GetSettings
                    | Command::GetSessionStats
                    | Command::ListSessions { .. }
                    | Command::GetAvailableModels
                    | Command::GetAvailableThinkingLevels
                    | Command::GetCommands
                    | Command::GetBackendInfo
                    | Command::GetCustomEntries { .. }
                    | Command::GetAuthProviders
                    | Command::GetProjectTrust
                    | Command::ListPackages
                    | Command::ClearQueue
                    | Command::Abort
                    | Command::AbortBash
            )
        {
            self.notice(
                "Command not sent: session setup must complete successfully first.",
                cx,
            );
            return None;
        }
        if self.demo {
            for record in super::demo::command(&self.model, command) {
                self.receive(TransportEvent::Record(record), cx);
            }
            return None;
        }
        let name = command.name();
        match &self.client {
            Some(client) => match client.send(command) {
                Ok(id) => {
                    self.diagnostics.push("request", &format!("{name} · {id}"));
                    self.publish(Changes::DIAGNOSTICS, cx);
                    if exclusive {
                        self.view_request = Some(id.clone());
                        self.publish(Changes::RUN, cx);
                    }
                    Some(id)
                }
                Err(error) => {
                    self.error(error.to_string(), cx);
                    None
                }
            },
            None => {
                self.notice("This needs a connected pi session.", cx);
                None
            }
        }
    }
    pub fn submit_shell(&mut self, draft: String, cx: &mut Context<Self>) -> bool {
        if self.demo {
            self.notice("Shell execution is disabled in the offline demo.", cx);
            return false;
        }
        if !self.can_navigate() {
            self.notice(
                "Wait for this session's current work before running a shell command.",
                cx,
            );
            return false;
        }
        let excluded = draft.starts_with("!!");
        let Some(command) = draft.strip_prefix(if excluded { "!!" } else { "!" }) else {
            return false;
        };
        let command = command.to_owned();
        if command.trim().is_empty() {
            return false;
        }
        let Some(id) = self.command(
            Command::Bash {
                command: command.clone(),
                exclude_from_context: excluded,
            },
            cx,
        ) else {
            return false;
        };
        self.model.shell = Some(pi_core::session::ShellExecution {
            id: id.clone(),
            command,
            exclude_from_context: excluded,
            output: String::new(),
            finished: false,
            result: None,
        });
        self.submissions.insert(id, draft);
        self.publish(Changes::RUN | Changes::STATUS, cx);
        cx.emit(SessionEvent::RevealTail);
        true
    }
    pub fn submit(&mut self, content: String, follow_up: bool, cx: &mut Context<Self>) -> bool {
        self.submit_with(content, Vec::new(), follow_up, cx)
    }
    /// A prompt with images: text alone, images alone, or both.
    pub fn submit_with(
        &mut self,
        content: String,
        images: Vec<ImageContent>,
        follow_up: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if content.is_empty() && images.is_empty()
            || !self.ready()
            || self.view_request.is_some()
            || self.model.shell_running()
            || self.jj.busy
        {
            return false;
        }
        if let Some((root, other)) = self.crowded(cx) {
            let cwd = self.model.cwd.clone();
            match crate::prefs::choice(cx, "jj.parallelSessions", Some(&cwd)).as_str() {
                "same" => {
                    self.parallel_ok = true;
                    return self.submit_with(content, images, follow_up, cx);
                }
                "workspace" => {
                    let folder = super::parallel::automatic_folder(&root, &content);
                    self.move_to_workspace(folder, content, images, cx);
                }
                _ => cx.emit(SessionEvent::ParallelQuestion(Box::new(ParallelPrompt {
                    other,
                    root,
                    content,
                    images,
                    follow_up,
                }))),
            }
            return true;
        }
        if self.demo {
            let record = if self.model.busy() {
                let mut steering = self.model.steering.clone();
                let mut follow = self.model.follow_up.clone();
                if follow_up {
                    follow.push(content);
                } else {
                    steering.push(content);
                }
                json!({"type":"queue_update","steering":steering,"followUp":follow})
            } else {
                let content = if images.is_empty() {
                    json!(content)
                } else {
                    let mut blocks = vec![json!({"type":"text","text":content})];
                    blocks.extend(images.iter().map(|image| json!(image)));
                    json!(blocks)
                };
                json!({"type":"message_end","message":{"role":"user","content":content}})
            };
            self.receive(TransportEvent::Record(record), cx);
        } else if self.jj.starting {
            // The previous prompt is still waiting for jj; keep this draft.
            return false;
        } else if self.jj.project.is_some() && !self.working() {
            // A new run: jj records the files before pi can change them, then
            // the prompt goes out.
            self.jj.starting = true;
            self.jj.tool_baseline = self.model.tools.iter().map(|t| t.id.clone()).collect();
            self.publish(Changes::RUN, cx);
            let description = content.clone();
            self.jj_run(
                move |project| project.begin_turn(&description),
                move |this, result, cx| {
                    this.jj.starting = false;
                    match result {
                        Ok(turn) => this.jj.turn = Some(turn),
                        Err(error) => this.jj_error("this turn is not recorded", error, cx),
                    }
                    if !this.send_prompt(content.clone(), images.clone(), follow_up, cx) {
                        cx.emit(SessionEvent::RecoverDraft(content));
                        if !images.is_empty() {
                            cx.emit(SessionEvent::RecoverImages(images));
                        }
                    }
                    this.publish(Changes::RUN, cx);
                },
                cx,
            );
        } else if !self.send_prompt(content, images, follow_up, cx) {
            return false;
        }
        cx.emit(SessionEvent::RevealTail);
        true
    }
    fn send_prompt(
        &mut self,
        content: String,
        images: Vec<ImageContent>,
        follow_up: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let command = self
            .model
            .prompt(content.clone(), images.clone(), follow_up);
        let Some(id) = self.command(command, cx) else {
            return false;
        };
        if !images.is_empty() {
            self.submitted_images.insert(id.clone(), images);
        }
        self.submissions.insert(id, content);
        self.publish(Changes::RUN, cx);
        true
    }

    /// The git-only project jj can be turned on for, if it was not declined.
    pub fn jj_offer(&self) -> Option<&Path> {
        self.jj.offer.as_deref()
    }
    pub fn enable_jj(&mut self, cx: &mut Context<Self>) {
        if self.working() {
            return;
        }
        if let Some(root) = self.jj.offer.take() {
            self.open_jj(root, true, cx);
        }
    }
    /// "Not now": remembered for the project when `general.rememberDismissed` is on.
    pub fn decline_jj(&mut self, cx: &mut Context<Self>) {
        if let Some(root) = self.jj.offer.take() {
            crate::prefs::decline_jj(cx, &root);
        }
        self.publish(Changes::JJ, cx);
    }
    /// Whether jj actions (undo, redo) may run: never during a run.
    pub fn jj_idle(&self) -> bool {
        !self.working() && !self.jj.busy
    }
    pub fn undo_turn(&mut self, record: usize, cx: &mut Context<Self>) {
        let Some(change) = self.jj.records.get(record).map(|r| r.change.clone()) else {
            return;
        };
        if !self.jj_idle() {
            return;
        }
        if pi_editor::has_unsaved_buffers(cx) {
            self.notice(
                "Save or close unsaved editor buffers before undoing files.",
                cx,
            );
            return;
        }
        self.jj.busy = true;
        self.jj_run(
            move |project| project.undo_turn(&change),
            move |this, result, cx| {
                this.jj.busy = false;
                match result {
                    Ok(operation) => {
                        this.save_link(
                            super::turn_links::Event::Undone {
                                change: this.jj.records[record].change.hex(),
                                operation: operation.hex(),
                            },
                            cx,
                        );
                        this.jj.records[record].undone = Some(operation);
                    }
                    Err(error) => match error.downcast_ref::<pi_jj::UndoConflict>() {
                        Some(conflict) => {
                            cx.emit(SessionEvent::UndoConflict(record, conflict.clone()))
                        }
                        None => this.jj_error("undo failed", error, cx),
                    },
                }
                this.jj_record_changed(record, cx);
            },
            cx,
        );
    }
    /// Redoes the undo that hid this turn, which brings back every turn it hid.
    pub fn redo_turn(&mut self, record: usize, cx: &mut Context<Self>) {
        let Some(operation) = self.jj.records.get(record).and_then(|r| r.undone.clone()) else {
            return;
        };
        if !self.files_may_change("redoing", cx) {
            return;
        }
        self.jj.busy = true;
        let undo = operation.clone();
        self.jj_run(
            move |project| project.redo(&undo),
            move |this, result, cx| {
                this.jj.busy = false;
                match result {
                    Ok(()) => {
                        for index in 0..this.jj.records.len() {
                            if this.jj.records[index].undone.as_ref() == Some(&operation) {
                                this.jj.records[index].undone = None;
                                let change = this.jj.records[index].change.hex();
                                this.save_link(super::turn_links::Event::Redone { change }, cx);
                                this.jj_record_changed(index, cx);
                            }
                        }
                    }
                    Err(error) => this.jj_error("redo failed", error, cx),
                }
                this.jj_record_changed(record, cx);
            },
            cx,
        );
    }

    /// Whether jj may rewrite files now: never during a run, nor while an
    /// editor holds unsaved text that the new files would conflict with.
    fn files_may_change(&mut self, doing: &str, cx: &mut Context<Self>) -> bool {
        if !self.jj_idle() || self.jj.project.is_none() {
            return false;
        }
        if pi_editor::has_unsaved_buffers(cx) {
            self.notice(
                format!("Save or close unsaved editor buffers before {doing} files."),
                cx,
            );
            return false;
        }
        true
    }

    /// Puts one file of a turn back as it was before it (design study 05, 02).
    pub fn restore_file(&mut self, record: usize, path: String, cx: &mut Context<Self>) {
        let Some(change) = self.jj.records.get(record).map(|r| r.change.clone()) else {
            return;
        };
        if !self.files_may_change("restoring", cx) {
            return;
        }
        self.jj.busy = true;
        let file = path.clone();
        self.jj_run(
            move |project| {
                project.restore_file(&change, &path)?;
                project.changes(&change)
            },
            move |this, result, cx| {
                this.jj.busy = false;
                match result {
                    Ok(files) => {
                        let record = &mut this.jj.records[record];
                        let keep = (record.after_message, record.anchored);
                        let mut fresh =
                            super::jj::TurnRecord::new(keep.0, record.change.clone(), &files);
                        fresh.anchored = keep.1;
                        fresh.after = record.after;
                        fresh.commit = std::mem::take(&mut record.commit);
                        fresh.description = std::mem::take(&mut record.description);
                        fresh.tool_ids = std::mem::take(&mut record.tool_ids);
                        *record = fresh;
                    }
                    Err(error) => match error.downcast_ref::<pi_jj::UndoConflict>() {
                        Some(conflict) => {
                            this.notice(format!("Can't restore {file} alone: {conflict}."), cx)
                        }
                        None => this.jj_error("restore failed", error, cx),
                    },
                }
                this.jj_record_changed(record, cx);
            },
            cx,
        );
    }

    /// Undoes a turn and the later turns that build on it, in one operation.
    pub fn undo_turns(&mut self, records: Vec<usize>, cx: &mut Context<Self>) {
        let changes: Option<Vec<_>> = records
            .iter()
            .map(|&index| self.jj.records.get(index).map(|r| r.change.clone()))
            .collect();
        let Some(changes) = changes else {
            return;
        };
        if !self.files_may_change("undoing", cx) {
            return;
        }
        self.jj.busy = true;
        self.jj_run(
            move |project| project.undo_turns(&changes),
            move |this, result, cx| {
                this.jj.busy = false;
                match result {
                    Ok(operation) => {
                        for &index in &records {
                            let change = this.jj.records[index].change.hex();
                            this.save_link(
                                super::turn_links::Event::Undone {
                                    change,
                                    operation: operation.hex(),
                                },
                                cx,
                            );
                            this.jj.records[index].undone = Some(operation.clone());
                            this.jj_record_changed(index, cx);
                        }
                    }
                    Err(error) => this.jj_error("undo failed", error, cx),
                }
                this.publish(Changes::JJ, cx);
            },
            cx,
        );
    }

    /// Undoes a turn although later changes build on it (design study 05, 10):
    /// jj keeps their conflict, and the composer gets a drafted prompt asking pi
    /// to resolve the markers. The user reads and sends it.
    pub fn undo_turn_keeping_conflicts(&mut self, record: usize, cx: &mut Context<Self>) {
        let Some((change, short, description)) = self
            .jj
            .records
            .get(record)
            .map(|r| (r.change.clone(), r.short.clone(), r.description.clone()))
        else {
            return;
        };
        if !self.files_may_change("undoing", cx) {
            return;
        }
        self.jj.busy = true;
        self.jj_run(
            move |project| project.undo_turn_keeping_conflicts(&change),
            move |this, result, cx| {
                this.jj.busy = false;
                match result {
                    Ok(kept) => {
                        let change = this.jj.records[record].change.hex();
                        this.save_link(
                            super::turn_links::Event::Undone {
                                change,
                                operation: kept.operation.hex(),
                            },
                            cx,
                        );
                        this.jj.records[record].undone = Some(kept.operation);
                        if kept.files.is_empty() {
                            this.notice(
                                "The later changes keep a conflict in jj; no file on disk has markers.",
                                cx,
                            );
                        } else {
                            let files: Vec<_> =
                                kept.files.iter().map(|file| format!("@{file}")).collect();
                            let title = description.lines().next().unwrap_or_default();
                            cx.emit(SessionEvent::RecoverDraft(format!(
                                "Resolve the conflict markers in {} left by undoing turn {short} (\u{201c}{title}\u{201d}). Keep what the later changes did.",
                                files.join(", ")
                            )));
                        }
                    }
                    Err(error) => this.jj_error("undo failed", error, cx),
                }
                this.jj_record_changed(record, cx);
            },
            cx,
        );
    }

    /// The project's jj operations, newest first, for the Operations tab.
    pub fn operations(&self, cx: &App) -> Option<Task<anyhow::Result<Vec<pi_jj::OperationInfo>>>> {
        self.jj_call(|project| project.operations(200), cx)
    }

    /// What restoring to an operation would change in this folder's files.
    pub fn operation_files(
        &self,
        operation: pi_jj::OperationId,
        cx: &App,
    ) -> Option<Task<anyhow::Result<Vec<pi_jj::FileChange>>>> {
        self.jj_call(move |project| project.operation_files(&operation), cx)
    }

    /// `jj op restore` (design study 05, 11). Turns this restore hid can be
    /// redone, which undoes the restore.
    pub fn restore_operation(&mut self, operation: pi_jj::OperationId, cx: &mut Context<Self>) {
        if !self.files_may_change("restoring", cx) {
            return;
        }
        self.jj.busy = true;
        let records: Vec<_> = self
            .jj
            .records
            .iter()
            .map(|r| (r.change.clone(), r.commit.clone()))
            .collect();
        self.jj_run(
            move |project| {
                let restore = project.restore_operation(&operation)?;
                let turns = records
                    .iter()
                    .map(|(change, commit)| {
                        let commit = pi_jj::CommitId::try_from_hex(commit)?;
                        project.recorded_turn(change, &commit).ok()
                    })
                    .collect::<Vec<_>>();
                Ok((restore, turns))
            },
            move |this, result, cx| {
                this.jj.busy = false;
                match result {
                    Ok((restore, turns)) => {
                        for (record, turn) in this.jj.records.iter_mut().zip(turns) {
                            let Some(turn) = turn else { continue };
                            if turn.visible {
                                record.undone = None;
                            } else if record.undone.is_none() {
                                record.undone = Some(restore.clone());
                            }
                        }
                        this.notice("Project restored. Restoring is in Operations too.", cx);
                        cx.emit(SessionEvent::Content(ContentChange::Reset));
                    }
                    Err(error) => this.jj_error("restore failed", error, cx),
                }
                this.publish(Changes::JJ, cx);
            },
            cx,
        );
    }

    /// The extension's requests that are not about language servers: jj
    /// snapshots around `bash` calls and pi's read-only jj tools.
    pub fn bridge_request(
        &mut self,
        request: Value,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<String>> {
        let op = request["op"].as_str().unwrap_or_default();
        let id = text(&request, "toolCallId");
        match op {
            "snapshot_before" => {
                if !crate::prefs::flag(cx, "jj.snapshotBeforeCommands", Some(&self.model.cwd)) {
                    return Task::ready(Ok(String::new()));
                }
                let Some(task) = self.jj_call(|project| project.take_snapshot(), cx) else {
                    return Task::ready(Ok(String::new()));
                };
                cx.spawn(async move |this, cx| {
                    let before = task.await?;
                    this.update(cx, |this, _| {
                        this.jj.commands.insert(
                            id,
                            super::jj::CommandFiles {
                                before,
                                files: None,
                                restored: false,
                            },
                        );
                    })?;
                    Ok(String::new())
                })
            }
            "snapshot_after" => {
                let Some(before) = self.jj.commands.get(&id).map(|c| c.before.clone()) else {
                    return Task::ready(Ok(String::new()));
                };
                let Some(task) = self.jj_call(move |project| project.changed_since(&before), cx)
                else {
                    return Task::ready(Ok(String::new()));
                };
                cx.spawn(async move |this, cx| {
                    let (_, files) = task.await?;
                    this.update(cx, |this, cx| {
                        if let Some(command) = this.jj.commands.get_mut(&id) {
                            command.files = Some(files);
                        }
                        cx.emit(SessionEvent::Content(ContentChange::Tool(id)));
                        this.publish(Changes::JJ, cx);
                    })?;
                    Ok(String::new())
                })
            }
            "jj_log" | "jj_diff" | "jj_show" => {
                let limit = request["limit"].as_u64().unwrap_or(10).clamp(1, 50) as usize;
                let revision = text(&request, "revision");
                let path = request["path"].as_str().map(str::to_owned);
                let op = op.to_owned();
                let Some(task) = self.jj_call(
                    move |project| match op.as_str() {
                        "jj_log" => project.log_text(limit),
                        "jj_diff" => project.diff_text(&revision, path.as_deref()),
                        _ => project.show_text(&revision),
                    },
                    cx,
                ) else {
                    return Task::ready(Ok(
                        "jj is off for this project, so there is no history to read.".into(),
                    ));
                };
                // The model reads a failure as text rather than an empty answer.
                cx.background_executor().spawn(async move {
                    Ok(task
                        .await
                        .unwrap_or_else(|error| format!("jj could not answer: {error:#}")))
                })
            }
            _ => Task::ready(Err(anyhow!("Unknown request"))),
        }
    }

    /// Puts back what one `bash` call changed, as the files were just before it.
    pub fn restore_command(&mut self, tool_id: String, cx: &mut Context<Self>) {
        let Some(command) = self.jj.commands.get(&tool_id).cloned() else {
            return;
        };
        let Some(files) = command.files.filter(|files| !files.is_empty()) else {
            return;
        };
        if !self.files_may_change("restoring", cx) {
            return;
        }
        let paths: Vec<String> = files.iter().map(|file| file.path.clone()).collect();
        self.jj.busy = true;
        let before = command.before;
        self.jj_run(
            move |project| project.restore_paths(&before, &paths),
            move |this, result, cx| {
                this.jj.busy = false;
                match result {
                    Ok(_) => {
                        if let Some(command) = this.jj.commands.get_mut(&tool_id) {
                            command.restored = true;
                        }
                        cx.emit(SessionEvent::Content(ContentChange::Tool(tool_id)));
                    }
                    Err(error) => this.jj_error("restore failed", error, cx),
                }
                this.publish(Changes::JJ, cx);
            },
            cx,
        );
    }

    /// Runs `work` with the jj project locked, off the UI thread, for a view that
    /// shows the answer. `None` without a jj project.
    pub fn jj_call<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Project) -> anyhow::Result<R> + Send + 'static,
        cx: &App,
    ) -> Option<Task<anyhow::Result<R>>> {
        let project = self.jj.project.clone()?;
        Some(cx.background_executor().spawn(async move {
            let mut project = project
                .lock()
                .map_err(|_| anyhow!("an earlier jj call panicked"))?;
            work(&mut project)
        }))
    }

    fn open_jj(&mut self, root: PathBuf, init: bool, cx: &mut Context<Self>) {
        self.jj.busy = true;
        self.publish(Changes::JJ, cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if init {
                        Project::init(&root)
                    } else {
                        Project::open(&root)
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.jj.busy = false;
                match result {
                    Ok(project) => {
                        this.jj.root = Some(project.root().to_owned());
                        this.jj.project = Some(Arc::new(Mutex::new(project)));
                    }
                    Err(error) => this.jj_error("jj is off for this session", error, cx),
                }
                this.publish(Changes::JJ, cx);
                this.restore_links(cx);
                this.send_pending_prompt(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Records the finished run's edits as its own change, if it made any.
    fn end_jj_turn(&mut self, cx: &mut Context<Self>) {
        let Some(turn) = self.jj.turn.take() else {
            return;
        };
        let after_message = self.model.messages.len().saturating_sub(1);
        let after = self
            .model
            .messages
            .get(after_message)
            .and_then(|message| message["timestamp"].as_u64());
        let tool_ids = self
            .model
            .tools
            .iter()
            .filter(|t| !self.jj.tool_baseline.contains(&t.id))
            .map(|t| t.id.clone())
            .collect();
        self.jj.busy = true;
        self.jj_run(
            move |project| {
                let Some(change) = project.end_turn(&turn)? else {
                    return Ok(None);
                };
                let files = project.changes(&change)?;
                let commit = project
                    .visible_commit(&change)?
                    .map(|commit| commit.id().hex());
                let mut record = super::jj::TurnRecord::new(after_message, change, &files);
                record.description = turn.description().to_owned();
                record.tool_ids = tool_ids;
                record.after = after;
                record.commit = commit.unwrap_or_default();
                Ok(Some(record))
            },
            move |this, result, cx| {
                this.jj.busy = false;
                match result {
                    Ok(Some(record)) => {
                        this.save_link(super::turn_links::Event::Recorded(record.link()), cx);
                        this.jj.records.push(record);
                        this.jj_record_changed(this.jj.records.len() - 1, cx);
                    }
                    Ok(None) => this.publish(Changes::JJ, cx),
                    Err(error) => this.jj_error("this turn is not recorded", error, cx),
                }
            },
            cx,
        );
    }

    /// Keeps a change to the file history in the session file. A backend without
    /// the command (plain `pi --mode rpc`) answers with an error, which only the
    /// diagnostics show: the history then lasts until the session closes.
    fn save_link(&mut self, event: super::turn_links::Event, cx: &mut Context<Self>) {
        if !self.demo && self.client.is_some() {
            self.command(event.command(), cx);
        }
    }

    /// Turns the session file's links back into records once both they and the
    /// jj project are loaded: after reopening a session or restarting the app.
    fn restore_links(&mut self, cx: &mut Context<Self>) {
        if self.jj.project.is_none() || self.jj.busy {
            return;
        }
        let Some(links) = self.jj.saved_links.take().filter(|links| !links.is_empty()) else {
            return;
        };
        let known: HashSet<String> = self.jj.records.iter().map(|r| r.change.hex()).collect();
        let links: Vec<_> = links
            .into_iter()
            .filter(|link| !known.contains(&link.change))
            .collect();
        self.jj.busy = true;
        self.jj_run(
            move |project| {
                Ok(links
                    .into_iter()
                    .filter_map(|link| {
                        let change = pi_jj::ChangeId::try_from_hex(&link.change)?;
                        let commit = pi_jj::CommitId::try_from_hex(&link.commit)?;
                        let turn = project.recorded_turn(&change, &commit).ok()?;
                        let undone = link
                            .undone
                            .as_deref()
                            .and_then(pi_jj::OperationId::try_from_hex);
                        // Hidden without an undo of ours: abandoned outside the app.
                        if !turn.visible && undone.is_none() {
                            return None;
                        }
                        let mut record = super::jj::TurnRecord::new(0, change, &turn.files);
                        record.anchored = false;
                        record.description = turn.description;
                        record.commit = link.commit;
                        record.after = link.after;
                        record.tool_ids = link.tools.into_iter().collect();
                        record.undone = undone.filter(|_| !turn.visible);
                        Some(record)
                    })
                    .collect::<Vec<_>>())
            },
            |this, result, cx| {
                this.jj.busy = false;
                match result {
                    Ok(records) => {
                        this.jj.records.splice(0..0, records);
                        this.reanchor_records();
                        cx.emit(SessionEvent::Content(ContentChange::Reset));
                    }
                    Err(error) => this.jj_error("earlier turns are not shown", error, cx),
                }
                this.publish(Changes::JJ, cx);
            },
            cx,
        );
    }

    /// Finds each record's message again by its timestamp; a message on another
    /// branch leaves the record in Changes only.
    fn reanchor_records(&mut self) {
        let messages = &self.model.messages;
        for record in &mut self.jj.records {
            let found = record.after.and_then(|after| {
                messages
                    .iter()
                    .rposition(|message| message["timestamp"].as_u64() == Some(after))
            });
            record.anchored = found.is_some();
            record.after_message = found.unwrap_or(record.after_message);
        }
    }

    fn jj_record_changed(&mut self, record: usize, cx: &mut Context<Self>) {
        if let Some(record) = self.jj.records.get(record).filter(|record| record.anchored) {
            cx.emit(SessionEvent::Content(ContentChange::Message(
                record.after_message,
            )));
        }
        self.publish(Changes::JJ, cx);
    }

    fn jj_error(&mut self, what: &str, error: anyhow::Error, cx: &mut Context<Self>) {
        self.notice(format!("jj: {what}: {error:#}"), cx);
    }

    /// Runs `work` on the background executor with the jj project locked, then
    /// `done` with its result on this controller.
    fn jj_run<R: Send + 'static>(
        &mut self,
        work: impl FnOnce(&mut Project) -> anyhow::Result<R> + Send + 'static,
        done: impl FnOnce(&mut Self, anyhow::Result<R>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.jj.project.clone() else {
            return;
        };
        self.publish(Changes::JJ, cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut project = project
                        .lock()
                        .map_err(|_| anyhow!("an earlier jj call panicked"))?;
                    work(&mut project)
                })
                .await;
            this.update(cx, |this, cx| done(this, result, cx)).ok();
        })
        .detach();
    }
    pub fn clear_queue(&mut self, abort: bool, cx: &mut Context<Self>) {
        if abort && self.model.shell_running() {
            self.command(Command::AbortBash, cx);
        }
        if let Some((_, pending_abort)) = &mut self.clear_request {
            *pending_abort |= abort;
            return;
        }
        if self.demo {
            let queued = self
                .model
                .steering
                .iter()
                .chain(&self.model.follow_up)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            if !queued.is_empty() {
                cx.emit(SessionEvent::RecoverDraft(queued));
            }
            self.receive(
                TransportEvent::Record(json!({"type":"queue_update","steering":[],"followUp":[]})),
                cx,
            );
            if abort {
                self.receive(TransportEvent::Record(json!({"type":"agent_settled"})), cx);
            }
        } else if let Some(id) = self.command(Command::ClearQueue, cx) {
            self.clear_request = Some((id, abort));
        }
    }

    pub fn receive(&mut self, event: TransportEvent, cx: &mut Context<Self>) {
        if self.is_remote()
            && let TransportEvent::Record(record) = &event
        {
            if record["type"] == "remote_snapshot" {
                let target = self.remote.as_ref().expect("remote target");
                if record["version"] != pi_core::ssh::PROTOCOL_VERSION
                    || record["key"] != target.key
                {
                    self.bootstrap_failed = true;
                    self.remote_starting = false;
                    self.connected = false;
                    self.error("Remote session handshake mismatch", cx);
                    return;
                }
                let was_connected = self.connected;
                match self.model.apply(record) {
                    Ok(()) => {
                        self.remote_starting = false;
                        self.connected = true;
                        self.remote.as_mut().expect("remote target").session_file =
                            self.model.state.session_file.clone();
                        cx.emit(SessionEvent::Content(ContentChange::Reset));
                        self.publish(
                            Changes::RUN
                                | Changes::METADATA
                                | Changes::QUEUE
                                | Changes::STATUS
                                | Changes::CONTEXT
                                | Changes::CATALOG
                                | Changes::SUMMARY,
                            cx,
                        );
                        if !was_connected {
                            self.command(Command::GetCommands, cx);
                            self.command(Command::GetBackendInfo, cx);
                            self.command(Command::GetAvailableModels, cx);
                            self.command(Command::GetActiveTools, cx);
                        }
                    }
                    Err(error) => {
                        self.bootstrap_failed = true;
                        self.remote_starting = false;
                        self.error(error.to_string(), cx);
                    }
                }
                return;
            }
            if record["type"] == "remote_error" {
                self.diagnostics
                    .push("remote error", &text(record, "error"));
                self.error(text(record, "error"), cx);
                return;
            }
        }
        let old_run = self.model.run;
        let old_error = self.model.error.clone();
        let old_notice = self.model.notice.clone();
        let old_metrics = self.model.turn_metrics.clone();
        let mut changes = Changes::default();
        let mut content = None;
        let mut reanchor = false;
        match event {
            TransportEvent::Record(record) => {
                let id = text(&record, "id");
                let bootstrap = self.bootstrap.remove(&id);
                if bootstrap {
                    changes |= Changes::STATUS;
                }
                if bootstrap && record["success"] == false {
                    self.bootstrap_failed = true;
                }
                let response = record["type"] == "response";
                if response {
                    self.diagnostics.push(
                        "response",
                        &format!(
                            "{} · {} · {}",
                            text(&record, "command"),
                            id,
                            if record["success"] == true {
                                "ok"
                            } else {
                                "failed"
                            }
                        ),
                    );
                    if let Some(error) = record["error"].as_str() {
                        self.diagnostics.push("RPC error", error);
                    }
                    changes |= Changes::DIAGNOSTICS;
                    if self.view_request.as_deref() == Some(&id) {
                        self.view_request = None;
                        changes |= Changes::RUN;
                    }
                    cx.emit(SessionEvent::RequestFinished(id.clone()));
                    let images = self.submitted_images.remove(&id);
                    if let Some(submitted) = self.submissions.remove(&id) {
                        changes |= Changes::RUN;
                        if record["success"] == false {
                            cx.emit(SessionEvent::RecoverDraft(submitted));
                            if let Some(images) = images {
                                cx.emit(SessionEvent::RecoverImages(images));
                            }
                        }
                    }
                }
                if bootstrap
                    && record["command"] == "get_state"
                    && record["success"] == true
                    && self.expected_resume_id.as_deref().is_some_and(|expected| {
                        record["data"]["sessionId"].as_str() != Some(expected)
                    })
                {
                    self.bootstrap_failed = true;
                    self.model.error = Some("Pi did not resume the selected session. Submission is disabled to avoid writing to a different session.".into());
                }
                let message_index = self
                    .model
                    .streaming_message_index()
                    .filter(|_| record["message"]["role"] == "assistant")
                    .unwrap_or(self.model.messages.len());
                match record["type"].as_str().unwrap_or("") {
                    "message_start" if record["message"]["role"] == "assistant" => {
                        content = Some(ContentChange::Message(self.model.messages.len()))
                    }
                    "message_update" => {
                        content = self
                            .model
                            .streaming_message_index()
                            .map(ContentChange::Message)
                    }
                    "message_end" if record["message"]["role"] == "toolResult" => {
                        content = Some(ContentChange::Tool(text(&record["message"], "toolCallId")));
                        changes |= Changes::METADATA;
                    }
                    "message_end" => content = Some(ContentChange::Message(message_index)),
                    "entry_appended" if record["entry"]["type"] == "custom_message" => {
                        content = Some(ContentChange::Message(message_index))
                    }
                    "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
                        content = Some(ContentChange::Tool(text(&record, "toolCallId")));
                        if record["type"] != "tool_execution_update" {
                            changes |= Changes::METADATA;
                        }
                    }
                    "extension_ui_cancel" => {
                        cx.emit(SessionEvent::CancelExtensionDialog(id.clone()))
                    }
                    "bash_execution_update" => changes |= Changes::STATUS,
                    "queue_update" => changes |= Changes::QUEUE,
                    "thinking_level_changed" | "compaction_end" => {
                        changes |= Changes::METADATA | Changes::CONTEXT
                    }
                    "auto_retry_start" | "auto_retry_end" | "summarization_retry_scheduled" => {
                        changes |= Changes::CONTEXT
                    }
                    "extension_ui_request" if record["method"] == "setStatus" => {
                        changes |= Changes::METADATA
                    }
                    "response" if record["success"] == true => {
                        match record["command"].as_str().unwrap_or("") {
                            "get_messages" if record["data"]["remoteSnapshot"] == true => {}
                            "get_messages"
                                if self.model.shell_snapshot_appends(&record["data"]) =>
                            {
                                content = Some(ContentChange::Append(self.model.messages.len()));
                                changes |= Changes::METADATA;
                            }
                            "get_messages" => {
                                // File history survives a conversation reload; numeric
                                // transcript anchors are found again by timestamp
                                // once the model holds the new messages.
                                reanchor = true;
                                changes |= Changes::JJ;
                                content = Some(ContentChange::Reset);
                                changes |= Changes::METADATA;
                            }
                            "get_state"
                            | "get_active_tools"
                            | "get_session_stats"
                            | "set_model"
                            | "set_thinking_level"
                            | "cycle_model"
                            | "cycle_thinking_level" => changes |= Changes::METADATA,
                            "get_entries" => changes |= Changes::HISTORY,
                            "get_settings" => changes |= Changes::CONTEXT,
                            "list_sessions" => changes |= Changes::SAVED,
                            "get_available_models"
                            | "get_available_thinking_levels"
                            | "get_commands"
                            | "get_auth_providers"
                            | "get_project_trust"
                            | "list_packages" => changes |= Changes::CATALOG,
                            _ => {}
                        }
                    }
                    _ => {}
                }
                if response && self.jj.links_request.as_deref() == Some(&id) {
                    self.jj.links_request = None;
                    self.jj.saved_links = Some(if record["success"] == true {
                        super::turn_links::replay(&record["data"])
                    } else {
                        self.diagnostics.push(
                            "turn links",
                            "This backend cannot keep which jj change each turn made; file history lasts until the session closes.",
                        );
                        Vec::new()
                    });
                    self.restore_links(cx);
                }
                if let Err(error) = self.model.apply(&record) {
                    if bootstrap {
                        self.bootstrap_failed = true;
                    }
                    self.model.error = Some(error.to_string());
                }
                if let Some(target) = &mut self.remote
                    && target.session_file != self.model.state.session_file
                {
                    target.session_file = self.model.state.session_file.clone();
                    changes |= Changes::SUMMARY;
                }
                if reanchor {
                    self.reanchor_records();
                }
                if record["type"] == "extension_ui_request" {
                    match record["method"].as_str() {
                        Some("confirm" | "select") => {
                            cx.emit(SessionEvent::ExtensionDialog(record.clone()));
                        }
                        Some("input" | "editor") => {
                            if let Some(client) = &self.client
                                && let Err(error) = client.send_record(json!({"type":"extension_ui_response","id":record["id"],"cancelled":true})) {
                                self.model.error = Some(error.to_string());
                            }
                            self.model.notice = Some(format!(
                                "Extension dialog cancelled (not yet supported): {}",
                                text(&record, "title")
                            ));
                        }
                        Some("set_editor_text") => {
                            cx.emit(SessionEvent::RecoverDraft(text(&record, "text")))
                        }
                        _ => {}
                    }
                }
                if response
                    && self
                        .clear_request
                        .as_ref()
                        .is_some_and(|(pending, _)| pending == &id)
                {
                    let (_, abort) = self.clear_request.take().expect("matched clear request");
                    if record["success"] == true {
                        for queue in ["steering", "followUp"] {
                            if let Some(messages) = record["data"][queue].as_array() {
                                for message in messages.iter().filter_map(Value::as_str) {
                                    cx.emit(SessionEvent::RecoverDraft(message.to_owned()));
                                }
                            }
                        }
                        if abort {
                            self.command(Command::Abort, cx);
                        }
                    }
                }
                if response
                    && record["success"] == true
                    && matches!(
                        record["command"].as_str(),
                        Some("set_model" | "set_thinking_level")
                    )
                    && !self.demo
                {
                    self.command(Command::GetState, cx);
                }
                if response && record["success"] == true && !self.demo {
                    match record["command"].as_str() {
                        // pi's state has no tools; the extension reports them.
                        Some("get_state") => {
                            self.command(Command::GetActiveTools, cx);
                        }
                        Some("set_model" | "set_scoped_models" | "set_model_thinking_level") => {
                            self.command(Command::GetSettings, cx);
                            self.command(Command::GetAvailableThinkingLevels, cx);
                            self.command(Command::GetState, cx);
                        }
                        Some("install_package" | "remove_package" | "update_packages") => {
                            self.command(Command::ListPackages, cx);
                            self.model.notice = Some("Package configuration changed. Use Reload resources to apply it to this session.".into());
                        }
                        Some("set_project_trust") => {
                            self.command(Command::GetProjectTrust, cx);
                            self.model.notice = Some("Trust decision saved. Close and reopen this session for it to take effect.".into());
                        }
                        Some("bash") => {
                            self.command(Command::GetMessages, cx);
                            self.command(Command::GetState, cx);
                            self.command(Command::GetSessionStats, cx);
                            if self.views_loaded {
                                self.command(Command::GetEntries, cx);
                            }
                            changes |= Changes::RUN | Changes::STATUS;
                        }
                        Some("prompt") if record["data"]["disposition"] == "handled" => {
                            self.command(Command::GetState, cx);
                        }
                        Some("reload") => {
                            self.command(Command::GetState, cx);
                            self.command(Command::GetCommands, cx);
                            self.command(Command::ListPackages, cx);
                            self.model.notice = Some("Resources reloaded.".into());
                        }
                        _ => {}
                    }
                }
                if response && record["success"] == true {
                    match record["command"].as_str() {
                        Some("navigate_tree") if record["data"]["cancelled"] != true => {
                            // Keep file history independent from conversation navigation.
                            for record in &mut self.jj.records {
                                record.anchored = false;
                            }
                            changes |= Changes::JJ;
                            if let Some(text) = record["data"]["editorText"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                            {
                                cx.emit(SessionEvent::RecoverDraft(text.into()));
                            }
                            for command in [
                                Command::GetState,
                                Command::GetMessages,
                                Command::GetSessionStats,
                                Command::GetEntries,
                            ] {
                                if let Some(id) = self.command(command, cx) {
                                    self.bootstrap.insert(id);
                                } else if !self.demo {
                                    self.bootstrap_failed = true;
                                }
                            }
                            cx.emit(SessionEvent::BranchChanged);
                        }
                        Some("set_label") => {
                            self.command(Command::GetEntries, cx);
                        }
                        Some("set_auto_compaction") => {
                            self.command(Command::GetState, cx);
                            self.command(Command::GetSettings, cx);
                        }
                        Some("compact") => {
                            self.command(Command::GetMessages, cx);
                            self.command(Command::GetSessionStats, cx);
                            self.command(Command::GetEntries, cx);
                        }
                        _ => {}
                    }
                }
                if record["type"] == "agent_start" && !self.demo {
                    self.command(Command::GetState, cx);
                }
                if record["type"] == "agent_settled" {
                    if self.views_loaded {
                        self.command(Command::GetEntries, cx);
                    }
                    if !self.demo {
                        self.command(Command::GetSessionStats, cx);
                        self.command(Command::GetState, cx);
                    }
                    self.end_jj_turn(cx);
                }
            }
            TransportEvent::RequestFailed { id, command, error } => {
                self.diagnostics
                    .push("request failed", &format!("{command} · {id}\n{error}"));
                if self.jj.links_request.as_deref() == Some(&id) {
                    self.jj.links_request = None;
                    self.jj.saved_links = Some(Vec::new());
                }
                changes |= Changes::DIAGNOSTICS;
                if self.view_request.as_deref() == Some(&id) {
                    self.view_request = None;
                    changes |= Changes::RUN;
                }
                cx.emit(SessionEvent::RequestFinished(id.clone()));
                if self.bootstrap.remove(&id) {
                    self.bootstrap_failed = true;
                }
                if self
                    .clear_request
                    .as_ref()
                    .is_some_and(|(pending, _)| pending == &id)
                {
                    self.clear_request = None;
                }
                if let Some(submitted) = self.submissions.remove(&id) {
                    cx.emit(SessionEvent::RecoverDraft(submitted));
                    if let Some(images) = self.submitted_images.remove(&id) {
                        cx.emit(SessionEvent::RecoverImages(images));
                    }
                    changes |= Changes::RUN;
                }
                self.model.error = Some(format!("{command}: {error}"));
                changes |= Changes::STATUS;
            }
            TransportEvent::ProtocolError(error) => {
                self.diagnostics.push("protocol error", &error);
                changes |= Changes::DIAGNOSTICS;
                self.model.error = Some(error);
            }
            TransportEvent::Exited {
                description,
                stderr,
            } => {
                self.diagnostics.push("process exit", &description);
                self.diagnostics.push(
                    "stderr tail (max 8 KiB)",
                    if stderr.is_empty() {
                        "No stderr captured."
                    } else {
                        &stderr
                    },
                );
                changes |= Changes::DIAGNOSTICS;
                self.connected = false;
                self.remote_starting = false;
                self.view_request = None;
                self.bootstrap.clear();
                self.clear_request = None;
                for (id, draft) in self.submissions.drain() {
                    cx.emit(SessionEvent::RecoverDraft(draft));
                    if let Some(images) = self.submitted_images.remove(&id) {
                        cx.emit(SessionEvent::RecoverImages(images));
                    }
                }
                if !self.is_remote() {
                    self.model.run = RunState::Idle;
                    self.model.shell = None;
                    self.model.state.is_bash_running = Some(false);
                }
                let description = if self.is_remote() {
                    self.model
                        .error
                        .as_ref()
                        .map(|error| format!("{description}\n{error}"))
                        .unwrap_or(description)
                } else {
                    description
                };
                self.model.error = Some(format!(
                    "{description}{}{}",
                    if self.is_remote() {
                        "\nRemote work may still be running. Reconnect to check its state; no prompt was retried."
                    } else {
                        ""
                    },
                    if stderr.is_empty() {
                        String::new()
                    } else {
                        format!("\n{stderr}")
                    }
                ));
                changes |= Changes::STATUS | Changes::RUN;
            }
        }
        self.apply_initial_model(cx);
        if old_run != self.model.run {
            changes |= Changes::RUN;
        }
        if old_error != self.model.error
            || old_notice != self.model.notice
            || old_metrics != self.model.turn_metrics
        {
            changes |= Changes::STATUS;
        }
        if let Some(content) = content {
            cx.emit(SessionEvent::Content(content));
        }
        self.publish(changes, cx);
        self.send_pending_prompt(cx);
    }

    /// Attaches a jj project with a turn already begun, as `submit` would for a
    /// live session.
    #[cfg(test)]
    pub fn use_jj(&mut self, mut project: Project, prompt: &str) {
        self.jj.turn = Some(project.begin_turn(prompt).unwrap());
        self.jj.tool_baseline = self.model.tools.iter().map(|t| t.id.clone()).collect();
        self.jj.root = Some(project.root().to_owned());
        self.jj.project = Some(Arc::new(Mutex::new(project)));
    }
    /// Opens `project` and restores the turn links in a `get_custom_entries`
    /// answer, as reopening a session does.
    #[cfg(test)]
    pub fn restore_links_from(&mut self, project: Project, data: &Value, cx: &mut Context<Self>) {
        self.jj.root = Some(project.root().to_owned());
        self.jj.project = Some(Arc::new(Mutex::new(project)));
        self.jj.saved_links = Some(super::turn_links::replay(data));
        self.restore_links(cx);
    }
    #[cfg(test)]
    pub fn expect_bootstrap(&mut self, id: &str) {
        self.bootstrap.insert(id.into());
    }
    #[cfg(test)]
    pub fn expect_submission(&mut self, id: &str, text: &str) {
        self.submissions.insert(id.into(), text.into());
    }
}

/// The main folder of a workspace the app made in `<main>-ws/<name>`, when that
/// folder is a jj workspace; a folder chosen by hand is only known while the
/// session that made it is open.
fn main_folder_of(cwd: &Path) -> Option<PathBuf> {
    let workspaces = cwd.parent()?;
    let name = workspaces.file_name()?.to_str()?.strip_suffix("-ws")?;
    let main = workspaces.parent()?.join(name);
    main.join(".jj").is_dir().then_some(main)
}

/// pi's extension asks the bridge after edits and runs; answers stay empty until
/// the user starts language services for this folder in the Files view, and
/// while the project's language-server settings turn that kind of check off.
fn lsp_bridge(
    cwd: &Path,
    launch: &mut Launch,
    handler: pi_lsp_bridge::Handler,
    diagnostics: &mut super::diagnostics::DiagnosticLog,
    cx: &mut gpui::App,
) -> Option<pi_lsp_bridge::Bridge> {
    // Projects are shared by canonical root, as the Files view opens them.
    let root = std::fs::canonicalize(cwd).ok()?;
    let project = cwd.to_owned();
    let allow = move |request: &pi_lsp_bridge::Request, cx: &gpui::App| {
        let key = match request {
            pi_lsp_bridge::Request::File { .. } => "languageServers.afterEdits",
            pi_lsp_bridge::Request::RunEnd => "languageServers.beforeRunEnds",
        };
        crate::prefs::flag(cx, key, Some(&project))
    };
    match pi_lsp_bridge::Bridge::start(
        move |cx| pi_editor::language_project(&root, cx),
        allow,
        handler,
        cx,
    ) {
        Ok(bridge) => {
            launch.env.extend(
                bridge
                    .env()
                    .iter()
                    .map(|(name, value)| ((*name).into(), value.clone())),
            );
            Some(bridge)
        }
        Err(error) => {
            diagnostics.push(
                "language servers",
                &format!("Errors are not fed back to pi: {error:#}"),
            );
            None
        }
    }
}
