//! What the screens read and act on: a computer, its sessions and projects.
//! Behind it is a connected computer (`live`), or the sample sessions that run
//! on their own (`demo`), for trying the app and for previews.

use crate::{
    demo,
    live::{Live, Update},
    model::*,
};
use std::{collections::HashMap, time::Duration};

/// What happened, for notifications and notices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    NeedsYou(SessionId),
    Finished(SessionId),
    Deleted(SessionId),
    /// Something went wrong, in a session or with the computer.
    Problem(Option<SessionId>, String),
}

/// A folder on the computer new sessions can start in.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub name: String,
    /// As shown: `~/repos/pi`.
    pub folder: String,
    /// On the computer: `/Users/nick/repos/pi`.
    pub path: String,
}

impl Project {
    fn new(path: &str, folder: String) -> Self {
        let trimmed = path.trim_end_matches('/');
        Self {
            name: trimmed.rsplit('/').next().unwrap_or(trimmed).to_owned(),
            folder,
            path: path.to_owned(),
        }
    }
}

pub struct Store {
    pub computer: Computer,
    /// Every computer the phone knows, the connected one first.
    pub computers: Vec<Computer>,
    pub sessions: Vec<Session>,
    /// Older sessions the computer has, not loaded on the phone.
    pub earlier: u32,
    pub projects: Vec<Project>,
    next_id: u32,
    /// The connected computer; `None` for the sample.
    pub(crate) live: Option<Live>,
    /// When each live session last moved, for the list's order.
    recency: HashMap<SessionId, u64>,
    /// The sample's subagents, by conversation: a live computer sends its own.
    pub(crate) sample_subagents: HashMap<String, pi_core::session::Session>,
}

impl Store {
    pub fn sample(computer: Computer) -> Self {
        Self {
            computers: demo::computers(computer.clone()),
            computer,
            sessions: demo::sessions(),
            earlier: 24,
            projects: demo::PROJECTS
                .iter()
                .map(|(name, folder)| Project {
                    name: (*name).into(),
                    folder: (*folder).into(),
                    path: folder.replacen('~', crate::projects::SAMPLE_HOME, 1),
                })
                .collect(),
            next_id: 8,
            live: None,
            recency: HashMap::new(),
            sample_subagents: HashMap::new(),
        }
    }

    pub fn live(computer: Computer, live: Live) -> Self {
        let mut store = Self {
            computers: vec![computer.clone()],
            computer,
            sessions: Vec::new(),
            earlier: 0,
            projects: Vec::new(),
            next_id: 1,
            live: Some(live),
            recency: HashMap::new(),
            sample_subagents: HashMap::new(),
        };
        store.refresh_projects();
        store
    }

    pub fn is_sample(&self) -> bool {
        self.live.is_none()
    }

    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|session| session.id == id)
    }

    pub fn remove(&mut self, id: SessionId) {
        if let Some(live) = &mut self.live {
            live.remove(id);
        }
        self.sessions.retain(|session| session.id != id);
        self.recency.remove(&id);
    }

    fn session_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    /// Sessions as the list shows them: needing you, working, then finished.
    pub fn grouped(&self) -> [Vec<&Session>; 3] {
        let mut groups: [Vec<&Session>; 3] = Default::default();
        for session in &self.sessions {
            let group = match session.state {
                State::NeedsYou => 0,
                State::Working => 1,
                _ => 2,
            };
            groups[group].push(session);
        }
        for group in &mut groups {
            if self.live.is_some() {
                group.sort_by_key(|session| {
                    std::cmp::Reverse((self.recency.get(&session.id).copied(), session.id))
                });
            } else {
                // Newest first; sample ids grow with time.
                group.sort_by_key(|session| std::cmp::Reverse(session.id));
            }
        }
        groups
    }

    pub fn running(&self) -> impl Iterator<Item = &Session> {
        self.sessions
            .iter()
            .filter(|session| session.state == State::Working)
    }

    /// Moves time on: sample runs advance; live timers count up.
    pub fn tick(&mut self, elapsed: Duration) -> Vec<Event> {
        if self.live.is_some() {
            let running: Vec<SessionId> = self.running().map(|session| session.id).collect();
            let mut events = Vec::new();
            for id in running {
                events.extend(self.reproject(id));
            }
            return events;
        }
        let mut events = Vec::new();
        for session in &mut self.sessions {
            if session.state != State::Working {
                continue;
            }
            session.elapsed += elapsed;
            if let Some(event) = demo::advance(session, elapsed) {
                events.push(event);
            }
        }
        for event in &events {
            if let Event::Finished(id) = *event {
                self.start_queued(id);
            }
        }
        events
    }

    /// Takes in what the computer sent.
    pub fn apply(&mut self, updates: Vec<(Option<SessionId>, Update)>) -> Vec<Event> {
        let Some(live) = &mut self.live else {
            return Vec::new();
        };
        let mut changed = Vec::new();
        let mut events = Vec::new();
        let mut listed = false;
        for (id, update) in updates {
            listed |= id.is_none();
            let (ids, problem) = live.apply(id, update);
            if let Some(problem) = problem {
                events.push(Event::Problem(problem.session, problem.text));
            }
            for id in ids {
                if !changed.contains(&id) {
                    changed.push(id);
                }
            }
        }
        if listed {
            self.refresh_projects();
        }
        for id in changed {
            events.extend(self.reproject(id));
        }
        events
    }

    /// Shows a live session as it is now; returns what changed for the user.
    fn reproject(&mut self, id: SessionId) -> Vec<Event> {
        let Some(live) = &self.live else {
            return Vec::new();
        };
        let Some(session) = live.session(id) else {
            if self.session(id).is_some() {
                self.remove(id);
                return vec![Event::Deleted(id)];
            }
            return Vec::new();
        };
        let recency = live.recency(id);
        self.recency.insert(id, recency);
        let before = self.session(id).map(|session| session.state);
        let after = session.state;
        match self.session_mut(id) {
            Some(shown) => *shown = session,
            None => self.sessions.push(session),
        }
        match (before, after) {
            (Some(before), after) if before == after => Vec::new(),
            (_, State::NeedsYou) => vec![Event::NeedsYou(id)],
            (Some(State::Working | State::NeedsYou), State::Done | State::Failed) => {
                vec![Event::Finished(id)]
            }
            _ => Vec::new(),
        }
    }

    fn refresh_projects(&mut self) {
        let Some(live) = &self.live else {
            return;
        };
        // Keep stable indices: a background session refresh must not silently
        // change the project selected in an unsent composer.
        for path in live.folders() {
            if !self.projects.iter().any(|known| known.path == path) {
                self.projects
                    .push(Project::new(&path, live.helper.short(&path)));
            }
        }
    }

    /// A folder typed on the phone, offered from now on.
    pub fn add_project(&mut self, path: &str) -> usize {
        if let Some(index) = self
            .projects
            .iter()
            .position(|project| project.path == path)
        {
            return index;
        }
        let folder = self.live.as_ref().map_or_else(
            || path.replacen(crate::projects::SAMPLE_HOME, "~", 1),
            |live| live.helper.short(path),
        );
        self.projects.push(Project::new(path, folder));
        self.projects.len() - 1
    }

    /// Opens a session: a live one is attached so it stays current.
    pub fn watch(&mut self, id: SessionId) {
        if let Some(live) = &mut self.live {
            live.watch(id);
        }
    }

    pub fn answer(&mut self, id: SessionId, answer: Answer) {
        if let Some(live) = &mut self.live {
            live.answer(id, answer);
            self.reproject(id);
            return;
        }
        let Some(session) = self.session_mut(id) else {
            return;
        };
        if session.state != State::NeedsYou || session.question.take().is_none() {
            return;
        }
        session.state = State::Working;
        if answer == Answer::Deny {
            demo::decline(session);
        }
    }

    pub fn stop(&mut self, id: SessionId) -> Result<(), String> {
        if let Some(live) = &mut self.live {
            return live.stop(id);
        }
        let Some(session) = self.session_mut(id) else {
            return Err("This session is no longer available.".into());
        };
        if !session.state.is_running() {
            return Ok(());
        }
        session.state = State::Stopped;
        session.question = None;
        session.script = None;
        session.queued.clear();
        session.activity = "Stopped".into();
        session.finished_at = Some(clock_now());
        for stage in &mut session.turn_mut().stages {
            if matches!(stage.status, StageStatus::Live | StageStatus::Planned) {
                stage.status = StageStatus::Skipped;
                stage.what = "Stopped".into();
            }
        }
        Ok(())
    }

    /// A follow-up: queued while the session runs, started when it is idle.
    pub fn send(
        &mut self,
        id: SessionId,
        prompt: crate::prompt::Prompt,
        attachments: Vec<String>,
    ) -> Result<(), String> {
        if let Some(live) = &mut self.live {
            live.prompt(id, prompt)?;
            self.reproject(id);
            return Ok(());
        }
        let Some(session) = self.session_mut(id) else {
            return Err("This session is no longer available.".into());
        };
        if session.state.is_running() {
            session.queued.push(prompt.label());
            return Ok(());
        }
        demo::begin_turn(session, prompt.message, attachments);
        Ok(())
    }

    pub fn unqueue(&mut self, id: SessionId, index: usize) -> Result<(), String> {
        if let Some(live) = &mut self.live {
            live.unqueue(id, index)?;
            self.reproject(id);
            return Ok(());
        }
        if let Some(session) = self.session_mut(id)
            && index < session.queued.len()
        {
            session.queued.remove(index);
        }
        Ok(())
    }

    fn start_queued(&mut self, id: SessionId) {
        if let Some(session) = self.session_mut(id)
            && session.state == State::Done
            && !session.queued.is_empty()
        {
            let prompt = session.queued.remove(0);
            demo::begin_turn(session, prompt, Vec::new());
        }
    }

    /// Starts a session in a project; returns its id.
    pub fn start(
        &mut self,
        project: usize,
        prompt: crate::prompt::Prompt,
        attachments: Vec<String>,
    ) -> Result<SessionId, String> {
        if let Some(live) = &mut self.live {
            let Some(project) = self.projects.get(project) else {
                return Err("Choose a project folder on the computer first.".into());
            };
            let id = live.start(&project.path, prompt)?;
            self.reproject(id);
            return Ok(id);
        }
        let id = SessionId(self.next_id);
        self.next_id += 1;
        self.sessions
            .push(demo::new_session(id, project, prompt.message, attachments));
        Ok(id)
    }
}
