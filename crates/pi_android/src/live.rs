//! The computer the phone is connected to: the sessions its helper lists, the
//! ones the phone watches, and what the phone sends them.
//!
//! Running sessions are watched as soon as they are listed, so the list stays
//! current and notifications fire; others are attached when opened. New
//! sessions are durable: their state lives in a database on the computer, so
//! nothing is lost when the phone goes away.

use crate::{
    model::{Answer, SessionId},
    projection,
    remote::{self, Helper, Listed},
    ssh::{self, Connection},
};
use pi_core::{
    session::Session as Pi,
    ssh::{RemoteBackend, SshTarget},
};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Something that happened on the computer, for the store to apply.
pub enum Update {
    /// A session's pipe opened; commands can go in.
    Opened(async_channel::Sender<Value>),
    Record(Value),
    /// The pipe closed, with the reason.
    Ended(String),
    /// A fresh listing of the computer's sessions.
    Listed(Result<Vec<Listed>, String>),
}

/// What a request was for, to act on its response.
enum Request {
    Prompt(String),
    Models,
    SetModel,
    /// Its failure doesn't matter: a model without thinking levels refuses one.
    Quiet,
    Other,
}

struct Watch {
    target: SshTarget,
    pi: Pi,
    input: Option<async_channel::Sender<Value>>,
    /// Whether Pi's state has arrived since the pipe opened.
    current: bool,
    /// Whether commands may go: the state arrived and a model is chosen.
    ready: bool,
    /// Prompts typed on the phone that Pi hasn't taken yet.
    outbox: Vec<String>,
    sent: HashMap<String, Request>,
    /// Pi's open questions, oldest first.
    dialogs: Vec<Value>,
    ended: Option<String>,
    /// When the phone began watching, in seconds since 1970.
    since: u64,
}

fn seconds_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl Watch {
    fn new(target: SshTarget) -> Self {
        let pi = Pi::new(target.cwd.clone().into());
        Self {
            target,
            pi,
            input: None,
            current: false,
            ready: false,
            outbox: Vec::new(),
            sent: HashMap::new(),
            dialogs: Vec::new(),
            ended: None,
            since: seconds_now(),
        }
    }
}

/// A problem for the person to see, from a session or the computer.
pub struct Problem {
    pub session: Option<SessionId>,
    pub text: String,
}

pub struct Live {
    pub connection: Connection,
    pub helper: Helper,
    /// The SSH address, as session identities name the host.
    host: String,
    sender: async_channel::Sender<(Option<SessionId>, Update)>,
    pub updates: async_channel::Receiver<(Option<SessionId>, Update)>,
    watches: HashMap<SessionId, Watch>,
    listed: HashMap<SessionId, Listed>,
    keys: HashMap<String, SessionId>,
    next_id: u32,
    /// The model new sessions start with, by name or id, as the phone's settings say.
    pub model: String,
    /// Their thinking level, as the model sheet names it: "High".
    pub thinking: String,
    /// What the computer offers, from the first session that reported it.
    pub models: Vec<pi_core::protocol::Model>,
}

fn request_id() -> String {
    format!("phone-{:016x}", rand::random::<u64>())
}

impl Live {
    pub fn new(connection: Connection, helper: Helper, host: String, model: String) -> Self {
        let (sender, updates) = async_channel::unbounded();
        Self {
            connection,
            helper,
            host,
            sender,
            updates,
            watches: HashMap::new(),
            listed: HashMap::new(),
            keys: HashMap::new(),
            next_id: 1,
            model,
            thinking: String::new(),
            models: Vec::new(),
        }
    }

    fn id_for(&mut self, key: &str) -> SessionId {
        if let Some(id) = self.keys.get(key) {
            return *id;
        }
        let id = SessionId(self.next_id);
        self.next_id += 1;
        self.keys.insert(key.to_owned(), id);
        id
    }

    /// Asks the computer for its sessions; the answer arrives as an update.
    pub fn refresh(&self) {
        let (connection, helper, sender) = (
            self.connection.clone(),
            self.helper.clone(),
            self.sender.clone(),
        );
        ssh::spawn(async move {
            let listed = remote::sessions(&connection, &helper)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send((None, Update::Listed(listed))).await;
        });
    }

    fn attach(&self, id: SessionId, target: SshTarget) {
        let (connection, helper, sender) = (
            self.connection.clone(),
            self.helper.clone(),
            self.sender.clone(),
        );
        ssh::spawn(async move {
            let pipe = match remote::attach(&connection, &helper, &target).await {
                Ok(pipe) => pipe,
                Err(error) => {
                    let _ = sender
                        .send((Some(id), Update::Ended(format!("{error:#}"))))
                        .await;
                    return;
                }
            };
            if sender
                .send((Some(id), Update::Opened(pipe.input.clone())))
                .await
                .is_err()
            {
                return;
            }
            while let Ok(record) = pipe.records.recv().await {
                if sender
                    .send((Some(id), Update::Record(record)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let reason = pipe.ended.recv().await.unwrap_or_default();
            let _ = sender.send((Some(id), Update::Ended(reason))).await;
        });
    }

    /// Starts watching a listed session; nothing happens if already watched.
    pub fn watch(&mut self, id: SessionId) {
        if self
            .watches
            .get(&id)
            .is_some_and(|watch| watch.ended.is_none())
        {
            return;
        }
        let target = match self.watches.get(&id) {
            Some(watch) => watch.target.clone(),
            None => match self
                .listed
                .get(&id)
                .and_then(|listed| remote::listed_target(&self.host, listed).ok())
            {
                Some(target) => target,
                None => return,
            },
        };
        let mut watch = self
            .watches
            .remove(&id)
            .unwrap_or_else(|| Watch::new(target.clone()));
        watch.input = None;
        watch.current = false;
        watch.ready = false;
        watch.ended = None;
        self.watches.insert(id, watch);
        self.attach(id, target);
    }

    pub fn is_watched(&self, id: SessionId) -> bool {
        self.watches.contains_key(&id)
    }

    /// Picks the connection back up after it dropped: watched sessions attach again.
    pub fn resume(&mut self, connection: Connection) {
        self.connection = connection;
        let watched: Vec<SessionId> = self.watches.keys().copied().collect();
        for id in watched {
            if let Some(watch) = self.watches.get_mut(&id) {
                watch.ended = Some("Reconnecting".into());
            }
            self.watch(id);
        }
        self.refresh();
    }

    /// A new durable session in `folder`, starting with `prompt`.
    pub fn start(&mut self, folder: &str, prompt: String) -> Result<SessionId, String> {
        let target =
            remote::new_target(&self.host, folder).map_err(|error| format!("{error:#}"))?;
        let id = self.id_for(&target.key.clone());
        let mut watch = Watch::new(target.clone());
        watch.outbox.push(prompt);
        self.watches.insert(id, watch);
        self.attach(id, target);
        Ok(id)
    }

    fn send(watch: &mut Watch, mut record: Value, request: Request) {
        let id = request_id();
        record["id"] = json!(id);
        if let Request::Prompt(_) = request {
            // Durable sessions admit each prompt once, by this id.
            record["requestId"] = json!(id);
        }
        if let Some(input) = &watch.input
            && input.try_send(record).is_ok()
        {
            watch.sent.insert(id, request);
        }
    }

    fn flush(watch: &mut Watch) {
        if !watch.ready || watch.input.is_none() {
            return;
        }
        let pending: Vec<String> = watch
            .outbox
            .iter()
            .filter(|prompt| {
                !watch
                    .sent
                    .values()
                    .any(|request| matches!(request, Request::Prompt(sent) if sent == *prompt))
            })
            .cloned()
            .collect();
        for prompt in pending {
            let mut record = json!({"type": "prompt", "message": prompt});
            if watch.pi.busy() || !watch.sent.is_empty() {
                record["streamingBehavior"] = json!("followUp");
            }
            Self::send(watch, record, Request::Prompt(prompt));
        }
    }

    /// A prompt for a session: it runs now, or follows the current run.
    pub fn prompt(&mut self, id: SessionId, prompt: String) {
        self.watch(id);
        if let Some(watch) = self.watches.get_mut(&id) {
            watch.outbox.push(prompt);
            Self::flush(watch);
        }
    }

    pub fn stop(&mut self, id: SessionId) {
        if let Some(watch) = self.watches.get_mut(&id) {
            watch.outbox.clear();
            Self::send(watch, json!({"type": "abort"}), Request::Other);
        }
    }

    /// Drops one queued follow-up: Pi clears its queue, and the rest go back in.
    pub fn unqueue(&mut self, id: SessionId, index: usize) {
        let Some(watch) = self.watches.get_mut(&id) else {
            return;
        };
        let queued: Vec<String> = watch
            .pi
            .steering
            .iter()
            .chain(&watch.pi.follow_up)
            .cloned()
            .collect();
        if index >= queued.len() {
            let local = index - queued.len();
            let unsent = watch
                .outbox
                .len()
                .saturating_sub(usize::from(!watch.pi.busy()));
            if local < unsent {
                let at = watch.outbox.len() - unsent + local;
                watch.outbox.remove(at);
            }
            return;
        }
        Self::send(watch, json!({"type": "clear_queue"}), Request::Other);
        for (position, prompt) in queued.into_iter().enumerate() {
            if position != index {
                watch.outbox.push(prompt);
            }
        }
        Self::flush(watch);
    }

    pub fn answer(&mut self, id: SessionId, answer: Answer) {
        let Some(watch) = self.watches.get_mut(&id) else {
            return;
        };
        if watch.dialogs.is_empty() {
            return;
        }
        let request = watch.dialogs.remove(0);
        if let Some(input) = &watch.input {
            let _ = input.try_send(projection::answer(&request, answer));
        }
    }

    /// Applies an update; returns the session it changed and any problem.
    pub fn apply(
        &mut self,
        id: Option<SessionId>,
        update: Update,
    ) -> (Vec<SessionId>, Option<Problem>) {
        let Some(id) = id else {
            let Update::Listed(listed) = update else {
                return (Vec::new(), None);
            };
            return match listed {
                Ok(listed) => (self.listed(listed), None),
                Err(text) => (
                    Vec::new(),
                    Some(Problem {
                        session: None,
                        text,
                    }),
                ),
            };
        };
        let Some(watch) = self.watches.get_mut(&id) else {
            return (Vec::new(), None);
        };
        let mut problem = None;
        match update {
            Update::Opened(input) => watch.input = Some(input),
            Update::Ended(reason) => {
                watch.input = None;
                watch.ready = false;
                if watch.ended.is_none() {
                    log::info!("Session {} closed: {reason}", watch.target.key);
                    watch.ended = Some(reason);
                }
            }
            Update::Record(record) => {
                problem = Self::record(
                    watch,
                    &record,
                    &self.model,
                    &self.thinking,
                    &mut self.models,
                )
                .map(|text| Problem {
                    session: Some(id),
                    text,
                });
            }
            Update::Listed(_) => {}
        }
        (vec![id], problem)
    }

    fn record(
        watch: &mut Watch,
        record: &Value,
        wanted: &str,
        thinking: &str,
        models: &mut Vec<pi_core::protocol::Model>,
    ) -> Option<String> {
        let kind = record["type"].as_str().unwrap_or("");
        if kind == "remote_error" {
            return record["error"].as_str().map(str::to_owned);
        }
        if kind == "extension_ui_request"
            && matches!(record["method"].as_str(), Some("confirm" | "select"))
        {
            watch.dialogs.push(record.clone());
        }
        if kind == "extension_ui_cancel" {
            watch.dialogs.retain(|dialog| dialog["id"] != record["id"]);
        }
        if let Err(error) = watch.pi.apply(record) {
            log::warn!("Skipping a record Pi's session model refused: {error:#}");
        }
        if kind == "remote_snapshot" && !watch.current {
            watch.current = true;
            if watch.pi.state.model.is_none() && watch.target.backend == RemoteBackend::Durable {
                Self::send(
                    watch,
                    json!({"type": "get_available_models"}),
                    Request::Models,
                );
            } else {
                watch.ready = true;
                Self::flush(watch);
            }
        }
        if kind != "response" {
            return None;
        }
        let request = record["id"].as_str().and_then(|id| watch.sent.remove(id))?;
        let failed = record["success"] != true;
        let error = record["error"]
            .as_str()
            .unwrap_or("Pi refused it")
            .to_owned();
        match request {
            Request::Prompt(prompt) => {
                watch.outbox.retain(|queued| *queued != prompt);
                failed.then(|| {
                    if watch.pi.messages.is_empty() {
                        watch.pi.error = Some(error.clone());
                    }
                    error
                })
            }
            Request::Models => {
                if let Ok(available) = serde_json::from_value::<Vec<pi_core::protocol::Model>>(
                    record["data"]["models"].clone(),
                ) {
                    *models = available;
                }
                let Some(model) = choose_model(models, wanted) else {
                    let text =
                        "No model is set up for durable sessions on the computer.".to_owned();
                    watch.outbox.clear();
                    watch.pi.error = Some(text.clone());
                    return Some(text);
                };
                let record =
                    json!({"type": "set_model", "provider": model.provider, "modelId": model.id});
                Self::send(watch, record, Request::SetModel);
                if let Some(level) = thinking_level(thinking) {
                    Self::send(
                        watch,
                        json!({"type": "set_thinking_level", "level": level}),
                        Request::Quiet,
                    );
                }
                None
            }
            Request::SetModel => {
                watch.ready = true;
                Self::flush(watch);
                failed.then_some(error)
            }
            Request::Quiet => None,
            Request::Other => failed.then_some(error),
        }
    }

    /// Takes in a listing; returns the sessions it named.
    fn listed(&mut self, listed: Vec<Listed>) -> Vec<SessionId> {
        let mut ids = Vec::new();
        for session in listed {
            let id = self.id_for(&session.key);
            let running = session.running;
            self.listed.insert(id, session);
            if running && !self.is_watched(id) {
                self.watch(id);
            }
            ids.push(id);
        }
        ids
    }

    /// The session as the phone shows it.
    pub fn session(&self, id: SessionId) -> Option<crate::model::Session> {
        if let Some(watch) = self.watches.get(&id)
            && (watch.current || !watch.outbox.is_empty() || watch.pi.error.is_some())
        {
            return Some(projection::project(
                &watch.pi,
                projection::Facts {
                    id,
                    cwd: &watch.target.cwd,
                    folder: self.helper.short(&watch.target.cwd),
                    question: watch.dialogs.first().and_then(projection::question),
                    outbox: &watch.outbox,
                    key: &watch.target.key,
                },
            ));
        }
        let listed = self.listed.get(&id)?;
        Some(projection::listed(
            id,
            listed,
            self.helper.short(&listed.cwd),
        ))
    }

    /// When a session last moved, in seconds since 1970, for the list's order.
    pub fn recency(&self, id: SessionId) -> u64 {
        let listed = self.listed.get(&id).map_or(0, |listed| listed.updated);
        let watched = self.watches.get(&id).map_or(0, |watch| {
            let last = watch
                .pi
                .messages
                .iter()
                .filter_map(|message| message["timestamp"].as_u64())
                .max()
                .map_or(0, |millis| millis / 1000);
            last.max(watch.since * u64::from(watch.pi.messages.is_empty()))
        });
        listed.max(watched)
    }

    /// Project folders the computer has sessions in, most recent first.
    pub fn folders(&self) -> Vec<String> {
        let mut listed: Vec<&Listed> = self.listed.values().collect();
        listed.sort_by_key(|session| std::cmp::Reverse(session.updated));
        let mut folders: Vec<String> = Vec::new();
        for session in listed {
            if !folders.contains(&session.cwd) {
                folders.push(session.cwd.clone());
            }
        }
        for watch in self.watches.values() {
            if !folders.contains(&watch.target.cwd) {
                folders.push(watch.target.cwd.clone());
            }
        }
        folders
    }
}

/// Pi's name for a thinking level the model sheet shows.
fn thinking_level(shown: &str) -> Option<&'static str> {
    Some(match shown {
        "Off" => "off",
        "Low" => "low",
        "Medium" => "medium",
        "High" => "high",
        "Max" => "xhigh",
        _ => return None,
    })
}

/// The model the settings name, by name or id, else the first offered.
fn choose_model<'a>(
    models: &'a [pi_core::protocol::Model],
    wanted: &str,
) -> Option<&'a pi_core::protocol::Model> {
    let wanted = wanted.to_lowercase();
    let words: Vec<&str> = wanted.split_whitespace().collect();
    models
        .iter()
        .find(|model| {
            let name = model.name.as_deref().unwrap_or(&model.id).to_lowercase();
            let id = model.id.to_lowercase();
            !words.is_empty()
                && words
                    .iter()
                    .all(|word| name.contains(word) || id.contains(&word.replace('.', "-")))
        })
        .or_else(|| models.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_core::protocol::Model;

    fn model(provider: &str, id: &str, name: &str) -> Model {
        Model {
            provider: provider.into(),
            id: id.into(),
            name: Some(name.into()),
            ..Default::default()
        }
    }

    #[test]
    fn the_preferred_model_is_found_by_name_or_id() {
        let models = [
            model("anthropic", "claude-sonnet-5-5", "Claude Sonnet 5.5"),
            model("anthropic", "claude-opus-5-5", "Claude Opus 5.5"),
        ];
        assert_eq!(
            choose_model(&models, "Opus 5.5").unwrap().id,
            "claude-opus-5-5"
        );
        assert_eq!(
            choose_model(&models, "sonnet").unwrap().id,
            "claude-sonnet-5-5"
        );
        assert_eq!(
            choose_model(&models, "GPT 9").unwrap().id,
            "claude-sonnet-5-5",
            "anything offered beats nothing"
        );
        assert!(choose_model(&[], "Opus 5.5").is_none());
    }

    fn watch() -> (Watch, async_channel::Receiver<Value>) {
        let target = remote::new_target("nick@studio", "/Users/nick/repos/pi").unwrap();
        let mut watch = Watch::new(target);
        let (input, sent) = async_channel::unbounded();
        watch.input = Some(input);
        (watch, sent)
    }

    #[test]
    fn a_new_durable_session_gets_a_model_before_its_prompt() {
        let (mut watch, sent) = watch();
        let mut models = Vec::new();
        watch.outbox.push("Explain this project".into());
        let snapshot = json!({"type":"remote_snapshot","data":serde_json::to_value(Pi::new("/Users/nick/repos/pi".into())).unwrap()});
        assert!(Live::record(&mut watch, &snapshot, "Opus 5.5", "", &mut models).is_none());
        let ask = sent.try_recv().unwrap();
        assert_eq!(ask["type"], "get_available_models");
        assert!(sent.try_recv().is_err(), "no prompt before a model");
        let offered = json!({"type":"response","id":ask["id"],"command":"get_available_models","success":true,"data":{"models":[
            {"provider":"anthropic","id":"claude-opus-5-5","name":"Claude Opus 5.5"}
        ]}});
        Live::record(&mut watch, &offered, "Opus 5.5", "", &mut models);
        let set = sent.try_recv().unwrap();
        assert_eq!(
            (set["type"].as_str(), set["modelId"].as_str()),
            (Some("set_model"), Some("claude-opus-5-5"))
        );
        let done = json!({"type":"response","id":set["id"],"command":"set_model","success":true});
        Live::record(&mut watch, &done, "Opus 5.5", "", &mut models);
        let prompt = sent.try_recv().unwrap();
        assert_eq!(prompt["type"], "prompt");
        assert_eq!(prompt["message"], "Explain this project");
        assert_eq!(prompt["requestId"], prompt["id"]);
        assert!(prompt.get("streamingBehavior").is_none());
        let refused = json!({"type":"response","id":prompt["id"],"command":"prompt","success":false,"error":"No API key"});
        assert_eq!(
            Live::record(&mut watch, &refused, "Opus 5.5", "", &mut models).as_deref(),
            Some("No API key")
        );
        assert!(watch.outbox.is_empty());
        assert_eq!(watch.pi.error.as_deref(), Some("No API key"));
    }

    /// Runs a durable session over real SSH, as the phone does. Needs an SSH
    /// server with a durable-enabled helper installed for the account, e.g.
    /// with the faux runner: PI_ANDROID_TEST_SSH=user@host:port,
    /// PI_ANDROID_TEST_KEYS=its authorized_keys (the phone's key is added),
    /// PI_ANDROID_TEST_PROJECT=a folder there.
    #[test]
    #[ignore = "needs an SSH server; set PI_ANDROID_TEST_SSH, PI_ANDROID_TEST_KEYS and PI_ANDROID_TEST_PROJECT"]
    fn a_durable_session_runs_over_ssh() {
        use crate::model::State;
        use std::time::{Duration, Instant};
        let variable = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
        let address = ssh::Address::parse(&variable("PI_ANDROID_TEST_SSH")).unwrap();
        let project = variable("PI_ANDROID_TEST_PROJECT");
        let dir = std::env::temp_dir().join(format!("pi-android-e2e-{}", std::process::id()));
        let identity = ssh::Identity::load_or_create(&dir.join("id_ed25519")).unwrap();
        let keys = variable("PI_ANDROID_TEST_KEYS");
        let mut authorized = std::fs::read_to_string(&keys).unwrap_or_default();
        authorized.push_str(&format!("{}\n", identity.public_line()));
        std::fs::write(&keys, authorized).unwrap();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let connection = Connection::open(address.clone(), identity.clone(), None)
                .await
                .unwrap();
            assert!(connection.fingerprint.starts_with("SHA256:"));
            let wrong = Some("SHA256:not-this-computer".to_owned());
            let refused = Connection::open(address.clone(), identity, wrong).await;
            assert!(
                matches!(
                    refused
                        .err()
                        .and_then(|e| e.downcast::<ssh::Failure>().ok()),
                    Some(ssh::Failure::HostKeyChanged { .. })
                ),
                "a changed host key is refused"
            );
            let helper = remote::find(&connection).await.unwrap();
            assert!(helper.path.ends_with("/pi-desktop-remote"), "{helper:?}");
            let mut live = Live::new(
                connection.clone(),
                helper.clone(),
                address.to_string(),
                "faux".into(),
            );
            let id = live.start(&project, "hello from the phone".into()).unwrap();
            let shown = live.session(id).unwrap();
            assert_eq!(
                shown.state,
                State::Working,
                "shown at once, before Pi has it"
            );
            assert_eq!(shown.title, "hello from the phone");

            let until = async |live: &mut Live, done: &dyn Fn(&crate::model::Session) -> bool| {
                let deadline = Instant::now() + Duration::from_secs(60);
                loop {
                    let session = live.session(id).unwrap();
                    if done(&session) {
                        return session;
                    }
                    assert!(Instant::now() < deadline, "timed out: {session:#?}");
                    let left = deadline - Instant::now();
                    let update = tokio::time::timeout(left, live.updates.recv())
                        .await
                        .expect("an update")
                        .unwrap();
                    let (_, problem) = live.apply(update.0, update.1);
                    if let Some(problem) = problem {
                        panic!("{}", problem.text);
                    }
                }
            };
            let finished = until(&mut live, &|s| {
                s.state == State::Done && !s.turns.is_empty()
            })
            .await;
            let summary = finished.turns[0].summary.clone().unwrap();
            assert_eq!(summary.headline, "Finished: hello from the phone");
            assert!(
                live.models.iter().any(|model| model.provider == "faux"),
                "the model was chosen"
            );

            live.prompt(id, "and again".into());
            let again = until(&mut live, &|s| s.state == State::Done && s.turns.len() == 2).await;
            assert_eq!(
                again.turns[1].summary.as_ref().unwrap().headline,
                "Finished: and again"
            );

            let listed = remote::sessions(&connection, &helper).await.unwrap();
            let key = live.watches[&id].target.key.clone();
            let session = listed
                .iter()
                .find(|listed| listed.key == key)
                .expect("listed");
            assert_eq!(session.title.as_deref(), Some("hello from the phone"));
            assert_eq!(session.backend, RemoteBackend::Durable);
            assert!(session.running && !session.busy);

            // A second phone attaches to the same session and sees it all.
            let mut other = Live::new(
                connection.clone(),
                helper,
                address.to_string(),
                "faux".into(),
            );
            let (ids, _) = other.apply(None, Update::Listed(Ok(listed)));
            let twin = ids
                .into_iter()
                .find(|twin| other.listed[twin].key == key)
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            while other.session(twin).unwrap().turns.len() < 2 {
                assert!(Instant::now() < deadline);
                let update = tokio::time::timeout(Duration::from_secs(30), other.updates.recv())
                    .await
                    .unwrap()
                    .unwrap();
                other.apply(update.0, update.1);
            }
            assert_eq!(other.session(twin).unwrap().title, "hello from the phone");
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn questions_wait_for_an_answer_and_cancel_with_pi() {
        let (mut watch, _) = watch();
        let mut models = Vec::new();
        let ask = json!({"type":"extension_ui_request","id":"q","method":"confirm","title":"Run the tests?"});
        Live::record(&mut watch, &ask, "", "", &mut models);
        assert_eq!(watch.dialogs.len(), 1);
        Live::record(
            &mut watch,
            &json!({"type":"extension_ui_cancel","id":"q"}),
            "",
            "",
            &mut models,
        );
        assert!(watch.dialogs.is_empty());
    }
}
