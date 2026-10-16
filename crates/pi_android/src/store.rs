//! What the screens read and act on: a computer, its sessions and projects.
//! Behind it is a connected computer (`live`), or the sample sessions that run
//! on their own (`demo`), for trying the app and for previews.

use crate::{
    demo,
    live::{Live, Update},
    model::*,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

/// Keep a few fully hydrated transcripts warm after their idle SSH channels
/// are released. Older entries fall back to lightweight listing rows.
const HYDRATED_SESSION_CACHE: usize = 3;

/// What happened, for notifications and notices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    NeedsYou(SessionId),
    Finished(SessionId),
    /// Local manual compaction settled after canonical checkpoint/VM teardown.
    #[cfg(feature = "local-backend")]
    Compacted(SessionId, Option<String>),
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
    pub(crate) generation: u64,
    #[cfg(feature = "local-backend")]
    pub(crate) local: Option<crate::local_chat::Local>,
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
    /// Phone receipt time of accepted session data; never advanced by repaint or disconnect.
    pub(crate) observed_at: HashMap<SessionId, u64>,
    /// Fully projected live transcripts retained across idle channel release.
    hydrated: HashSet<SessionId>,
    hydrated_lru: VecDeque<SessionId>,
    /// The sample's subagents, by conversation: a live computer sends its own.
    pub(crate) sample_subagents: HashMap<String, pi_core::session::Session>,
}

fn generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
impl Store {
    #[cfg(feature = "local-backend")]
    pub(crate) fn retire_generation(&mut self) {
        self.generation = generation();
    }
    pub fn sample(computer: Computer) -> Self {
        Self {
            generation: generation(),
            #[cfg(feature = "local-backend")]
            local: None,
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
            observed_at: HashMap::new(),
            hydrated: HashSet::new(),
            hydrated_lru: VecDeque::new(),
            sample_subagents: HashMap::new(),
        }
    }

    pub fn live(computer: Computer, live: Live) -> Self {
        let mut store = Self {
            generation: generation(),
            #[cfg(feature = "local-backend")]
            local: None,
            computers: vec![computer.clone()],
            computer,
            sessions: Vec::new(),
            earlier: 0,
            projects: Vec::new(),
            next_id: 1,
            live: Some(live),
            recency: HashMap::new(),
            observed_at: HashMap::new(),
            hydrated: HashSet::new(),
            hydrated_lru: VecDeque::new(),
            sample_subagents: HashMap::new(),
        };
        store.refresh_projects();
        store
    }

    /// Shared UI contract. Local catalogs are authoritative; None retains the
    /// existing SSH legacy/default or sample presentation behavior.
    pub fn supports(&self, _feature: &str) -> bool {
        #[cfg(feature = "local-backend")]
        if let Some(backend) = &self.local {
            return backend.supports(_feature);
        }
        true
    }
    pub fn commands(&self, id: SessionId) -> Option<&[pi_core::protocol::SlashCommand]> {
        #[cfg(feature = "local-backend")]
        if let Some(backend) = &self.local {
            return backend.commands(id);
        }
        self.live.as_ref()?.commands(id)
    }
    pub fn models(&self) -> Option<&[pi_core::protocol::Model]> {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &self.local {
            return Some(local.models());
        }
        self.live.as_ref().map(|live| live.models.as_slice())
    }
    pub fn current_model(&self, id: Option<SessionId>) -> Option<&pi_core::protocol::Model> {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &self.local {
            return local.current_model(id);
        }
        self.live.as_ref()?.current_model(id?)
    }
    pub fn model_settings(&self, id: Option<SessionId>) -> Option<(String, String)> {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &self.local {
            return Some(local.model_settings(id));
        }
        self.live.as_ref()?.model_settings(id?)
    }
    pub fn thinking_levels(&self, id: Option<SessionId>) -> Option<Vec<String>> {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &self.local {
            return Some(local.thinking_levels(id));
        }
        self.live.as_ref()?.thinking_levels(id?)
    }
    pub fn set_model(
        &mut self,
        id: Option<SessionId>,
        provider: &str,
        model: &str,
    ) -> Result<(), String> {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &mut self.local {
            local.set_model(id, provider, model)?;
            if let Some(id) = id
                && let Some(session) = self.sessions.iter_mut().find(|s| s.id == id)
            {
                *session = local.project(id);
            }
            return Ok(());
        }
        let live = self.live.as_mut().ok_or("No execution backend")?;
        if let Some(id) = id {
            live.set_model(id, provider, model)
        } else {
            live.model = format!("{provider}/{model}");
            Ok(())
        }
    }
    pub fn set_thinking(&mut self, id: Option<SessionId>, level: &str) -> Result<(), String> {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &mut self.local {
            local.set_thinking(id, level)?;
            if let Some(id) = id
                && let Some(session) = self.sessions.iter_mut().find(|s| s.id == id)
            {
                *session = local.project(id);
            }
            return Ok(());
        }
        let live = self.live.as_mut().ok_or("No execution backend")?;
        if let Some(id) = id {
            live.set_thinking(id, level)
        } else {
            live.thinking = level.into();
            Ok(())
        }
    }

    pub fn is_sample(&self) -> bool {
        self.live.is_none() && !self.is_local()
    }
    pub fn is_local(&self) -> bool {
        #[cfg(feature = "local-backend")]
        {
            self.local.is_some()
        }
        #[cfg(not(feature = "local-backend"))]
        {
            false
        }
    }
    pub fn is_stopping(&self, id: SessionId) -> bool {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &self.local {
            return local.stopping(id);
        }
        self.live.as_ref().is_some_and(|live| live.is_stopping(id))
    }
    #[cfg(feature = "local-backend")]
    pub fn cancel_local(&self) {
        if let Some(local) = &self.local {
            local.cancel();
        }
    }
    #[cfg(feature = "local-backend")]
    pub fn local(local: crate::local_chat::Local) -> Self {
        let computer = Computer {
            name: "This phone".into(),
            address: "local-bun".into(),
            pi_version: None,
            connected: true,
        };
        let sessions = local.restored();
        Self {
            generation: generation(),
            computer: computer.clone(),
            computers: vec![computer],
            sessions,
            earlier: 0,
            projects: Vec::new(),
            next_id: 1,
            live: None,
            local: Some(local),
            recency: HashMap::new(),
            observed_at: HashMap::new(),
            hydrated: HashSet::new(),
            hydrated_lru: VecDeque::new(),
            sample_subagents: HashMap::new(),
        }
    }

    pub(crate) fn is_hydrated(&self, id: SessionId) -> bool {
        self.hydrated.contains(&id)
    }

    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|session| session.id == id)
    }

    pub fn remove(&mut self, id: SessionId) {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &mut self.local {
            local.remove(id);
        }
        if let Some(live) = &mut self.live {
            live.remove(id);
        }
        self.sessions.retain(|session| session.id != id);
        self.recency.remove(&id);
        self.observed_at.remove(&id);
        self.hydrated.remove(&id);
        self.hydrated_lru.retain(|cached| *cached != id);
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
                _ if session.is_working() => 1,
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
        self.sessions.iter().filter(|session| session.is_working())
    }

    /// Moves time on: sample runs advance; live timers count up.
    pub fn tick(&mut self, elapsed: Duration) -> Vec<Event> {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &mut self.local {
            return local.tick(&mut self.sessions, elapsed);
        }
        if self.live.is_some() {
            // Remote records already carry every transcript mutation. The
            // one-second clock only advances fixed-size time labels; rebuilding
            // every message, diff, and tool output here made active composers
            // and scrolling pay a full projection even without new content.
            for session in &mut self.sessions {
                if session.state != State::Working {
                    continue;
                }
                session.elapsed += elapsed;
                if let Some(turn) = session.turns.last_mut()
                    && let Some(stage) = turn.live_stage()
                {
                    turn.add_time(stage, elapsed);
                }
            }
            return Vec::new();
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
                if let Some(at) = live.observed_at(id) {
                    self.observed_at.insert(id, at);
                }
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
        // A listing can release several now-idle watches after their snapshots
        // were retained. Retry eviction once those watches are gone.
        self.trim_hydrated(None);
        events
    }

    /// Shows a live session as it is now; returns what changed for the user.
    fn reproject(&mut self, id: SessionId) -> Vec<Event> {
        let _trace = gpui_android::trace::scope(c"pi.app.reproject");
        let Some(live) = &self.live else {
            return Vec::new();
        };
        let has_snapshot = live.has_snapshot(id);
        let recency = live.recency(id);
        // Releasing an idle SSH channel must not replace an already hydrated
        // transcript with its lightweight listing row. Reopening can paint this
        // bounded cache immediately while a fresh authoritative attach catches up.
        if self.hydrated.contains(&id) && !has_snapshot && live.target(id).is_some() {
            self.recency.insert(id, recency);
            return Vec::new();
        }
        let Some(session) = live.session(id) else {
            if self.session(id).is_some() {
                self.remove(id);
                return vec![Event::Deleted(id)];
            }
            return Vec::new();
        };
        self.recency.insert(id, recency);
        let before = self.session(id).map(|session| session.state);
        let after = session.state;
        match self.session_mut(id) {
            Some(shown) => *shown = session,
            None => self.sessions.push(session),
        }
        if has_snapshot {
            self.retain_hydrated(id);
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

    fn retain_hydrated(&mut self, id: SessionId) {
        self.hydrated.insert(id);
        self.hydrated_lru.retain(|cached| *cached != id);
        self.hydrated_lru.push_back(id);
        self.trim_hydrated(Some(id));
    }

    fn trim_hydrated(&mut self, protected: Option<SessionId>) {
        while self.hydrated.len() > HYDRATED_SESSION_CACHE {
            let Some(index) = self.hydrated_lru.iter().position(|candidate| {
                Some(*candidate) != protected
                    && self
                        .live
                        .as_ref()
                        .is_none_or(|live| !live.is_watched(*candidate))
            }) else {
                break;
            };
            let victim = self.hydrated_lru.remove(index).unwrap();
            self.hydrated.remove(&victim);
            let listed = self
                .live
                .as_ref()
                .and_then(|live| live.listed_session(victim));
            if let Some(listed) = listed
                && let Some(shown) = self.session_mut(victim)
            {
                *shown = listed;
            }
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

    pub(crate) fn show(&mut self, id: Option<SessionId>) {
        if let Some(live) = &mut self.live {
            live.show(id);
        }
        // Navigation can release idle watches immediately, without waiting for
        // another remote record to trigger projection and cache maintenance.
        self.trim_hydrated(None);
    }

    pub fn answer(&mut self, id: SessionId, answer: Answer) {
        if self.is_local() {
            return;
        }
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
        #[cfg(feature = "local-backend")]
        if let Some(local) = &self.local {
            return local.stop(id);
        }
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
        session.queued_keys.clear();
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

    pub fn supports_compact(&self, id: SessionId) -> bool {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &self.local {
            return local.supports("manual_compaction") && local.commands(id).is_some();
        }
        self.live
            .as_ref()
            .is_some_and(|live| live.commands(id).is_some() && live.supports_compact(id))
    }

    /// Manually compacts an existing live session without inserting slash text
    /// into its transcript.
    pub fn compact(
        &mut self,
        id: SessionId,
        custom_instructions: Option<String>,
    ) -> Result<(), String> {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &mut self.local {
            let session = self
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Chat no longer available")?;
            return local.compact(session, custom_instructions);
        }
        let Some(live) = &mut self.live else {
            return Err("Compaction is available after connecting to a computer.".into());
        };
        live.compact(id, custom_instructions)?;
        self.reproject(id);
        Ok(())
    }

    /// A prompt for an existing session. Live runs steer; samples queue it.
    pub fn send(
        &mut self,
        id: SessionId,
        prompt: crate::prompt::Prompt,
        attachments: Vec<String>,
    ) -> Result<(), String> {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &mut self.local {
            let session = self
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Chat no longer available")?;
            return local.send(session, prompt, attachments);
        }
        if let Some(live) = &mut self.live {
            live.prompt(id, prompt)?;
            self.reproject(id);
            return Ok(());
        }
        let Some(session) = self.session_mut(id) else {
            return Err("This session is no longer available.".into());
        };
        if session.state.is_running() {
            // Hand-authored demo rows may lack identities; never rebind a new
            // prompt's key to one of those older labels.
            session.queued_keys.resize(session.queued.len(), None);
            session.queued.push(prompt.label());
            session
                .queued_keys
                .push(Some(QueueKey::Local(prompt.request_id)));
            return Ok(());
        }
        demo::begin_turn(session, prompt.message, attachments);
        Ok(())
    }

    #[cfg(test)]
    pub fn unqueue(&mut self, id: SessionId, index: usize) -> Result<(), String> {
        #[cfg(feature = "local-backend")]
        if self.local.is_some() {
            let key = self
                .session(id)
                .and_then(|session| session.queued_keys.get(index))
                .cloned()
                .flatten()
                .ok_or("This queued message has no confirmed durable identity")?;
            return self.unqueue_key(id, &key);
        }
        if let Some(live) = &mut self.live {
            live.unqueue(id, index)?;
            self.reproject(id);
            return Ok(());
        }
        if let Some(session) = self.session_mut(id)
            && index < session.queued.len()
        {
            session.queued.remove(index);
            if index < session.queued_keys.len() {
                session.queued_keys.remove(index);
            }
        }
        Ok(())
    }

    /// Cancels only the payload identified in the rendered snapshot.
    pub fn unqueue_key(&mut self, id: SessionId, key: &QueueKey) -> Result<(), String> {
        #[cfg(feature = "local-backend")]
        if let Some(local) = &mut self.local {
            return local.unqueue_key(id, key);
        }
        if let Some(live) = &mut self.live {
            live.unqueue_key(id, key)?;
            self.reproject(id);
            return Ok(());
        }
        if let Some(session) = self.session_mut(id)
            && let Some(index) = session
                .queued_keys
                .iter()
                .position(|row| row.as_ref() == Some(key))
            && index < session.queued.len()
        {
            session.queued.remove(index);
            session.queued_keys.remove(index);
        }
        Ok(())
    }

    fn start_queued(&mut self, id: SessionId) {
        if !self.is_sample() {
            return;
        }
        if let Some(session) = self.session_mut(id)
            && session.state == State::Done
            && !session.queued.is_empty()
        {
            let prompt = session.queued.remove(0);
            if !session.queued_keys.is_empty() {
                session.queued_keys.remove(0);
            }
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
        #[cfg(feature = "local-backend")]
        if let Some(local) = &mut self.local {
            let mut session = local.create()?;
            let id = session.id;
            if let Err(error) = local.send(&mut session, prompt, attachments) {
                self.sessions.push(local.project(id));
                return Err(error);
            }
            self.sessions.push(session);
            return Ok(id);
        }
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

#[cfg(test)]
mod queue_tests {
    use super::*;
    use crate::prompt::Prompt;

    fn sample_queue() -> Store {
        let mut store = Store::sample(Computer::from_address("sample@computer.local"));
        store.session_mut(SessionId(1)).unwrap().state = State::Working;
        for request_id in ["sample-first", "sample-second"] {
            store
                .send(
                    SessionId(1),
                    Prompt {
                        request_id: request_id.into(),
                        message: "duplicate".into(),
                        images: Vec::new(),
                    },
                    Vec::new(),
                )
                .unwrap();
        }
        store
    }

    #[test]
    fn observation_time_does_not_advance_on_ticks_reads_disconnect_or_sample_records() {
        let mut store = sample_queue();
        store.observed_at.insert(SessionId(1), 123);
        store.observed_at.insert(SessionId(2), 456);
        store.tick(Duration::from_secs(1));
        store.session(SessionId(1));
        store.computer.connected = false;
        store.apply(vec![(
            Some(SessionId(1)),
            Update::Record(1, serde_json::json!({"type":"remote_snapshot", "data":{}})),
        )]);
        assert_eq!(store.observed_at[&SessionId(1)], 123);
        assert_eq!(store.observed_at[&SessionId(2)], 456);
        store.remove(SessionId(1));
        assert!(!store.observed_at.contains_key(&SessionId(1)));
        assert_eq!(store.observed_at[&SessionId(2)], 456);
    }

    #[test]
    fn background_subagents_keep_a_finished_parent_in_working() {
        let mut store = sample_queue();
        let id = SessionId(2);
        let session = store.session_mut(id).unwrap();
        session.state = State::Done;
        let mut handoff = pi_core::subagent::Handoff::from_args(&serde_json::json!({
            "agent": "scout",
            "task": "Keep working"
        }));
        handoff.background = true;
        session.turn_mut().handoffs.push(handoff);

        let [_, working, _] = store.grouped();
        assert!(working.iter().any(|session| session.id == id));
        assert!(store.running().any(|session| session.id == id));
        assert_eq!(
            store.session(id).unwrap().status_line(),
            "1 subagent working"
        );

        store.session_mut(id).unwrap().turn_mut().handoffs[0].subagents[0].status =
            pi_core::subagent::Status::Done;
        let [_, working, finished] = store.grouped();
        assert!(!working.iter().any(|session| session.id == id));
        assert!(finished.iter().any(|session| session.id == id));
    }

    #[test]
    fn hydrated_cache_is_lru_bounded_and_deletion_safe() {
        let mut store = sample_queue();
        for id in [SessionId(1), SessionId(2), SessionId(3)] {
            store.retain_hydrated(id);
        }
        assert_eq!(
            store.hydrated_lru,
            [SessionId(1), SessionId(2), SessionId(3)]
        );

        // A cache hit refreshes recency. Adding a fourth transcript therefore
        // evicts the least recently reopened one, not simply the lowest ID.
        store.retain_hydrated(SessionId(1));
        store.retain_hydrated(SessionId(4));
        assert_eq!(
            store.hydrated_lru,
            [SessionId(3), SessionId(1), SessionId(4)]
        );
        assert!(!store.is_hydrated(SessionId(2)));
        assert!(store.is_hydrated(SessionId(1)));
        assert!(store.is_hydrated(SessionId(3)));
        assert!(store.is_hydrated(SessionId(4)));

        store.remove(SessionId(3));
        assert!(!store.is_hydrated(SessionId(3)));
        assert!(!store.hydrated_lru.contains(&SessionId(3)));
        assert!(store.session(SessionId(3)).is_none());
    }

    #[test]
    fn hydrated_cache_retries_eviction_after_watches_are_released() {
        let mut store = sample_queue();
        // This is the state left when four hydrated transcripts were all
        // protected by active watches. Once those watches are released, the
        // oldest entry must be evicted even if no transcript changes again.
        store.hydrated = [SessionId(1), SessionId(2), SessionId(3), SessionId(4)]
            .into_iter()
            .collect();
        store.hydrated_lru = [SessionId(1), SessionId(2), SessionId(3), SessionId(4)]
            .into_iter()
            .collect();

        store.trim_hydrated(None);

        assert_eq!(store.hydrated.len(), HYDRATED_SESSION_CACHE);
        assert_eq!(
            store.hydrated_lru,
            [SessionId(2), SessionId(3), SessionId(4)]
        );
        assert!(!store.is_hydrated(SessionId(1)));
    }

    #[test]
    fn sample_queue_keys_survive_reordering_and_cancel_exact_duplicate() {
        let mut store = sample_queue();
        let key = store.session(SessionId(1)).unwrap().queued_keys[0]
            .clone()
            .unwrap();
        let session = store.session_mut(SessionId(1)).unwrap();
        session.queued.swap(0, 1);
        session.queued_keys.swap(0, 1);
        store.unqueue_key(SessionId(1), &key).unwrap();
        store.unqueue_key(SessionId(1), &key).unwrap();
        let session = store.session(SessionId(1)).unwrap();
        assert_eq!(session.queued, ["duplicate"]);
        assert_eq!(
            session.queued_keys,
            [Some(QueueKey::Local("sample-second".into()))]
        );
    }

    #[test]
    fn sample_queue_started_row_old_key_is_noop_and_stop_clears_keys() {
        let mut store = sample_queue();
        let key = store.session(SessionId(1)).unwrap().queued_keys[0]
            .clone()
            .unwrap();
        store.session_mut(SessionId(1)).unwrap().state = State::Done;
        store.start_queued(SessionId(1));
        store.unqueue_key(SessionId(1), &key).unwrap();
        let session = store.session(SessionId(1)).unwrap();
        assert_eq!(session.queued, ["duplicate"]);
        assert_eq!(
            session.queued_keys,
            [Some(QueueKey::Local("sample-second".into()))]
        );
        store.stop(SessionId(1)).unwrap();
        let session = store.session(SessionId(1)).unwrap();
        assert!(session.queued.is_empty());
        assert!(session.queued_keys.is_empty());
    }

    #[test]
    fn sample_queue_never_assigns_a_new_prompt_key_to_an_unidentified_demo_row() {
        let mut store = sample_queue();
        let session = store.session_mut(SessionId(1)).unwrap();
        session.queued = vec!["duplicate".into()];
        session.queued_keys.clear();
        store
            .send(
                SessionId(1),
                Prompt {
                    request_id: "sample-new".into(),
                    message: "duplicate".into(),
                    images: Vec::new(),
                },
                Vec::new(),
            )
            .unwrap();
        let session = store.session(SessionId(1)).unwrap();
        assert_eq!(
            session.queued_keys,
            [None, Some(QueueKey::Local("sample-new".into()))]
        );
        store
            .unqueue_key(SessionId(1), &QueueKey::Local("sample-new".into()))
            .unwrap();
        let session = store.session(SessionId(1)).unwrap();
        assert_eq!(session.queued, ["duplicate"]);
        assert_eq!(session.queued_keys, [None]);
    }
}
