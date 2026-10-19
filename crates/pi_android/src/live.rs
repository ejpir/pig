//! The computer the phone is connected to: the sessions its helper lists, the
//! ones the phone watches, and what the phone sends them.
//!
//! Running sessions are watched as soon as they are listed, so the list stays
//! current and notifications fire; others are attached when opened. New
//! sessions are durable: their state lives in a database on the computer, so
//! nothing is lost when the phone goes away.

use crate::{
    model::{Answer, QueueKey, SessionId},
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
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

mod transport_diagnostics;

use self::transport_diagnostics::TransportDiagnostics;
pub(crate) use self::transport_diagnostics::{
    TRANSPORT_DIAGNOSTIC_MAX_BYTES, TRANSPORT_DIAGNOSTIC_MAX_ENTRIES, TransportDiagnostic,
    TransportDirection,
};

// Never reuse a pipe identity, even if a released watch is recreated.
fn transport_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Something that happened on the computer, for the store to apply.
pub enum Update {
    /// A session's pipe opened; commands can go in.
    Opened(u64, async_channel::Sender<Value>),
    Record(u64, Value),
    /// The pipe closed, with the reason.
    Ended(u64, String),
    /// A fresh listing of the computer's sessions.
    Listed(Result<Vec<Listed>, String>),
    ModelCatalog(u64, Result<Vec<pi_core::protocol::Model>, String>),
    CommandCatalog(
        u64,
        Option<String>,
        Result<Vec<pi_core::protocol::SlashCommand>, String>,
    ),
}

/// What a request was for, to act on its response.
enum Request {
    Prompt(String),
    /// Cancellation of one exact durable submission, correlated by transport id.
    CancelSubmission(String),
    Models,
    AvailableModels,
    ImageState,
    Commands,
    SetModel,
    SetThinking(String),
    SetSwarmEnabled(bool),
    SetCompactionThreshold(f64),
    InitialThinkingLevels(String),
    Stop,
    Compact,
    /// A tool image explicitly demanded by a visible card.
    Image(ImageRequest),
    /// A subagent's messages, by its conversation's id.
    Subagent(String),
    /// Direct input to one exact child conversation and, for steering, run.
    SubagentPrompt {
        conversation: String,
        request_id: String,
    },
    /// Its failure doesn't matter: a model without thinking levels refuses one.
    Quiet,
    Other,
}

struct SentRequest {
    command: String,
    request: Request,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImageRequest {
    id: String,
    conversation: Option<String>,
}

/// Large tool-image responses are deliberately serialized in a small window.
/// Visibility can expose several cards in one frame, but must never recreate
/// the old lifetime-history network burst.
const MAX_IMAGE_REQUESTS_IN_FLIGHT: usize = 2;

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
    /// An explicit mode chosen before transport admission. Unlike `admission`,
    /// this does not bypass the capability checks for a pending image.
    requested_behavior: Option<&'static str>,
    admission: Option<Admission>,
    /// User turns visible when an idle/new-turn admission first left the phone.
    /// This distinguishes its later snapshot from an older identical prompt.
    user_turns_before: Option<usize>,
    /// Hidden immediately after its queued-row X is tapped. If delivery is
    /// already in flight, its acknowledgement supplies the durable id to cancel.
    cancelled: bool,
    submission_id: Option<String>,
}

struct AcceptedTurn {
    text: String,
    user_turns_before: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SubagentInputTarget {
    Steer { run_id: String },
    FollowUp,
}

struct PendingSubagentInput {
    request_id: String,
    message: String,
    target: SubagentInputTarget,
}

impl PendingSubagentInput {
    fn record(&self, conversation: &str) -> Value {
        let mut record = json!({
            "type":"prompt_subagent",
            "conversationId":conversation,
            "message":self.message,
            "requestId":self.request_id,
            "streamingBehavior":match &self.target {
                SubagentInputTarget::Steer { .. } => "steer",
                SubagentInputTarget::FollowUp => "followUp",
            },
        });
        if let SubagentInputTarget::Steer { run_id } = &self.target {
            record["expectedRunId"] = json!(run_id);
        }
        record
    }
}

impl PendingPrompt {
    fn new(prompt: Prompt) -> Self {
        Self::with_behavior(prompt, None)
    }

    fn follow_up(prompt: Prompt) -> Self {
        Self::with_behavior(prompt, Some("followUp"))
    }

    fn with_behavior(prompt: Prompt, streaming_behavior: Option<&'static str>) -> Self {
        Self {
            prompt,
            // An explicit follow-up must stay a follow-up even if the run
            // settles before the transport accepts it.
            requested_behavior: streaming_behavior,
            admission: None,
            user_turns_before: None,
            cancelled: false,
            submission_id: None,
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

/// Only confirmed, session-local authority can permit a first image admission.
enum ImageReadiness {
    Ready,
    Waiting(&'static str),
    Rejected(&'static str),
}

struct Watch {
    generation: u64,
    target: SshTarget,
    pi: Pi,
    input: Option<async_channel::Sender<Value>>,
    /// Whether Pi's state has arrived since the pipe opened.
    current: bool,
    /// A valid authoritative snapshot remains displayable across pipe resets.
    cached_snapshot: bool,
    /// Phone receipt time of successfully applied session data, not command acks.
    observed_at: Option<u64>,
    /// Failed pipes retry independently of SSH reconnection and session listings.
    retry_at: Option<Instant>,
    retry_failures: u32,
    /// Whether commands may go: the state arrived and a model is chosen.
    ready: bool,
    /// At most one demand-driven metadata refresh per waiting cohort/selection.
    image_state_requested: bool,
    image_backend_requested: bool,
    image_catalog_requested: bool,
    /// Prompts typed on the phone that Pi hasn't taken yet. Each item owns its
    /// immutable admission mode after the first transport accepts it.
    outbox: Vec<PendingPrompt>,
    /// An idle admission acknowledged before its user-message snapshot. Keep
    /// projecting it as the new turn so stale tools cannot leak into it.
    accepted_turn: Option<AcceptedTurn>,
    /// Submission-ID tombstones survive transport reset. `true` means the
    /// cancel succeeded: hide until a subsequent authoritative queue omits it.
    cancellations: HashMap<String, bool>,
    /// Receipts keep a rendered local request addressable after the outbox row
    /// is replaced by a durable snapshot. Never correlate by payload text.
    queue_receipts: HashMap<String, String>,
    /// Reverse lookup keeps queue rendering independent of total past admissions.
    queue_request_ids: HashMap<String, String>,
    /// Rejected payloads remain recoverable, including images. They never retry
    /// automatically or make the session look as though it is still running.
    failed: Vec<FailedPrompt>,
    sent: HashMap<String, SentRequest>,
    /// Pi's open questions, oldest first.
    dialogs: Vec<Value>,
    /// Tool images fetched from the computer, by id: absent until a visible
    /// card asks for one, `None` while queued or on the way.
    images: HashMap<String, Option<Result<ToolImageBytes, String>>>,
    /// Visible image demands waiting behind the bounded in-flight window.
    image_queue: VecDeque<ImageRequest>,
    /// The subagent on screen, by its conversation's id: fetched again as
    /// its parent reports progress, one request at a time.
    following: Option<String>,
    /// Parent updates/target changes coalesced while a child read is in flight.
    subagent_refresh_pending: bool,
    /// Subagents fetched from the computer: absent until asked for.
    subagents: HashMap<String, Result<pi_core::session::Session, String>>,
    /// Monotonic child-read revisions; rendering projects each response once.
    subagent_revisions: HashMap<String, u64>,
    /// Exact live run identity from the same child read; `None` means idle.
    subagent_runs: HashMap<String, Option<String>>,
    /// Finished child conversations the backend explicitly allows resuming.
    resumable_subagents: HashSet<String>,
    /// Direct child inputs survive pipe replacement with frozen target semantics.
    subagent_outbox: HashMap<String, PendingSubagentInput>,
    /// Successfully admitted text, consumed by that child's independent composer.
    accepted_subagent_inputs: HashMap<String, String>,
    /// A running followed child is refreshed on this bounded polling cadence.
    subagent_refresh_at: Option<Instant>,
    /// Local-only, sanitized and bounded descriptions of this session pipe.
    diagnostics: TransportDiagnostics,
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

fn user_turns(pi: &Pi) -> usize {
    pi.messages
        .iter()
        .filter(|message| message["role"] == "user")
        .count()
}

fn milliseconds_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn seconds_now() -> u64 {
    milliseconds_now() / 1_000
}

impl Watch {
    fn compacting(&self) -> bool {
        self.pi.run == pi_core::session::RunState::Compacting
            || self
                .sent
                .values()
                .any(|sent| matches!(&sent.request, Request::Compact))
    }

    /// An acknowledged idle admission is already running on the computer even
    /// when its busy snapshot has not reached this phone yet.
    fn effective_busy(&self) -> bool {
        self.pi.busy()
            || self.accepted_turn.is_some()
            || self
                .sent
                .values()
                .any(|sent| matches!(&sent.request, Request::Prompt(_)))
    }

    fn starts_turn(&self, pending: &PendingPrompt) -> bool {
        pending
            .admission
            .map_or(!self.effective_busy(), |admission| {
                admission.streaming_behavior.is_none()
            })
    }

    fn queue_row_visible(&self, index: usize) -> bool {
        self.pi
            .queued_submissions
            .get(index)
            .is_none_or(|id| !self.cancellations.contains_key(id))
    }

    fn queue_row_key(&self, index: usize) -> Option<QueueKey> {
        let submission = self
            .pi
            .queued_submissions
            .get(index)
            .filter(|id| !id.is_empty())?;
        Some(match self.queue_request_ids.get(submission) {
            Some(request_id) => QueueKey::Local(request_id.clone()),
            None => QueueKey::Durable(submission.clone()),
        })
    }

    fn visible_outbox(&self) -> impl Iterator<Item = (usize, &PendingPrompt)> + Clone {
        self.outbox.iter().enumerate().filter(|(_, pending)| {
            !pending.cancelled
                && pending
                    .submission_id
                    .as_ref()
                    .is_none_or(|id| !self.pi.queued_submissions.contains(id))
        })
    }

    fn projected_outbox(&self) -> Vec<projection::PendingPrompt> {
        self.accepted_turn
            .iter()
            .map(|accepted| projection::PendingPrompt {
                text: accepted.text.clone(),
                key: None,
                starts_turn: true,
            })
            .chain(
                self.visible_outbox()
                    .map(|(_, pending)| projection::PendingPrompt {
                        text: pending.prompt.label(),
                        key: Some(QueueKey::Local(pending.prompt.request_id.clone())),
                        starts_turn: self.starts_turn(pending),
                    }),
            )
            .collect()
    }

    /// Nothing to follow: Pi and its subagents rest, and nothing waits on
    /// the phone or the computer.
    fn idle(&self) -> bool {
        self.current
            && !self.pi.busy()
            && self.outbox.is_empty()
            && self.accepted_turn.is_none()
            && self.cancellations.is_empty()
            && self.failed.is_empty()
            && self.sent.is_empty()
            && self.image_queue.is_empty()
            && self.subagent_outbox.is_empty()
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
            generation: transport_generation(),
            target,
            pi,
            input: None,
            current: false,
            cached_snapshot: false,
            observed_at: None,
            retry_at: None,
            retry_failures: 0,
            ready: false,
            image_state_requested: false,
            image_backend_requested: false,
            image_catalog_requested: false,
            outbox: Vec::new(),
            accepted_turn: None,
            cancellations: HashMap::new(),
            queue_receipts: HashMap::new(),
            queue_request_ids: HashMap::new(),
            failed: Vec::new(),
            sent: HashMap::new(),
            dialogs: Vec::new(),
            images: HashMap::new(),
            image_queue: VecDeque::new(),
            following: None,
            subagent_refresh_pending: false,
            subagents: HashMap::new(),
            subagent_revisions: HashMap::new(),
            subagent_runs: HashMap::new(),
            resumable_subagents: HashSet::new(),
            subagent_outbox: HashMap::new(),
            accepted_subagent_inputs: HashMap::new(),
            subagent_refresh_at: None,
            diagnostics: TransportDiagnostics::default(),
            ended: None,
            since: seconds_now(),
        }
    }

    fn reset_transport(&mut self) {
        self.generation = transport_generation();
        self.retry_at = None;
        self.input = None;
        self.current = false;
        self.ready = false;
        self.image_state_requested = false;
        self.image_backend_requested = false;
        self.image_catalog_requested = false;
        self.ended = None;
        // Session-local commands can change while disconnected. Keep their
        // display cache, but do not use it for action shadowing until refreshed.
        self.pi.commands_loaded = false;
        // Requests on the old pipe cannot be acknowledged by the new pipe.
        // Prompts remain in the outbox with their stable admission identities.
        // Requeue only image demands that were actually in flight; unlike the
        // old full-history scan, reconnect never invents new image work.
        let retrying_images: Vec<_> = self
            .sent
            .values()
            .filter_map(|sent| match &sent.request {
                Request::Image(request) => Some(request.clone()),
                _ => None,
            })
            .collect();
        self.sent.clear();
        for request in retrying_images.into_iter().rev() {
            if self.images.get(&request.id).is_some_and(Option::is_none)
                && !self
                    .image_queue
                    .iter()
                    .any(|queued| queued.id == request.id)
            {
                self.image_queue.push_front(request);
            }
        }
        self.subagent_refresh_pending = self.following.is_some();
        self.subagent_refresh_at = None;
    }

    fn end_transport(&mut self, reason: String, now: Instant) {
        // Retire the generation immediately, including any queued old records.
        self.reset_transport();
        self.ended = Some(reason);
        // 1, 2, 4, 8, 16, then at most one attempt every 32 seconds.
        let delay = Duration::from_secs(1 << self.retry_failures.min(5));
        self.retry_failures = self.retry_failures.saturating_add(1);
        self.retry_at = Some(now + delay);
    }

    fn recover(
        &mut self,
        now: Instant,
        ssh_healthy: bool,
        can_attach: bool,
    ) -> Option<(SshTarget, u64)> {
        if !ssh_healthy || !can_attach || self.retry_at.is_none_or(|due| now < due) {
            return None;
        }
        // Clearing the deadline before dispatch also prevents in-flight repeats.
        self.reset_transport();
        Some((self.target.clone(), self.generation))
    }

    fn project(&self, id: SessionId, folder: String) -> Option<crate::model::Session> {
        if !self.cached_snapshot
            && !self.current
            && self.outbox.is_empty()
            && self.cancellations.is_empty()
            && self.pi.error.is_none()
        {
            return None;
        }
        Some(projection::project_queue_keys(
            &self.pi,
            projection::Facts {
                id,
                cwd: &self.target.cwd,
                folder,
                question: self.dialogs.first().and_then(projection::question),
                outbox: &self.projected_outbox(),
                key: &self.target.key,
            },
            |index| self.queue_row_visible(index),
            |index| self.queue_row_key(index),
        ))
    }

    fn image_readiness(&self) -> ImageReadiness {
        use ImageReadiness::*;
        if !self.current || !self.ready || self.input.as_ref().is_none_or(|input| input.is_closed())
        {
            return Waiting("Wait for the session to finish connecting before sending images.");
        }
        if self
            .sent
            .values()
            .any(|sent| matches!(sent.request, Request::SetModel))
        {
            return Waiting("Wait for the selected model to be confirmed before sending images.");
        }
        if self.target.backend == RemoteBackend::Durable {
            match &self.pi.backend {
                pi_core::session::BackendInfo::Unknown => {
                    return Waiting("Wait for the session to report image support before sending.");
                }
                pi_core::session::BackendInfo::Found(info)
                    if info["features"].as_array().is_some_and(|features| {
                        features.iter().any(|feature| feature == "image_prompts")
                    }) => {}
                _ => {
                    return Rejected(
                        "This session is running an older helper without image support. Start a new session with the updated helper; your draft is kept.",
                    );
                }
            }
        }
        let Some(model) = self.pi.state.model.as_ref() else {
            return Waiting("Wait for the session's model to load before sending images.");
        };
        if model.input.is_empty() {
            return Waiting(
                "This model's image capabilities have not loaded yet. Refresh models or select a vision-capable model; your draft is kept.",
            );
        }
        if !model.input.iter().any(|kind| kind == "image") {
            return Rejected("This model does not accept images. Choose a vision-capable model.");
        }
        Ready
    }

    #[cfg(test)]
    fn require_images(&self) -> Result<(), String> {
        match self.image_readiness() {
            ImageReadiness::Ready => Ok(()),
            ImageReadiness::Waiting(error) | ImageReadiness::Rejected(error) => Err(error.into()),
        }
    }

    fn accepts(&self, update: &Update) -> bool {
        match update {
            Update::Opened(generation, _)
            | Update::Record(generation, _)
            | Update::Ended(generation, _) => *generation == self.generation,
            Update::Listed(_) | Update::ModelCatalog(_, _) | Update::CommandCatalog(_, _, _) => {
                false
            }
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
    /// Model catalog replies belong to the connection that requested them.
    catalog_generation: u64,
    /// Command discovery also changes when the selected project changes, even
    /// while the SSH connection stays the same.
    command_generation: u64,
    /// Commands discovered for this exact project. A matching attached session
    /// remains authoritative because it can refresh resources after `/reload`.
    pub commands: Vec<pi_core::protocol::SlashCommand>,
    pub commands_path: Option<String>,
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
            catalog_generation: 1,
            command_generation: 1,
            commands: Vec::new(),
            commands_path: None,
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
        let generation = self.catalog_generation;
        let (connection, helper, sender) = (
            self.connection.clone(),
            self.helper.clone(),
            self.sender.clone(),
        );
        ssh::spawn(async move {
            let models = remote::models(&connection, &helper)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = sender
                .send((None, Update::ModelCatalog(generation, models)))
                .await;
        });
    }

    pub fn refresh_commands(&mut self, path: Option<String>) {
        self.command_generation = self.command_generation.wrapping_add(1);
        self.commands_loading = true;
        self.commands_error = None;
        let generation = self.command_generation;
        let (connection, helper, sender) = (
            self.connection.clone(),
            self.helper.clone(),
            self.sender.clone(),
        );
        ssh::spawn(async move {
            let commands = remote::commands(&connection, &helper, path.as_deref())
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = sender
                .send((None, Update::CommandCatalog(generation, path, commands)))
                .await;
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
        // An existing watch owns its retry deadline; navigation cannot bypass
        // backoff or create a second attachment while one is in flight.
        if self.watches.contains_key(&id) {
            return;
        }
        let Some(target) = self
            .listed
            .get(&id)
            .and_then(|listed| remote::listed_target(&self.host, listed).ok())
        else {
            return;
        };
        let mut watch = Watch::new(target.clone());
        watch.reset_transport();
        let generation = watch.generation;
        self.watches.insert(id, watch);
        self.attach(id, target, generation);
    }

    /// Called by the app ticker even when no session is currently busy.
    pub fn poll_recovery(&mut self, now: Instant) {
        let healthy = !self.connection.is_closed();
        let can_attach = self.helper.can_attach();
        let attaches: Vec<_> = self
            .watches
            .iter_mut()
            .filter_map(|(id, watch)| {
                watch
                    .recover(now, healthy, can_attach)
                    .map(|plan| (*id, plan))
            })
            .collect();
        for (id, (target, generation)) in attaches {
            self.attach(id, target, generation);
        }
    }

    pub fn is_watched(&self, id: SessionId) -> bool {
        self.watches.contains_key(&id)
    }

    pub(crate) fn has_snapshot(&self, id: SessionId) -> bool {
        self.watches
            .get(&id)
            .is_some_and(|watch| watch.cached_snapshot)
    }

    pub(crate) fn listed_session(&self, id: SessionId) -> Option<crate::model::Session> {
        let listed = self.listed.get(&id)?;
        Some(projection::listed(
            id,
            listed,
            self.helper.short(&listed.cwd),
        ))
    }

    /// No authoritative snapshot is available for display yet. This is not
    /// command readiness: a cached session stays visible while reconnecting.
    pub fn is_loading(&self, id: SessionId) -> bool {
        self.watches
            .get(&id)
            .is_some_and(|watch| !watch.cached_snapshot && !watch.current && watch.ended.is_none())
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
        self.catalog_generation += 1;
        self.command_generation += 1;
        self.models.clear();
        self.models_error = None;
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
            let watch = self.watches.get_mut(&id).unwrap();
            watch.reset_transport();
            let (target, generation) = (watch.target.clone(), watch.generation);
            self.attach(id, target, generation);
        }
        if self.helper.can_list_sessions() {
            self.refresh();
        }
        self.refresh_models();
    }

    /// Attach a ledger's existing durable session without sending any prompt,
    /// configuring a model, or generating a new admission identity.
    pub fn open_scheduled(&mut self, key: &str, cwd: &str) -> Result<SessionId, String> {
        self.helper
            .require_attach("opening a scheduled session")
            .map_err(|e| e.to_string())?;
        let target = SshTarget {
            host: self.host.clone(),
            cwd: cwd.to_owned(),
            key: key.to_owned(),
            backend: pi_core::ssh::RemoteBackend::Durable,
            session_file: None,
        };
        target.validate().map_err(|e| e.to_string())?;
        let id = self.id_for(key);
        if let std::collections::hash_map::Entry::Vacant(entry) = self.watches.entry(id) {
            let mut watch = Watch::new(target.clone());
            watch.reset_transport();
            let generation = watch.generation;
            entry.insert(watch);
            self.attach(id, target, generation);
        }
        Ok(id)
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
        // Startup selects from the session's freshly fetched catalog. A cached
        // global catalog must not reject an image against a different model.
        // The backend validates image support for the confirmed selection.
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
        let diagnostic = TransportDiagnostic::from_record(TransportDirection::Outbound, &record);
        if let Some(input) = &watch.input
            && input.try_send(record).is_ok()
        {
            watch.diagnostics.push(diagnostic);
            watch.sent.insert(id, SentRequest { command, request });
            true
        } else {
            false
        }
    }

    fn flush_cancellations(watch: &mut Watch) {
        if !watch.ready || watch.input.is_none() {
            return;
        }
        let cancellations: Vec<String> = watch
            .cancellations
            .iter()
            .filter(|(_, acknowledged)| !**acknowledged)
            .filter(|(submission, _)| {
                !watch.sent.values().any(|sent| {
                    matches!(&sent.request, Request::CancelSubmission(id) if id == *submission)
                })
            })
            .map(|(submission, _)| submission.clone())
            .collect();
        for submission in cancellations {
            let mut record = json!({
                "type":"cancel_submission",
                "submissionId":submission,
            });
            if let Some(pending) = watch.outbox.iter().find(|pending| {
                pending.cancelled && pending.submission_id.as_ref() == Some(&submission)
            }) {
                record["requestId"] = json!(pending.prompt.request_id);
            }
            Self::send(watch, record, Request::CancelSubmission(submission));
        }
    }

    fn flush_image_requests(watch: &mut Watch) {
        if watch.input.as_ref().is_none_or(|input| input.is_closed()) {
            return;
        }
        let mut in_flight = watch
            .sent
            .values()
            .filter(|sent| matches!(&sent.request, Request::Image(_)))
            .count();
        while in_flight < MAX_IMAGE_REQUESTS_IN_FLIGHT {
            let Some(request) = watch.image_queue.front().cloned() else {
                break;
            };
            let mut record = json!({"type": "get_image", "imageId": request.id.clone()});
            if let Some(conversation) = &request.conversation {
                record["conversationId"] = conversation.clone().into();
            }
            if !Self::send(watch, record, Request::Image(request)) {
                break;
            }
            watch.image_queue.pop_front();
            in_flight += 1;
        }
    }

    fn flush_subagent_inputs(watch: &mut Watch) {
        if !watch.current
            || watch.input.as_ref().is_none_or(|input| input.is_closed())
            || !Self::supports_subagent_input_watch(watch)
        {
            return;
        }
        let pending: Vec<_> = watch
            .subagent_outbox
            .iter()
            .filter(|(_, pending)| {
                !watch.sent.values().any(|sent| {
                    matches!(
                        &sent.request,
                        Request::SubagentPrompt { request_id, .. }
                            if request_id == &pending.request_id
                    )
                })
            })
            .map(|(conversation, pending)| {
                (
                    conversation.clone(),
                    pending.request_id.clone(),
                    pending.record(conversation),
                )
            })
            .collect();
        for (conversation, request_id, record) in pending {
            Self::send(
                watch,
                record,
                Request::SubagentPrompt {
                    conversation,
                    request_id,
                },
            );
        }
    }

    fn flush(watch: &mut Watch) {
        Self::flush_image_requests(watch);
        Self::flush_subagent_inputs(watch);
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
        Self::flush_cancellations(watch);
        let mut index = 0;
        while index < watch.outbox.len() {
            let pending = &watch.outbox[index];
            if pending.submission_id.is_some() || watch.sent.values().any(|sent| {
                matches!(&sent.request, Request::Prompt(id) if *id == pending.prompt.request_id)
            }) {
                index += 1;
                continue;
            }
            // An attempted admission is immutable: even a new text-only model
            // must not stop its deduplicated retry of the original bytes/mode/id.
            if pending.admission.is_none() && !pending.prompt.images.is_empty() {
                match watch.image_readiness() {
                    ImageReadiness::Ready => {}
                    ImageReadiness::Waiting(reason) => {
                        log::debug!("Queued image admission waiting: {reason}");
                        Self::request_image_metadata(watch);
                        break; // A held image must not be overtaken by later text.
                    }
                    ImageReadiness::Rejected(error) => {
                        let pending = watch.outbox.remove(index);
                        watch.failed.push(FailedPrompt {
                            prompt: pending.prompt,
                            error: error.into(),
                        });
                        if watch.pi.messages.is_empty() {
                            watch.pi.error = Some(error.into());
                        }
                        continue;
                    }
                }
            }
            let admission = watch.outbox[index].admission.unwrap_or_else(|| Admission {
                streaming_behavior: watch.outbox[index]
                    .requested_behavior
                    .or_else(|| watch.effective_busy().then_some("steer")),
            });
            let request_id = watch.outbox[index].prompt.request_id.clone();
            let record = watch.outbox[index].record(admission);
            let user_turns_before = admission
                .streaming_behavior
                .is_none()
                .then(|| user_turns(&watch.pi));
            if !Self::send(watch, record, Request::Prompt(request_id)) {
                break;
            }
            // Freeze only once the channel accepted the record.
            watch.outbox[index].admission.get_or_insert(admission);
            if let Some(user_turns_before) = user_turns_before {
                watch.outbox[index]
                    .user_turns_before
                    .get_or_insert(user_turns_before);
            }
            index += 1;
        }
    }

    fn request_image_metadata(watch: &mut Watch) {
        if !watch.current
            || !watch.ready
            || watch.input.as_ref().is_none_or(|input| input.is_closed())
        {
            return;
        }
        if watch.pi.state.model.is_none() && !watch.image_state_requested {
            watch.image_state_requested =
                watch.sent.values().any(|sent| sent.command == "get_state")
                    || Self::send(watch, json!({"type":"get_state"}), Request::ImageState);
        }
        if watch.target.backend == RemoteBackend::Durable
            && matches!(watch.pi.backend, pi_core::session::BackendInfo::Unknown)
            && !watch.image_backend_requested
        {
            watch.image_backend_requested = watch
                .sent
                .values()
                .any(|sent| sent.command == "get_backend_info")
                || Self::send(watch, json!({"type":"get_backend_info"}), Request::Quiet);
        }
        if watch
            .pi
            .state
            .model
            .as_ref()
            .is_some_and(|model| model.input.is_empty())
            && !watch.image_catalog_requested
        {
            // Opening already requests a fresh session catalog. Join that read
            // rather than issuing another; never consult Live's global cache.
            watch.image_catalog_requested = watch
                .sent
                .values()
                .any(|sent| matches!(sent.request, Request::Models | Request::AvailableModels));
            if !watch.image_catalog_requested {
                watch.image_catalog_requested = Self::send(
                    watch,
                    json!({"type":"get_available_models"}),
                    Request::AvailableModels,
                );
            }
        }
    }

    fn fail_waiting_images(watch: &mut Watch, error: &str) {
        for pending in std::mem::take(&mut watch.outbox) {
            if pending.admission.is_none() && !pending.prompt.images.is_empty() {
                watch.failed.push(FailedPrompt {
                    prompt: pending.prompt,
                    error: error.into(),
                });
            } else {
                watch.outbox.push(pending);
            }
        }
    }

    fn enqueue(watch: &mut Watch, prompt: Prompt) -> Result<(), String> {
        Self::enqueue_with_mode(watch, prompt, false)
    }

    fn enqueue_with_mode(watch: &mut Watch, prompt: Prompt, follow_up: bool) -> Result<(), String> {
        if !prompt.images.is_empty()
            && let ImageReadiness::Rejected(error) = watch.image_readiness()
        {
            return Err(error.into());
        }
        if !prompt.images.is_empty()
            && !watch
                .outbox
                .iter()
                .any(|pending| pending.admission.is_none() && !pending.prompt.images.is_empty())
        {
            // Explicit submission/recovery starts a new wait, not a polling
            // loop. Any still-outstanding metadata requests are joined below.
            watch.image_state_requested = false;
            watch.image_backend_requested = false;
            watch.image_catalog_requested = false;
        }
        watch.outbox.push(if follow_up {
            PendingPrompt::follow_up(prompt)
        } else {
            prompt.into()
        });
        Self::flush(watch);
        Ok(())
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
        self.prompt_with_mode(id, prompt, false)
    }

    /// A review response remains a follow-up instead of changing an active run.
    pub fn follow_up(&mut self, id: SessionId, prompt: Prompt) -> Result<(), String> {
        self.prompt_with_mode(id, prompt, true)
    }

    fn prompt_with_mode(
        &mut self,
        id: SessionId,
        prompt: Prompt,
        follow_up: bool,
    ) -> Result<(), String> {
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
        let watch = self
            .watches
            .get_mut(&id)
            .ok_or("This session is no longer available.")?;
        if follow_up {
            Self::enqueue_with_mode(watch, prompt, true)
        } else {
            Self::enqueue(watch, prompt)
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

    fn supports_subagent_input_watch(watch: &Watch) -> bool {
        if watch.target.backend != RemoteBackend::Durable {
            return false;
        }
        let pi_core::session::BackendInfo::Found(info) = &watch.pi.backend else {
            return false;
        };
        info["commands"]
            .as_array()
            .is_some_and(|commands| commands.iter().any(|command| command == "prompt_subagent"))
            && info["features"]
                .as_array()
                .is_some_and(|features| features.iter().any(|feature| feature == "subagent_input"))
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
        (self.commands_path.as_deref() == Some(path))
            .then_some(self.commands.as_slice())
            .or_else(|| {
                self.watches
                    .values()
                    .find(|watch| watch.target.cwd == path && watch.pi.commands_loaded)
                    .map(|watch| watch.pi.commands.as_slice())
            })
    }

    pub fn is_stopping(&self, id: SessionId) -> bool {
        self.watches.get(&id).is_some_and(|watch| {
            watch
                .sent
                .values()
                .any(|sent| matches!(&sent.request, Request::Stop))
        })
    }

    /// A locally requested compaction is visible before the computer's first
    /// compaction snapshot reaches the phone.
    pub fn is_compacting(&self, id: SessionId) -> bool {
        self.watches.get(&id).is_some_and(Watch::compacting)
    }

    pub fn failed_prompts(&self, id: SessionId) -> &[FailedPrompt] {
        self.watches
            .get(&id)
            .map_or(&[], |watch| watch.failed.as_slice())
    }

    pub(crate) fn transport_diagnostics(
        &self,
        id: SessionId,
    ) -> Option<&VecDeque<TransportDiagnostic>> {
        self.watches
            .get(&id)
            .map(|watch| &watch.diagnostics.entries)
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

    fn compaction_threshold_supported(watch: &Watch) -> bool {
        watch.target.backend == RemoteBackend::Durable
            && matches!(&watch.pi.backend, pi_core::session::BackendInfo::Found(info)
                if info["commands"].as_array().is_some_and(|commands|
                    commands.iter().any(|command| command == "set_compaction_threshold")))
    }

    pub(crate) fn supports_compaction_threshold(&self, id: SessionId) -> bool {
        self.watches
            .get(&id)
            .is_some_and(Self::compaction_threshold_supported)
    }

    pub(crate) fn compaction_threshold_max(&self, id: SessionId) -> Option<f64> {
        let watch = self.watches.get(&id)?;
        let window = watch.pi.stats.context_usage.as_ref()?.context_window;
        let reserve = watch.pi.stats.compaction.as_ref()?.reserve_tokens;
        (window > 0).then(|| {
            // Round down so moving to the end cannot cross the blocking limit.
            ((window.saturating_sub(reserve) as f64 / window as f64 * 1000.).floor() / 10.).min(95.)
        })
    }

    pub(crate) fn compaction_threshold_pending(&self, id: SessionId) -> Option<f64> {
        self.watches
            .get(&id)?
            .sent
            .values()
            .find_map(|sent| match sent.request {
                Request::SetCompactionThreshold(percent) => Some(percent),
                _ => None,
            })
    }

    pub(crate) fn set_compaction_threshold(
        &mut self,
        id: SessionId,
        percent: f64,
    ) -> Result<(), String> {
        let watch = self
            .watches
            .get_mut(&id)
            .ok_or("Wait for this session to connect.")?;
        Self::set_compaction_threshold_watch(watch, percent)
    }

    fn set_compaction_threshold_watch(watch: &mut Watch, percent: f64) -> Result<(), String> {
        if !percent.is_finite() || !(10. ..=95.).contains(&percent) {
            return Err("Choose a compaction threshold between 10% and 95%.".into());
        }
        if !Self::compaction_threshold_supported(watch) {
            return Err("Update the computer helper to change the compaction threshold.".into());
        }
        if !watch.current || watch.input.as_ref().is_none_or(|input| input.is_closed()) {
            return Err("Reconnect before changing compaction. Nothing was queued.".into());
        }
        if watch
            .sent
            .values()
            .any(|sent| matches!(sent.request, Request::SetCompactionThreshold(_)))
        {
            return Err("Wait for the computer to confirm the compaction threshold.".into());
        }
        if Self::send(
            watch,
            json!({"type":"set_compaction_threshold", "percent":percent}),
            Request::SetCompactionThreshold(percent),
        ) {
            Ok(())
        } else {
            Err("The session disconnected before the threshold could be changed.".into())
        }
    }

    fn swarm_enabled_watch(watch: &Watch) -> Option<bool> {
        if watch.target.backend != RemoteBackend::Durable {
            return None;
        }
        let pi_core::session::BackendInfo::Found(info) = &watch.pi.backend else {
            return None;
        };
        info["commands"]
            .as_array()
            .filter(|commands| {
                commands
                    .iter()
                    .any(|command| command == "set_swarm_enabled")
            })
            .and(watch.pi.state.swarm_enabled)
    }

    pub(crate) fn swarm_enabled(&self, id: SessionId) -> Option<bool> {
        Self::swarm_enabled_watch(self.watches.get(&id)?)
    }

    pub(crate) fn swarm_toggle_pending(&self, id: SessionId) -> bool {
        self.watches.get(&id).is_some_and(|watch| {
            watch
                .sent
                .values()
                .any(|sent| matches!(sent.request, Request::SetSwarmEnabled(_)))
        })
    }

    pub(crate) fn set_swarm_enabled(&mut self, id: SessionId, enabled: bool) -> Result<(), String> {
        let watch = self
            .watches
            .get_mut(&id)
            .ok_or("Wait for this session to connect.")?;
        Self::set_swarm_enabled_watch(watch, enabled)
    }

    fn set_swarm_enabled_watch(watch: &mut Watch, enabled: bool) -> Result<(), String> {
        if Self::swarm_enabled_watch(watch).is_none() {
            return Err("This session needs an updated computer helper to change Swarm.".into());
        }
        if !watch.current || watch.input.as_ref().is_none_or(|input| input.is_closed()) {
            return Err("Reconnect before changing Swarm. Nothing was queued.".into());
        }
        if watch
            .sent
            .values()
            .any(|sent| matches!(sent.request, Request::SetSwarmEnabled(_)))
        {
            return Err("Wait for the computer to confirm the Swarm setting.".into());
        }
        if Self::send(
            watch,
            json!({"type":"set_swarm_enabled", "enabled":enabled}),
            Request::SetSwarmEnabled(enabled),
        ) {
            Ok(())
        } else {
            Err("The session disconnected before Swarm could be changed.".into())
        }
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
    #[cfg(test)]
    pub fn unqueue(&mut self, id: SessionId, index: usize) -> Result<(), String> {
        let Some(watch) = self.watches.get_mut(&id) else {
            return Err("Session is not connected.".into());
        };
        Self::cancel_queued(watch, index)
    }

    /// The UI supplies the key captured from its rendered Session, not an index
    /// looked up in the possibly newer session at tap time.
    pub fn unqueue_key(&mut self, id: SessionId, key: &QueueKey) -> Result<(), String> {
        let Some(watch) = self.watches.get_mut(&id) else {
            return Err("Session is not connected.".into());
        };
        Self::cancel_queue_key(watch, key)
    }

    // Offline legacy index tests deliberately address the currently visible queue.
    #[cfg(test)]
    fn cancel_queued(watch: &mut Watch, index: usize) -> Result<(), String> {
        let queued: Vec<usize> = watch
            .pi
            .steering
            .iter()
            .chain(&watch.pi.follow_up)
            .enumerate()
            .filter(|(index, _)| watch.queue_row_visible(*index))
            .map(|(index, _)| index)
            .collect();
        let key = if index < queued.len() {
            let submission = watch.pi.queued_submissions.get(queued[index])
                .filter(|id| !id.is_empty())
                .ok_or("This helper cannot remove one queued prompt safely. Update the helper or use Stop to clear all queued work.")?;
            QueueKey::Durable(submission.clone())
        } else {
            let mut visible = watch.visible_outbox();
            if watch.accepted_turn.is_none()
                && visible
                    .clone()
                    .next()
                    .is_some_and(|(_, pending)| watch.starts_turn(pending))
            {
                visible.next();
            }
            let Some((_, pending)) = visible.nth(index - queued.len()) else {
                return Ok(());
            };
            QueueKey::Local(pending.prompt.request_id.clone())
        };
        Self::cancel_queue_key(watch, &key)
    }

    fn cancel_queue_key(watch: &mut Watch, key: &QueueKey) -> Result<(), String> {
        let submission = match key {
            QueueKey::Durable(submission) => submission.clone(),
            QueueKey::Local(request_id) => {
                if let Some(at) = watch
                    .outbox
                    .iter()
                    .position(|pending| pending.prompt.request_id == *request_id)
                {
                    if watch.outbox[at].cancelled {
                        return Ok(());
                    }
                    if watch.outbox[at].submission_id.as_deref() == Some("") {
                        return Err("This helper cannot remove one queued prompt safely. Update the helper or use Stop to clear all queued work.".into());
                    }
                    let delivering = watch.outbox[at].admission.is_some()
                        || watch.sent.values().any(
                            |sent| matches!(&sent.request, Request::Prompt(id) if id == request_id),
                        );
                    if delivering {
                        watch.outbox[at].cancelled = true;
                        if let Some(submission) = watch.outbox[at].submission_id.clone() {
                            watch.cancellations.entry(submission).or_insert(false);
                        }
                        Self::flush_cancellations(watch);
                    } else {
                        watch.outbox.remove(at);
                    }
                    return Ok(());
                }
                let Some(submission) = watch.queue_receipts.get(request_id) else {
                    return Ok(());
                };
                submission.clone()
            }
        };
        // Removed or already hidden rows are safe no-ops, including a second tap
        // on the same retained frame. A key never falls through to the next row.
        if submission.is_empty()
            || watch.cancellations.contains_key(&submission)
            || !watch
                .pi
                .queued_submissions
                .iter()
                .enumerate()
                .any(|(index, id)| {
                    id == &submission && index < watch.pi.steering.len() + watch.pi.follow_up.len()
                })
        {
            return Ok(());
        }
        watch.cancellations.insert(submission.clone(), false);
        for pending in &mut watch.outbox {
            if pending.submission_id.as_ref() == Some(&submission) {
                pending.cancelled = true;
            }
        }
        Self::flush_cancellations(watch);
        Ok(())
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
        if !watch.current || !watch.ready || watch.dialogs.is_empty() {
            return;
        }
        let request = watch.dialogs.remove(0);
        let record = projection::answer(&request, answer);
        let diagnostic = TransportDiagnostic::from_record(TransportDirection::Outbound, &record);
        if input.try_send(record).is_ok() {
            watch.diagnostics.push(diagnostic);
        }
    }

    /// Applies an update; returns the session it changed and any problem.
    pub fn apply(
        &mut self,
        id: Option<SessionId>,
        update: Update,
    ) -> (Vec<SessionId>, Option<Problem>) {
        let Some(id) = id else {
            if let Update::CommandCatalog(generation, path, commands) = update {
                apply_command_catalog(
                    generation,
                    self.command_generation,
                    path,
                    commands,
                    &mut self.commands,
                    &mut self.commands_path,
                    &mut self.commands_loading,
                    &mut self.commands_error,
                );
                return (Vec::new(), None);
            }
            if let Update::ModelCatalog(generation, models) = update {
                apply_catalog(
                    generation,
                    self.catalog_generation,
                    models,
                    &mut self.models,
                    &mut self.models_loading,
                    &mut self.models_error,
                );
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
        // Child transcript reads update their own retained screen cache only.
        // Reprojecting the complete parent for every one-second child poll is
        // both redundant and especially costly for long parent histories.
        let parent_changed = !matches!(
            &update,
            Update::Record(_, record)
                if record["type"] == "response" && record["command"] == "get_subagent"
        );
        let Some(watch) = self.watches.get_mut(&id) else {
            return (Vec::new(), None);
        };
        let problem = Self::apply_watch(
            watch,
            update,
            &self.model,
            &self.thinking,
            &mut self.models,
            Instant::now(),
        )
        .map(|text| Problem {
            session: Some(id),
            text,
        });
        (parent_changed.then_some(id).into_iter().collect(), problem)
    }

    fn apply_watch(
        watch: &mut Watch,
        update: Update,
        wanted: &str,
        thinking: &str,
        models: &mut Vec<pi_core::protocol::Model>,
        now: Instant,
    ) -> Option<String> {
        if !watch.accepts(&update) {
            return None;
        }
        match update {
            Update::Opened(_, input) => {
                watch
                    .diagnostics
                    .record(TransportDirection::Outbound, &watch.target.attach_record());
                watch.input = Some(input);
                Self::flush_image_requests(watch);
            }
            Update::Ended(_, reason) => {
                log::info!("Session {} closed: {reason}", watch.target.key);
                watch.end_transport(reason, now);
            }
            Update::Record(_, record) => {
                watch
                    .diagnostics
                    .record(TransportDirection::Inbound, &record);
                return Self::record(watch, &record, wanted, thinking, models);
            }
            Update::Listed(_) | Update::ModelCatalog(_, _) | Update::CommandCatalog(_, _, _) => {}
        }
        None
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
        let previous_model = watch
            .pi
            .state
            .model
            .as_ref()
            .map(|model| (model.provider.clone(), model.id.clone()));
        let applied = match watch.pi.apply(record) {
            Ok(()) => true,
            Err(error) => {
                log::warn!("Skipping a record Pi's session model refused: {error:#}");
                false
            }
        };
        if applied && observes_session(record) {
            watch.observed_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |time| time.as_millis() as u64),
            );
        }
        if applied
            && previous_model
                != watch
                    .pi
                    .state
                    .model
                    .as_ref()
                    .map(|model| (model.provider.clone(), model.id.clone()))
        {
            watch.image_state_requested = false;
            watch.image_catalog_requested = false;
        }
        if applied && kind == "remote_snapshot" {
            watch.cached_snapshot = true;
            watch.retry_failures = 0;
            watch.cancellations.retain(|id, acknowledged| {
                !*acknowledged || watch.pi.queued_submissions.contains(id)
            });
            // Only an exact durable identity can replace a local row. Helpers
            // without receipts leave their recoverable placeholder visible.
            watch.outbox.retain(|pending| {
                pending.cancelled
                    || pending
                        .submission_id
                        .as_ref()
                        .is_none_or(|id| !watch.pi.queued_submissions.contains(id))
            });
        }
        if (applied && kind == "remote_snapshot")
            || watch
                .accepted_turn
                .as_ref()
                .is_some_and(|accepted| user_turns(&watch.pi) > accepted.user_turns_before)
        {
            // A snapshot replaces optimism even when compaction/rebase reduced
            // the retained user-message count. Snapshots have no admission
            // watermark yet: an older idle frame racing an acknowledgement can
            // still clear this hint. A correlated daemon read is not a barrier.
            watch.accepted_turn = None;
        }
        if applied && kind == "remote_snapshot" {
            Self::fetch_subagent(watch);
        }
        if applied && kind == "remote_snapshot" && !watch.current {
            watch.current = true;
            if watch.pi.state.model.is_none() && watch.target.backend == RemoteBackend::Durable {
                Self::send(
                    watch,
                    json!({"type": "get_available_models"}),
                    Request::Models,
                );
            } else {
                watch.ready = true;
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
        let Some(request) = response else {
            if applied {
                Self::flush(watch);
            }
            return None;
        };
        let failed = record["success"] != true;
        let error = record["error"]
            .as_str()
            .unwrap_or("Pi refused it")
            .to_owned();
        let result = match request {
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
                if !failed
                    && let Some(submission) = record["data"]["submissionId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                {
                    watch
                        .queue_request_ids
                        .insert(submission.to_owned(), request_id.clone());
                    watch
                        .queue_receipts
                        .insert(request_id, submission.to_owned());
                }
                if watch.outbox[index].cancelled {
                    if failed {
                        watch.outbox.remove(index);
                        return None;
                    }
                    let Some(submission) = record["data"]["submissionId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                    else {
                        // Keep it visible after acknowledgement rather than let
                        // unsupported hidden work execute. A later queue snapshot
                        // replaces this local placeholder with the remote row.
                        watch.outbox[index].cancelled = false;
                        watch.outbox[index].submission_id = Some(String::new());
                        return Some(
                            "This helper cannot cancel a prompt while it is being delivered. Update the helper and try again."
                                .into(),
                        );
                    };
                    watch.outbox[index].submission_id = Some(submission.to_owned());
                    watch
                        .cancellations
                        .entry(submission.to_owned())
                        .or_insert(false);
                    Self::flush(watch);
                    None
                } else {
                    let pending = watch.outbox.remove(index);
                    if failed {
                        watch.failed.push(FailedPrompt {
                            prompt: pending.prompt,
                            error: error.clone(),
                        });
                        if watch.pi.messages.is_empty() {
                            watch.pi.error = Some(error.clone());
                        }
                        Some(error)
                    } else {
                        if let Some(user_turns_before) = pending.user_turns_before
                            && user_turns(&watch.pi) <= user_turns_before
                        {
                            watch.accepted_turn = Some(AcceptedTurn {
                                text: pending.prompt.label(),
                                user_turns_before,
                            });
                        }
                        None
                    }
                }
            }
            Request::CancelSubmission(submission) => {
                if failed {
                    watch.cancellations.remove(&submission);
                    for pending in &mut watch.outbox {
                        if pending.submission_id.as_ref() == Some(&submission) {
                            pending.cancelled = false;
                        }
                    }
                    watch.outbox.retain(|pending| {
                        pending.submission_id.as_ref() != Some(&submission)
                            || !watch.pi.queued_submissions.contains(&submission)
                    });
                    Some(format!(
                        "Could not cancel the queued prompt: {error}. Update the helper if it does not support per-prompt cancellation."
                    ))
                } else {
                    watch.cancellations.insert(submission.clone(), true);
                    watch
                        .outbox
                        .retain(|pending| pending.submission_id.as_ref() != Some(&submission));
                    None
                }
            }
            Request::Models | Request::AvailableModels => {
                let initial = matches!(request, Request::Models);
                if failed {
                    if initial {
                        Self::hold_failed(watch, &error);
                    } else {
                        if watch
                            .pi
                            .state
                            .model
                            .as_ref()
                            .is_some_and(|model| model.input.is_empty())
                            && !watch
                                .sent
                                .values()
                                .any(|sent| matches!(sent.request, Request::SetModel))
                        {
                            Self::fail_waiting_images(watch, &error);
                        }
                        Self::flush(watch);
                    }
                    return Some(error);
                }
                if let Ok(available) = serde_json::from_value::<Vec<pi_core::protocol::Model>>(
                    record["data"]["models"].clone(),
                ) {
                    *models = available;
                }
                if !initial {
                    if watch
                        .pi
                        .state
                        .model
                        .as_ref()
                        .is_some_and(|model| model.input.is_empty())
                        && !watch
                            .sent
                            .values()
                            .any(|sent| matches!(sent.request, Request::SetModel))
                    {
                        Self::fail_waiting_images(
                            watch,
                            "The session's model catalog did not report image capabilities. Refresh models or choose a vision-capable model, then recover your prompt.",
                        );
                    }
                    Self::flush(watch);
                    return None;
                }
                if !applied {
                    let error = "The computer sent an unreadable session model catalog.";
                    Self::hold_failed(watch, error);
                    return Some(error.into());
                }
                let Some(model) = choose_model(&watch.pi.available_models, wanted) else {
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
                if !failed && !applied {
                    watch.ready = false;
                    let error = "The session did not confirm the selected model. Select a model again, then recover your prompt.";
                    Self::hold_failed(watch, error);
                    return Some(error.into());
                }
                watch.image_state_requested = false;
                watch.image_catalog_requested = false;
                watch.ready = !failed && applied;
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
            Request::SetCompactionThreshold(percent) => {
                if failed {
                    Some(error)
                } else if record["data"]["percent"].as_f64() != Some(percent) {
                    Self::send(watch, json!({"type":"get_session_stats"}), Request::Quiet);
                    Some("The computer did not confirm the compaction threshold. Refresh before trying again.".into())
                } else if let Ok(policy) = serde_json::from_value::<
                    pi_core::protocol::CompactionPolicy,
                >(record["data"]["compaction"].clone())
                {
                    watch.pi.stats.compaction = Some(policy);
                    None
                } else {
                    Self::send(watch, json!({"type":"get_session_stats"}), Request::Quiet);
                    Some("The computer sent an unreadable compaction policy.".into())
                }
            }
            Request::SetSwarmEnabled(enabled) => {
                if failed {
                    Some(error)
                } else if record["data"]["enabled"].as_bool() != Some(enabled) {
                    Self::send(watch, json!({"type":"get_state"}), Request::Quiet);
                    Some("The computer did not confirm the Swarm setting. Refresh before trying again.".into())
                } else {
                    watch.pi.state.swarm_enabled = Some(enabled);
                    None
                }
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
            Request::Compact => {
                if !failed {
                    // Durable compaction returns an immediate post-compaction
                    // estimate. Publish it now instead of leaving Details on
                    // the pre-compaction percentage until the next stats pass.
                    if let Some(tokens) = record["data"]["estimatedTokensAfter"].as_u64()
                        && let Some(usage) = watch.pi.stats.context_usage.as_mut()
                    {
                        usage.tokens = Some(tokens);
                        usage.percent = (usage.context_window > 0)
                            .then(|| tokens as f64 * 100. / usage.context_window as f64);
                    }
                    // The estimate is only a bridge. Refresh every backend's
                    // authoritative context usage as soon as compaction ends.
                    Self::send(watch, json!({"type":"get_session_stats"}), Request::Quiet);
                }
                failed.then_some(error)
            }
            Request::Image(request) => {
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
                watch.images.insert(request.id, Some(fetched));
                None
            }
            Request::Subagent(conversation) => {
                let identity_matches =
                    record["data"]["conversationId"].as_str() == Some(conversation.as_str());
                let fetched = if failed {
                    Err(error)
                } else if !identity_matches {
                    Err("The computer returned a different subagent conversation.".into())
                } else {
                    pi_core::subagent::session(&record["data"], watch.target.cwd.clone().into())
                        .map_err(|error| error.to_string())
                };
                let busy = fetched.as_ref().is_ok_and(pi_core::session::Session::busy);
                let revision = watch
                    .subagent_revisions
                    .entry(conversation.clone())
                    .or_default();
                *revision = revision.wrapping_add(1).max(1);
                let run_id = record["data"]["runId"].as_str().map(str::to_owned);
                if busy && run_id.is_none() {
                    watch.subagents.insert(
                        conversation.clone(),
                        Err("The computer did not identify the subagent's active run.".into()),
                    );
                    watch.subagent_runs.remove(&conversation);
                    watch.resumable_subagents.remove(&conversation);
                } else {
                    watch.subagents.insert(conversation.clone(), fetched);
                    watch.subagent_runs.insert(conversation.clone(), run_id);
                    if record["data"]["resumable"] == true {
                        watch.resumable_subagents.insert(conversation.clone());
                    } else {
                        watch.resumable_subagents.remove(&conversation);
                    }
                }
                // Catch up once if progress or a different child arrived while
                // this request was outstanding; otherwise poll a running child
                // at a bounded cadence, including direct follow-up runs that no
                // longer update the parent's hand-off document.
                if watch.subagent_refresh_pending {
                    Self::fetch_subagent(watch);
                } else {
                    watch.subagent_refresh_at =
                        busy.then(|| Instant::now() + Duration::from_secs(1));
                }
                None
            }
            Request::SubagentPrompt {
                conversation,
                request_id,
            } => {
                let pending = watch.subagent_outbox.get(&conversation)?;
                if pending.request_id != request_id {
                    return None;
                }
                let pending = watch.subagent_outbox.remove(&conversation).unwrap();
                if failed {
                    Some(error)
                } else {
                    watch
                        .accepted_subagent_inputs
                        .insert(conversation.clone(), pending.message);
                    if watch.following.as_deref() == Some(&conversation) {
                        Self::fetch_subagent(watch);
                    }
                    None
                }
            }
            Request::ImageState => {
                if watch.pi.state.model.is_none() {
                    Self::fail_waiting_images(
                        watch,
                        if failed {
                            &error
                        } else {
                            "The session did not report its selected model. Select a vision-capable model, then recover your prompt."
                        },
                    );
                }
                failed.then_some(error)
            }
            Request::Quiet => {
                if record["command"] == "get_state" && watch.pi.state.model.is_none() {
                    Self::fail_waiting_images(
                        watch,
                        if failed {
                            &error
                        } else {
                            "The session did not report its selected model. Select a vision-capable model, then recover your prompt."
                        },
                    );
                }
                None
            }
            Request::Other => failed.then_some(error),
        };
        Self::flush(watch);
        result
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

    /// A tool image fetched from the computer: `None` while queued or on the way.
    pub fn image(&self, id: SessionId, key: &str) -> Option<&Result<ToolImageBytes, String>> {
        self.watches.get(&id)?.images.get(key)?.as_ref()
    }

    /// Whether a visible card has already admitted this image to the bounded
    /// hydration queue. This distinguishes absence from an in-flight `None`.
    pub(crate) fn image_requested(&self, id: SessionId, key: &str) -> bool {
        self.watches
            .get(&id)
            .is_some_and(|watch| watch.images.contains_key(key))
    }

    /// Admit one visible tool image. Duplicate transcript records and repeated
    /// renders share the content-derived image ID, so they never add another
    /// queue entry or transport request.
    pub(crate) fn request_image(
        &mut self,
        id: SessionId,
        key: &str,
        conversation: Option<&str>,
    ) -> bool {
        let Some(watch) = self.watches.get_mut(&id) else {
            return false;
        };
        Self::request_image_watch(watch, key, conversation)
    }

    fn request_image_watch(watch: &mut Watch, key: &str, conversation: Option<&str>) -> bool {
        if watch.images.contains_key(key) {
            return false;
        }
        let request = ImageRequest {
            id: key.to_owned(),
            conversation: conversation.map(str::to_owned),
        };
        watch.images.insert(request.id.clone(), None);
        watch.image_queue.push_back(request);
        Self::flush_image_requests(watch);
        true
    }

    /// A subagent's own session, once fetched: see [`Self::follow_subagent`].
    pub fn subagent(
        &self,
        id: SessionId,
        conversation: &str,
    ) -> Option<&Result<pi_core::session::Session, String>> {
        self.watches.get(&id)?.subagents.get(conversation)
    }

    pub(crate) fn subagent_revision(&self, id: SessionId, conversation: &str) -> Option<u64> {
        self.watches
            .get(&id)?
            .subagent_revisions
            .get(conversation)
            .copied()
    }

    fn subagent_input_target_watch(
        watch: &Watch,
        conversation: &str,
    ) -> Result<SubagentInputTarget, String> {
        if watch.following.as_deref() != Some(conversation) {
            return Err("Open this subagent again before sending it a message.".into());
        }
        if !Self::supports_subagent_input_watch(watch) {
            return Err("This durable session does not support direct subagent input. Update the computer helper and durable runner.".into());
        }
        let session = watch
            .subagents
            .get(conversation)
            .ok_or("Wait for this subagent conversation to finish loading.")?
            .as_ref()
            .map_err(Clone::clone)?;
        if session.busy() {
            let run_id = watch
                .subagent_runs
                .get(conversation)
                .and_then(Clone::clone)
                .ok_or("Refresh this subagent before steering its active run.")?;
            Ok(SubagentInputTarget::Steer { run_id })
        } else if watch.resumable_subagents.contains(conversation) {
            Ok(SubagentInputTarget::FollowUp)
        } else {
            Err("This subagent conversation cannot be resumed yet.".into())
        }
    }

    /// Exact semantics currently safe for input to this fetched child.
    pub(crate) fn subagent_input_target(
        &self,
        id: SessionId,
        conversation: &str,
    ) -> Result<SubagentInputTarget, String> {
        let watch = self
            .watches
            .get(&id)
            .ok_or("Reconnect to the computer before messaging this subagent.")?;
        Self::subagent_input_target_watch(watch, conversation)
    }

    pub(crate) fn subagent_input_pending(&self, id: SessionId, conversation: &str) -> bool {
        self.watches
            .get(&id)
            .is_some_and(|watch| watch.subagent_outbox.contains_key(conversation))
    }

    pub(crate) fn take_accepted_subagent_input(
        &mut self,
        id: SessionId,
        conversation: &str,
    ) -> Option<String> {
        self.watches
            .get_mut(&id)?
            .accepted_subagent_inputs
            .remove(conversation)
    }

    /// Queue text for one exact fetched child. Its target mode and active run
    /// identity are frozen before the first send and retained across reconnects.
    pub(crate) fn prompt_subagent(
        &mut self,
        id: SessionId,
        conversation: &str,
        message: String,
    ) -> Result<(), String> {
        self.helper
            .require_attach("messaging a subagent")
            .map_err(|error| error.to_string())?;
        let message = message.trim().to_owned();
        if message.is_empty() {
            return Err("Type a message for this subagent first.".into());
        }
        let target = self.subagent_input_target(id, conversation)?;
        let watch = self
            .watches
            .get_mut(&id)
            .ok_or("This session is no longer available.")?;
        if watch.subagent_outbox.contains_key(conversation) {
            return Err("Wait for the previous subagent message to be accepted.".into());
        }
        watch.subagent_outbox.insert(
            conversation.to_owned(),
            PendingSubagentInput {
                request_id: crate::prompt::request_id(),
                message,
                target,
            },
        );
        Self::flush_subagent_inputs(watch);
        Ok(())
    }

    /// Keeps a subagent's messages fresh while its screen is open; `None` stops.
    pub fn follow_subagent(&mut self, id: SessionId, conversation: Option<String>) {
        if conversation.is_some() {
            self.watch(id);
        }
        let Some(watch) = self.watches.get_mut(&id) else {
            return;
        };
        Self::select_subagent(watch, conversation);
    }

    fn select_subagent(watch: &mut Watch, conversation: Option<String>) {
        if watch.following == conversation {
            return;
        }
        watch.following = conversation;
        watch.subagent_refresh_at = None;
        Self::fetch_subagent(watch);
    }

    /// Asks for the followed subagent, unless a request is on the way.
    fn fetch_subagent(watch: &mut Watch) {
        let Some(conversation) = watch.following.clone() else {
            watch.subagent_refresh_pending = false;
            return;
        };
        watch.subagent_refresh_pending = true;
        if watch.input.is_none()
            || watch
                .sent
                .values()
                .any(|sent| matches!(&sent.request, Request::Subagent(_)))
        {
            return;
        }
        if Self::send(
            watch,
            json!({"type": "get_subagent", "conversationId": conversation}),
            Request::Subagent(conversation),
        ) {
            watch.subagent_refresh_pending = false;
            watch.subagent_refresh_at = None;
        }
    }

    /// Poll the selected running child without depending on parent updates.
    pub(crate) fn poll_subagents(&mut self, now: Instant) {
        for watch in self.watches.values_mut() {
            if watch.subagent_refresh_at.is_some_and(|due| now >= due) {
                watch.subagent_refresh_at = None;
                Self::fetch_subagent(watch);
            }
        }
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
        if Self::send(
            watch,
            json!({"type": "stop_subagent", "conversationId": conversation}),
            Request::Other,
        ) {
            Ok(())
        } else {
            Err("The Stop request was not sent. Reconnect and explicitly retry.".into())
        }
    }

    /// The session as the phone shows it.
    pub fn session(&self, id: SessionId) -> Option<crate::model::Session> {
        if let Some(watch) = self.watches.get(&id)
            && let Some(session) = watch.project(id, self.helper.short(&watch.target.cwd))
        {
            return Some(session);
        }
        let listed = self.listed.get(&id)?;
        Some(projection::listed(
            id,
            listed,
            self.helper.short(&listed.cwd),
        ))
    }

    /// A healthy SSH connection is not proof that this particular session pipe is current.
    pub(crate) fn session_current(&self, id: SessionId) -> bool {
        self.watches.get(&id).is_some_and(|watch| {
            watch.current && watch.input.as_ref().is_some_and(|input| !input.is_closed())
        })
    }

    pub(crate) fn observed_at(&self, id: SessionId) -> Option<u64> {
        self.watches.get(&id)?.observed_at
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

/// Only successfully parsed session-data records count as observation. Child
/// responses and command receipts do not refresh an older parent swarm snapshot.
fn observes_session(record: &Value) -> bool {
    match record["type"].as_str() {
        Some(
            "remote_snapshot"
            | "state_update"
            | "stats_update"
            | "agent_start"
            | "agent_settled"
            | "compaction_start"
            | "compaction_end"
            | "auto_retry_start"
            | "auto_retry_end"
            | "summarization_retry_scheduled"
            | "message_start"
            | "message_update"
            | "message_end"
            | "tool_execution_start"
            | "tool_execution_update"
            | "tool_execution_end"
            | "queue_update"
            | "session_info_changed"
            | "thinking_level_changed"
            | "bash_execution_update",
        ) => true,
        Some("response") => {
            record["success"] == true
                && matches!(
                    record["command"].as_str(),
                    Some("get_state" | "get_messages" | "get_session_stats" | "get_active_tools")
                )
        }
        _ => false,
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

/// Project command replies are correlated independently from models because
/// changing folders does not replace the SSH connection.
#[allow(clippy::too_many_arguments)]
fn apply_command_catalog(
    generation: u64,
    current: u64,
    path: Option<String>,
    result: Result<Vec<pi_core::protocol::SlashCommand>, String>,
    values: &mut Vec<pi_core::protocol::SlashCommand>,
    current_path: &mut Option<String>,
    loading: &mut bool,
    error: &mut Option<String>,
) {
    if generation != current {
        return;
    }
    *loading = false;
    match result {
        Ok(catalog) => {
            *values = catalog;
            *current_path = path;
            *error = None;
        }
        Err(reason) => *error = Some(reason),
    }
}

/// The model the settings name, by name or id, else the first offered.
/// Ignore both successful and failed catalog replies from an obsolete SSH
/// connection, including their attempts to clear the current loading state.
fn apply_catalog<T>(
    generation: u64,
    current: u64,
    result: Result<Vec<T>, String>,
    values: &mut Vec<T>,
    loading: &mut bool,
    error: &mut Option<String>,
) {
    if generation != current {
        return;
    }
    *loading = false;
    match result {
        Ok(catalog) => {
            *values = catalog;
            *error = None;
        }
        Err(reason) => *error = Some(reason),
    }
}

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
    fn transport_diagnostics_are_bounded_by_count_and_retained_bytes() {
        let mut diagnostics = TransportDiagnostics::default();
        for index in 0..256 {
            diagnostics.record(
                TransportDirection::Inbound,
                &json!({"type":"unknown-record","id":format!("record-{index}")}),
            );
        }
        assert_eq!(diagnostics.entries.len(), TRANSPORT_DIAGNOSTIC_MAX_ENTRIES);
        assert!(diagnostics.entries.iter().all(|entry| entry.has_id));
        assert!(
            diagnostics.entries.iter().all(|entry| !entry
                .preview
                .as_deref()
                .unwrap()
                .contains("record-"))
        );

        for _ in 0..TRANSPORT_DIAGNOSTIC_MAX_ENTRIES {
            diagnostics.push(TransportDiagnostic {
                at_millis: 0,
                direction: TransportDirection::Inbound,
                record_type: "response".into(),
                command: Some("get_state".into()),
                has_id: false,
                byte_size: 0,
                preview: Some("x".repeat(1_024)),
            });
        }
        assert!(diagnostics.entries.len() <= TRANSPORT_DIAGNOSTIC_MAX_ENTRIES);
        assert!(diagnostics.retained_bytes <= TRANSPORT_DIAGNOSTIC_MAX_BYTES);
        assert!(
            diagnostics.entries.len() < TRANSPORT_DIAGNOSTIC_MAX_ENTRIES,
            "the byte budget should evict before the count budget"
        );
    }

    #[test]
    fn live_pipe_records_both_successful_sends_and_inbound_records() {
        let (mut watch, sent) = watch();
        assert!(Live::send(
            &mut watch,
            json!({"type":"get_state"}),
            Request::Quiet
        ));
        let outbound = sent.try_recv().unwrap();
        let outbound_id = outbound["id"].as_str().unwrap();
        assert_eq!(watch.diagnostics.entries.len(), 1);
        assert_eq!(
            watch.diagnostics.entries[0].direction,
            TransportDirection::Outbound
        );
        assert_eq!(watch.diagnostics.entries[0].record_type, "get_state");
        assert!(watch.diagnostics.entries[0].has_id);
        assert!(
            !watch.diagnostics.entries[0]
                .preview
                .as_deref()
                .unwrap()
                .contains(outbound_id)
        );

        let generation = watch.generation;
        pipe_update(
            &mut watch,
            Update::Record(
                generation,
                json!({"type":"transport_probe","id":"server-record-1","data":{"messages":["not retained"]}}),
            ),
            Instant::now(),
        );
        assert_eq!(watch.diagnostics.entries.len(), 2);
        assert_eq!(
            watch.diagnostics.entries[1].direction,
            TransportDirection::Inbound
        );
        assert!(watch.diagnostics.entries[1].has_id);
        assert!(
            !watch.diagnostics.entries[1]
                .preview
                .as_deref()
                .unwrap()
                .contains("not retained")
        );
    }

    #[test]
    fn observation_time_requires_accepted_session_data_not_stale_or_rejected_records() {
        let (mut watch, sent) = watch();
        watch.observed_at = Some(123);
        let now = Instant::now();
        let generation = watch.generation;
        pipe_update(
            &mut watch,
            Update::Record(
                generation + 1,
                json!({"type":"remote_snapshot", "data":Pi::default()}),
            ),
            now,
        );
        assert_eq!(watch.observed_at, Some(123));
        pipe_update(
            &mut watch,
            Update::Record(generation, json!({"type":"remote_snapshot", "data":[]})),
            now,
        );
        assert_eq!(watch.observed_at, Some(123));
        pipe_update(
            &mut watch,
            Update::Record(generation, json!({"type":"future_record"})),
            now,
        );
        assert_eq!(watch.observed_at, Some(123));
        pipe_update(
            &mut watch,
            Update::Record(
                generation,
                json!({"type":"response", "id":"unknown-request", "command":"get_messages", "success":true, "data":{"messages":[]}}),
            ),
            now,
        );
        assert_eq!(watch.observed_at, Some(123));

        assert!(Live::send(
            &mut watch,
            json!({"type":"get_messages"}),
            Request::Other
        ));
        let request = sent.try_recv().unwrap();
        pipe_update(
            &mut watch,
            Update::Record(
                generation,
                json!({"type":"response", "id":request["id"], "command":"get_state", "success":true, "data":{}}),
            ),
            now,
        );
        assert_eq!(
            watch.observed_at,
            Some(123),
            "mismatched command must not count"
        );
        pipe_update(
            &mut watch,
            Update::Record(
                generation,
                json!({"type":"response", "id":request["id"], "command":"get_messages", "success":true, "data":{"messages":"invalid"}}),
            ),
            now,
        );
        assert_eq!(
            watch.observed_at,
            Some(123),
            "rejected schema must not count"
        );
        pipe_update(
            &mut watch,
            Update::Record(
                generation,
                json!({"type":"remote_snapshot", "data":Pi::default()}),
            ),
            now,
        );
        assert!(watch.observed_at.is_some_and(|at| at > 123));
        assert!(watch.current);
        watch.observed_at = Some(456);
        pipe_update(
            &mut watch,
            Update::Ended(generation, "pipe lost".into()),
            now,
        );
        assert!(!watch.current);
        assert_eq!(watch.observed_at, Some(456));
        pipe_update(
            &mut watch,
            Update::Record(
                generation,
                json!({"type":"remote_snapshot", "data":Pi::default()}),
            ),
            now,
        );
        assert_eq!(
            watch.observed_at,
            Some(456),
            "retired generation cannot refresh cached data"
        );
        let (input, _receipts) = async_channel::unbounded();
        let next = watch.generation;
        pipe_update(&mut watch, Update::Opened(next, input), now);
        assert!(
            !watch.current,
            "an open pipe alone is not a session observation"
        );
        assert_eq!(watch.observed_at, Some(456));
    }

    #[test]
    fn child_responses_and_command_acks_do_not_refresh_parent_observation() {
        let (mut watch, sent) = watch();
        for command in [
            "get_subagent",
            "stop_subagent",
            "prompt_subagent",
            "prompt",
            "get_backend_info",
        ] {
            watch.observed_at = Some(123);
            assert!(Live::send(
                &mut watch,
                json!({"type":command}),
                Request::Other
            ));
            let request = sent.try_recv().unwrap();
            Live::record(
                &mut watch,
                &json!({"type":"response", "id":request["id"], "command":command, "success":true, "data":{}}),
                "",
                "",
                &mut Vec::new(),
            );
            assert_eq!(
                watch.observed_at,
                Some(123),
                "{command} is not a parent snapshot"
            );
        }
        watch.observed_at = Some(123);
        assert!(Live::send(
            &mut watch,
            json!({"type":"get_messages"}),
            Request::Other
        ));
        let request = sent.try_recv().unwrap();
        let response = json!({"type":"response", "id":request["id"], "command":"get_messages", "success":true, "data":{"messages":[]}});
        Live::record(&mut watch, &response, "", "", &mut Vec::new());
        assert!(watch.observed_at.is_some_and(|at| at > 123));
        watch.observed_at = Some(456);
        Live::record(&mut watch, &response, "", "", &mut Vec::new());
        assert_eq!(
            watch.observed_at,
            Some(456),
            "duplicate receipt is not a new observation"
        );
    }

    fn pipe_update(watch: &mut Watch, update: Update, now: Instant) {
        assert!(Live::apply_watch(watch, update, "", "", &mut Vec::new(), now).is_none());
    }

    fn authoritative(watch: &mut Watch, pi: Pi, now: Instant) {
        pipe_update(
            watch,
            Update::Record(
                watch.generation,
                json!({"type":"remote_snapshot", "data":pi}),
            ),
            now,
        );
    }

    #[test]
    fn healthy_ssh_recovers_a_displayed_finished_parent_with_full_cached_history() {
        let now = Instant::now();
        let (mut watch, _) = watch();
        let mut pi = Pi::new(watch.target.cwd.clone().into());
        pi.state.model = Some(model("host", "chosen", "Chosen"));
        pi.messages = vec![
            json!({"role":"user", "content":"first turn"}),
            json!({"role":"assistant", "content":[{"type":"text", "text":"first answer"}], "stopReason":"stop"}),
            json!({"role":"user", "content":"parent with child"}),
            json!({"role":"assistant", "content":[{"type":"text", "text":"child handed off"}], "stopReason":"stop"}),
        ];
        authoritative(&mut watch, pi.clone(), now);
        let id = SessionId(7);
        let before = watch.project(id, "pi".into()).unwrap();
        assert_eq!(before.state, crate::model::State::Done);
        assert_eq!(before.turns.len(), 2);
        let target = watch.target.clone();
        let old = watch.generation;
        pipe_update(&mut watch, Update::Ended(old, "channel only".into()), now);
        assert!(!watch.current && !watch.ready);
        assert_eq!(watch.project(id, "pi".into()).unwrap(), before);
        assert!(watch.recover(now, true, true).is_none());
        // The scheduler does not consult busy listings: a retained displayed
        // finished parent recovers just like any other retained watch.
        let due = now + Duration::from_secs(1);
        let (reattach, generation) = watch.recover(due, true, true).unwrap();
        assert_eq!(reattach, target);
        assert_eq!(reattach.attach_record(), target.attach_record());
        assert_ne!(generation, old);
        assert!(
            watch
                .recover(due + Duration::from_secs(60), true, true)
                .is_none()
        );
        assert_eq!(watch.project(id, "pi".into()).unwrap(), before);
        let (input, commands) = async_channel::unbounded();
        pipe_update(&mut watch, Update::Opened(generation, input), due);
        assert!(!watch.current && !watch.ready);
        assert!(
            Live::configure(&mut watch, json!({"type":"set_model"}), Request::SetModel).is_err()
        );
        assert!(commands.try_recv().is_err());
        pi.run = pi_core::session::RunState::Running;
        authoritative(&mut watch, pi.clone(), due);
        assert_eq!(
            watch.project(id, "pi".into()).unwrap().state,
            crate::model::State::Working
        );
        pi.run = pi_core::session::RunState::Idle;
        pi.messages.push(json!({"role":"assistant", "content":[{"type":"text", "text":"child completed; fresh parent"}], "stopReason":"stop"}));
        authoritative(&mut watch, pi, due);
        let fresh = watch.project(id, "pi".into()).unwrap();
        assert_eq!(fresh.id, id);
        assert_eq!(fresh.state, crate::model::State::Done);
        assert_eq!(fresh.turns.len(), 2);
        assert!(
            fresh.turns[1]
                .summary
                .as_ref()
                .unwrap()
                .text()
                .contains("fresh parent")
        );
        assert!(watch.current && watch.ready && watch.cached_snapshot);
        assert!(watch.retry_at.is_none());
    }

    #[test]
    fn watch_recovery_backs_off_is_health_gated_and_never_repeats_in_flight() {
        let mut now = Instant::now();
        let (mut watch, _) = watch();
        for failure in 0..9 {
            let generation = watch.generation;
            pipe_update(
                &mut watch,
                Update::Ended(generation, "attach refused".into()),
                now,
            );
            let due = now + Duration::from_secs(1 << failure.min(5));
            assert_eq!(watch.retry_at, Some(due));
            assert!(
                watch
                    .recover(due - Duration::from_millis(1), true, true)
                    .is_none()
            );
            assert!(
                watch.recover(due, false, true).is_none(),
                "SSH reconnect owns unhealthy connections"
            );
            assert!(watch.recover(due, true, false).is_none());
            let (_, generation) = watch.recover(due, true, true).unwrap();
            let (input, _) = async_channel::unbounded();
            pipe_update(&mut watch, Update::Opened(generation, input), due);
            assert_eq!(
                watch.retry_failures,
                failure + 1,
                "Opened alone is not recovery"
            );
            assert!(
                watch.recover(due, true, true).is_none(),
                "one dispatch per attempt"
            );
            assert!(
                watch
                    .recover(due + Duration::from_secs(300), true, true)
                    .is_none(),
                "no second in-flight attach even after the cap"
            );
            now = due;
        }
        let mut pi = Pi::new(watch.target.cwd.clone().into());
        pi.state.model = Some(model("host", "chosen", "Chosen"));
        authoritative(&mut watch, pi, now);
        assert_eq!(
            watch.retry_failures, 0,
            "reset backoff only after authoritative success, not Opened"
        );
        let generation = watch.generation;
        pipe_update(
            &mut watch,
            Update::Ended(generation, "later drop".into()),
            now,
        );
        assert_eq!(watch.retry_at, Some(now + Duration::from_secs(1)));
    }

    #[test]
    fn recovery_rejects_all_old_pipe_updates_and_recreated_watch_generations() {
        let now = Instant::now();
        let (mut watch, _) = watch();
        let old = watch.generation;
        pipe_update(&mut watch, Update::Ended(old, "ended".into()), now);
        let (_, generation) = watch
            .recover(now + Duration::from_secs(1), true, true)
            .unwrap();
        let (input, _) = async_channel::unbounded();
        pipe_update(&mut watch, Update::Opened(old, input), now);
        pipe_update(
            &mut watch,
            Update::Record(
                old,
                json!({"type":"remote_snapshot", "data":Pi::new("/stale".into())}),
            ),
            now,
        );
        pipe_update(&mut watch, Update::Ended(old, "stale close".into()), now);
        assert_eq!(watch.generation, generation);
        assert!(watch.input.is_none());
        assert!(!watch.cached_snapshot && !watch.current && !watch.ready);
        assert!(watch.ended.is_none() && watch.retry_at.is_none());
        let recreated = Watch::new(watch.target.clone());
        assert_ne!(recreated.generation, old);
        assert_ne!(recreated.generation, generation);
    }

    #[test]
    fn invalid_snapshots_never_grant_cached_availability_or_handshake_readiness() {
        let now = Instant::now();
        let (mut watch, commands) = watch();
        let generation = watch.generation;
        pipe_update(
            &mut watch,
            Update::Record(generation, json!({"type":"remote_snapshot", "data":{}})),
            now,
        );
        assert!(!watch.cached_snapshot && !watch.current && !watch.ready);
        assert!(watch.project(SessionId(1), "pi".into()).is_none());
        assert!(commands.try_recv().is_err());
        let mut pi = Pi::new(watch.target.cwd.clone().into());
        pi.state.model = Some(model("host", "chosen", "Chosen"));
        authoritative(&mut watch, pi, now);
        assert!(watch.cached_snapshot && watch.current && watch.ready);
        watch.reset_transport();
        let generation = watch.generation;
        pipe_update(
            &mut watch,
            Update::Record(generation, json!({"type":"remote_snapshot", "data":{}})),
            now,
        );
        assert!(watch.cached_snapshot && !watch.current && !watch.ready);
    }

    #[test]
    fn review_follow_up_is_frozen_before_transport_admission() {
        let (mut watch, commands) = watch();
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        watch.pi.state.model = Some(model("host", "chosen", "Chosen"));

        Live::enqueue_with_mode(&mut watch, "review this next".into(), true).unwrap();
        let sent = commands.try_recv().unwrap();
        assert_eq!(sent["streamingBehavior"], "followUp");
        assert_eq!(
            watch.outbox[0].admission.unwrap().streaming_behavior,
            Some("followUp")
        );
    }

    #[test]
    fn automatic_recovery_preserves_frozen_admission_cancellation_and_child_follow() {
        let now = Instant::now();
        let (mut watch, first_pipe) = watch();
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        watch.pi.state.model = Some(model("host", "chosen", "Chosen"));
        watch.outbox.push("immutable cancelled steer".into());
        Live::flush(&mut watch);
        let first = first_pipe.try_recv().unwrap();
        assert_eq!(first["streamingBehavior"], "steer");
        watch.outbox[0].cancelled = true; // Cancellation waiting for its receipt.
        watch
            .cancellations
            .insert("another-submission".into(), false);
        watch
            .queue_receipts
            .insert("earlier-request".into(), "another-submission".into());
        watch
            .queue_request_ids
            .insert("another-submission".into(), "earlier-request".into());
        watch.following = Some("child-A".into());
        let old = watch.generation;
        pipe_update(&mut watch, Update::Ended(old, "channel ended".into()), now);
        let (_, generation) = watch
            .recover(now + Duration::from_secs(1), true, true)
            .unwrap();
        assert!(watch.outbox[0].cancelled);
        assert_eq!(
            watch.outbox[0].record(watch.outbox[0].admission.unwrap()),
            admission(first.clone())
        );
        assert!(!watch.cancellations["another-submission"]);
        assert_eq!(
            watch.queue_receipts["earlier-request"],
            "another-submission"
        );
        assert_eq!(
            watch.queue_request_ids["another-submission"],
            "earlier-request"
        );
        assert_eq!(watch.following.as_deref(), Some("child-A"));
        assert!(watch.subagent_refresh_pending);
        let (input, commands) = async_channel::unbounded();
        pipe_update(&mut watch, Update::Opened(generation, input), now);
        Live::flush(&mut watch);
        assert!(
            commands.try_recv().is_err(),
            "cached state cannot admit commands"
        );
        let mut pi = watch.pi.clone();
        pi.run = pi_core::session::RunState::Idle; // Do not refreeze as idle.
        authoritative(&mut watch, pi, now);
        let records: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        let retry = records
            .iter()
            .find(|record| record["type"] == "prompt")
            .unwrap();
        assert_ne!(first["id"], retry["id"]);
        assert_eq!(admission(first), admission(retry.clone()));
        assert!(
            records
                .iter()
                .any(|record| record["type"] == "cancel_submission"
                    && record["submissionId"] == "another-submission")
        );
        assert!(records.iter().any(
            |record| record["type"] == "get_subagent" && record["conversationId"] == "child-A"
        ));
        assert!(watch.outbox[0].cancelled);
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
        image_ready(&mut watch);
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
    fn a_prompt_in_the_acknowledgement_snapshot_gap_is_sent_as_steering_immediately() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.outbox.push("Start working".into());
        Live::flush(&mut watch);
        let first = sent.try_recv().unwrap();
        assert!(first.get("streamingBehavior").is_none());
        Live::record(
            &mut watch,
            &json!({"type":"response","id":first["id"],"command":"prompt","success":true}),
            "",
            "",
            &mut Vec::new(),
        );
        assert!(!watch.pi.busy(), "busy snapshot has not arrived");
        assert!(watch.accepted_turn.is_some());
        watch.outbox.push("Change direction".into());
        let projected = watch.projected_outbox();
        assert!(
            projected[0].starts_turn,
            "acknowledged admission remains a new turn"
        );
        assert!(
            !projected[1].starts_turn,
            "the next prompt must project as steering"
        );
        Live::flush(&mut watch);
        let second = sent.try_recv().unwrap();
        assert_eq!(second["streamingBehavior"], "steer");
        assert_eq!(second["message"], "Change direction");
        assert_eq!(second["requestId"], watch.outbox[0].prompt.request_id);
    }

    #[test]
    fn effective_busy_never_rewrites_an_already_attempted_idle_admission() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.outbox.push("Retry the same admission".into());
        Live::flush(&mut watch);
        let first = sent.try_recv().unwrap();
        assert!(first.get("streamingBehavior").is_none());
        watch.reset_transport();
        watch.accepted_turn = Some(AcceptedTurn {
            text: "Another admitted turn".into(),
            user_turns_before: 0,
        });
        watch.current = true;
        watch.ready = true;
        let (input, sent) = async_channel::unbounded();
        watch.input = Some(input);
        assert!(watch.effective_busy());
        assert!(watch.starts_turn(&watch.outbox[0]));
        Live::flush(&mut watch);
        let retry = sent.try_recv().unwrap();
        assert_eq!(admission(first), admission(retry));
    }

    #[test]
    fn an_acknowledged_idle_prompt_stays_a_new_turn_until_its_snapshot_arrives() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch
            .pi
            .apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Fix it"},
                {"role":"assistant","content":[{"type":"toolCall","id":"old-edit","name":"edit","arguments":{"path":"/Users/nick/repos/pi/src/app.rs","oldText":"old","newText":"new"}}]}
            ]}}))
            .unwrap();
        watch
            .pi
            .apply(&json!({"type":"tool_execution_start","toolCallId":"old-edit","toolName":"edit","args":{"path":"/Users/nick/repos/pi/src/app.rs","oldText":"old","newText":"new"}}))
            .unwrap();
        watch.pi.apply(&json!({"type":"agent_settled"})).unwrap();
        watch.outbox.push("One more thing".into());

        Live::flush(&mut watch);
        let request = sent.try_recv().unwrap();
        assert!(request.get("streamingBehavior").is_none());
        Live::record(
            &mut watch,
            &json!({"type":"agent_start"}),
            "",
            "",
            &mut Vec::new(),
        );
        Live::record(
            &mut watch,
            &json!({"type":"response","id":request["id"],"command":"prompt","success":true}),
            "",
            "",
            &mut Vec::new(),
        );
        assert!(watch.outbox.is_empty());
        assert!(watch.accepted_turn.is_some());

        let shown = projection::project(
            &watch.pi,
            projection::Facts {
                id: SessionId(1),
                cwd: &watch.target.cwd,
                folder: "~/repos/pi".into(),
                question: None,
                outbox: &watch.projected_outbox(),
                key: &watch.target.key,
            },
        );
        let latest = shown.turns.last().unwrap();
        assert_eq!(latest.prompt, "One more thing");
        assert!(
            latest
                .stages
                .iter()
                .all(|stage| stage.kind != crate::model::StageKind::Change)
        );

        let mut rebased = watch.pi.clone();
        rebased.messages.truncate(1);
        rebased.run = pi_core::session::RunState::Compacting;
        Live::record(
            &mut watch,
            &json!({"type":"remote_snapshot","data":serde_json::to_value(rebased).unwrap()}),
            "",
            "",
            &mut Vec::new(),
        );
        assert!(
            watch.accepted_turn.is_none(),
            "an authoritative compacted snapshot clears optimism even when its user count did not grow"
        );
    }

    #[test]
    fn a_genuine_collision_rejection_remains_recoverable_with_images_and_a_new_identity() {
        let (mut watch, sent) = watch();
        image_ready(&mut watch);
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

    fn queue_session(watch: &Watch) -> crate::model::Session {
        projection::project_queue_keys(
            &watch.pi,
            projection::Facts {
                id: SessionId(1),
                cwd: &watch.target.cwd,
                folder: String::new(),
                question: None,
                outbox: &watch.projected_outbox(),
                key: &watch.target.key,
            },
            |index| watch.queue_row_visible(index),
            |index| watch.queue_row_key(index),
        )
    }

    fn queue_ui(watch: &Watch) -> Vec<String> {
        queue_session(watch).queued
    }

    fn queue_watch() -> (Watch, async_channel::Receiver<Value>) {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        watch.pi.steering = vec!["duplicate".into()];
        watch.pi.follow_up = vec!["duplicate".into(), "third".into()];
        watch.pi.queued_submissions = vec!["41".into(), "42".into(), "43".into()];
        (watch, sent)
    }

    fn cancel_reply(watch: &mut Watch, cancel: &Value, success: bool) -> Option<String> {
        Live::record(
            watch,
            &json!({
                "type":"response", "id":cancel["id"], "command":"cancel_submission",
                "success":success, "error":"already started", "data":{}
            }),
            "",
            "",
            &mut Vec::new(),
        )
    }

    fn queue_snapshot(watch: &mut Watch, ids: &[&str], texts: &[&str]) {
        let mut pi = watch.pi.clone();
        pi.steering.clear();
        pi.follow_up = texts.iter().map(|text| (*text).to_owned()).collect();
        pi.queued_submissions = ids.iter().map(|id| (*id).to_owned()).collect();
        assert!(
            Live::record(
                watch,
                &json!({"type":"remote_snapshot", "data":pi}),
                "",
                "",
                &mut Vec::new()
            )
            .is_none()
        );
    }

    #[test]
    fn queue_key_stale_frame_removed_first_row_is_noop_not_next_duplicate() {
        let (mut watch, sent) = queue_watch();
        let rendered = queue_session(&watch);
        let first = rendered.queued_keys[0].as_ref().unwrap();
        queue_snapshot(&mut watch, &["42", "43"], &["duplicate", "third"]);
        Live::cancel_queue_key(&mut watch, first).unwrap();
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
        assert!(sent.try_recv().is_err());
        assert!(watch.cancellations.is_empty());
    }

    #[test]
    fn queue_key_double_tap_retained_row_never_cancels_next_row() {
        let (mut watch, sent) = queue_watch();
        let rendered = queue_session(&watch);
        let first = rendered.queued_keys[0].as_ref().unwrap();
        Live::cancel_queue_key(&mut watch, first).unwrap();
        let cancel = sent.try_recv().unwrap();
        assert_eq!(cancel["submissionId"], "41");
        Live::cancel_queue_key(&mut watch, first).unwrap();
        assert!(sent.try_recv().is_err());
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
        cancel_reply(&mut watch, &cancel, true);
        queue_snapshot(&mut watch, &["42", "43"], &["duplicate", "third"]);
        Live::cancel_queue_key(&mut watch, first).unwrap();
        assert!(sent.try_recv().is_err());
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
    }

    #[test]
    fn queue_key_reordered_ids_cancel_original_exact_duplicate() {
        let (mut watch, sent) = queue_watch();
        let rendered = queue_session(&watch);
        let first = rendered.queued_keys[0].as_ref().unwrap();
        queue_snapshot(
            &mut watch,
            &["42", "43", "41"],
            &["duplicate", "third", "duplicate"],
        );
        Live::cancel_queue_key(&mut watch, first).unwrap();
        assert_eq!(sent.try_recv().unwrap()["submissionId"], "41");
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
        assert_eq!(queue_session(&watch).queued_keys, rendered.queued_keys[1..]);
    }

    #[test]
    fn queue_key_local_receipt_and_snapshot_preserve_rendered_identity() {
        let (mut watch, sent) = queue_watch();
        watch.outbox.push("duplicate".into());
        let rendered = queue_session(&watch);
        let local = rendered.queued_keys[3].as_ref().unwrap();
        assert_eq!(
            local,
            &QueueKey::Local(watch.outbox[0].prompt.request_id.clone())
        );
        Live::flush(&mut watch);
        let prompt = sent.try_recv().unwrap();
        Live::record(
            &mut watch,
            &json!({"type":"response", "id":prompt["id"], "command":"prompt",
                "success":true, "data":{"submissionId":"44"}}),
            "",
            "",
            &mut Vec::new(),
        );
        assert!(watch.outbox.is_empty());
        queue_snapshot(
            &mut watch,
            &["44", "42", "43", "41"],
            &["duplicate", "duplicate", "third", "duplicate"],
        );
        assert_eq!(queue_session(&watch).queued_keys[0].as_ref(), Some(local));
        Live::cancel_queue_key(&mut watch, local).unwrap();
        assert_eq!(sent.try_recv().unwrap()["submissionId"], "44");
        assert_eq!(queue_ui(&watch), ["duplicate", "third", "duplicate"]);
        Live::cancel_queue_key(&mut watch, local).unwrap();
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn queue_key_local_tombstone_and_removed_outbox_double_taps_are_noops() {
        let (mut watch, sent) = queue_watch();
        watch
            .outbox
            .extend(["duplicate".into(), "duplicate".into()]);
        let rendered = queue_session(&watch);
        let local = rendered.queued_keys[3].as_ref().unwrap();
        Live::cancel_queue_key(&mut watch, local).unwrap();
        Live::cancel_queue_key(&mut watch, local).unwrap();
        assert_eq!(watch.outbox.len(), 1);
        assert_eq!(
            queue_session(&watch).queued_keys[3],
            rendered.queued_keys[4]
        );
        assert!(sent.try_recv().is_err());

        let remaining = rendered.queued_keys[4].as_ref().unwrap();
        Live::flush(&mut watch);
        let prompt = sent.try_recv().unwrap();
        Live::cancel_queue_key(&mut watch, remaining).unwrap();
        Live::cancel_queue_key(&mut watch, remaining).unwrap();
        assert!(watch.outbox[0].cancelled);
        Live::record(
            &mut watch,
            &json!({"type":"response", "id":prompt["id"], "command":"prompt",
                "success":true, "data":{"submissionId":"44"}}),
            "",
            "",
            &mut Vec::new(),
        );
        assert_eq!(sent.try_recv().unwrap()["submissionId"], "44");
        Live::cancel_queue_key(&mut watch, remaining).unwrap();
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn queue_cancel_immediately_filters_only_the_exact_duplicate_id() {
        let (mut watch, sent) = queue_watch();
        let before = serde_json::to_value(&watch.pi).unwrap();
        Live::cancel_queued(&mut watch, 1).unwrap();
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
        assert_eq!(serde_json::to_value(&watch.pi).unwrap(), before);
        assert!(watch.queue_row_visible(0));
        assert!(!watch.queue_row_visible(1));
        let cancel = sent.try_recv().unwrap();
        assert_eq!(cancel["submissionId"], "42");
        Live::flush(&mut watch);
        assert!(
            sent.try_recv().is_err(),
            "do not duplicate in-flight cancels"
        );
    }

    #[test]
    fn queue_cancel_rapid_x_uses_next_visible_id_then_local_row() {
        let (mut watch, sent) = queue_watch();
        watch.outbox.push("local".into());
        Live::cancel_queued(&mut watch, 0).unwrap();
        assert_eq!(sent.try_recv().unwrap()["submissionId"], "41");
        Live::cancel_queued(&mut watch, 0).unwrap();
        assert_eq!(sent.try_recv().unwrap()["submissionId"], "42");
        assert_eq!(queue_ui(&watch), ["third", "local"]);
        Live::cancel_queued(&mut watch, 1).unwrap();
        assert!(watch.outbox.is_empty());
        assert_eq!(queue_ui(&watch), ["third"]);
        assert!(
            sent.try_recv().is_err(),
            "cancelling cannot admit unrelated prompts"
        );
    }

    #[test]
    fn queue_cancel_snapshot_before_ack_cannot_retire_cancel_intent() {
        let (mut watch, sent) = queue_watch();
        Live::cancel_queued(&mut watch, 0).unwrap();
        let cancel = sent.try_recv().unwrap();
        queue_snapshot(&mut watch, &["42", "43"], &["duplicate", "third"]);
        assert_eq!(watch.cancellations.get("41"), Some(&false));
        queue_snapshot(
            &mut watch,
            &["41", "42", "43"],
            &["duplicate", "duplicate", "third"],
        );
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
        assert!(cancel_reply(&mut watch, &cancel, true).is_none());
        queue_snapshot(
            &mut watch,
            &["41", "42", "43"],
            &["duplicate", "duplicate", "third"],
        );
        assert_eq!(
            queue_ui(&watch),
            ["duplicate", "third"],
            "no stale flash after ack"
        );
        queue_snapshot(&mut watch, &["42", "43"], &["duplicate", "third"]);
        assert!(watch.cancellations.is_empty());
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
    }

    #[test]
    fn queue_cancel_absence_before_ack_still_waits_for_post_ack_absence() {
        let (mut watch, sent) = queue_watch();
        Live::cancel_queued(&mut watch, 0).unwrap();
        let cancel = sent.try_recv().unwrap();
        queue_snapshot(&mut watch, &["42", "43"], &["duplicate", "third"]);
        cancel_reply(&mut watch, &cancel, true);
        assert_eq!(watch.cancellations.get("41"), Some(&true));
        queue_snapshot(
            &mut watch,
            &["41", "42", "43"],
            &["duplicate", "duplicate", "third"],
        );
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
        queue_snapshot(&mut watch, &["42", "43"], &["duplicate", "third"]);
        assert!(watch.cancellations.is_empty());
    }

    #[test]
    fn queue_cancel_reconnect_retries_only_unacknowledged_exact_submission() {
        let (mut watch, sent) = queue_watch();
        Live::cancel_queued(&mut watch, 1).unwrap();
        let first = sent.try_recv().unwrap();
        watch.reset_transport();
        let (input, retried) = async_channel::unbounded();
        watch.input = Some(input);
        watch.current = true;
        watch.ready = true;
        Live::flush(&mut watch);
        let retry = retried.try_recv().unwrap();
        assert_eq!(retry["type"], "cancel_submission");
        assert_eq!(retry["submissionId"], "42");
        assert_ne!(retry["id"], first["id"]);
        assert!(retried.try_recv().is_err());
        assert!(
            cancel_reply(&mut watch, &first, false).is_none(),
            "old pipe cannot restore row"
        );
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
        cancel_reply(&mut watch, &retry, true);
        watch.reset_transport();
        let (input, retried) = async_channel::unbounded();
        watch.input = Some(input);
        watch.ready = true;
        Live::flush(&mut watch);
        assert!(
            retried.try_recv().is_err(),
            "acknowledged cancels must not retry"
        );
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
    }

    #[test]
    fn queue_cancel_failure_restores_authoritative_row_and_reports_error() {
        let (mut watch, sent) = queue_watch();
        Live::cancel_queued(&mut watch, 1).unwrap();
        let cancel = sent.try_recv().unwrap();
        assert!(
            cancel_reply(&mut watch, &cancel, false)
                .unwrap()
                .contains("already started")
        );
        assert_eq!(queue_ui(&watch), ["duplicate", "duplicate", "third"]);
        Live::flush(&mut watch);
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn queue_cancel_offline_intent_hides_and_retries_when_transport_returns() {
        let (mut watch, sent) = queue_watch();
        watch.reset_transport();
        Live::cancel_queued(&mut watch, 1).unwrap();
        assert_eq!(queue_ui(&watch), ["duplicate", "third"]);
        assert!(sent.try_recv().is_err());
        let (input, retried) = async_channel::unbounded();
        watch.input = Some(input);
        watch.ready = true;
        Live::flush(&mut watch);
        let cancel = retried.try_recv().unwrap();
        assert_eq!(cancel["type"], "cancel_submission");
        assert_eq!(cancel["submissionId"], "42");
        assert!(retried.try_recv().is_err());
    }

    #[test]
    fn queue_cancel_local_failure_restores_exact_backend_id_not_duplicate_text() {
        let (mut watch, sent) = queue_watch();
        watch.outbox.push("duplicate".into());
        Live::flush(&mut watch);
        let prompt = sent.try_recv().unwrap();
        Live::cancel_queued(&mut watch, 3).unwrap();
        Live::record(
            &mut watch,
            &json!({"type":"response", "id":prompt["id"],
            "command":"prompt", "success":true, "data":{"submissionId":"44"}}),
            "",
            "",
            &mut Vec::new(),
        );
        let cancel = sent.try_recv().unwrap();
        queue_snapshot(
            &mut watch,
            &["41", "42", "43", "44"],
            &["duplicate", "duplicate", "third", "duplicate"],
        );
        assert!(
            cancel_reply(&mut watch, &cancel, false)
                .unwrap()
                .contains("already started")
        );
        assert!(watch.outbox.is_empty());
        assert_eq!(
            queue_ui(&watch),
            ["duplicate", "duplicate", "third", "duplicate"]
        );
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn queue_cancel_preserves_other_prompts_original_admission_and_payload() {
        let (mut watch, sent) = queue_watch();
        watch.outbox.push("unrelated".into());
        Live::flush(&mut watch);
        let prompt = sent.try_recv().unwrap();
        assert_eq!(prompt["streamingBehavior"], "steer");
        Live::cancel_queued(&mut watch, 1).unwrap();
        assert_eq!(sent.try_recv().unwrap()["submissionId"], "42");
        watch.reset_transport();
        watch.pi.run = pi_core::session::RunState::Idle;
        let (input, retried) = async_channel::unbounded();
        watch.input = Some(input);
        watch.ready = true;
        Live::flush(&mut watch);
        let cancel = retried.try_recv().unwrap();
        assert_eq!(cancel["type"], "cancel_submission");
        assert_eq!(cancel["submissionId"], "42");
        assert_eq!(admission(prompt), admission(retried.try_recv().unwrap()));
        assert!(retried.try_recv().is_err());
    }

    #[test]
    fn queue_cancel_unsupported_authoritative_identity_stays_visible() {
        let (mut watch, sent) = queue_watch();
        watch.pi.queued_submissions.clear();
        assert!(
            Live::cancel_queued(&mut watch, 1)
                .unwrap_err()
                .contains("cannot remove")
        );
        assert_eq!(queue_ui(&watch), ["duplicate", "duplicate", "third"]);
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn queue_cancel_local_receipt_hides_same_id_in_backend_until_cancel_ack_and_absence() {
        let (mut watch, sent) = queue_watch();
        watch.outbox.push("duplicate".into());
        Live::flush(&mut watch);
        let prompt = sent.try_recv().unwrap();
        Live::cancel_queued(&mut watch, 3).unwrap();
        Live::record(
            &mut watch,
            &json!({"type":"response", "id":prompt["id"],
            "command":"prompt", "success":true, "data":{"submissionId":"44"}}),
            "",
            "",
            &mut Vec::new(),
        );
        let cancel = sent.try_recv().unwrap();
        assert_eq!(cancel["submissionId"], "44");
        assert_eq!(cancel["requestId"], prompt["requestId"]);
        queue_snapshot(
            &mut watch,
            &["41", "42", "43", "44"],
            &["duplicate", "duplicate", "third", "duplicate"],
        );
        assert_eq!(queue_ui(&watch), ["duplicate", "duplicate", "third"]);
        cancel_reply(&mut watch, &cancel, true);
        assert!(watch.outbox.is_empty());
        assert_eq!(queue_ui(&watch), ["duplicate", "duplicate", "third"]);
        queue_snapshot(
            &mut watch,
            &["41", "42", "43"],
            &["duplicate", "duplicate", "third"],
        );
        assert!(watch.cancellations.is_empty());
    }

    #[test]
    fn removing_one_queued_prompt_hides_it_immediately_and_cancels_after_delivery() {
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
        let prompt = sent.try_recv().unwrap();
        assert_eq!(prompt["type"], "prompt");
        Live::cancel_queued(&mut watch, 1).unwrap();
        assert!(watch.outbox[0].cancelled);
        assert!(watch.projected_outbox().is_empty());

        let accepted = json!({"type":"response","id":prompt["id"],"command":"prompt","success":true,"data":{"submissionId":"43"}});
        assert!(Live::record(&mut watch, &accepted, "", "", &mut Vec::new()).is_none());
        let cancel = sent.try_recv().unwrap();
        assert_eq!(cancel["type"], "cancel_submission");
        assert_eq!(cancel["submissionId"], "43");
        assert_eq!(cancel["requestId"], prompt["requestId"]);
        assert_eq!(watch.outbox.len(), 1, "a hidden tombstone remains");
        let cancelled = json!({"type":"response","id":cancel["id"],"command":"cancel_submission","success":true,"data":{}});
        assert!(Live::record(&mut watch, &cancelled, "", "", &mut Vec::new()).is_none());
        assert!(watch.outbox.is_empty());
    }

    #[test]
    fn a_hidden_cancel_tombstone_retries_after_the_transport_returns() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        watch.outbox.push("remove me".into());
        Live::flush(&mut watch);
        let prompt = sent.try_recv().unwrap();
        Live::cancel_queued(&mut watch, 0).unwrap();
        watch.input = None;
        let accepted = json!({"type":"response","id":prompt["id"],"command":"prompt","success":true,"data":{"submissionId":"44"}});
        Live::record(&mut watch, &accepted, "", "", &mut Vec::new());
        assert!(watch.outbox[0].cancelled);
        assert_eq!(watch.outbox[0].submission_id.as_deref(), Some("44"));

        let (input, retried) = async_channel::unbounded();
        watch.input = Some(input);
        Live::flush(&mut watch);
        let cancel = retried.try_recv().unwrap();
        assert_eq!(cancel["type"], "cancel_submission");
        assert_eq!(cancel["submissionId"], "44");
        let response = json!({"type":"response","id":cancel["id"],"command":"cancel_submission","success":true,"data":{}});
        Live::record(&mut watch, &response, "", "", &mut Vec::new());
        assert!(watch.outbox.is_empty());
    }

    #[test]
    fn an_older_helper_cancel_failure_never_stops_other_queued_work() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        watch.outbox.push("remove me".into());
        Live::flush(&mut watch);
        let prompt = sent.try_recv().unwrap();
        Live::cancel_queued(&mut watch, 0).unwrap();
        let accepted = json!({"type":"response","id":prompt["id"],"command":"prompt","success":true,"data":{"submissionId":"44"}});
        Live::record(&mut watch, &accepted, "", "", &mut Vec::new());
        let cancel = sent.try_recv().unwrap();
        let failed = json!({"type":"response","id":cancel["id"],"command":"cancel_submission","success":false,"error":"already started"});
        let notice = Live::record(&mut watch, &failed, "", "", &mut Vec::new()).unwrap();
        assert!(notice.contains("Update the helper"));
        assert_eq!(watch.projected_outbox()[0].text, "remove me");
        assert!(!watch.outbox[0].cancelled);
        assert!(
            sent.try_recv().is_err(),
            "must not send a conversation-wide abort"
        );
    }

    #[test]
    fn a_helper_without_durable_receipts_makes_the_uncancelled_row_visible_again() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        watch.outbox.push("still queued".into());
        Live::flush(&mut watch);
        let prompt = sent.try_recv().unwrap();
        Live::cancel_queued(&mut watch, 0).unwrap();
        let accepted = json!({"type":"response","id":prompt["id"],"command":"prompt","success":true,"data":{"disposition":"queued"}});
        assert!(
            Live::record(&mut watch, &accepted, "", "", &mut Vec::new())
                .unwrap()
                .contains("cannot cancel")
        );
        assert!(!watch.outbox[0].cancelled);
        assert_eq!(watch.outbox[0].submission_id.as_deref(), Some(""));
        assert_eq!(watch.projected_outbox()[0].text, "still queued");

        Live::record(
            &mut watch,
            &json!({"type":"queue_update","steering":[],"followUp":["still queued"]}),
            "",
            "",
            &mut Vec::new(),
        );
        assert_eq!(
            watch.outbox.len(),
            1,
            "without a receipt, duplicate text cannot safely identify the local row"
        );
    }

    #[test]
    fn compact_sends_an_action_without_admitting_prompt_text() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.pi.stats.context_usage = Some(pi_core::protocol::ContextUsage {
            tokens: Some(80_000),
            context_window: 200_000,
            percent: Some(40.),
        });
        let messages = watch.pi.messages.len();

        Live::compact_watch(&mut watch, Some("keep API names".into())).unwrap();
        let request = sent.try_recv().unwrap();
        assert_eq!(
            admission(request.clone()),
            json!({"type":"compact","customInstructions":"keep API names"})
        );
        assert!(watch.outbox.is_empty());
        assert_eq!(watch.pi.messages.len(), messages);
        assert!(watch.compacting());
        assert!(sent.try_recv().is_err());
        assert_eq!(
            Live::compact_watch(&mut watch, None).unwrap_err(),
            "This session is already compacting."
        );

        let response = json!({"type":"response","id":request["id"],"command":"compact","success":true,"data":{"estimatedTokensAfter":20_000}});
        assert!(Live::record(&mut watch, &response, "", "", &mut Vec::new()).is_none());
        assert!(!watch.compacting());
        let usage = watch.pi.stats.context_usage.as_ref().unwrap();
        assert_eq!(usage.tokens, Some(20_000));
        assert_eq!(usage.percent, Some(10.));
        assert_eq!(
            admission(sent.try_recv().unwrap()),
            json!({"type":"get_session_stats"})
        );
        Live::compact_watch(&mut watch, None).unwrap();
        assert_eq!(
            admission(sent.try_recv().unwrap()),
            json!({"type":"compact"})
        );
    }

    #[test]
    fn compaction_threshold_is_capability_gated_confirmed_and_never_queued_offline() {
        let (mut watch, sent) = watch();
        watch.target.backend = RemoteBackend::Durable;
        let old = json!({"enabled":true,"reserveTokens":16384,"backgroundTokens":32768});
        watch.pi.stats.compaction = Some(serde_json::from_value(old.clone()).unwrap());
        assert!(Live::set_compaction_threshold_watch(&mut watch, 70.).is_err());
        watch.pi.backend =
            pi_core::session::BackendInfo::Found(json!({"commands":["set_compaction_threshold"]}));
        assert!(Live::set_compaction_threshold_watch(&mut watch, 70.).is_err());
        watch.current = true;
        for percent in [f64::NAN, f64::INFINITY, 9.9, 95.1] {
            assert!(Live::set_compaction_threshold_watch(&mut watch, percent).is_err());
        }
        Live::set_compaction_threshold_watch(&mut watch, 70.).unwrap();
        let request = sent.try_recv().unwrap();
        assert_eq!(
            admission(request.clone()),
            json!({"type":"set_compaction_threshold","percent":70.})
        );
        assert_eq!(
            watch
                .pi
                .stats
                .compaction
                .as_ref()
                .unwrap()
                .background_tokens,
            32768
        );
        assert!(Live::set_compaction_threshold_watch(&mut watch, 60.).is_err());
        let failed = json!({"type":"response","id":request["id"],"command":"set_compaction_threshold","success":false,"error":"not committed"});
        assert_eq!(
            Live::record(&mut watch, &failed, "", "", &mut Vec::new()).as_deref(),
            Some("not committed")
        );
        assert_eq!(
            watch
                .pi
                .stats
                .compaction
                .as_ref()
                .unwrap()
                .background_tokens,
            32768
        );
        Live::set_compaction_threshold_watch(&mut watch, 70.).unwrap();
        let request = sent.try_recv().unwrap();
        let applied = json!({"type":"response","id":request["id"],"command":"set_compaction_threshold","success":true,
            "data":{"percent":70.,"compaction":{"enabled":true,"reserveTokens":16384,"backgroundTokens":43616}}});
        assert!(Live::record(&mut watch, &applied, "", "", &mut Vec::new()).is_none());
        assert_eq!(
            watch
                .pi
                .stats
                .compaction
                .as_ref()
                .unwrap()
                .background_tokens,
            43616
        );
        watch.current = false;
        assert!(Live::set_compaction_threshold_watch(&mut watch, 60.).is_err());
        assert!(sent.try_recv().is_err());
        assert!(watch.outbox.is_empty());
    }

    #[test]
    fn swarm_toggle_waits_for_capability_and_ack_and_never_queues_offline() {
        let (mut watch, sent) = watch();
        watch.target.backend = RemoteBackend::Durable;
        watch.pi.state.swarm_enabled = Some(true);
        assert!(Live::set_swarm_enabled_watch(&mut watch, false).is_err());
        watch.pi.backend =
            pi_core::session::BackendInfo::Found(json!({"commands":["set_swarm_enabled"]}));
        assert!(Live::set_swarm_enabled_watch(&mut watch, false).is_err());
        watch.current = true;
        Live::set_swarm_enabled_watch(&mut watch, false).unwrap();
        let request = sent.try_recv().unwrap();
        assert_eq!(request["type"], "set_swarm_enabled");
        assert_eq!(request["enabled"], false);
        assert_eq!(watch.pi.state.swarm_enabled, Some(true), "not optimistic");
        assert!(Live::set_swarm_enabled_watch(&mut watch, false).is_err());
        assert!(sent.try_recv().is_err());
        let failed = json!({"type":"response","id":request["id"],"command":"set_swarm_enabled","success":false,"error":"not committed"});
        assert_eq!(
            Live::record(&mut watch, &failed, "", "", &mut Vec::new()).as_deref(),
            Some("not committed")
        );
        assert_eq!(watch.pi.state.swarm_enabled, Some(true));
        Live::set_swarm_enabled_watch(&mut watch, false).unwrap();
        let request = sent.try_recv().unwrap();
        let applied = json!({"type":"response","id":request["id"],"command":"set_swarm_enabled","success":true,"data":{"enabled":false}});
        assert!(Live::record(&mut watch, &applied, "", "", &mut Vec::new()).is_none());
        assert_eq!(watch.pi.state.swarm_enabled, Some(false));
        watch.current = false;
        assert!(Live::set_swarm_enabled_watch(&mut watch, true).is_err());
        assert!(sent.try_recv().is_err());
        assert!(watch.outbox.is_empty());
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

    fn image_ready(watch: &mut Watch) {
        watch.current = true;
        watch.ready = true;
        watch.pi.backend =
            pi_core::session::BackendInfo::Found(json!({"features":["image_prompts"]}));
        watch.pi.state.model = Some(Model {
            input: vec!["text".into(), "image".into()],
            ..model("host", "vision", "Vision")
        });
    }

    fn image_tool(index: usize, ids: &[String]) -> pi_core::session::Tool {
        pi_core::session::Tool {
            id: format!("tool-{index}"),
            name: "read".into(),
            args: json!({"path":format!("/tmp/{index}.png")}),
            output: String::new(),
            diff: None,
            finished: true,
            started: None,
            is_error: false,
            ended_at: None,
            images: ids
                .iter()
                .map(|id| json!({"type":"image","data":"","imageId":id,"mimeType":"image/png"}))
                .collect(),
            details: Value::Null,
        }
    }

    fn image_response(request: &Value, data: &str) -> Value {
        json!({
            "type":"response", "id":request["id"], "command":"get_image", "success":true,
            "data":{"image":{"type":"image","data":data,"mimeType":"image/png"}}
        })
    }

    #[test]
    fn large_old_histories_do_not_fetch_images_until_one_is_demanded() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        let ids: Vec<String> = (0..160).map(|index| format!("{index:064x}")).collect();
        let mut snapshot = watch.pi.clone();
        snapshot.tools = ids
            .iter()
            .enumerate()
            .map(|(index, id)| image_tool(index, std::slice::from_ref(id)))
            .collect();
        assert!(
            Live::record(
                &mut watch,
                &json!({"type":"remote_snapshot","data":snapshot}),
                "",
                "",
                &mut Vec::new(),
            )
            .is_none()
        );
        assert!(
            sent.try_recv().is_err(),
            "a snapshot with lifetime-history images must not start hydration"
        );
        assert!(watch.images.is_empty());

        let visible = &ids[117];
        assert!(Live::request_image_watch(&mut watch, visible, None));
        let request = sent.try_recv().unwrap();
        assert_eq!(request["type"], "get_image");
        assert_eq!(request["imageId"], visible.as_str());
        assert!(
            sent.try_recv().is_err(),
            "only the demanded image is fetched"
        );
        assert_eq!(watch.images.len(), 1);
    }

    #[test]
    fn visible_image_hydration_is_bounded_and_advances_one_response_at_a_time() {
        let (mut watch, sent) = watch();
        let ids: Vec<String> = (0..7).map(|index| format!("{index:064x}")).collect();
        for id in &ids {
            assert!(Live::request_image_watch(&mut watch, id, None));
        }
        let initial: Vec<_> = std::iter::from_fn(|| sent.try_recv().ok()).collect();
        assert_eq!(initial.len(), MAX_IMAGE_REQUESTS_IN_FLIGHT);
        assert_eq!(watch.image_queue.len(), ids.len() - initial.len());

        assert!(
            Live::record(
                &mut watch,
                &image_response(&initial[0], "aGVsbG8="),
                "",
                "",
                &mut Vec::new(),
            )
            .is_none()
        );
        assert_eq!(
            watch.images[initial[0]["imageId"].as_str().unwrap()]
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .bytes
                .as_slice(),
            b"hello"
        );
        let next = sent.try_recv().unwrap();
        assert_eq!(next["type"], "get_image");
        assert!(sent.try_recv().is_err());
        assert_eq!(
            watch
                .sent
                .values()
                .filter(|sent| matches!(&sent.request, Request::Image(_)))
                .count(),
            MAX_IMAGE_REQUESTS_IN_FLIGHT
        );
    }

    #[test]
    fn duplicate_image_records_and_failed_responses_never_retry_flood() {
        let (mut watch, sent) = watch();
        let id = "a".repeat(64);
        watch.pi.tools = vec![
            image_tool(0, std::slice::from_ref(&id)),
            image_tool(1, &[id.clone(), id.clone()]),
        ];
        assert!(Live::request_image_watch(&mut watch, &id, None));
        assert!(!Live::request_image_watch(&mut watch, &id, None));
        let request = sent.try_recv().unwrap();
        assert!(sent.try_recv().is_err());

        assert_eq!(
            Live::record(
                &mut watch,
                &json!({"type":"response","id":request["id"],"command":"get_image",
                    "success":false,"error":"gone"}),
                "",
                "",
                &mut Vec::new(),
            ),
            None
        );
        assert!(
            watch.images[&id]
                .as_ref()
                .is_some_and(|result| result.as_ref().err().is_some_and(|error| error == "gone"))
        );
        assert!(!Live::request_image_watch(&mut watch, &id, None));
        assert!(sent.try_recv().is_err());
        watch.reset_transport();
        let (input, retried) = async_channel::unbounded();
        watch.input = Some(input);
        Live::flush_image_requests(&mut watch);
        assert!(retried.try_recv().is_err(), "terminal failures stay cached");
    }

    #[test]
    fn reconnect_requeues_only_demanded_in_flight_images_and_keeps_completed_bytes() {
        let (mut watch, first_pipe) = watch();
        let ids: Vec<String> = (10..13).map(|index| format!("{index:064x}")).collect();
        for id in &ids {
            assert!(Live::request_image_watch(&mut watch, id, Some("child-7")));
        }
        let first: Vec<_> = std::iter::from_fn(|| first_pipe.try_recv().ok()).collect();
        assert_eq!(first.len(), MAX_IMAGE_REQUESTS_IN_FLIGHT);
        assert!(
            first
                .iter()
                .all(|request| request["conversationId"] == "child-7")
        );

        Live::record(
            &mut watch,
            &image_response(&first[0], "ZG9uZQ=="),
            "",
            "",
            &mut Vec::new(),
        );
        let third = first_pipe.try_recv().unwrap();
        assert_eq!(third["imageId"], ids[2]);
        let completed = first[0]["imageId"].as_str().unwrap().to_owned();

        watch.reset_transport();
        assert!(watch.images[&completed].as_ref().unwrap().is_ok());
        let (input, retry_pipe) = async_channel::unbounded();
        watch.input = Some(input);
        Live::flush_image_requests(&mut watch);
        let retried: Vec<_> = std::iter::from_fn(|| retry_pipe.try_recv().ok()).collect();
        assert_eq!(retried.len(), 2);
        assert!(
            retried
                .iter()
                .all(|request| request["conversationId"] == "child-7")
        );
        assert!(
            retried
                .iter()
                .all(|request| request["imageId"] != completed)
        );
    }

    fn child_response(request: &Value, text: &str) -> Value {
        json!({"type":"response","id":request["id"],"command":"get_subagent","success":true,
            "data":{"conversationId":request["conversationId"],"messages":[{"role":"user","content":text}],
                "tools":[],"busy":false,"runId":null,"resumable":true}})
    }

    fn direct_input_watch(running: bool) -> (Watch, async_channel::Receiver<Value>) {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.following = Some("A".into());
        watch.pi.backend = pi_core::session::BackendInfo::Found(json!({
            "commands":["get_subagent","prompt_subagent"],
            "features":["subagents","subagent_input"]
        }));
        let mut child = Pi::new("/Users/nick/repos/pi".into());
        if running {
            child.apply(&json!({"type":"agent_start"})).unwrap();
            watch.subagent_runs.insert("A".into(), Some("run-7".into()));
        } else {
            watch.subagent_runs.insert("A".into(), None);
            watch.resumable_subagents.insert("A".into());
        }
        watch.subagents.insert("A".into(), Ok(child));
        (watch, sent)
    }

    #[test]
    fn direct_child_input_freezes_the_exact_run_and_retries_without_touching_parent_state() {
        let (mut watch, first_pipe) = direct_input_watch(true);
        assert_eq!(
            Live::subagent_input_target_watch(&watch, "A").unwrap(),
            SubagentInputTarget::Steer {
                run_id: "run-7".into()
            }
        );
        watch.subagent_outbox.insert(
            "A".into(),
            PendingSubagentInput {
                request_id: "child-request".into(),
                message: "change course".into(),
                target: SubagentInputTarget::Steer {
                    run_id: "run-7".into(),
                },
            },
        );
        Live::flush_subagent_inputs(&mut watch);
        let first = first_pipe.try_recv().unwrap();
        assert_eq!(first["type"], "prompt_subagent");
        assert_eq!(first["conversationId"], "A");
        assert_eq!(first["streamingBehavior"], "steer");
        assert_eq!(first["expectedRunId"], "run-7");
        assert_eq!(first["requestId"], "child-request");
        assert!(
            watch.outbox.is_empty(),
            "the parent prompt queue stays independent"
        );
        assert!(!watch.pi.busy(), "child input never changes the parent run");

        watch.reset_transport();
        let (input, retry_pipe) = async_channel::unbounded();
        watch.input = Some(input);
        watch.current = true;
        Live::flush_subagent_inputs(&mut watch);
        let retry = retry_pipe.try_recv().unwrap();
        assert_ne!(retry["id"], first["id"]);
        assert_eq!(retry["requestId"], first["requestId"]);
        assert_eq!(retry["expectedRunId"], first["expectedRunId"]);

        assert!(Live::record(
            &mut watch,
            &json!({"type":"response","id":retry["id"],"command":"prompt_subagent","success":true,
                "data":{"disposition":"queued","submissionId":"91"}}),
            "",
            "",
            &mut Vec::new(),
        )
        .is_none());
        assert!(watch.subagent_outbox.is_empty());
        assert_eq!(
            watch.accepted_subagent_inputs.get("A").map(String::as_str),
            Some("change course")
        );
        assert_eq!(retry_pipe.try_recv().unwrap()["type"], "get_subagent");
        assert!(!watch.pi.busy());
    }

    #[test]
    fn finished_child_input_is_follow_up_only_when_the_backend_explicitly_supports_it() {
        let (mut watch, sent) = direct_input_watch(false);
        assert_eq!(
            Live::subagent_input_target_watch(&watch, "A").unwrap(),
            SubagentInputTarget::FollowUp
        );
        watch.subagent_outbox.insert(
            "A".into(),
            PendingSubagentInput {
                request_id: "follow-up".into(),
                message: "check one more thing".into(),
                target: SubagentInputTarget::FollowUp,
            },
        );
        Live::flush_subagent_inputs(&mut watch);
        let request = sent.try_recv().unwrap();
        assert_eq!(request["conversationId"], "A");
        assert_eq!(request["streamingBehavior"], "followUp");
        assert!(request.get("expectedRunId").is_none());

        watch.pi.backend = pi_core::session::BackendInfo::Found(json!({
            "commands":["get_subagent"],"features":["subagents"]
        }));
        assert!(
            Live::subagent_input_target_watch(&watch, "A")
                .unwrap_err()
                .contains("does not support")
        );
    }

    #[test]
    fn child_progress_during_a_fetch_is_coalesced_into_one_catch_up_read() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        Live::select_subagent(&mut watch, Some("A".into()));
        let first = sent.try_recv().unwrap();
        let snapshot =
            json!({"type":"remote_snapshot","data":serde_json::to_value(&watch.pi).unwrap()});
        for _ in 0..3 {
            Live::record(&mut watch, &snapshot, "", "", &mut Vec::new());
        }
        assert!(
            sent.try_recv().is_err(),
            "only one child read may be in flight"
        );
        assert!(watch.subagent_refresh_pending);
        Live::record(
            &mut watch,
            &child_response(&first, "old progress"),
            "",
            "",
            &mut Vec::new(),
        );
        let catch_up = sent.try_recv().unwrap();
        assert_eq!(catch_up["conversationId"], "A");
        assert!(!watch.subagent_refresh_pending);
        Live::record(
            &mut watch,
            &child_response(&catch_up, "latest progress"),
            "",
            "",
            &mut Vec::new(),
        );
        assert!(
            sent.try_recv().is_err(),
            "responses alone must not start a polling loop"
        );
        assert_eq!(
            watch.subagents["A"].as_ref().unwrap().messages[0]["content"],
            "latest progress"
        );
    }

    #[test]
    fn switching_children_fetches_the_new_child_as_soon_as_the_old_read_finishes() {
        let (mut watch, sent) = watch();
        Live::select_subagent(&mut watch, Some("A".into()));
        let first = sent.try_recv().unwrap();
        Live::select_subagent(&mut watch, Some("B".into()));
        Live::select_subagent(&mut watch, Some("B".into()));
        assert!(sent.try_recv().is_err());
        Live::record(
            &mut watch,
            &child_response(&first, "A only"),
            "",
            "",
            &mut Vec::new(),
        );
        let next = sent.try_recv().unwrap();
        assert_eq!(next["conversationId"], "B");
        assert!(watch.subagents.contains_key("A"));
        assert!(!watch.subagents.contains_key("B"));
        Live::record(
            &mut watch,
            &child_response(&next, "B only"),
            "",
            "",
            &mut Vec::new(),
        );
        assert_eq!(
            watch.subagents["A"].as_ref().unwrap().messages[0]["content"],
            "A only"
        );
        assert_eq!(
            watch.subagents["B"].as_ref().unwrap().messages[0]["content"],
            "B only"
        );
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn closing_a_child_view_cancels_catch_up_but_not_its_in_flight_receipt() {
        let (mut watch, sent) = watch();
        Live::select_subagent(&mut watch, Some("A".into()));
        let first = sent.try_recv().unwrap();
        Live::fetch_subagent(&mut watch);
        assert!(watch.subagent_refresh_pending);
        Live::select_subagent(&mut watch, None);
        assert!(!watch.subagent_refresh_pending);
        Live::record(
            &mut watch,
            &child_response(&first, "cached result"),
            "",
            "",
            &mut Vec::new(),
        );
        assert!(sent.try_recv().is_err());
        assert!(watch.subagents["A"].is_ok());
    }

    #[test]
    fn a_child_fetch_resumes_on_reconnect_with_a_new_transport_identity() {
        let (mut watch, sent) = watch();
        Live::select_subagent(&mut watch, Some("A".into()));
        let first = sent.try_recv().unwrap();
        watch.reset_transport();
        assert!(watch.subagent_refresh_pending);
        let (input, reconnected) = async_channel::unbounded();
        watch.input = Some(input);
        watch.current = true;
        watch.ready = true;
        let snapshot =
            json!({"type":"remote_snapshot","data":serde_json::to_value(&watch.pi).unwrap()});
        Live::record(&mut watch, &snapshot, "", "", &mut Vec::new());
        let retry = reconnected.try_recv().unwrap();
        assert_eq!(retry["conversationId"], "A");
        assert_ne!(retry["id"], first["id"]);
        Live::record(
            &mut watch,
            &child_response(&first, "obsolete"),
            "",
            "",
            &mut Vec::new(),
        );
        assert!(!watch.subagents.contains_key("A"));
        Live::record(
            &mut watch,
            &child_response(&retry, "fresh"),
            "",
            "",
            &mut Vec::new(),
        );
        assert_eq!(
            watch.subagents["A"].as_ref().unwrap().messages[0]["content"],
            "fresh"
        );
        assert!(reconnected.try_recv().is_err());
    }

    #[test]
    fn child_refusals_are_cached_as_errors_and_mismatched_responses_cannot_consume_reads() {
        let (mut watch, sent) = watch();
        Live::select_subagent(&mut watch, Some("A".into()));
        let first = sent.try_recv().unwrap();
        Live::record(
            &mut watch,
            &json!({"type":"response","id":first["id"],"command":"prompt","success":false}),
            "",
            "",
            &mut Vec::new(),
        );
        assert_eq!(watch.sent.len(), 1);
        assert!(watch.subagents.is_empty());
        Live::record(
            &mut watch,
            &json!({"type":"response","id":first["id"],"command":"get_subagent","success":false,"error":"Timed out reading subagent"}),
            "",
            "",
            &mut Vec::new(),
        );
        assert_eq!(
            watch.subagents["A"].as_ref().unwrap_err(),
            "Timed out reading subagent"
        );
        assert!(watch.sent.is_empty());
        assert!(sent.try_recv().is_err());
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
    fn new_image_prompt_uses_fresh_session_models_and_preserves_rejected_bytes() {
        let (mut watch, sent) = watch();
        let prompt = Prompt::new(
            "Describe this".into(),
            vec![pi_core::protocol::ImageContent::new(
                "aW1hZ2U=".into(),
                "image/png",
            )],
        );
        watch.outbox.push(prompt.clone().into());
        let mut models = vec![Model {
            input: vec!["text".into()],
            ..model("host", "stale", "Stale")
        }];
        Live::record(
            &mut watch,
            &json!({"type":"remote_snapshot","data":Pi::new("/Users/nick/repos/pi".into())}),
            "vision",
            "",
            &mut models,
        );
        let ask = sent.try_recv().unwrap();
        assert_eq!(sent.try_recv().unwrap()["type"], "get_commands");
        assert!(sent.try_recv().is_err(), "images wait for fresh selection");
        Live::record(
            &mut watch,
            &json!({"type":"response","id":ask["id"],"command":"get_available_models","success":true,
                "data":{"models":[{"provider":"host","id":"vision","input":["text","image"]}]}}),
            "vision",
            "",
            &mut models,
        );
        let set = sent.try_recv().unwrap();
        assert_eq!(set["modelId"], "vision");
        assert!(
            sent.try_recv().is_err(),
            "images wait for model confirmation"
        );
        Live::record(
            &mut watch,
            &json!({"type":"response","id":set["id"],"command":"set_model","success":true,
                "data":{"provider":"host","id":"vision","input":["text","image"]}}),
            "vision",
            "",
            &mut models,
        );
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        let backend = sent.try_recv().unwrap();
        assert_eq!(backend["type"], "get_backend_info");
        assert!(
            sent.try_recv().is_err(),
            "backend image authority is still unknown"
        );
        Live::record(
            &mut watch,
            &json!({"type":"response","id":backend["id"],"command":"get_backend_info","success":true,
            "data":{"features":["image_prompts"]}}),
            "vision",
            "",
            &mut models,
        );
        let admission = sent.try_recv().unwrap();
        assert_eq!(admission["requestId"], prompt.request_id);
        assert_eq!(admission["images"], json!(prompt.images));
        Live::record(
            &mut watch,
            &json!({"type":"response","id":admission["id"],"command":"prompt","success":false,"error":"This model does not accept images"}),
            "vision",
            "",
            &mut models,
        );
        assert!(watch.outbox.is_empty());
        assert_eq!(
            watch.failed[0].prompt, prompt,
            "the backend's authoritative rejection remains recoverable"
        );
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
    fn obsolete_catalogs_cannot_replace_new_models_or_finish_their_refresh() {
        let mut models = vec![model("host", "current", "Current")];
        let mut loading = true;
        let mut error = None;
        apply_catalog(
            1,
            2,
            Ok(vec![model("host", "stale", "Stale")]),
            &mut models,
            &mut loading,
            &mut error,
        );
        apply_catalog(
            1,
            2,
            Err("old connection failed".into()),
            &mut models,
            &mut loading,
            &mut error,
        );
        assert_eq!(models[0].id, "current");
        assert!(loading);
        assert!(error.is_none());
        apply_catalog(
            2,
            2,
            Ok(vec![model("host", "fresh", "Fresh")]),
            &mut models,
            &mut loading,
            &mut error,
        );
        assert_eq!(models[0].id, "fresh");
        assert!(!loading);
        assert!(error.is_none());
        apply_catalog(
            2,
            2,
            Err("current connection failed".into()),
            &mut models,
            &mut loading,
            &mut error,
        );
        assert_eq!(models[0].id, "fresh");
        assert_eq!(error.as_deref(), Some("current connection failed"));
    }

    #[test]
    fn project_command_catalogs_ignore_stale_folder_replies() {
        let command = |name: &str| pi_core::protocol::SlashCommand {
            name: name.into(),
            description: None,
            source: "skill".into(),
            source_info: None,
        };
        let mut commands = vec![command("skill:current")];
        let mut path = Some("/repo/current".into());
        let mut loading = true;
        let mut error = None;
        apply_command_catalog(
            1,
            2,
            Some("/repo/stale".into()),
            Ok(vec![command("skill:stale")]),
            &mut commands,
            &mut path,
            &mut loading,
            &mut error,
        );
        assert_eq!(commands[0].name, "skill:current");
        assert_eq!(path.as_deref(), Some("/repo/current"));
        assert!(loading);

        apply_command_catalog(
            2,
            2,
            Some("/repo/new".into()),
            Ok(vec![command("skill:project")]),
            &mut commands,
            &mut path,
            &mut loading,
            &mut error,
        );
        assert_eq!(commands[0].name, "skill:project");
        assert_eq!(path.as_deref(), Some("/repo/new"));
        assert!(!loading);
        assert!(error.is_none());
    }

    #[test]
    fn cold_vision_session_uses_catalog_metadata_without_reselecting_its_model() {
        let (mut watch, sent) = watch();
        let mut pi = Pi::new("/Users/nick/repos/pi".into());
        pi.state.model = Some(model("host", "vision", "Vision"));
        pi.backend = pi_core::session::BackendInfo::Found(json!({"features":["image_prompts"]}));
        let mut models = Vec::new();
        Live::record(
            &mut watch,
            &json!({"type":"remote_snapshot","data":pi}),
            "text",
            "",
            &mut models,
        );
        let ask = sent.try_recv().unwrap();
        assert_eq!(ask["type"], "get_available_models");
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        assert_eq!(sent.try_recv().unwrap()["type"], "get_commands");
        let unknown = watch.require_images().unwrap_err();
        assert!(
            unknown.contains("capabilities have not loaded"),
            "{unknown}"
        );
        assert!(!unknown.contains("does not accept images"));
        Live::record(
            &mut watch,
            &json!({"type":"response","id":ask["id"],"command":"get_available_models","success":true,
                "data":{"models":[{"provider":"host","id":"vision","input":["text","image"]}]}}),
            "text",
            "",
            &mut models,
        );
        assert!(watch.require_images().is_ok());
        assert_eq!(watch.pi.state.model.as_ref().unwrap().id, "vision");
        assert!(
            sent.try_recv().is_err(),
            "catalog hydration must not change the selected model"
        );
    }

    #[test]
    fn image_checks_wait_for_reconnect_and_pending_model_changes_in_both_directions() {
        let (mut watch, sent) = watch();
        watch.current = true;
        watch.ready = true;
        watch.pi.backend =
            pi_core::session::BackendInfo::Found(json!({"features":["image_prompts"]}));
        watch.pi.state.model = Some(Model {
            input: vec!["text".into()],
            ..model("host", "text", "Text")
        });
        assert!(
            watch
                .require_images()
                .unwrap_err()
                .contains("does not accept images")
        );
        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"vision"}),
            Request::SetModel,
        )
        .unwrap();
        let set = sent.try_recv().unwrap();
        assert!(
            watch
                .require_images()
                .unwrap_err()
                .contains("selected model to be confirmed")
        );
        let mut models = Vec::new();
        Live::record(
            &mut watch,
            &json!({"type":"response","id":set["id"],"command":"set_model","success":true,
                "data":{"provider":"host","id":"vision","input":["text","image"]}}),
            "",
            "",
            &mut models,
        );
        assert!(watch.require_images().is_ok());
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"text"}),
            Request::SetModel,
        )
        .unwrap();
        let set = sent.try_recv().unwrap();
        assert!(
            watch
                .require_images()
                .unwrap_err()
                .contains("selected model to be confirmed")
        );
        Live::record(
            &mut watch,
            &json!({"type":"response","id":set["id"],"command":"set_model","success":true,
                "data":{"provider":"host","id":"text","input":["text"]}}),
            "",
            "",
            &mut models,
        );
        assert!(
            watch
                .require_images()
                .unwrap_err()
                .contains("does not accept images")
        );
        watch
            .pi
            .state
            .model
            .as_mut()
            .unwrap()
            .input
            .push("image".into());
        watch.reset_transport();
        assert!(
            watch
                .require_images()
                .unwrap_err()
                .contains("finish connecting")
        );
        let (input, _sent) = async_channel::unbounded();
        watch.input = Some(input);
        watch.current = true;
        watch.ready = true;
        assert!(watch.require_images().is_ok());
        watch.ready = false;
        assert!(
            watch
                .require_images()
                .unwrap_err()
                .contains("finish connecting")
        );
    }

    fn image_prompt() -> Prompt {
        Prompt::new(
            "full image message, not its label".into(),
            vec![
                pi_core::protocol::ImageContent::new("AAECA/8=".into(), "image/png"),
                pi_core::protocol::ImageContent::new("/wD+AQ==".into(), "image/jpeg"),
            ],
        )
    }

    fn model_ack(watch: &mut Watch, set: &Value, input: &[&str]) {
        Live::record(
            watch,
            &json!({"type":"response","id":set["id"],"command":"set_model","success":true,
            "data":{"provider":"host","id":"next","input":input}}),
            "",
            "",
            &mut Vec::new(),
        );
    }

    #[test]
    fn image_during_pending_selection_is_local_then_admitted_as_steer_with_exact_bytes() {
        let (mut watch, sent) = watch();
        image_ready(&mut watch);
        watch.pi.run = pi_core::session::RunState::Running;
        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"next"}),
            Request::SetModel,
        )
        .unwrap();
        let set = sent.try_recv().unwrap();
        let prompt = image_prompt();
        Live::enqueue(&mut watch, prompt.clone()).unwrap();
        assert_eq!(watch.outbox[0].prompt, prompt);
        assert!(watch.outbox[0].admission.is_none());
        assert_eq!(
            watch.projected_outbox()[0].key,
            Some(QueueKey::Local(prompt.request_id.clone()))
        );
        assert!(watch.failed.is_empty());
        assert!(sent.try_recv().is_err());
        model_ack(&mut watch, &set, &["text", "image"]);
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        let admission = sent.try_recv().unwrap();
        assert_eq!(admission["requestId"], prompt.request_id);
        assert_eq!(admission["message"], prompt.message);
        assert_eq!(admission["images"], json!(prompt.images));
        assert_eq!(admission["streamingBehavior"], "steer");
        Live::flush(&mut watch);
        assert!(sent.try_recv().is_err(), "one attempt per active pipe");
    }

    #[test]
    fn selected_text_only_after_ack_rejects_waiting_images_recoverably() {
        let (mut watch, sent) = watch();
        image_ready(&mut watch);
        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"next"}),
            Request::SetModel,
        )
        .unwrap();
        let set = sent.try_recv().unwrap();
        let prompt = image_prompt();
        Live::enqueue(&mut watch, prompt.clone()).unwrap();
        model_ack(&mut watch, &set, &["text"]);
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        assert!(sent.try_recv().is_err());
        assert!(watch.outbox.is_empty());
        assert_eq!(watch.failed[0].prompt, prompt);
        assert!(watch.failed[0].error.contains("does not accept images"));
        let recovered = Live::recover_failed(&mut watch, &prompt.request_id).unwrap();
        assert_eq!(recovered.images, prompt.images);
        assert_eq!(recovered.message, prompt.message);
        assert_ne!(recovered.request_id, prompt.request_id);
    }

    #[test]
    fn definite_image_rejection_at_enqueue_leaves_the_draft_and_queue_unchanged() {
        let (mut watch, sent) = watch();
        image_ready(&mut watch);
        watch.pi.state.model.as_mut().unwrap().input = vec!["text".into()];
        assert!(
            Live::enqueue(&mut watch, image_prompt())
                .unwrap_err()
                .contains("does not accept images")
        );
        assert!(watch.outbox.is_empty());
        image_ready(&mut watch);
        watch.pi.backend = pi_core::session::BackendInfo::Unsupported;
        assert!(
            Live::enqueue(&mut watch, image_prompt())
                .unwrap_err()
                .contains("older helper")
        );
        assert!(watch.failed.is_empty());
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn delayed_session_catalog_resumes_images_without_rerouting_and_preserves_fifo() {
        let (mut watch, sent) = watch();
        image_ready(&mut watch);
        watch.pi.state.model.as_mut().unwrap().input.clear();
        let prompt = image_prompt();
        Live::enqueue(&mut watch, prompt.clone()).unwrap();
        let catalog = sent.try_recv().unwrap();
        assert_eq!(catalog["type"], "get_available_models");
        Live::enqueue(&mut watch, "later text".into()).unwrap();
        for _ in 0..3 {
            Live::record(
                &mut watch,
                &json!({"type":"agent_start"}),
                "global-default",
                "",
                &mut Vec::new(),
            );
        }
        assert!(
            sent.try_recv().is_err(),
            "no polling reads or overtaking text"
        );
        let mut global = vec![model("host", "wrong", "Wrong default")];
        Live::record(
            &mut watch,
            &json!({"type":"response","id":catalog["id"],"command":"get_available_models","success":true,
            "data":{"models":[{"provider":"host","id":"vision","input":["text","image"]}]}}),
            "wrong",
            "",
            &mut global,
        );
        let admission = sent.try_recv().unwrap();
        assert_eq!(admission["type"], "prompt");
        assert_eq!(admission["requestId"], prompt.request_id);
        assert_eq!(admission["images"], json!(prompt.images));
        assert_eq!(sent.try_recv().unwrap()["message"], "later text");
        assert_eq!(watch.pi.state.model.as_ref().unwrap().id, "vision");
        assert!(sent.try_recv().is_err(), "no set_model rerouting");
    }

    #[test]
    fn unknown_backend_and_state_get_one_needed_read_then_resume_from_metadata() {
        let (mut watch, sent) = watch();
        image_ready(&mut watch);
        watch.pi.backend = pi_core::session::BackendInfo::Unknown;
        watch.pi.state.model = None;
        let prompt = image_prompt();
        Live::enqueue(&mut watch, prompt.clone()).unwrap();
        let state = sent.try_recv().unwrap();
        let backend = sent.try_recv().unwrap();
        assert_eq!(state["type"], "get_state");
        assert_eq!(backend["type"], "get_backend_info");
        for _ in 0..3 {
            Live::flush(&mut watch);
        }
        assert!(sent.try_recv().is_err());
        Live::record(
            &mut watch,
            &json!({"type":"response","id":state["id"],"command":"get_state","success":true,
            "data":{"model":{"provider":"host","id":"vision","input":["text","image"]}}}),
            "",
            "",
            &mut Vec::new(),
        );
        assert!(sent.try_recv().is_err());
        Live::record(
            &mut watch,
            &json!({"type":"response","id":backend["id"],"command":"get_backend_info","success":true,
            "data":{"features":["image_prompts"]}}),
            "",
            "",
            &mut Vec::new(),
        );
        let admission = sent.try_recv().unwrap();
        assert_eq!(admission["requestId"], prompt.request_id);
        assert_eq!(admission["images"], json!(prompt.images));
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn reconnect_waiting_image_resumes_from_authoritative_snapshot_not_set_model_ack() {
        let (mut watch, old_pipe) = watch();
        image_ready(&mut watch);
        watch.reset_transport();
        let prompt = image_prompt();
        Live::enqueue(&mut watch, prompt.clone()).unwrap();
        assert!(old_pipe.try_recv().is_err());
        let (input, sent) = async_channel::unbounded();
        watch.input = Some(input);
        let mut snapshot = watch.pi.clone();
        snapshot.run = pi_core::session::RunState::Running;
        Live::record(
            &mut watch,
            &json!({"type":"remote_snapshot","data":snapshot}),
            "wrong-default",
            "",
            &mut Vec::new(),
        );
        assert_eq!(sent.try_recv().unwrap()["type"], "get_available_models");
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        assert_eq!(sent.try_recv().unwrap()["type"], "get_commands");
        let admission = sent.try_recv().unwrap();
        assert_eq!(admission["requestId"], prompt.request_id);
        assert_eq!(admission["images"], json!(prompt.images));
        assert_eq!(admission["streamingBehavior"], "steer");
        assert_eq!(watch.pi.state.model.as_ref().unwrap().id, "vision");
    }

    #[test]
    fn failed_selection_preserves_complete_waiting_image_for_recovery() {
        let (mut watch, sent) = watch();
        image_ready(&mut watch);
        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"next"}),
            Request::SetModel,
        )
        .unwrap();
        let set = sent.try_recv().unwrap();
        let prompt = image_prompt();
        Live::enqueue(&mut watch, prompt.clone()).unwrap();
        assert_eq!(Live::record(&mut watch, &json!({"type":"response","id":set["id"],"command":"set_model","success":false,"error":"Selection failed"}), "", "", &mut Vec::new()).as_deref(), Some("Selection failed"));
        assert!(watch.outbox.is_empty());
        assert_eq!(watch.failed[0].prompt, prompt);
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn waiting_image_cancellation_uses_stable_local_key_without_touching_next_prompt() {
        let (mut watch, sent) = watch();
        image_ready(&mut watch);
        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"next"}),
            Request::SetModel,
        )
        .unwrap();
        let set = sent.try_recv().unwrap();
        let prompt = image_prompt();
        Live::enqueue(&mut watch, prompt.clone()).unwrap();
        Live::enqueue(&mut watch, "later".into()).unwrap();
        let key = QueueKey::Local(prompt.request_id);
        Live::cancel_queue_key(&mut watch, &key).unwrap();
        Live::cancel_queue_key(&mut watch, &key).unwrap();
        assert_eq!(watch.outbox.len(), 1);
        model_ack(&mut watch, &set, &["text", "image"]);
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        assert_eq!(sent.try_recv().unwrap()["message"], "later");
        assert!(sent.try_recv().is_err());
        assert!(watch.failed.is_empty());
    }

    #[test]
    fn attempted_image_retry_bypasses_new_capability_check_and_keeps_frozen_admission() {
        let (mut watch, first_pipe) = watch();
        image_ready(&mut watch);
        let prompt = image_prompt();
        Live::enqueue(&mut watch, prompt.clone()).unwrap();
        let first = first_pipe.try_recv().unwrap();
        watch.reset_transport();
        let (input, sent) = async_channel::unbounded();
        watch.input = Some(input);
        watch.current = true;
        watch.ready = true;
        watch.pi.run = pi_core::session::RunState::Running;
        watch.pi.backend = pi_core::session::BackendInfo::Unsupported;
        Live::configure(
            &mut watch,
            json!({"type":"set_model","provider":"host","modelId":"next"}),
            Request::SetModel,
        )
        .unwrap();
        let set = sent.try_recv().unwrap();
        Live::flush(&mut watch);
        assert!(
            sent.try_recv().is_err(),
            "unconfirmed selection still blocks retry"
        );
        model_ack(&mut watch, &set, &["text"]);
        assert_eq!(
            sent.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        let retry = sent.try_recv().unwrap();
        assert_ne!(first["id"], retry["id"]);
        assert_eq!(admission(first), admission(retry));
        assert!(watch.failed.is_empty());
        Live::flush(&mut watch);
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn unavailable_catalog_metadata_fails_images_recoverably_instead_of_polling_forever() {
        for success in [false, true] {
            let (mut watch, sent) = watch();
            image_ready(&mut watch);
            watch.pi.state.model.as_mut().unwrap().input.clear();
            let prompt = image_prompt();
            Live::enqueue(&mut watch, prompt.clone()).unwrap();
            let catalog = sent.try_recv().unwrap();
            Live::record(
                &mut watch,
                &json!({"type":"response","id":catalog["id"],"command":"get_available_models",
                "success":success,"error":"Read failed","data":{"models":[]}}),
                "",
                "",
                &mut Vec::new(),
            );
            assert_eq!(watch.failed[0].prompt, prompt);
            assert!(watch.outbox.is_empty());
            Live::flush(&mut watch);
            assert!(sent.try_recv().is_err());
            let recovered = Live::recover_failed(&mut watch, &prompt.request_id).unwrap();
            Live::enqueue(&mut watch, recovered).unwrap();
            assert_eq!(
                sent.try_recv().unwrap()["type"],
                "get_available_models",
                "explicit recovery may retry one fresh read"
            );
            Live::flush(&mut watch);
            assert!(sent.try_recv().is_err());
        }
    }

    #[test]
    fn unsupported_backend_after_metadata_read_preserves_waiting_image_bytes() {
        let (mut watch, sent) = watch();
        image_ready(&mut watch);
        watch.pi.backend = pi_core::session::BackendInfo::Unknown;
        let prompt = image_prompt();
        Live::enqueue(&mut watch, prompt.clone()).unwrap();
        let read = sent.try_recv().unwrap();
        assert_eq!(read["type"], "get_backend_info");
        Live::record(
            &mut watch,
            &json!({"type":"response","id":read["id"],"command":"get_backend_info",
            "success":false,"error":"Unsupported"}),
            "",
            "",
            &mut Vec::new(),
        );
        assert!(watch.outbox.is_empty());
        assert_eq!(watch.failed[0].prompt, prompt);
        assert!(watch.failed[0].error.contains("older helper"));
        assert!(sent.try_recv().is_err());
    }

    #[test]
    fn later_authoritative_snapshot_hydrates_a_waiting_image_without_waiting_for_catalog_ack() {
        let (mut watch, sent) = watch();
        image_ready(&mut watch);
        watch.pi.state.model.as_mut().unwrap().input.clear();
        let prompt = image_prompt();
        Live::enqueue(&mut watch, prompt.clone()).unwrap();
        let catalog = sent.try_recv().unwrap();
        let mut snapshot = watch.pi.clone();
        snapshot.state.model.as_mut().unwrap().input = vec!["text".into(), "image".into()];
        Live::record(
            &mut watch,
            &json!({"type":"remote_snapshot","data":snapshot}),
            "",
            "",
            &mut Vec::new(),
        );
        let admission = sent.try_recv().unwrap();
        assert_eq!(admission["requestId"], prompt.request_id);
        assert_eq!(admission["images"], json!(prompt.images));
        Live::record(
            &mut watch,
            &json!({"type":"response","id":catalog["id"],"command":"get_available_models",
            "success":false,"error":"Catalog unavailable"}),
            "",
            "",
            &mut Vec::new(),
        );
        assert!(
            watch.failed.is_empty(),
            "an unrelated catalog failure cannot reject confirmed or attempted images"
        );
        assert!(sent.try_recv().is_err());
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
