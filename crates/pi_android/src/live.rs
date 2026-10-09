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
    prompt::Prompt,
    remote::{self, Helper, Listed},
    ssh::{self, Connection},
};
use pi_core::{
    session::Session as Pi,
    ssh::{RemoteBackend, SshTarget},
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Something that happened on the computer, for the store to apply.
pub enum Update {
    /// A session's pipe opened; commands can go in.
    Opened(u64, async_channel::Sender<Value>),
    Record(u64, Value),
    /// The pipe closed, with the reason.
    Ended(u64, String),
    /// A fresh listing of the computer's sessions.
    Listed(Result<Vec<Listed>, String>),
    ModelCatalog(Result<Vec<pi_core::protocol::Model>, String>),
    CommandCatalog(Result<Vec<pi_core::protocol::SlashCommand>, String>),
}

/// What a request was for, to act on its response.
enum Request {
    Prompt(String),
    Models,
    AvailableModels,
    Commands,
    SetModel,
    SetThinking(String),
    InitialThinkingLevels(String),
    Stop,
    Compact,
    /// A tool's image, by its id in the durable session.
    Image(String),
    /// A subagent's messages, by its conversation's id.
    Subagent(String),
    /// Its failure doesn't matter: a model without thinking levels refuses one.
    Quiet,
    Other,
}

struct SentRequest {
    command: String,
    request: Request,
}

#[derive(Clone, Copy)]
struct Admission {
    /// Omitted for an idle admission that must reject rather than silently queue
    /// if another client made the session busy before it arrived.
    streaming_behavior: Option<&'static str>,
}

struct PendingPrompt {
    /// The sole owned payload while delivery is pending. Once `admission` is
    /// present, neither text nor images are rebuilt or replaced.
    prompt: Prompt,
    admission: Option<Admission>,
}

impl PendingPrompt {
    fn new(prompt: Prompt) -> Self {
        Self {
            prompt,
            admission: None,
        }
    }

    fn record(&self, admission: Admission) -> Value {
        let mut record = json!({
            "type": "prompt",
            "message": self.prompt.message,
            "images": self.prompt.images,
            "requestId": self.prompt.request_id,
        });
        if let Some(behavior) = admission.streaming_behavior {
            record["streamingBehavior"] = json!(behavior);
        }
        record
    }
}

impl From<Prompt> for PendingPrompt {
    fn from(prompt: Prompt) -> Self {
        Self::new(prompt)
    }
}

impl From<String> for PendingPrompt {
    fn from(message: String) -> Self {
        Self::new(message.into())
    }
}

impl From<&str> for PendingPrompt {
    fn from(message: &str) -> Self {
        Self::new(message.into())
    }
}

struct Watch {
    generation: u64,
    target: SshTarget,
    pi: Pi,
    input: Option<async_channel::Sender<Value>>,
    /// Whether Pi's state has arrived since the pipe opened.
    current: bool,
    /// Whether commands may go: the state arrived and a model is chosen.
    ready: bool,
    /// Prompts typed on the phone that Pi hasn't taken yet. Each item owns its
    /// immutable admission mode after the first transport accepts it.
    outbox: Vec<PendingPrompt>,
    /// Rejected payloads remain recoverable, including images. They never retry
    /// automatically or make the session look as though it is still running.
    failed: Vec<FailedPrompt>,
    sent: HashMap<String, SentRequest>,
    /// Pi's open questions, oldest first.
    dialogs: Vec<Value>,
    /// Tool images fetched from the computer, by id: absent until asked for,
    /// `None` while on the way.
    images: HashMap<String, Option<Result<ToolImageBytes, String>>>,
    /// The subagent on screen, by its conversation's id: fetched again as
    /// its parent reports progress, one request at a time.
    following: Option<String>,
    /// Subagents fetched from the computer: absent until asked for.
    subagents: HashMap<String, Result<pi_core::session::Session, String>>,
    ended: Option<String>,
    /// When the phone began watching, in seconds since 1970.
    since: u64,
}

/// A tool image's bytes and type.
#[derive(Clone)]
pub struct ToolImageBytes {
    pub mime: String,
    pub bytes: std::sync::Arc<Vec<u8>>,
}

#[derive(Clone)]
pub struct FailedPrompt {
    pub prompt: Prompt,
    pub error: String,
}

fn seconds_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl Watch {
    /// Nothing to follow: Pi and its subagents rest, and nothing waits on
    /// the phone or the computer.
    fn idle(&self) -> bool {
        self.current
            && !self.pi.busy()
            && self.outbox.is_empty()
            && self.failed.is_empty()
            && self.sent.is_empty()
            && self.dialogs.is_empty()
            && !self.pi.tools.iter().any(|tool| {
                pi_core::subagent::Handoff::of(tool).is_some_and(|handoff| {
                    handoff
                        .subagents
                        .iter()
                        .any(|subagent| !subagent.status.finished())
                })
            })
    }

    fn new(target: SshTarget) -> Self {
        let pi = Pi::new(target.cwd.clone().into());
        Self {
            generation: 1,
            target,
            pi,
            input: None,
            current: false,
            ready: false,
            outbox: Vec::new(),
            failed: Vec::new(),
            sent: HashMap::new(),
            dialogs: Vec::new(),
            images: HashMap::new(),
            following: None,
            subagents: HashMap::new(),
            ended: None,
            since: seconds_now(),
        }
    }

    fn reset_transport(&mut self) {
        self.generation += 1;
        self.input = None;
        self.current = false;
        self.ready = false;
        self.ended = None;
        // Session-local commands can change while disconnected. Keep their
        // display cache, but do not use it for action shadowing until refreshed.
        self.pi.commands_loaded = false;
        // Requests on the old pipe cannot be acknowledged by the new pipe.
        // Prompts remain in the outbox with their stable admission identities.
        self.sent.clear();
        // Images asked for on the old pipe are asked for again.
        self.images.retain(|_, image| image.is_some());
    }

    fn accepts(&self, update: &Update) -> bool {
        match update {
            Update::Opened(generation, _)
            | Update::Record(generation, _)
            | Update::Ended(generation, _) => *generation == self.generation,
            Update::Listed(_) | Update::ModelCatalog(_) | Update::CommandCatalog(_) => false,
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
    /// The session on screen, whose watch stays while it is shown.
    shown: Option<SessionId>,
    listed: HashMap<SessionId, Listed>,
    keys: HashMap<String, SessionId>,
    deleted: HashSet<String>,
    next_id: u32,
    /// The model new sessions start with, by name or id, as the phone's settings say.
    pub model: String,
    /// Their thinking level, as the model sheet names it: "High".
    pub thinking: String,
    /// What the computer offers, from the first session that reported it.
    pub models: Vec<pi_core::protocol::Model>,
    pub models_loading: bool,
    pub models_error: Option<String>,
    /// Commands available before a new session exists. A session-specific
    /// catalog replaces this once that session attaches.
    pub commands: Vec<pi_core::protocol::SlashCommand>,
    pub commands_loading: bool,
    pub commands_error: Option<String>,
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
            shown: None,
            listed: HashMap::new(),
            keys: HashMap::new(),
            deleted: HashSet::new(),
            next_id: 1,
            model,
            thinking: String::new(),
            models: Vec::new(),
            models_loading: false,
            models_error: None,
            commands: Vec::new(),
            commands_loading: false,
            commands_error: None,
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
        if !self.helper.can_list_sessions() {
            return;
        }
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

    pub fn refresh_models(&mut self) {
        if self.models_loading {
            return;
        }
        self.models_loading = true;
        self.models_error = None;
        let (connection, helper, sender) = (
            self.connection.clone(),
            self.helper.clone(),
            self.sender.clone(),
        );
        ssh::spawn(async move {
            let models = remote::models(&connection, &helper)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send((None, Update::ModelCatalog(models))).await;
        });
    }

    pub fn refresh_commands(&mut self) {
        if self.commands_loading {
            return;
        }
        self.commands_loading = true;
        self.commands_error = None;
        let (connection, helper, sender) = (
            self.connection.clone(),
            self.helper.clone(),
            self.sender.clone(),
        );
        ssh::spawn(async move {
            let commands = remote::commands(&connection, &helper)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send((None, Update::CommandCatalog(commands))).await;
        });
    }

    fn attach(&self, id: SessionId, target: SshTarget, generation: u64) {
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
                        .send((Some(id), Update::Ended(generation, format!("{error:#}"))))
                        .await;
                    return;
                }
            };
            // The watch holds the only sender: dropping it ends the channel.
            let crate::ssh::Pipe {
                records,
                input,
                ended,
            } = pipe;
            if sender
                .send((Some(id), Update::Opened(generation, input)))
                .await
                .is_err()
            {
                return;
            }
            while let Ok(record) = records.recv().await {
                if sender
                    .send((Some(id), Update::Record(generation, record)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let reason = ended.recv().await.unwrap_or_default();
            let _ = sender
                .send((Some(id), Update::Ended(generation, reason)))
                .await;
        });
    }

    /// Starts watching a listed session; nothing happens if already watched.
    pub fn watch(&mut self, id: SessionId) {
        if !self.helper.can_attach() {
            return;
        }
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
        watch.reset_transport();
        let generation = watch.generation;
        self.watches.insert(id, watch);
        self.attach(id, target, generation);
    }

    pub fn is_watched(&self, id: SessionId) -> bool {
        self.watches.contains_key(&id)
    }

    pub fn target(&self, id: SessionId) -> Option<SshTarget> {
        self.watches
            .get(&id)
            .map(|watch| watch.target.clone())
            .or_else(|| remote::listed_target(&self.host, self.listed.get(&id)?).ok())
    }

    pub fn remove(&mut self, id: SessionId) {
        if let Some(target) = self.target(id) {
            self.deleted.insert(target.key);
        }
        self.watches.remove(&id);
        self.listed.remove(&id);
    }

    /// Picks the connection back up after it dropped: watched sessions attach again.
    pub fn resume(&mut self, connection: Connection, helper: Helper) {
        self.connection = connection;
        self.helper = helper;
        self.models_loading = false;
        self.commands_loading = false;
        let watched: Vec<SessionId> = self.watches.keys().copied().collect();
        if !self.helper.can_attach() {
            let reason = self
                .helper
                .require_attach("opening sessions")
                .unwrap_err()
                .to_string();
            for watch in self.watches.values_mut() {
                watch.reset_transport();
                watch.ended = Some(reason.clone());
            }
            return;
        }
        for id in watched {
            if let Some(watch) = self.watches.get_mut(&id) {
                watch.ended = Some("Reconnecting".into());
            }
            self.watch(id);
        }
        if self.helper.can_list_sessions() {
            self.refresh();
        }
        self.refresh_models();
        self.refresh_commands();
    }

    /// A new durable session in `folder`, starting with `prompt`.
    pub fn start(&mut self, folder: &str, prompt: Prompt) -> Result<SessionId, String> {
        self.helper
            .require_attach("starting a session")
            .map_err(|error| error.to_string())?;
        if !prompt.images.is_empty() && !self.helper.images {
            return Err(
                "Update the computer's helper to send images. Your draft has been kept.".into(),
            );
        }
        if !prompt.images.is_empty()
            && choose_model(&self.models, &self.model)
                .is_some_and(|model| !model.input.iter().any(|kind| kind == "image"))
        {
            return Err("This model does not accept images. Choose a vision-capable model.".into());
        }
        let target =
            remote::new_target(&self.host, folder).map_err(|error| format!("{error:#}"))?;
        let id = self.id_for(&target.key.clone());
        let mut watch = Watch::new(target.clone());
        watch.outbox.push(prompt.into());
        let generation = watch.generation;
        self.watches.insert(id, watch);
        self.attach(id, target, generation);
        Ok(id)
    }

    /// Sends one transport request. Prompt admission fields are already frozen
    /// in `record`; this adds only the per-connection correlation identity.
    fn send(watch: &mut Watch, mut record: Value, request: Request) -> bool {
        let Some(command) = record["type"].as_str().map(str::to_owned) else {
            return false;
        };
        let id = crate::prompt::request_id();
        record["id"] = json!(id);
        if let Some(input) = &watch.input
            && input.try_send(record).is_ok()
        {
            watch.sent.insert(id, SentRequest { command, request });
            true
        } else {
            false
        }
    }

    fn flush(watch: &mut Watch) {
        if !watch.ready
            || watch.input.is_none()
            || watch.sent.values().any(|sent| {
                matches!(
                    &sent.request,
                    Request::SetModel
                        | Request::SetThinking(_)
                        | Request::InitialThinkingLevels(_)
                        | Request::Stop
                        | Request::Compact
                )
            })
        {
            return;
        }
        let pending: Vec<usize> = watch
            .outbox
            .iter()
            .enumerate()
            .filter(|(_, pending)| {
                !watch.sent.values().any(|sent| {
                    matches!(&sent.request, Request::Prompt(request_id) if *request_id == pending.prompt.request_id)
                })
            })
            .map(|(index, _)| index)
            .collect();
        for index in pending {
            let admission = watch.outbox[index].admission.unwrap_or_else(|| Admission {
                streaming_behavior: (watch.pi.busy()
                    || watch
                        .sent
                        .values()
                        .any(|sent| matches!(&sent.request, Request::Prompt(_))))
                .then_some("steer"),
            });
            let request_id = watch.outbox[index].prompt.request_id.clone();
            let record = watch.outbox[index].record(admission);
            if Self::send(watch, record, Request::Prompt(request_id)) {
                // Freeze only once the channel accepted the record. Before that,
                // no admission was attempted and current session state may still
                // choose its behavior.
                watch.outbox[index].admission.get_or_insert(admission);
            }
        }
    }

    fn compact_watch(watch: &mut Watch, custom_instructions: Option<String>) -> Result<(), String> {
        if !watch.current || !watch.ready {
            return Err("Wait for the session to finish connecting before compacting.".into());
        }
        if watch
            .sent
            .values()
            .any(|sent| matches!(&sent.request, Request::Compact))
        {
            return Err("This session is already compacting.".into());
        }
        if watch.pi.busy()
            || !watch.outbox.is_empty()
            || watch
                .sent
                .values()
                .any(|sent| matches!(&sent.request, Request::Prompt(_)))
        {
            return Err("Stop the session and clear queued prompts before compacting.".into());
        }
        let mut record = json!({"type":"compact"});
        if let Some(instructions) = custom_instructions {
            record["customInstructions"] = json!(instructions);
        }
        if Self::send(watch, record, Request::Compact) {
            Ok(())
        } else {
            Err("The session disconnected before compaction could start.".into())
        }
    }

    /// Manually compacts a session. Unlike a prompt this action is never queued
    /// or retried as text; the computer owns its long-running operation.
    pub fn compact(
        &mut self,
        id: SessionId,
        custom_instructions: Option<String>,
    ) -> Result<(), String> {
        self.helper
            .require_attach("compacting a session")
            .map_err(|error| error.to_string())?;
        self.watch(id);
        if self.commands(id).is_none() {
            return Err("Wait for this session's commands to finish loading.".into());
        }
        if !self.supports_compact(id) {
            return Err("This session is still using an older helper. Start a new session to use manual compaction; your draft has been kept.".into());
        }
        let watch = self
            .watches
            .get_mut(&id)
            .ok_or("This session is no longer available.")?;
        Self::compact_watch(watch, custom_instructions)
    }

    /// A prompt for a session: it runs now, or steers the current run.
    pub fn prompt(&mut self, id: SessionId, prompt: Prompt) -> Result<(), String> {
        self.helper
            .require_attach("sending a prompt")
            .map_err(|error| error.to_string())?;
        self.watch(id);
        if self.watches.get(&id).is_some_and(|watch| {
            watch
                .sent
                .values()
                .any(|sent| matches!(&sent.request, Request::Compact))
        }) {
            return Err("Wait for compaction to finish before sending another prompt.".into());
        }
        if !prompt.images.is_empty() {
            if let Some(watch) = self.watches.get(&id)
                && watch.target.backend == RemoteBackend::Durable
            {
                let pi_core::session::BackendInfo::Found(info) = &watch.pi.backend else {
                    return Err(
                        "Wait for the session to report image support before sending.".into(),
                    );
                };
                if !info["features"].as_array().is_some_and(|features| {
                    features.iter().any(|feature| feature == "image_prompts")
                }) {
                    return Err("This session is running an older helper without image support. Start a new session with the updated helper; your draft is kept.".into());
                }
            }
            let model = self
                .current_model(id)
                .ok_or("Wait for the session's model to load before sending images.")?;
            if !model.input.iter().any(|kind| kind == "image") {
                return Err(
                    "This model does not accept images. Choose a vision-capable model.".into(),
                );
            }
        }
        if let Some(watch) = self.watches.get_mut(&id) {
            watch.outbox.push(prompt.into());
            Self::flush(watch);
            Ok(())
        } else {
            Err("This session is no longer available.".into())
        }
    }

    pub fn model_settings(&self, id: SessionId) -> Option<(String, String)> {
        let watch = self.watches.get(&id)?;
        let model = watch.pi.state.model.as_ref()?;
        let thinking = match watch.pi.state.thinking_level.as_str() {
            "off" | "" => "Off",
            "minimal" => "Minimal",
            "low" => "Low",
            "medium" => "Medium",
            "high" => "High",
            "xhigh" => "Max",
            other => other,
        };
        Some((
            model.name.clone().unwrap_or_else(|| model.id.clone()),
            thinking.to_owned(),
        ))
    }

    pub fn current_model(&self, id: SessionId) -> Option<&pi_core::protocol::Model> {
        self.watches.get(&id)?.pi.state.model.as_ref()
    }

    /// The unshortened computer path used by file-channel operations.
    pub fn session_cwd(&self, id: SessionId) -> Option<&str> {
        self.watches
            .get(&id)
            .map(|watch| watch.target.cwd.as_str())
            .or_else(|| self.listed.get(&id).map(|listed| listed.cwd.as_str()))
    }

    pub fn thinking_levels(&self, id: SessionId) -> Option<Vec<String>> {
        let watch = self.watches.get(&id)?;
        if watch.pi.thinking_levels.is_empty() {
            return self
                .current_model(id)
                .filter(|model| model.reasoning == Some(false))
                .map(|_| vec!["Off".into()]);
        }
        Some(
            watch
                .pi
                .thinking_levels
                .iter()
                .map(|level| thinking_label(level).to_owned())
                .collect(),
        )
    }

    pub fn commands(&self, id: SessionId) -> Option<&[pi_core::protocol::SlashCommand]> {
        let watch = self.watches.get(&id)?;
        watch
            .pi
            .commands_loaded
            .then_some(watch.pi.commands.as_slice())
    }

    pub fn supports_compact(&self, id: SessionId) -> bool {
        let Some(watch) = self.watches.get(&id) else {
            return false;
        };
        if watch.target.backend == RemoteBackend::Pi {
            return true;
        }
        let pi_core::session::BackendInfo::Found(info) = &watch.pi.backend else {
            return false;
        };
        info["commands"]
            .as_array()
            .is_some_and(|commands| commands.iter().any(|command| command == "compact"))
            || info["features"].as_array().is_some_and(|features| {
                features
                    .iter()
                    .any(|feature| feature == "manual_compaction")
            })
    }

    pub fn commands_for_path(&self, path: &str) -> Option<&[pi_core::protocol::SlashCommand]> {
        self.watches
            .values()
            .find(|watch| watch.target.cwd == path && watch.pi.commands_loaded)
            .map(|watch| watch.pi.commands.as_slice())
            .or_else(|| (!self.commands_loading).then_some(self.commands.as_slice()))
    }

    pub fn is_stopping(&self, id: SessionId) -> bool {
        self.watches.get(&id).is_some_and(|watch| {
            watch
                .sent
                .values()
                .any(|sent| matches!(&sent.request, Request::Stop))
        })
    }

    pub fn failed_prompts(&self, id: SessionId) -> &[FailedPrompt] {
        self.watches
            .get(&id)
            .map_or(&[], |watch| watch.failed.as_slice())
    }

    pub fn recover_prompt(&mut self, id: SessionId, request_id: &str) -> Option<Prompt> {
        Self::recover_failed(self.watches.get_mut(&id)?, request_id)
    }

    fn recover_failed(watch: &mut Watch, request_id: &str) -> Option<Prompt> {
        let index = watch
            .failed
            .iter()
            .position(|failed| failed.prompt.request_id == request_id)?;
        let prompt = watch.failed.remove(index).prompt;
        // Recovery restores editable content. If it is sent again, it is a new
        // admission even when the person makes no edit.
        Some(Prompt::new(prompt.message, prompt.images))
    }

    fn hold_failed(watch: &mut Watch, error: &str) {
        for pending in std::mem::take(&mut watch.outbox) {
            if pending.admission.is_some()
                || watch.sent.values().any(|sent| {
                    matches!(&sent.request, Request::Prompt(id) if *id == pending.prompt.request_id)
                })
            {
                // This payload may already be admitted. Wait for its receipt;
                // never offer a second admission after a configuration error.
                watch.outbox.push(pending);
            } else {
                watch.failed.push(FailedPrompt {
                    prompt: pending.prompt,
                    error: error.to_owned(),
                });
            }
        }
        if watch.pi.messages.is_empty() {
            watch.pi.error = Some(error.to_owned());
        }
    }

    pub fn set_model(
        &mut self,
        id: SessionId,
        provider: &str,
        model_id: &str,
    ) -> Result<(), String> {
        if !self
            .models
            .iter()
            .any(|model| model.provider == provider && model.id == model_id)
        {
            return Err("This model is no longer available on the computer.".into());
        }
        let watch = self
            .watches
            .get_mut(&id)
            .ok_or("Wait for this session to connect.")?;
        Self::configure(
            watch,
            json!({"type":"set_model", "provider":provider, "modelId":model_id}),
            Request::SetModel,
        )
    }

    pub fn set_thinking(&mut self, id: SessionId, shown: &str) -> Result<(), String> {
        let level = thinking_level(shown).ok_or("Unknown thinking level.")?;
        let watch = self
            .watches
            .get_mut(&id)
            .ok_or("Wait for this session to connect.")?;
        Self::configure(
            watch,
            json!({"type":"set_thinking_level", "level":level}),
            Request::SetThinking(level.into()),
        )
    }

    fn configure(watch: &mut Watch, record: Value, request: Request) -> Result<(), String> {
        if !watch.current || watch.input.as_ref().is_none_or(|input| input.is_closed()) {
            return Err("Reconnect to the computer before changing this session's model.".into());
        }
        Self::send(watch, record, request);
        Ok(())
    }

    pub fn stop(&mut self, id: SessionId) -> Result<(), String> {
        let watch = self
            .watches
            .get_mut(&id)
            .ok_or("Wait for this session to connect before stopping it.")?;
        if !watch.current || watch.input.as_ref().is_none_or(|input| input.is_closed()) {
            return Err("Reconnect to the computer before stopping this session.".into());
        }
        if watch
            .sent
            .values()
            .any(|sent| matches!(&sent.request, Request::Stop))
        {
            return Ok(());
        }
        watch.outbox.clear();
        // Stop must not immediately run a previously queued prompt.
        Self::send(watch, json!({"type": "clear_queue"}), Request::Other);
        Self::send(watch, json!({"type": "abort"}), Request::Stop);
        Ok(())
    }

    /// Cancels one durable submission without touching any other payload.
    pub fn unqueue(&mut self, id: SessionId, index: usize) -> Result<(), String> {
        let Some(watch) = self.watches.get_mut(&id) else {
            return Err("Session is not connected.".into());
        };
        Self::cancel_queued(watch, index)
    }

    fn cancel_queued(watch: &mut Watch, index: usize) -> Result<(), String> {
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
                if watch.outbox[at].admission.is_some()
                    || watch.sent.values().any(|sent| matches!(&sent.request, Request::Prompt(request_id) if *request_id == watch.outbox[at].prompt.request_id))
                {
                    return Err("This prompt is being delivered. Wait for its queue confirmation, then remove it.".into());
                }
                watch.outbox.remove(at);
            }
            return Ok(());
        }
        let submission = watch.pi.queued_submissions.get(index).ok_or("This helper cannot remove one queued prompt safely. Update the helper or use Stop to clear all queued work.")?.clone();
        Self::configure(
            watch,
            json!({"type":"cancel_submission", "submissionId":submission}),
            Request::Other,
        )
    }

    pub fn answer(&mut self, id: SessionId, answer: Answer) {
        if !self.helper.can_attach() {
            return;
        }
        let Some(watch) = self.watches.get_mut(&id) else {
            return;
        };
        let Some(input) = watch.input.as_ref().filter(|input| !input.is_closed()) else {
            return;
        };
        if watch.dialogs.is_empty() {
            return;
        }
        let request = watch.dialogs.remove(0);
        let _ = input.try_send(projection::answer(&request, answer));
    }

    /// Applies an update; returns the session it changed and any problem.
    pub fn apply(
        &mut self,
        id: Option<SessionId>,
        update: Update,
    ) -> (Vec<SessionId>, Option<Problem>) {
        let Some(id) = id else {
            if let Update::CommandCatalog(commands) = update {
                self.commands_loading = false;
                match commands {
                    Ok(commands) => {
                        self.commands = commands;
                        self.commands_error = None;
                    }
                    Err(error) => self.commands_error = Some(error),
                }
                return (Vec::new(), None);
            }
            if let Update::ModelCatalog(models) = update {
                self.models_loading = false;
                match models {
                    Ok(models) => {
                        self.models = models;
                        self.models_error = None;
                    }
                    Err(error) => self.models_error = Some(error),
                }
                return (Vec::new(), None);
            }
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
        if !self
            .watches
            .get(&id)
            .is_some_and(|watch| watch.accepts(&update))
        {
            return (Vec::new(), None);
        }
        if let Update::Record(_, record) = &update
            && record["type"] == "remote_session_deleted"
            && self
                .target(id)
                .is_some_and(|target| record["key"] == target.key)
        {
            self.remove(id);
            return (vec![id], None);
        }
        let Some(watch) = self.watches.get_mut(&id) else {
            return (Vec::new(), None);
        };
        let mut problem = None;
        match update {
            Update::Opened(_, input) => watch.input = Some(input),
            Update::Ended(_, reason) => {
                watch.input = None;
                watch.ready = false;
                if watch.ended.is_none() {
                    log::info!("Session {} closed: {reason}", watch.target.key);
                    watch.ended = Some(reason);
                }
            }
            Update::Record(_, record) => {
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
            Update::Listed(_) | Update::ModelCatalog(_) | Update::CommandCatalog(_) => {}
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
        // A response belongs to one active request only when both its transport
        // identity and command match. A late duplicate is ignored, while a
        // mismatched response cannot consume the legitimate request behind it.
        let response = if kind == "response" {
            let id = record["id"].as_str()?;
            let command = record["command"].as_str()?;
            let sent = watch.sent.get(id)?;
            if sent.command != command {
                log::warn!(
                    "Ignoring response {id} for {command}; expected {}",
                    sent.command
                );
                return None;
            }
            Some(watch.sent.remove(id)?.request)
        } else {
            None
        };
        if let Err(error) = watch.pi.apply(record) {
            log::warn!("Skipping a record Pi's session model refused: {error:#}");
        }
        Self::fetch_images(watch);
        if kind == "remote_snapshot" {
            Self::fetch_subagent(watch);
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
                // Opening an existing session must populate its prompt controls
                // without replacing the session's model with the phone's default.
                Self::send(
                    watch,
                    json!({"type":"get_available_models"}),
                    Request::AvailableModels,
                );
                Self::send(
                    watch,
                    json!({"type":"get_available_thinking_levels"}),
                    Request::Quiet,
                );
            }
            // The computer owns slash-command semantics. Refresh on every
            // attachment while retaining the last catalog in the snapshot.
            Self::send(watch, json!({"type":"get_commands"}), Request::Commands);
        }
        let request = response?;
        let failed = record["success"] != true;
        let error = record["error"]
            .as_str()
            .unwrap_or("Pi refused it")
            .to_owned();
        match request {
            Request::Prompt(request_id) => {
                let Some(index) = watch
                    .outbox
                    .iter()
                    .position(|queued| queued.prompt.request_id == request_id)
                else {
                    // A prompt response is terminal. Duplicate or late responses
                    // must not turn an already accepted prompt into a failure.
                    return None;
                };
                let prompt = watch.outbox.remove(index).prompt;
                watch.sent.retain(
                    |_, sent| !matches!(&sent.request, Request::Prompt(id) if *id == request_id),
                );
                if failed {
                    watch.failed.push(FailedPrompt {
                        prompt,
                        error: error.clone(),
                    });
                    if watch.pi.messages.is_empty() {
                        watch.pi.error = Some(error.clone());
                    }
                    Some(error)
                } else {
                    None
                }
            }
            Request::Models | Request::AvailableModels => {
                let initial = matches!(request, Request::Models);
                if failed {
                    if initial {
                        Self::hold_failed(watch, &error);
                    }
                    return Some(error);
                }
                if let Ok(available) = serde_json::from_value::<Vec<pi_core::protocol::Model>>(
                    record["data"]["models"].clone(),
                ) {
                    *models = available;
                }
                if !initial {
                    return None;
                }
                let Some(model) = choose_model(models, wanted) else {
                    let text =
                        "No model is set up for durable sessions on the computer.".to_owned();
                    Self::hold_failed(watch, &text);
                    return Some(text);
                };
                let record =
                    json!({"type": "set_model", "provider": model.provider, "modelId": model.id});
                Self::send(watch, record, Request::SetModel);
                if let Some(level) = thinking_level(thinking) {
                    Self::send(
                        watch,
                        json!({"type": "get_available_thinking_levels"}),
                        Request::InitialThinkingLevels(level.into()),
                    );
                }
                None
            }
            Request::SetModel => {
                watch.ready = !failed;
                if failed {
                    Self::hold_failed(watch, &error);
                }
                if !failed {
                    watch.pi.thinking_levels.clear();
                    // New sessions already requested this as part of their startup
                    // transaction; existing sessions refresh for the new model.
                    if !watch
                        .sent
                        .values()
                        .any(|sent| matches!(&sent.request, Request::InitialThinkingLevels(_)))
                    {
                        Self::send(
                            watch,
                            json!({"type":"get_available_thinking_levels"}),
                            Request::Quiet,
                        );
                    }
                    Self::flush(watch);
                }
                failed.then_some(error)
            }
            Request::SetThinking(level) => {
                if !failed {
                    watch.ready = true;
                    watch.pi.state.thinking_level = level;
                    Self::flush(watch);
                } else {
                    watch.ready = false;
                    Self::hold_failed(watch, &error);
                }
                failed.then_some(error)
            }
            Request::InitialThinkingLevels(wanted) => {
                if failed || watch.pi.thinking_levels.is_empty() {
                    watch.ready = false;
                    let error = if failed {
                        error
                    } else {
                        "The computer did not report thinking levels for this model.".into()
                    };
                    Self::hold_failed(watch, &error);
                    return Some(error);
                }
                let levels = &watch.pi.thinking_levels;
                let supported = ["off", "minimal", "low", "medium", "high", "xhigh"];
                let wanted_index = supported
                    .iter()
                    .position(|level| *level == wanted)
                    .unwrap_or(0);
                let level = supported[..=wanted_index]
                    .iter()
                    .rev()
                    .find(|level| levels.iter().any(|supported| supported == **level))
                    .map(|level| (*level).to_owned())
                    .unwrap_or_else(|| levels[0].clone());
                Self::send(
                    watch,
                    json!({"type":"set_thinking_level", "level":level}),
                    Request::SetThinking(level),
                );
                None
            }
            Request::Commands => {
                if failed {
                    // An older backend may not expose commands. Treat that as
                    // an authoritative empty catalog without breaking prompts.
                    watch.pi.commands.clear();
                    watch.pi.commands_loaded = true;
                }
                None
            }
            Request::Stop => {
                if !failed {
                    Self::send(watch, json!({"type":"get_state"}), Request::Quiet);
                }
                failed.then_some(error)
            }
            Request::Compact => failed.then_some(error),
            Request::Image(id) => {
                use base64::Engine as _;
                let image = &record["data"]["image"];
                let fetched = if failed {
                    Err(error)
                } else {
                    image["data"]
                        .as_str()
                        .and_then(|data| {
                            base64::engine::general_purpose::STANDARD.decode(data).ok()
                        })
                        .map(|bytes| ToolImageBytes {
                            mime: image["mimeType"].as_str().unwrap_or("image/png").into(),
                            bytes: std::sync::Arc::new(bytes),
                        })
                        .ok_or_else(|| "The computer sent an unreadable image".to_owned())
                };
                watch.images.insert(id, Some(fetched));
                None
            }
            Request::Subagent(conversation) => {
                let fetched = if failed {
                    Err(error)
                } else {
                    pi_core::subagent::session(&record["data"], watch.target.cwd.clone().into())
                        .map_err(|error| error.to_string())
                };
                watch.subagents.insert(conversation, fetched);
                Self::fetch_images(watch);
                None
            }
            Request::Quiet => None,
            Request::Other => failed.then_some(error),
        }
    }

    /// Takes in a listing; returns the sessions it named.
    fn listed(&mut self, listed: Vec<Listed>) -> Vec<SessionId> {
        let mut ids = Vec::new();
        for session in listed {
            if self.deleted.contains(&session.key) {
                continue;
            }
            let id = self.id_for(&session.key);
            // A helper daemon outlives its run; only a run at work is followed.
            let working = session.running && session.busy;
            self.listed.insert(id, session);
            if working && !self.is_watched(id) {
                self.watch(id);
            }
            ids.push(id);
        }
        self.release_idle();
        ids
    }

    /// The session on screen now, if any: others that are idle let go of
    /// their channel.
    pub fn show(&mut self, id: Option<SessionId>) {
        if self.shown == id {
            return;
        }
        self.shown = id;
        if self.release_idle() {
            // What the list shows of them comes from the listing again.
            self.refresh();
        }
    }

    /// Stops watching sessions with nothing going on that aren't on screen.
    /// Each watch holds an SSH channel, and the computer allows a few per
    /// connection (OpenSSH's MaxSessions, 10 by default). Returns whether
    /// any went.
    fn release_idle(&mut self) -> bool {
        let idle: Vec<SessionId> = self
            .watches
            .iter()
            .filter(|(id, watch)| Some(**id) != self.shown && watch.idle())
            // The list shows it from the listing once it lets go.
            .filter(|(id, _)| self.listed.contains_key(id))
            .map(|(id, _)| *id)
            .collect();
        for id in &idle {
            self.watches.remove(id);
        }
        !idle.is_empty()
    }

    /// A tool image fetched from the computer: `None` while on the way.
    pub fn image(&self, id: SessionId, key: &str) -> Option<&Result<ToolImageBytes, String>> {
        self.watches.get(&id)?.images.get(key)?.as_ref()
    }

    /// A subagent's own session, once fetched: see [`Self::follow_subagent`].
    pub fn subagent(
        &self,
        id: SessionId,
        conversation: &str,
    ) -> Option<&Result<pi_core::session::Session, String>> {
        self.watches.get(&id)?.subagents.get(conversation)
    }

    /// Keeps a subagent's messages fresh while its screen is open; `None` stops.
    pub fn follow_subagent(&mut self, id: SessionId, conversation: Option<String>) {
        let Some(watch) = self.watches.get_mut(&id) else {
            return;
        };
        if watch.following == conversation {
            return;
        }
        watch.following = conversation;
        Self::fetch_subagent(watch);
    }

    /// Asks for the followed subagent, unless a request is on the way.
    fn fetch_subagent(watch: &mut Watch) {
        let Some(conversation) = watch.following.clone() else {
            return;
        };
        if watch.input.is_none()
            || watch
                .sent
                .values()
                .any(|sent| matches!(&sent.request, Request::Subagent(_)))
        {
            return;
        }
        Self::send(
            watch,
            json!({"type": "get_subagent", "conversationId": conversation}),
            Request::Subagent(conversation),
        );
    }

    /// Stops one subagent; the session carries on, and Pi is told.
    pub fn stop_subagent(&mut self, id: SessionId, conversation: &str) -> Result<(), String> {
        let watch = self
            .watches
            .get_mut(&id)
            .filter(|watch| {
                watch.current && watch.input.as_ref().is_some_and(|input| !input.is_closed())
            })
            .ok_or("Reconnect to the computer before stopping this subagent.")?;
        Self::send(
            watch,
            json!({"type": "stop_subagent", "conversationId": conversation}),
            Request::Other,
        );
        Ok(())
    }

    /// Asks once for each image a tool returned that the stream left out.
    fn fetch_images(watch: &mut Watch) {
        if watch.input.is_none() {
            return;
        }
        // The session's own, and those of the subagent whose screen is open.
        let followed = watch.following.as_ref().and_then(|conversation| {
            Some((
                conversation,
                watch.subagents.get(conversation)?.as_ref().ok()?,
            ))
        });
        let wanted: Vec<(String, Option<String>)> = watch
            .pi
            .tools
            .iter()
            .map(|tool| (tool, None))
            .chain(followed.into_iter().flat_map(|(conversation, pi)| {
                pi.tools.iter().map(move |tool| (tool, Some(conversation)))
            }))
            .flat_map(|(tool, conversation)| {
                tool.images.iter().map(move |block| (block, conversation))
            })
            .filter_map(|(block, conversation)| {
                Some((block["imageId"].as_str()?.to_owned(), conversation.cloned()))
            })
            .filter(|(id, _)| !watch.images.contains_key(id))
            .collect();
        for (id, conversation) in wanted {
            if watch.images.contains_key(&id) {
                continue;
            }
            watch.images.insert(id.clone(), None);
            let mut request = json!({"type": "get_image", "imageId": id});
            if let Some(conversation) = conversation {
                request["conversationId"] = conversation.into();
            }
            Self::send(watch, request, Request::Image(id));
        }
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
                    outbox: &watch
                        .outbox
                        .iter()
                        .map(|pending| pending.prompt.label())
                        .collect::<Vec<_>>(),
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
        "Minimal" => "minimal",
        "Low" => "low",
        "Medium" => "medium",
        "High" => "high",
        "Max" => "xhigh",
        _ => return None,
    })
}

fn thinking_label(level: &str) -> &str {
    match level {
        "off" => "Off",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Max",
        other => other,
    }
}

/// The model the settings name, by name or id, else the first offered.
fn choose_model<'a>(
    models: &'a [pi_core::protocol::Model],
    wanted: &str,
) -> Option<&'a pi_core::protocol::Model> {
    if wanted.contains('/') {
        // Explicit selections must never silently choose a different provider.
        return models
            .iter()
            .find(|model| format!("{}/{}", model.provider, model.id) == wanted);
    }
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

    fn admission(mut record: Value) -> Value {
        record.as_object_mut().unwrap().remove("id");
        record
    }

    #[test]
    fn obsolete_pipes_cannot_overwrite_new_state_or_close_the_replacement_connection() {
        let (mut watch, _) = watch();
        let old = watch.generation;
        watch.pi.commands_loaded = true;
        watch.reset_transport();
        assert!(!watch.pi.commands_loaded);
        assert!(!watch.accepts(&Update::Record(
            old,
            json!({"type":"remote_session_deleted"})
        )));
        assert!(!watch.accepts(&Update::Ended(old, "old socket closed".into())));
        assert!(watch.accepts(&Update::Record(
            watch.generation,
            json!({"type":"remote_snapshot"})
        )));
    }

    #[test]
    fn reconnect_retries_the_exact_idle_admission_and_ignores_late_collision_errors() {
        let (mut watch, first_pipe) = watch();
        watch.ready = true;
        let prompt = Prompt::new(
            "same text".into(),
            vec![pi_core::protocol::ImageContent::new(
                "aW1hZ2U=".into(),
                "image/png",
            )],
        );
        watch.outbox.push(prompt.into());
        let old_generation = watch.generation;
        Live::flush(&mut watch);
        let first = first_pipe.try_recv().unwrap();
        assert!(first.get("streamingBehavior").is_none());
        assert!(watch.outbox[0].admission.is_some());
        Live::flush(&mut watch);
        assert!(
            first_pipe.try_recv().is_err(),
            "no duplicate request on one pipe"
        );

        watch.reset_transport();
        watch.pi.run = pi_core::session::RunState::Running;
        let (input, next_pipe) = async_channel::unbounded();
        watch.input = Some(input);
        watch.ready = true;
        Live::flush(&mut watch);
        let retry = next_pipe.try_recv().unwrap();
        assert_ne!(first["id"], retry["id"]);
        assert_eq!(admission(first.clone()), admission(retry.clone()));
        assert!(
            retry.get("streamingBehavior").is_none(),
            "an idle reject admission must not become steer after reconnect"
        );

        let delivered =
            json!({"type":"response","id":retry["id"],"command":"prompt","success":true});
        assert!(Live::record(&mut watch, &delivered, "", "", &mut Vec::new()).is_none());
        let duplicate = json!({"type":"response","id":retry["id"],"command":"prompt","success":false,"error":"requestId already belongs to a different prompt"});
        assert!(Live::record(&mut watch, &duplicate, "", "", &mut Vec::new()).is_none());
        let obsolete = Update::Record(
            old_generation,
            json!({"type":"response","id":first["id"],"command":"prompt","success":false,"error":"requestId already belongs to a different prompt"}),
        );
        assert!(!watch.accepts(&obsolete));
        assert!(watch.outbox.is_empty());
        assert!(watch.failed.is_empty(), "no failed card after acceptance");
        assert!(
            watch.pi.error.is_none(),
            "no stale session error after acceptance"
        );
        assert!(
            next_pipe.try_recv().is_err(),
            "no duplicate prompt submitted"
        );
    }

    #[test]
    fn a_genuine_collision_rejection_remains_recoverable_with_images_and_a_new_identity() {
        let (mut watch, sent) = watch();
        watch.ready = true;
        let prompt = Prompt::new(
            "try this".into(),
            vec![pi_core::protocol::ImageContent::new(
                "aW1hZ2U=".into(),
                "image/png",
            )],
        );
        let original_id = prompt.request_id.clone();
        watch.outbox.push(prompt.clone().into());
        Live::flush(&mut watch);
        let request = sent.try_recv().unwrap();
        let error = json!({"type":"response","id":request["id"],"command":"prompt","success":false,"error":"requestId already belongs to a different prompt"});
        assert_eq!(
            Live::record(&mut watch, &error, "", "", &mut Vec::new()).as_deref(),
            Some("requestId already belongs to a different prompt")
        );
        assert!(watch.outbox.is_empty());
        assert_eq!(watch.failed[0].prompt, prompt);
        let recovered = Live::recover_failed(&mut watch, &original_id).unwrap();
        assert_ne!(recovered.request_id, original_id);
        assert_eq!(recovered.message, prompt.message);
        assert_eq!(recovered.images, prompt.images);
        assert!(watch.failed.is_empty());
    }

    #[test]
    fn a_mismatched_response_cannot_consume_or_fail_the_prompt_request() {
        let (mut watch, sent) = watch();
        watch.ready = true;
        watch.outbox.push("keep waiting".into());
        Live::flush(&mut watch);
        let request = sent.try_recv().unwrap();

        let wrong = json!({"type":"response","id":request["id"],"command":"set_model","success":false,"error":"wrong response"});
        assert!(Live::record(&mut watch, &wrong, "", "", &mut Vec::new()).is_none());
        assert_eq!(watch.outbox.len(), 1);
        assert!(watch.failed.is_empty());
        assert!(watch.pi.error.is_none());
        assert!(watch.sent.contains_key(request["id"].as_str().unwrap()));

        let refusal = json!({"type":"response","id":request["id"],"command":"prompt","success":false,"error":"real refusal"});
        assert_eq!(
            Live::record(&mut watch, &refusal, "", "", &mut Vec::new()).as_deref(),
            Some("real refusal")
        );
        assert!(watch.outbox.is_empty());
        assert_eq!(watch.failed.len(), 1);
    }

    #[test]
    fn the_first_terminal_prompt_response_wins_in_either_order() {
        let (mut watch, sent) = watch();
        watch.ready = true;
        watch.outbox.push("reject me".into());
        Live::flush(&mut watch);
        let request = sent.try_recv().unwrap();
        let failed = json!({"type":"response","id":request["id"],"command":"prompt","success":false,"error":"provider refused"});
        assert!(Live::record(&mut watch, &failed, "", "", &mut Vec::new()).is_some());
        let accepted =
            json!({"type":"response","id":request["id"],"command":"prompt","success":true});
        assert!(Live::record(&mut watch, &accepted, "", "", &mut Vec::new()).is_none());
        assert_eq!(
            watch.failed.len(),
            1,
            "late success cannot erase a real failure"
        );
        assert_eq!(watch.failed[0].error, "provider refused");
    }

    #[test]
    fn steering_mode_remains_stable_when_the_session_becomes_idle() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        watch.outbox.push("Change direction now".into());
        Live::flush(&mut watch);
        let first = sent.try_recv().unwrap();
        assert_eq!(first["streamingBehavior"], "steer");

        watch.reset_transport();
        watch.pi.run = pi_core::session::RunState::Idle;
        let (input, retried) = async_channel::unbounded();
        watch.input = Some(input);
        watch.ready = true;
        Live::flush(&mut watch);
        let retry = retried.try_recv().unwrap();
        assert_eq!(admission(first), admission(retry));
    }

    #[test]
    fn removing_one_queued_prompt_never_rebuilds_other_prompts_or_claims_an_inflight_cancel() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        watch.pi.follow_up = vec!["first".into(), "second with an image".into()];
        watch.pi.queued_submissions = vec!["41".into(), "42".into()];
        Live::cancel_queued(&mut watch, 0).unwrap();
        let request = sent.try_recv().unwrap();
        assert_eq!(request["type"], "cancel_submission");
        assert_eq!(request["submissionId"], "41");
        assert!(sent.try_recv().is_err());
        watch.outbox.push("delivering".into());
        Live::flush(&mut watch);
        assert_eq!(sent.try_recv().unwrap()["type"], "prompt");
        assert!(
            Live::cancel_queued(&mut watch, 2)
                .unwrap_err()
                .contains("being delivered")
        );
        assert_eq!(watch.outbox.len(), 1);
    }

    #[test]
    fn compact_sends_an_action_without_admitting_prompt_text() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        let messages = watch.pi.messages.len();

        Live::compact_watch(&mut watch, Some("keep API names".into())).unwrap();
        let request = sent.try_recv().unwrap();
        assert_eq!(
            admission(request.clone()),
            json!({"type":"compact","customInstructions":"keep API names"})
        );
        assert!(watch.outbox.is_empty());
        assert_eq!(watch.pi.messages.len(), messages);
        assert!(sent.try_recv().is_err());
        assert_eq!(
            Live::compact_watch(&mut watch, None).unwrap_err(),
            "This session is already compacting."
        );

        let response = json!({"type":"response","id":request["id"],"command":"compact","success":true,"data":{}});
        assert!(Live::record(&mut watch, &response, "", "", &mut Vec::new()).is_none());
        Live::compact_watch(&mut watch, None).unwrap();
        assert_eq!(
            admission(sent.try_recv().unwrap()),
            json!({"type":"compact"})
        );
    }

    #[test]
    fn compact_waits_for_a_current_ready_session_and_reports_backend_failure() {
        let (mut watch, sent) = watch();
        assert!(
            Live::compact_watch(&mut watch, None)
                .unwrap_err()
                .contains("finish connecting")
        );
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        assert!(
            Live::compact_watch(&mut watch, None)
                .unwrap_err()
                .contains("Stop the session")
        );
        assert!(sent.try_recv().is_err());
        watch.pi.run = pi_core::session::RunState::Idle;
        Live::compact_watch(&mut watch, None).unwrap();
        let request = sent.try_recv().unwrap();
        let failed = json!({"type":"response","id":request["id"],"command":"compact","success":false,"error":"Nothing to compact (session too small)"});
        assert_eq!(
            Live::record(&mut watch, &failed, "", "", &mut Vec::new()).as_deref(),
            Some("Nothing to compact (session too small)")
        );
        assert!(watch.outbox.is_empty());
        assert!(watch.failed.is_empty());
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
        assert_eq!(
            choose_model(&models, "anthropic/claude-opus-5-5")
                .unwrap()
                .id,
            "claude-opus-5-5"
        );
        assert!(choose_model(&models, "missing/claude-opus-5-5").is_none());
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
        let commands = sent.try_recv().unwrap();
        assert_eq!(commands["type"], "get_commands");
        assert!(sent.try_recv().is_err(), "no prompt before a model");
        Live::record(
            &mut watch,
            &json!({"type":"response","id":commands["id"],"command":"get_commands","success":true,"data":{"commands":[{"name":"review","description":"Review changes","source":"prompt","sourceInfo":null}]}}),
            "Opus 5.5",
            "",
            &mut models,
        );
        assert!(watch.pi.commands_loaded);
        assert_eq!(watch.pi.commands[0].name, "review");
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
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        let prompt = sent.try_recv().unwrap();
        assert_eq!(prompt["type"], "prompt");
        assert_eq!(prompt["message"], "Explain this project");
        assert_ne!(
            prompt["requestId"], prompt["id"],
            "admission identity survives transport retries"
        );
        assert!(prompt.get("streamingBehavior").is_none());
        let refused = json!({"type":"response","id":prompt["id"],"command":"prompt","success":false,"error":"No API key"});
        assert_eq!(
            Live::record(&mut watch, &refused, "Opus 5.5", "", &mut models).as_deref(),
            Some("No API key")
        );
        assert!(watch.outbox.is_empty());
        assert_eq!(watch.failed.len(), 1);
        assert_eq!(watch.failed[0].prompt.message, "Explain this project");
        assert_eq!(watch.pi.error.as_deref(), Some("No API key"));
    }

    #[test]
    fn an_existing_session_loads_choices_without_changing_its_model() {
        let (mut watch, sent) = watch();
        let mut pi = Pi::new("/Users/nick/repos/pi".into());
        pi.state.model = Some(model("host", "existing", "Existing model"));
        let mut models = Vec::new();
        Live::record(
            &mut watch,
            &json!({"type":"remote_snapshot","data":pi}),
            "Another model",
            "High",
            &mut models,
        );
        let ask = sent.try_recv().unwrap();
        assert_eq!(ask["type"], "get_available_models");
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        assert_eq!(sent.try_recv().unwrap()["type"], "get_commands");
        Live::record(
            &mut watch,
            &json!({"type":"response","id":ask["id"],"command":"get_available_models","success":true,"data":{"models":[{"provider":"host","id":"other"}]}}),
            "Another model",
            "High",
            &mut models,
        );
        assert!(
            sent.try_recv().is_err(),
            "opening a session must not reconfigure it"
        );
        assert_eq!(watch.pi.state.model.as_ref().unwrap().id, "existing");
        assert_eq!(models.len(), 1);
    }

    #[test]
    fn prompts_wait_for_model_confirmation_and_failed_changes_do_not_send() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"next"}),
            Request::SetModel,
        )
        .unwrap();
        let set = sent.try_recv().unwrap();
        watch.outbox.push("Use the chosen model".into());
        Live::flush(&mut watch);
        assert!(sent.try_recv().is_err());
        let mut models = Vec::new();
        assert!(Live::record(&mut watch, &json!({"type":"response","id":set["id"],"command":"set_model","success":false,"error":"Unavailable"}), "", "", &mut models).is_some());
        assert!(sent.try_recv().is_err());
        assert!(watch.outbox.is_empty());
        assert_eq!(
            watch.failed.len(),
            1,
            "keep the rejected prompt for explicit recovery"
        );
        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"next"}),
            Request::SetModel,
        )
        .unwrap();
        let set = sent.try_recv().unwrap();
        // Explicit user recovery resubmits the full payload under a new key,
        // never a label or the rejected admission identity.
        let rejected_id = watch.failed[0].prompt.request_id.clone();
        let recovered = Live::recover_failed(&mut watch, &rejected_id).unwrap();
        assert_ne!(recovered.request_id, rejected_id);
        watch.outbox.push(recovered.into());
        Live::record(
            &mut watch,
            &json!({"type":"response","id":set["id"],"command":"set_model","success":true,"data":{"provider":"host","id":"next"}}),
            "",
            "",
            &mut models,
        );
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        assert_eq!(sent.try_recv().unwrap()["type"], "prompt");
        assert_eq!(watch.pi.state.model.as_ref().unwrap().id, "next");
    }

    #[test]
    fn model_confirmation_cannot_mutate_or_fail_an_attempted_admission() {
        let (mut watch, first_pipe) = watch();
        watch.current = true;
        watch.ready = true;
        watch.outbox.push("Keep the original admission".into());
        Live::flush(&mut watch);
        let first = first_pipe.try_recv().unwrap();
        assert!(first.get("streamingBehavior").is_none());

        watch.reset_transport();
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        let (input, next_pipe) = async_channel::unbounded();
        watch.input = Some(input);
        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"next"}),
            Request::SetModel,
        )
        .unwrap();
        let set = next_pipe.try_recv().unwrap();
        Live::flush(&mut watch);
        assert!(
            next_pipe.try_recv().is_err(),
            "model confirmation blocks retry"
        );

        let mut models = Vec::new();
        assert!(Live::record(
            &mut watch,
            &json!({"type":"response","id":set["id"],"command":"set_model","success":false,"error":"Unavailable"}),
            "",
            "",
            &mut models,
        )
        .is_some());
        assert_eq!(watch.outbox.len(), 1, "attempted prompt stays pending");
        assert!(watch.failed.is_empty(), "no recoverable failure card yet");
        assert!(watch.outbox[0].admission.is_some());

        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"next"}),
            Request::SetModel,
        )
        .unwrap();
        let set = next_pipe.try_recv().unwrap();
        Live::record(
            &mut watch,
            &json!({"type":"response","id":set["id"],"command":"set_model","success":true,"data":{"provider":"host","id":"next"}}),
            "",
            "",
            &mut models,
        );
        assert_eq!(
            next_pipe.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        let retry = next_pipe.try_recv().unwrap();
        assert_eq!(admission(first), admission(retry));
    }

    /// Runs against the disposable SSH endpoint owned by scripts/test_ssh.py.
    /// Refuses a real account's keys or project before writing anything.
    #[test]
    #[ignore = "needs an SSH server; set PI_ANDROID_TEST_SSH, PI_ANDROID_TEST_KEYS and PI_ANDROID_TEST_PROJECT"]
    fn a_durable_session_runs_over_ssh() {
        use crate::model::State;
        use std::time::{Duration, Instant};
        let variable = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
        let address = ssh::Address::parse(&variable("PI_ANDROID_TEST_SSH")).unwrap();
        let project = variable("PI_ANDROID_TEST_PROJECT");
        let keys = variable("PI_ANDROID_TEST_KEYS");
        let keys = std::path::Path::new(&keys).canonicalize().unwrap();
        let directory = keys.parent().unwrap();
        assert!(
            directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pi-ssh-regression-"),
            "Use scripts/test_ssh.py; never an account's real authorized_keys"
        );
        assert_eq!(
            std::path::Path::new(&project).canonicalize().unwrap(),
            directory.join("project")
        );
        assert!(
            std::fs::read(&keys).unwrap().is_empty(),
            "The isolated authorized_keys must start empty"
        );
        let identity = ssh::Identity::load_or_create(&directory.join("client/id_ed25519")).unwrap();
        let forced = variable("PI_ANDROID_TEST_FORCE_COMMAND")
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        std::fs::write(
            &keys,
            format!("restrict,command=\"{forced}\" {}\n", identity.public_line()),
        )
        .unwrap();

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
            assert!(remote::sessions(&connection, &helper).await.unwrap().is_empty());
            assert!(remote::models(&connection, &helper).await.unwrap().iter().any(|model| model.provider == "faux"));
            let directory = remote::directories(&connection, &helper, &project, false).await.unwrap();
            assert_eq!(directory.path, project);
            assert!(remote::sessions(&connection, &helper).await.unwrap().is_empty(), "catalog and folder browsing must not create a session");
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

            live.prompt(id, "and again".into()).unwrap();
            let again = until(&mut live, &|s| s.state == State::Done && s.turns.len() == 2).await;
            assert_eq!(
                again.turns[1].summary.as_ref().unwrap().headline,
                "Finished: and again"
            );

            let image = pi_core::protocol::ImageContent::new(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==".into(),
                "image/png",
            );
            live.prompt(id, Prompt::new(String::new(), vec![image])).unwrap();
            let with_image = until(&mut live, &|s| s.state == State::Done && s.turns.len() == 3).await;
            assert_eq!(with_image.turns[2].summary.as_ref().unwrap().headline, "Received 1 image(s): image/png:70");

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
                helper.clone(),
                address.to_string(),
                "faux".into(),
            );
            let (ids, _) = other.apply(None, Update::Listed(Ok(listed)));
            let twin = ids
                .into_iter()
                .find(|twin| other.listed[twin].key == key)
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            while other.session(twin).unwrap().turns.len() < 3 {
                assert!(Instant::now() < deadline);
                let update = tokio::time::timeout(Duration::from_secs(30), other.updates.recv())
                    .await
                    .unwrap()
                    .unwrap();
                other.apply(update.0, update.1);
            }
            assert_eq!(other.session(twin).unwrap().title, "hello from the phone");
            let target = live.watches[&id].target.clone();
            remote::delete(&connection, &helper, &target).await.unwrap();
            assert!(remote::sessions(&connection, &helper).await.unwrap().is_empty());
            assert!(std::path::Path::new(&project).is_dir(), "deletion keeps the project folder");
        });
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
