//! Closing UI state never deletes project directories or saved conversation files.
use super::*;

#[derive(Clone, Debug)]
pub enum CloseTarget {
    Session(SessionId),
    Project(PathBuf),
    Saved(String),
}
impl WorkspaceController {
    fn close_ids(&self, target: &CloseTarget) -> Vec<SessionId> {
        self.tabs
            .iter()
            .filter(|tab| match target {
                CloseTarget::Session(id) => tab.id == *id,
                CloseTarget::Project(path) => {
                    self.summaries.get(&tab.id).is_some_and(|s| s.cwd == *path)
                }
                CloseTarget::Saved(_) => false,
            })
            .map(|tab| tab.id)
            .collect()
    }
    /// Whether closing loses work, unless `general.confirmClose` is off.
    fn close_warning(&self, ids: &[SessionId], cx: &App) -> bool {
        if !crate::prefs::flag(cx, "general.confirmClose", None) {
            return false;
        }
        ids.iter().filter_map(|id| self.tab(*id)).any(|tab| {
            let view = tab.view.read(cx);
            let composer = view.composer.read(cx);
            tab.controller.read(cx).working()
                || !tab.controller.read(cx).model().steering.is_empty()
                || !tab.controller.read(cx).model().follow_up.is_empty()
                || !composer.input.read(cx).content().is_empty()
                || composer.attached.is_some()
                || view.files.read(cx).has_unsaved(cx)
        })
    }
    fn can_close(&self, ids: &[SessionId], cx: &App) -> bool {
        ids.iter().filter_map(|id| self.tab(*id)).all(|tab| {
            !tab.controller.read(cx).close_blocked()
                && !tab.view.read(cx).files.read(cx).close_blocked()
        })
    }
    pub fn request_close(
        &mut self,
        target: CloseTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_prompt_pending {
            return;
        }
        let ids = self.close_ids(&target);
        if !self.can_close(&ids, cx) {
            drop(window.prompt(gpui::PromptLevel::Info,"Still finishing work",Some("Wait for file operations, jj recording, navigation or forking to finish before closing."),&["OK"],cx));
            return;
        }
        if matches!(target, CloseTarget::Project(_) | CloseTarget::Saved(_))
            || self.close_warning(&ids, cx)
        {
            let all_remote = !ids.is_empty()
                && ids.iter().all(|id| {
                    self.tab(*id)
                        .is_some_and(|tab| tab.controller.read(cx).is_remote())
                });
            let (title, detail) = match &target {
                CloseTarget::Session(_) if all_remote => (
                    "Detach from this SSH session?",
                    "The remote agent keeps running. Unsaved drafts in this tab are discarded. Use Stop before closing if you want to abort remote work.",
                ),
                CloseTarget::Project(_) if all_remote => (
                    "Remove this remote project from the sidebar?",
                    "Its sessions detach; remote agents keep running. Unsaved drafts are discarded. Remote files and saved conversations are not deleted.",
                ),
                CloseTarget::Session(_) => (
                    "Close this session?",
                    "Its running agent will be stopped. Unsaved draft/editor text in this tab will be discarded; buffers still open elsewhere are retained. An unfinished turn may not be recorded. Saved conversation and project files will not be deleted.",
                ),
                CloseTarget::Project(_) => (
                    "Remove this project from the sidebar?",
                    "Its active sessions will close and their agents will stop. Unsaved drafts/editor tabs will be discarded; buffers still open elsewhere are retained. Project files and saved conversations will not be deleted. Open the folder again to bring it back.",
                ),
                CloseTarget::Saved(_) => (
                    "Hide this saved session?",
                    "This only hides the row for this app run. Its saved conversation file is not deleted.",
                ),
            };
            let subject = match &target {
                CloseTarget::Session(id) => self
                    .summaries
                    .get(id)
                    .map(|s| s.title.clone())
                    .unwrap_or_default(),
                CloseTarget::Project(path) => path.display().to_string(),
                CloseTarget::Saved(path) => short_path(path),
            };
            let heading = format!("{title}\n{subject}");
            let answer = window.prompt(
                gpui::PromptLevel::Warning,
                &heading,
                Some(detail),
                &[
                    gpui::PromptButton::cancel("Cancel"),
                    gpui::PromptButton::ok("Close / remove"),
                ],
                cx,
            );
            self.close_prompt_pending = true;
            cx.spawn(async move |this, cx| {
                let answer = answer.await;
                this.update(cx, |this, cx| {
                    this.close_prompt_pending = false;
                    if answer == Ok(1) && this.close_ids(&target) == ids && this.can_close(&ids, cx)
                    {
                        this.close_confirmed(target, cx);
                    }
                })
                .ok();
            })
            .detach();
        } else {
            self.close_confirmed(target, cx);
        }
    }
    pub(super) fn close_confirmed(&mut self, target: CloseTarget, cx: &mut Context<Self>) {
        let ids = self.close_ids(&target);
        if !self.can_close(&ids, cx) {
            return;
        }
        for id in &ids {
            if let Some(tab) = self.tab(*id) {
                let controller = tab.controller.clone();
                if let Some(saved) = saved_session(controller.read(cx))
                    && !self.saved.iter().any(|s| s.path == saved.path)
                {
                    self.saved.push(saved);
                }
                controller.update(cx, |controller, _| controller.shutdown());
            }
            self.summaries.remove(id);
        }
        self.tabs.retain(|tab| !ids.contains(&tab.id));
        match target {
            CloseTarget::Project(path) => {
                self.projects.retain(|p| p != &path);
                if self.selected_project.as_ref() == Some(&path) {
                    self.selected_project = None;
                }
                self.removed_projects.insert(path);
            }
            CloseTarget::Saved(path) => {
                self.saved.retain(|s| s.path != path);
                self.hidden_sessions.insert(path);
            }
            CloseTarget::Session(_) => {}
        }
        if self.tab(self.active).is_none()
            && let Some(tab) = self.tabs.last()
        {
            self.active = tab.id;
        }
        if self.selected_project.is_none() {
            self.selected_project = self.summaries.get(&self.active).map(|s| s.cwd.clone());
        }
        self.refresh_catalog(cx);
        self.remember_open(cx);
        cx.emit(WorkspaceEvent::Navigation);
        cx.emit(WorkspaceEvent::Selection(self.active));
    }
}
