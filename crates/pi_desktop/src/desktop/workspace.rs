use super::*;
use super::{
    composer::ComposerEvent,
    session::{Changes, SessionController, SessionEvent, Summary},
    session_view::SessionView,
};
use gpui::{EventEmitter, Subscription};
mod close;
pub use close::CloseTarget;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u64);

/// The saved-session row for an open session, once pi has named its file.
fn saved_session(model: &Session) -> Option<SavedSession> {
    let path = model.state.session_file.clone()?;
    Some(SavedSession {
        id: model
            .state
            .session_id
            .clone()
            .unwrap_or_else(|| path.clone()),
        cwd: model.cwd.display().to_string(),
        name: model.state.session_name.clone(),
        first_message: model.title().to_owned(),
        path,
        ..Default::default()
    })
}
#[derive(Clone, Copy, Debug)]
pub enum WorkspaceEvent {
    Selection(SessionId),
    NewSessionRequested,
    /// An app view (All Sessions, Settings) opened.
    View,
    Navigation,
    Status(SessionId),
    Summary(SessionId),
}

pub struct SessionTab {
    pub id: SessionId,
    pub controller: Entity<SessionController>,
    pub view: Entity<SessionView>,
    _subscriptions: Vec<Subscription>,
}

pub struct WorkspaceController {
    pub tabs: Vec<SessionTab>,
    pub active: SessionId,
    pub summaries: std::collections::BTreeMap<SessionId, Summary>,
    pub saved: Vec<SavedSession>,
    pub projects: Vec<PathBuf>,
    /// Project choice outlives its last active session.
    pub selected_project: Option<PathBuf>,
    demo: bool,
    next_id: u64,
    removed_projects: HashSet<PathBuf>,
    hidden_sessions: HashSet<String>,
    close_prompt_pending: bool,
    /// An app view shown instead of the active session.
    pub view: Option<super::app_views::AppView>,
}
impl EventEmitter<WorkspaceEvent> for WorkspaceController {}
impl WorkspaceController {
    pub fn new(demo: bool) -> Self {
        Self {
            tabs: vec![],
            active: SessionId(0),
            summaries: Default::default(),
            saved: vec![],
            projects: vec![],
            selected_project: None,
            demo,
            next_id: 0,
            removed_projects: HashSet::new(),
            hidden_sessions: HashSet::new(),
            close_prompt_pending: false,
            view: None,
        }
    }
    pub fn is_demo(&self) -> bool {
        self.demo
    }
    pub fn active_tab_opt(&self) -> Option<&SessionTab> {
        self.tab(self.active)
    }
    pub fn active_summary(&self) -> &Summary {
        &self.summaries[&self.active]
    }
    pub fn tab(&self, id: SessionId) -> Option<&SessionTab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }
    pub fn open(
        &mut self,
        cwd: PathBuf,
        saved: Option<SavedSession>,
        cx: &mut Context<Self>,
    ) -> SessionId {
        self.removed_projects.remove(&cwd);
        if let Some(saved) = &saved {
            self.hidden_sessions.remove(&saved.path);
        }
        if let Some(saved) = &saved
            && let Some(id) = self.summaries.iter().find_map(|(id, summary)| {
                (summary.file.as_deref() == Some(&saved.path)).then_some(*id)
            })
        {
            self.select(id, cx);
            return id;
        }
        let first = self.next_id == 0;
        let id = SessionId(self.next_id);
        self.next_id += 1;
        let controller = cx.new(|cx| SessionController::new(cwd, saved, self.demo, first, cx));
        let draft = if self.demo
            && first
            && controller.read(cx).model().state.session_id.as_deref() != Some("demo-workspace")
        {
            "Run the qwen provider tests too once check passes"
        } else {
            ""
        };
        let view = cx.new(|cx| SessionView::new(controller.clone(), draft, cx));
        let composer = view.read(cx).composer.clone();
        let subscriptions = vec![
            cx.subscribe(&controller, move |this, controller, event, cx| {
                if this.tab(id).is_none() {
                    return;
                }
                if let SessionEvent::MoveToWorkspace(moved) = event {
                    let moved = (**moved).clone();
                    let new = this.open(moved.cwd.clone(), None, cx);
                    if let Some(tab) = this.tab(new) {
                        tab.controller.clone().update(cx, |controller, cx| {
                            controller.start_in_workspace(
                                moved.content,
                                moved.images,
                                moved.main,
                                cx,
                            )
                        });
                    }
                    // The session that asked had not started; after this event.
                    cx.spawn(async move |this, cx| {
                        this.update(cx, |this, cx| {
                            this.close_confirmed(CloseTarget::Session(id), cx)
                        })
                        .ok();
                    })
                    .detach();
                }
                if let SessionEvent::OpenFork(forked) = event {
                    let fork = this.open(forked.cwd.clone(), forked.saved.clone(), cx);
                    if let Some(tab) = this.tab(fork) {
                        let input = tab.view.read(cx).composer.read(cx).input.clone();
                        let draft = forked.draft.clone();
                        input.update(cx, |input, cx| input.set_content(draft, cx));
                    }
                }
                if let SessionEvent::Changed(changes) = event {
                    if changes.intersects(Changes::SUMMARY) {
                        this.summaries
                            .insert(id, controller.read(cx).summary().clone());
                        this.remember_open(cx);
                        cx.emit(WorkspaceEvent::Summary(id));
                    }
                    if changes.intersects(Changes::SAVED) {
                        this.refresh_catalog(cx);
                        cx.emit(WorkspaceEvent::Navigation);
                    }
                    if changes.intersects(
                        Changes::METADATA | Changes::RUN | Changes::STATUS | Changes::SUMMARY,
                    ) {
                        cx.emit(WorkspaceEvent::Status(id));
                    }
                }
            }),
            cx.subscribe(&composer, move |this, _, event, cx| match event {
                ComposerEvent::NewSession => {
                    if let Some(summary) = this.summaries.get(&id) {
                        this.selected_project = Some(summary.cwd.clone());
                        this.new_session(cx);
                    }
                }
                ComposerEvent::Mentions => {}
            }),
        ];
        self.summaries
            .insert(id, controller.read(cx).summary().clone());
        self.tabs.push(SessionTab {
            id,
            controller,
            view,
            _subscriptions: subscriptions,
        });
        if !self.projects.contains(&self.summaries[&id].cwd) {
            self.projects.push(self.summaries[&id].cwd.clone());
        }
        self.select(id, cx);
        self.refresh_catalog(cx);
        cx.emit(WorkspaceEvent::Navigation);
        id
    }
    pub fn show_view(&mut self, view: super::app_views::AppView, cx: &mut Context<Self>) {
        self.view = Some(view);
        self.refresh_catalog(cx);
        cx.emit(WorkspaceEvent::View);
    }
    /// A deleted session leaves the catalog before pi lists sessions again.
    pub fn forget_saved(&mut self, path: &str, cx: &mut Context<Self>) {
        self.saved.retain(|saved| saved.path != path);
        for tab in &self.tabs {
            tab.controller
                .update(cx, |controller, _| controller.forget_saved(path));
        }
        cx.emit(WorkspaceEvent::Navigation);
    }
    pub fn select(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if self.tab(id).is_none() {
            return;
        }
        self.view = None;
        if id != self.active
            && let Some(old) = self.tab(self.active)
        {
            let composer = old.view.read(cx).composer.clone();
            composer.update(cx, |composer, cx| composer.deactivate(cx));
        }
        self.active = id;
        self.selected_project = self.summaries.get(&id).map(|s| s.cwd.clone());
        self.refresh_catalog(cx);
        self.remember_open(cx);
        cx.emit(WorkspaceEvent::Selection(id));
    }
    /// Saves which sessions are open, for `general.reopenSessions`. The demo
    /// never writes.
    fn remember_open(&self, cx: &mut App) {
        if self.demo {
            return;
        }
        let sessions: Vec<_> = self
            .tabs
            .iter()
            .map(|tab| {
                let model = tab.controller.read(cx).model();
                crate::prefs::OpenSession {
                    cwd: model.cwd.clone(),
                    saved: saved_session(model),
                }
            })
            .collect();
        let active = self
            .tabs
            .iter()
            .position(|tab| tab.id == self.active)
            .unwrap_or(0);
        crate::prefs::remember_open_sessions(cx, &sessions, active);
    }
    pub fn select_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.projects.contains(&path) {
            self.selected_project = Some(path);
            cx.emit(WorkspaceEvent::Navigation);
        }
    }
    pub fn new_session(&mut self, cx: &mut Context<Self>) {
        cx.emit(WorkspaceEvent::NewSessionRequested);
    }
    fn refresh_catalog(&mut self, cx: &App) {
        let mut seen = HashSet::new();
        self.saved = self
            .active_tab_opt()
            .into_iter()
            .chain(self.tabs.iter())
            .flat_map(|tab| tab.controller.read(cx).model().saved.iter())
            .chain(self.saved.iter())
            .filter(|saved| {
                !self.hidden_sessions.contains(&saved.path) && seen.insert(saved.path.clone())
            })
            .cloned()
            .collect();
        for saved in &self.saved {
            if !saved.cwd.is_empty() {
                let path = PathBuf::from(&saved.cwd);
                if !self.removed_projects.contains(&path) && !self.projects.contains(&path) {
                    self.projects.push(path);
                }
            }
        }
    }
    pub fn open_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open project".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = paths.await;
            if let Err(error) = this.update(cx, |this, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.first() {
                        this.open(path.clone(), None, cx);
                    }
                }
                Ok(Ok(None)) => {}
                result => {
                    if let Some(tab) = this.active_tab_opt() {
                        let controller = tab.controller.clone();
                        controller.update(cx, |controller, cx| {
                            controller.error(format!("Folder picker failed: {result:?}"), cx)
                        });
                    } else {
                        log::error!("Folder picker failed: {result:?}");
                    }
                }
            }) {
                log::debug!("Folder picker closed with window: {error}");
            }
        })
        .detach();
    }
}
