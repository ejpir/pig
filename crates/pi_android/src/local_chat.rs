//! On-device execution adapter for the ordinary Store/session/thread UI.
//! Session history is memory-only; no SSH transport, sample scripts, tools or files.
use crate::{app::PhoneApp, model::*, prompt::Prompt, store::Event};
#[cfg(test)]
use crate::{app::Route, store::Store};
use gpui::{Context, Window};
use pi_quickjs_backend_poc::{
    auth::{Provider, Vault},
    chat::{self, ChatTask, Request, SessionInput},
    chat_fixture::{self, Mode},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU32, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const FEATURES: &[&str] = &[
    "models",
    "thinking",
    "attachments",
    "streaming",
    "stop",
    "visualize",
    "manual_compaction",
    "auto_compaction",
];
// Byte budgets cover retained canonical branches, projected context, visible failed
// turns and attachment metadata. Compaction is not transcript deletion.
const SESSION_BYTES: usize = 24 * 1024 * 1024;
const LOCAL_BYTES: usize = 64 * 1024 * 1024;
fn bytes(value: &impl serde::Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX / 16, |v| v.len())
}
#[derive(serde::Serialize)]
struct TurnMeta {
    attachments: Vec<String>,
    excluded: Option<String>,
}
struct Conversation {
    pi: pi_core::session::Session,
    // Display/model-input compatibility view only. The full SDK checkpoint is
    // the sole inference authority, including edits, compactions and all branches.
    history: Vec<Value>,
    checkpoint: Option<Value>,
    compact: bool,
    poisoned: bool,
    pending: Option<Value>,
    task: Option<ChatTask>,
    status: &'static str,
    turns: Vec<TurnMeta>,
    record_error: bool,
}
pub(crate) struct Local {
    default_provider: Provider,
    pub synthetic: bool,
    vault: Arc<dyn Vault>,
    default_model: String,
    default_thinking: String,
    catalog: Vec<pi_core::protocol::Model>,
    metadata: Vec<Value>,
    conversations: HashMap<SessionId, Conversation>,
    pub mode: Mode,
}
fn response(pi: &mut pi_core::session::Session, command: &str, data: Value) -> Result<(), String> {
    pi.apply(&json!({"type":"response","command":command,"success":true,"data":data}))
        .map_err(|_| "Invalid local backend record".into())
}
fn provider_id(id: &str) -> Option<Provider> {
    match id {
        "anthropic" => Some(Provider::Claude),
        "openai-codex" => Some(Provider::Codex),
        _ => None,
    }
}
fn thinking_label(level: &str) -> &str {
    match level {
        "off" => "Off",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra high",
        "max" => "Max",
        other => other,
    }
}
impl Local {
    pub fn new(provider: Provider, vault: Arc<dyn Vault>, synthetic: bool) -> Self {
        let mut metadata = Vec::new();
        for p in [
            provider,
            if provider == Provider::Claude {
                Provider::Codex
            } else {
                Provider::Claude
            },
        ] {
            let mut models = chat::catalog(p).as_array().unwrap().clone();
            models.sort_by_key(|m| m["id"] != chat::models(p)[0].0);
            metadata.extend(models);
        }
        let catalog: Vec<pi_core::protocol::Model> =
            serde_json::from_value(json!(metadata)).expect("pinned model catalog");
        Self {
            default_provider: provider,
            synthetic,
            vault,
            default_model: chat::models(provider)[0].0.clone(),
            default_thinking: "high".into(),
            catalog,
            metadata,
            conversations: HashMap::new(),
            mode: Mode::Reply,
        }
    }
    fn metadata(&self, provider: Provider, model: &str) -> &Value {
        self.metadata
            .iter()
            .find(|m| m["provider"] == provider.id() && m["id"] == model)
            .expect("validated model")
    }
    fn clamp(&self, provider: Provider, model: &str, level: &str) -> String {
        self.metadata(provider, model)["clampedThinkingLevels"][level]
            .as_str()
            .expect("known thinking level")
            .into()
    }
    fn raw_levels(&self, provider: Provider, model: &str) -> Vec<String> {
        self.metadata(provider, model)["supportedThinkingLevels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().into())
            .collect()
    }
    fn retained_bytes(&self) -> usize {
        self.conversations
            .values()
            .map(Conversation::retained_bytes)
            .sum()
    }
    fn admit_bytes(&self, id: SessionId, added: usize) -> Result<(), String> {
        let current = self
            .conversations
            .get(&id)
            .ok_or("Session no longer available")?;
        if current.poisoned {
            return Err("Session state could not be synchronized. Start a new session; the transcript and draft were kept.".into());
        }
        if current.retained_bytes().saturating_add(added) > SESSION_BYTES
            || self.retained_bytes().saturating_add(added) > LOCAL_BYTES
        {
            return Err("Local session memory limit reached. Delete an old session or start a new one; your draft was kept. Compaction reduces model context, not saved transcript size.".into());
        }
        Ok(())
    }
    pub fn create(&mut self) -> Result<Session, String> {
        if self.conversations.len() >= 8 {
            return Err(
                "Delete an old chat before starting another (eight in-memory sessions maximum)."
                    .into(),
            );
        }
        static NEXT: AtomicU32 = AtomicU32::new(0x8000_0000);
        let id = SessionId(
            NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .map_err(|_| "Session identity limit")?,
        );
        let mut pi = pi_core::session::Session::default();
        response(
            &mut pi,
            "get_state",
            json!({"sessionId":format!("phone-{}",id.0),"sessionFile":null,
            "model":self.current_model(None),"thinkingLevel":self.clamp(self.default_provider,&self.default_model,&self.default_thinking),
            "isStreaming":false,"isCompacting":false,"isBashRunning":false,"autoCompactionEnabled":true}),
        )?;
        response(&mut pi, "get_messages", json!({"messages":[]}))?;
        response(
            &mut pi,
            "get_available_models",
            json!({"models":self.catalog}),
        )?;
        response(
            &mut pi,
            "get_available_thinking_levels",
            json!({"levels":self.raw_levels(self.default_provider,&self.default_model)}),
        )?;
        response(&mut pi, "get_active_tools", json!({"activeTools":[]}))?;
        response(&mut pi, "get_commands", json!({"commands":[]}))?;
        response(
            &mut pi,
            "get_backend_info",
            json!({"backend":"quickjs","commands":["prompt","abort","set_model","set_thinking_level","get_messages","get_state","get_available_models","get_available_thinking_levels","compact","get_session_stats"],"features":FEATURES,"uiCapabilities":FEATURES}),
        )?;
        self.conversations.insert(
            id,
            Conversation {
                pi,
                history: Vec::new(),
                checkpoint: None,
                compact: false,
                poisoned: false,
                pending: None,
                task: None,
                status: "idle",
                turns: Vec::new(),
                record_error: false,
            },
        );
        Ok(self.project(id))
    }
    pub fn current_model(&self, id: Option<SessionId>) -> Option<&pi_core::protocol::Model> {
        if let Some(id) = id {
            return self.conversations.get(&id)?.pi.state.model.as_ref();
        }
        self.catalog
            .iter()
            .find(|m| m.provider == self.default_provider.id() && m.id == self.default_model)
    }
    pub fn provider(&self, id: Option<SessionId>) -> Provider {
        self.current_model(id).map_or(self.default_provider, |m| {
            provider_id(&m.provider).expect("known provider")
        })
    }
    pub fn model(&self, id: Option<SessionId>) -> String {
        self.current_model(id)
            .map_or_else(|| self.default_model.clone(), |m| m.id.clone())
    }
    pub fn model_settings(&self, id: Option<SessionId>) -> (String, String) {
        let model = self.current_model(id).expect("local model");
        let thinking = id
            .and_then(|id| self.conversations.get(&id))
            .map_or(self.default_thinking.as_str(), |c| {
                c.pi.state.thinking_level.as_str()
            });
        (
            model.name.clone().unwrap_or_else(|| model.id.clone()),
            thinking_label(thinking).into(),
        )
    }
    pub fn models(&self) -> &[pi_core::protocol::Model] {
        &self.catalog
    }
    pub fn thinking_levels(&self, id: Option<SessionId>) -> Vec<String> {
        self.raw_levels(self.provider(id), &self.model(id))
            .iter()
            .map(|l| thinking_label(l).to_owned())
            .collect()
    }
    pub fn set_thinking(&mut self, id: Option<SessionId>, label: &str) -> Result<(), String> {
        let level = self
            .raw_levels(self.provider(id), &self.model(id))
            .into_iter()
            .find(|l| thinking_label(l) == label)
            .ok_or("Thinking level is not supported by this model")?;
        if let Some(id) = id {
            let c = self
                .conversations
                .get_mut(&id)
                .ok_or("Session no longer available")?;
            if c.task.is_some() {
                return Err("Stop the current reply before changing thinking level".into());
            }
            c.pi.apply(&json!({"type":"thinking_level_changed","level":level}))
                .map_err(|_| "Invalid thinking configuration")?;
        } else {
            self.default_thinking = level;
        }
        Ok(())
    }
    pub fn set_model(
        &mut self,
        id: Option<SessionId>,
        provider: &str,
        model: &str,
    ) -> Result<(), String> {
        let selected = self
            .catalog
            .iter()
            .find(|m| m.provider == provider && m.id == model)
            .cloned()
            .ok_or("Model not available for this provider")?;
        let old = id
            .and_then(|id| self.conversations.get(&id))
            .map_or(&self.default_thinking, |c| &c.pi.state.thinking_level);
        let chosen_provider = provider_id(provider).ok_or("Unknown provider")?;
        let level = self.clamp(chosen_provider, model, old);
        let levels = self.raw_levels(chosen_provider, model);
        if let Some(id) = id {
            let c = self
                .conversations
                .get_mut(&id)
                .ok_or("Session no longer available")?;
            if c.task.is_some() {
                return Err("Stop the current reply before changing models".into());
            }
            if !selected.input.iter().any(|v| v == "image")
                && c.history.iter().any(|m| {
                    m["role"] == "user"
                        && m["content"]
                            .as_array()
                            .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "image"))
                })
            {
                return Err("This conversation contains images. Choose an image-capable model or start a new chat.".into());
            }
            response(&mut c.pi, "set_model", json!(selected))?;
            c.pi.apply(&json!({"type":"thinking_level_changed","level":level}))
                .map_err(|_| "Invalid model configuration")?;
            response(
                &mut c.pi,
                "get_available_thinking_levels",
                json!({"levels":levels}),
            )?;
        } else {
            self.default_provider = chosen_provider;
            self.default_model = model.into();
            self.default_thinking = level;
        }
        Ok(())
    }
    pub fn supports(&self, feature: &str) -> bool {
        FEATURES.contains(&feature)
    }
    pub fn commands(&self, id: SessionId) -> Option<&[pi_core::protocol::SlashCommand]> {
        Some(self.conversations.get(&id)?.pi.commands.as_slice())
    }
    pub fn running(&self) -> bool {
        self.conversations.values().any(|c| c.task.is_some())
    }
    pub fn stopping(&self, id: SessionId) -> bool {
        self.conversations
            .get(&id)
            .and_then(|c| c.task.as_ref())
            .is_some_and(|t| t.snapshot().status == "stopping")
    }
    pub fn leave_except(&self, shown: Option<SessionId>) {
        for (id, c) in &self.conversations {
            if Some(*id) != shown
                && let Some(t) = &c.task
            {
                t.cancel();
            }
        }
    }
    pub fn cancel(&self) {
        self.leave_except(None);
    }
    pub fn stop(&self, id: SessionId) -> Result<(), String> {
        let c = self
            .conversations
            .get(&id)
            .ok_or("Session no longer available")?;
        if let Some(t) = &c.task {
            t.cancel();
        }
        Ok(())
    }
    pub fn remove(&mut self, id: SessionId) {
        self.conversations.remove(&id);
    }
    pub fn send(
        &mut self,
        session: &mut Session,
        prompt: Prompt,
        attachments: Vec<String>,
    ) -> Result<(), String> {
        if self.running() {
            return Err(
                "A local reply is still running or stopping. Wait or press Stop first.".into(),
            );
        }
        if (prompt.message.trim().is_empty() && prompt.images.is_empty())
            || prompt.message.len() > 2 * 1024 * 1024 + 32 * 1024
            || attachments.len() > 8
        {
            return Err("Message/session limit reached. Shorten the message or start a new session. Your draft was kept.".into());
        }
        // A new user can be retained in UI + checkpoint entries + projection +
        // compatibility view. Reserve bounded assistant/summary/metadata space too.
        self.admit_bytes(
            session.id,
            (prompt.message.len() + bytes(&prompt.images) + bytes(&attachments))
                .saturating_mul(4)
                .saturating_add(2 * 1024 * 1024),
        )?;
        let c = self
            .conversations
            .get_mut(&session.id)
            .ok_or("Session no longer available")?;
        if !prompt.images.is_empty()
            && !c
                .pi
                .state
                .model
                .as_ref()
                .is_some_and(|m| m.input.iter().any(|i| i == "image"))
        {
            return Err("This model does not accept images. Choose another model or remove the images; your draft was kept.".into());
        }
        let mut content = vec![json!({"type":"text","text":prompt.message})];
        content.extend(
            prompt
                .images
                .into_iter()
                .map(|image| serde_json::to_value(image).expect("image content")),
        );
        let user = json!({"role":"user","content":content,"timestamp":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64});
        let mut messages = c.history.clone();
        messages.push(user.clone());
        if serde_json::to_vec(&messages).map_or(true, |m| m.len() > 8 * 1024 * 1024) {
            return Err("Session attachment/context limit reached. Start a new session; your draft was kept.".into());
        }
        let request = Request {
            provider: provider_id(&c.pi.state.model.as_ref().unwrap().provider)
                .expect("known provider"),
            model: c.pi.state.model.as_ref().unwrap().id.clone(),
            thinking_level: c.pi.state.thinking_level.clone(),
            messages,
        };
        if c.checkpoint.as_ref().is_some_and(|cp| {
            bytes(&cp["fileEntries"])
                .saturating_add(bytes(&user))
                .saturating_add(2 * 1024 * 1024)
                > 8 * 1024 * 1024
                || cp["fileEntries"].as_array().unwrap().len() > 19984
        }) {
            return Err("Canonical session memory limit reached. Start a new session; your transcript and draft were kept.".into());
        }
        let input = SessionInput {
            session_id: c.pi.state.session_id.clone(),
            checkpoint: c.checkpoint.clone(),
            ..Default::default()
        };
        let task=if self.synthetic {chat_fixture::start_session(request,input,self.vault.clone(),self.mode)} else {ChatTask::start_session(request,input,self.vault.clone())}
            .map_err(|_|"Could not start the reply. Check message limits and account availability; your draft was kept.".to_owned())?;
        // Same records consumed by the SSH frontend. No invented UI turn/summary.
        c.pi.apply(&json!({"type":"message_end","message":user}))
            .map_err(|_| "Invalid local prompt")?;
        c.pi.apply(&json!({"type":"agent_start"}))
            .map_err(|_| "Invalid local admission")?;
        c.task = Some(task);
        c.pending = Some(user);
        c.status = "preparing";
        c.compact = false;
        c.record_error = false;
        c.turns.push(TurnMeta {
            attachments,
            excluded: None,
        });
        *session = self.project(session.id);
        Ok(())
    }
    pub fn compact(
        &mut self,
        session: &mut Session,
        instructions: Option<String>,
    ) -> Result<(), String> {
        if self.running() {
            return Err(
                "A local operation is still running or stopping. Wait or press Stop first.".into(),
            );
        }
        self.admit_bytes(session.id, 2 * 1024 * 1024)?;
        let c = self
            .conversations
            .get_mut(&session.id)
            .ok_or("Session no longer available")?;
        let model = c.pi.state.model.as_ref().unwrap();
        let request = Request {
            provider: provider_id(&model.provider).unwrap(),
            model: model.id.clone(),
            thinking_level: c.pi.state.thinking_level.clone(),
            messages: c.history.clone(),
        };
        if c.checkpoint.as_ref().is_some_and(|cp| {
            bytes(&cp["fileEntries"]).saturating_add(2 * 1024 * 1024) > 8 * 1024 * 1024
                || cp["fileEntries"].as_array().unwrap().len() > 19984
        }) {
            return Err("Canonical session memory limit reached. Start a new session; your transcript and draft were kept.".into());
        }
        let input = SessionInput {
            session_id: c.pi.state.session_id.clone(),
            checkpoint: c.checkpoint.clone(),
            compact: true,
            custom_instructions: instructions,
        };
        let task=if self.synthetic {chat_fixture::start_session(request,input,self.vault.clone(),self.mode)} else {ChatTask::start_session(request,input,self.vault.clone())}
            .map_err(|_|"Could not start compaction. Check account availability and instruction limits; your draft was kept.")?;
        c.task = Some(task);
        c.compact = true;
        c.record_error = false;
        c.status = "compacting";
        c.pi.apply(&json!({"type":"compaction_start","reason":"manual"}))
            .map_err(|_| "Invalid compact admission")?;
        *session = self.project(session.id);
        Ok(())
    }
    pub fn project(&self, id: SessionId) -> Session {
        let c = &self.conversations[&id];
        let mut session = crate::projection::project(
            &c.pi,
            crate::projection::Facts {
                id,
                cwd: "",
                folder: String::new(),
                question: None,
                outbox: &[],
                key: &format!("local-{}", id.0),
            },
        );
        session.local = true;
        session.project = "Local".into();
        session.details.session_file = "In memory only · lost on restart".into();
        session.activity = if c.compact && c.task.is_some() && c.status != "stopping" {
            "Compacting…"
        } else {
            status_text(c.status)
        }
        .into();
        for (turn, meta) in session.turns.iter_mut().zip(&c.turns) {
            turn.attachments = meta.attachments.clone();
            turn.excluded = meta.excluded.clone();
        }
        session
    }
    pub fn tick(&mut self, sessions: &mut [Session], _elapsed: Duration) -> Vec<Event> {
        let mut events = Vec::new();
        for session in sessions {
            let Some(c) = self.conversations.get_mut(&session.id) else {
                continue;
            };
            let Some(task) = &c.task else {
                continue;
            };
            let finished = task.is_finished();
            let snapshot = task.snapshot();
            c.status = snapshot.status;
            for record in task.take_records() {
                if c.pi.apply(&record).is_err() {
                    c.record_error = true;
                    task.cancel();
                }
            }
            if finished {
                // Pending is a transient admission copy, not settled retention.
                c.pending = None;
                let (checkpoint, stats, _) = task.take_session_result();
                if let Some(checkpoint) = checkpoint
                    && !c.record_error
                {
                    let old_entries = c
                        .checkpoint
                        .as_ref()
                        .and_then(|cp| cp["fileEntries"].as_array())
                        .map_or(0, Vec::len);
                    for entry in checkpoint["fileEntries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .skip(old_entries)
                    {
                        if entry["type"] == "compaction" {
                            c.pi.messages.push(json!({"role":"compactionSummary","summary":entry["summary"],"timestamp":entry["timestamp"]}));
                        }
                    }
                    c.history = checkpoint["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|m| m["role"] != "system")
                        .cloned()
                        .collect();
                    c.checkpoint = Some(checkpoint);
                    if let Some(stats) = stats {
                        let _ = c.pi.apply(&json!({"type":"stats_update","stats":stats}));
                    }
                    if c.retained_bytes() > SESSION_BYTES {
                        c.status = "sync_failed";
                        c.poisoned = true;
                    }
                }
                if c.status == "sync_failed" || c.record_error {
                    c.poisoned = true;
                    c.status = "sync_failed";
                }
                if c.compact {
                    let success = c.status == "done" && !c.record_error;
                    let _=c.pi.apply(&json!({"type":"response","command":"compact","success":success,"error":if success {Value::Null} else {json!(status_text(c.status))}}));
                    events.push(Event::Compacted(
                        session.id,
                        if success {
                            None
                        } else {
                            Some(status_text(c.status).into())
                        },
                    ));
                } else if c.status != "done" || c.record_error {
                    let stop = if c.status == "stopped" {
                        "aborted"
                    } else {
                        "error"
                    };
                    let mut last=c.pi.messages.last().filter(|m|m["role"]=="assistant").cloned().unwrap_or(json!({"role":"assistant","content":[],"timestamp":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64}));
                    last["stopReason"] = json!(stop);
                    last["errorMessage"] = json!(status_text(c.status));
                    if c.pi
                        .messages
                        .last()
                        .is_some_and(|m| m["role"] == "assistant")
                    {
                        c.pi.messages.pop();
                    }
                    let _ = c.pi.apply(&json!({"type":"message_start","message":last}));
                    let _ = c.pi.apply(&json!({"type":"message_end","message":last}));
                    if let Some(meta) = c.turns.last_mut() {
                        meta.excluded = Some(format!(
                            "{} This turn is excluded from later requests.",
                            status_text(c.status)
                        ));
                    }
                }
                c.pending = None;
                c.task = None;
                let _ = c.pi.apply(&json!({"type":"agent_settled"}));
                if !c.compact && c.status != "stopped" {
                    events.push(Event::Finished(session.id));
                }
            }
            *session = self.project(session.id);
        }
        events
    }
}
impl Conversation {
    fn retained_bytes(&self) -> usize {
        bytes(&self.pi.messages)
            .saturating_add(bytes(&self.history))
            .saturating_add(bytes(&self.checkpoint))
            .saturating_add(bytes(&self.turns))
            .saturating_add(bytes(&self.pending))
    }
}
fn status_text(status: &str) -> &'static str {
    match status {
        "idle" => "Ready",
        "preparing" => "Checking account…",
        "streaming" => "Replying…",
        "compacting" => "Compacting…",
        "compaction_failed" => {
            "Context summarization did not complete. Previous context was kept; check connectivity/account and try again."
        }
        "nothing_to_compact" => "Nothing to compact yet (or already compacted).",
        "context_full" => {
            "Input still exceeds the model context after compaction. Shorten the message or choose a larger-context model."
        }
        "sync_failed" => {
            "Session state could not be synchronized. Start a new session; the transcript was kept."
        }
        "stopping" => "Stopping; waiting for account refresh to settle…",
        "done" => "Reply complete",
        "stopped" => "Stopped. Partial turns are not included in later requests.",
        "account_changed" => "Account signed out or changed. Check Local providers.",
        "auth_error" => {
            "Provider rejected authorization. Check / refresh the account or sign in again."
        }
        "rate_limited" => "Provider usage limit reached. Wait before sending again.",
        "request_rejected" => {
            "Provider rejected this request/model. Try another model or a new chat."
        }
        _ => "Could not finish the reply. Check connectivity/account, then try again.",
    }
}
// Independent coalescing writer: synthetic test geometry/status only, never chat text.
type Publication = (PathBuf, Option<Value>);
struct Telemetry {
    pending: Arc<Mutex<Option<Publication>>>,
    wake: mpsc::SyncSender<()>,
}
fn publish(path: PathBuf, value: Option<Value>) {
    static WRITER: OnceLock<Telemetry> = OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let pending = Arc::new(Mutex::new(None::<Publication>));
        let work = pending.clone();
        let (wake, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                let item = work.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some((path, value)) = item {
                    if let Some(value) = value {
                        if std::fs::create_dir_all(&path).is_ok()
                            && std::fs::write(path.join("ui.new"), value.to_string()).is_ok()
                        {
                            let _ = std::fs::rename(path.join("ui.new"), path.join("ui.json"));
                        }
                    } else {
                        let _ = std::fs::remove_file(path.join("ui.json"));
                    }
                }
            }
        });
        Telemetry { pending, wake }
    });
    *writer.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, value));
    let _ = writer.wake.try_send(());
}

impl PhoneApp {
    #[cfg(test)]
    pub(crate) fn open_local_chat(
        &mut self,
        provider: Provider,
        vault: Arc<dyn Vault>,
        synthetic: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.cancel_connection_attempt();
        if let Some(store) = &self.store
            && let Some(local) = &store.local
            && local.synthetic == synthetic
        {
            if let Some(id) = store.sessions.first().map(|s| s.id) {
                self.show_session(id, window, cx);
                return true;
            }
        }
        if self
            .store
            .as_ref()
            .and_then(|s| s.local.as_ref())
            .is_some_and(Local::running)
        {
            self.notify_user(
                "The previous local reply is still stopping. Please wait.",
                cx,
            );
            return false;
        }
        let mut backend = Local::new(provider, vault, synthetic);
        let Ok(session) = backend.create() else {
            return false;
        };
        let id = session.id;
        let mut store = Store::local(backend);
        store.sessions.push(session);
        self.install_store(store);
        self.paused = false;
        self.playing = false;
        if !synthetic {
            self.local_private = true;
            publish(self.data_dir.join("quickjs-chat-test"), None);
        }
        self.start.update(cx, |composer, cx| {
            composer.clear(cx);
            composer.set_workspace(true);
            composer.use_files(Some(Vec::new()));
            composer.use_commands(Some(Vec::new()));
        });
        self.show_session(id, window, cx);
        true
    }
    pub(crate) fn invalidate_local_account(
        &mut self,
        provider: Provider,
        synthetic: bool,
        cx: &mut Context<Self>,
    ) {
        // Account changes invalidate authorization, not the conversation. Workers
        // admitted under this account are cancelled; lease revocation still owns
        // logout/refresh coordination. An explicit Send is needed after re-login.
        for store in self.store.iter_mut().chain(self.parked_local.iter_mut()) {
            if let Some(backend) = store.local.as_mut().filter(|b| b.synthetic == synthetic) {
                for c in backend.conversations.values_mut() {
                    if provider_id(&c.pi.state.model.as_ref().unwrap().provider) == Some(provider)
                        && let Some(task) = &c.task
                    {
                        task.cancel();
                    }
                }
            }
        }
        cx.notify();
    }
    /// Shared UI test controls are intentionally synthetic-only; no input/credentials in links.
    pub(crate) fn local_fixture_url(
        &mut self,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if url == "pi://test/local-backend/synthetic" {
            if gpui_android::activity::is_debug_build()
                && !self.local_private
                && self.local_providers.is_synthetic()
            {
                self.select_local_backend(window, cx);
            }
            return true;
        }
        let mode = match url {
            "pi://test/local-chat/reply" => Mode::Reply,
            "pi://test/local-chat/stall" => Mode::Stall,
            "pi://test/local-chat/reject" => Mode::Reject,
            _ => return false,
        };
        if let Some(local) = self.store.as_mut().and_then(|s| s.local.as_mut())
            && local.synthetic
            && !local.running()
        {
            local.mode = mode;
            cx.notify();
        }
        true
    }
    pub(crate) fn tick_local_chat(&mut self, cx: &mut Context<Self>) {
        let Some(store) = &self.store else {
            return;
        };
        let Some(local) = &store.local else {
            return;
        };
        if !local.synthetic || self.local_private {
            return;
        }
        let current = self
            .model_session()
            .and_then(|id| local.conversations.get(&id));
        let session = self.model_session().and_then(|id| store.session(id));
        let Some(geometry) = cx.try_global::<crate::testing::State>() else {
            return;
        };
        let bounds: std::collections::BTreeMap<_, _> = geometry
            .bounds
            .iter()
            .filter_map(|(key, rect)| {
                let name = key
                    .strip_prefix("Name(\"")
                    .and_then(|s| s.strip_suffix("\")"))
                    .unwrap_or(key);
                ([
                    "draft",
                    "send",
                    "back",
                    "composer",
                    "composer-model",
                    "composer-thinking",
                    "model-search",
                    "backend-local",
                    "composer-attach",
                    "attach",
                    "stop",
                    "new-session",
                    "computers",
                    "local-accounts",
                    "search",
                ]
                .contains(&name)
                    || name.starts_with("model-choice-")
                    || name.starts_with("thinking-choice-")
                    || name.starts_with("session-row-")
                    || name.starts_with("thread-"))
                .then_some((name, *rect))
            })
            .collect();
        let visible = self.visible && self.window_active && !self.local_providers.visible;
        publish(
            self.data_dir.join("quickjs-chat-test"),
            Some(
                json!({"synthetic":true,"visible":visible,"pid":std::process::id(),
            "frame":self.local_frame,"route":format!("{:?}",self.route()),"sheet":self.sheet.map(|s|format!("{s:?}")),
            "provider":local.provider(self.model_session()).id(),"model":local.model(self.model_session()),"thinking":local.model_settings(self.model_session()).1,"status":current.map_or("idle",|c|c.status),
            "running":local.running(),"sessions":store.sessions.len(),"turns":session.map_or(0,|s|s.turns.len()),"context_messages":current.map_or(0,|c|c.history.len()),
            "reply_verified":session.and_then(|s|s.turn()).and_then(|t|t.summary.as_ref()).is_some_and(|s|s.text()=="Hello from the phone 🌍"),
            "bounds":if visible {json!(bounds)} else {json!({})},"scale":geometry.scale,"mode":format!("{:?}",local.mode)}),
            ),
        );
        if visible {
            cx.notify();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gpui::{Modifiers, px};
    use pi_quickjs_backend_poc::auth::VaultScope;
    pub(crate) static ACCOUNT_TEST_LOCK: Mutex<()> = Mutex::new(());
    pub(crate) struct Memory(Mutex<std::collections::BTreeMap<String, Value>>);
    impl Memory {
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self(Mutex::new([Provider::Claude,Provider::Codex].into_iter().map(|p|(p.id().to_owned(),json!({"generation":"0","credential":{
                "type":"oauth","access":"expired","refresh":"synthetic-chat-old","expires":0,"accountId":"acct_test_123"
            }}))).collect())))
        }
    }
    impl Vault for Memory {
        fn scope(&self) -> VaultScope {
            VaultScope::Synthetic
        }
        fn read(&self, provider: Provider) -> anyhow::Result<Value> {
            Ok(self.0.lock().unwrap()[provider.id()].clone())
        }
        fn write(
            &self,
            provider: Provider,
            expected: &str,
            data: &str,
        ) -> anyhow::Result<Option<String>> {
            let mut records = self.0.lock().unwrap();
            let v = records.get_mut(provider.id()).unwrap();
            if v["generation"] != expected {
                return Ok(None);
            }
            let next = (expected.parse::<u64>()? + 1).to_string();
            *v = json!({"generation":next,"credential":serde_json::from_str::<Value>(data)?});
            Ok(Some(next))
        }
        fn clear(&self, provider: Provider) -> anyhow::Result<()> {
            self.0.lock().unwrap().get_mut(provider.id()).unwrap()["credential"] = Value::Null;
            Ok(())
        }
    }
    pub(crate) fn drain(store: &mut Store) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while store.running().next().is_some() {
            assert!(std::time::Instant::now() < deadline);
            store.tick(Duration::from_millis(10));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn adapter_uses_canonical_completed_turns_without_demo_queue_tools_or_persistence() {
        let _serial = ACCOUNT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for provider in [Provider::Claude, Provider::Codex] {
            let mut backend = Local::new(provider, Memory::new(), true);
            let session = backend.create().unwrap();
            let id = session.id;
            let mut store = Store::local(backend);
            store.sessions.push(session);
            assert!(!store.is_sample());
            assert!(store.live.is_none());
            assert!(store.projects.is_empty());
            assert!(
                store
                    .send(id, "image".into(), vec!["picture".into(); 9])
                    .is_err()
            );
            assert!(store.session(id).unwrap().turns.is_empty());
            for _ in 0..2 {
                store.send(id, "synthetic question".into(), vec![]).unwrap();
                assert!(store.send(id, "duplicate".into(), vec![]).is_err());
                assert!(store.session(id).unwrap().queued.is_empty());
                drain(&mut store);
                assert_eq!(store.session(id).unwrap().state, State::Done);
                assert_eq!(
                    store
                        .session(id)
                        .unwrap()
                        .turn()
                        .unwrap()
                        .summary
                        .as_ref()
                        .unwrap()
                        .text(),
                    "Hello from the phone 🌍"
                );
                assert!(store.session(id).unwrap().turn().unwrap().stages.is_empty());
            }
            assert_eq!(
                store.local.as_ref().unwrap().conversations[&id]
                    .history
                    .len(),
                4
            );
            store.set_thinking(Some(id), "Medium").unwrap();
            let image=pi_core::protocol::ImageContent::new("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/l9sAAAAASUVORK5CYII=".into(),"image/png");
            store
                .send(
                    id,
                    Prompt::new(
                        "Attached text file: example.txt\nfixture-file-content".into(),
                        vec![image],
                    ),
                    vec!["example.txt".into(), "tiny.png".into()],
                )
                .unwrap();
            drain(&mut store);
            let last = store.local.as_ref().unwrap().conversations[&id]
                .history
                .last()
                .unwrap();
            assert_eq!(last["thinkingLevel"], "medium");
            assert_eq!(
                store.session(id).unwrap().turn().unwrap().attachments.len(),
                2
            );
            let canonical = store.local.as_ref().unwrap().conversations[&id]
                .history
                .clone();
            assert_eq!(canonical[4]["content"][1]["type"], "image");
            if provider == Provider::Codex {
                assert!(
                    store
                        .set_model(Some(id), provider.id(), "gpt-5.3-codex-spark")
                        .is_err()
                );
            }
            assert_eq!(canonical[1]["role"], "assistant");
            assert!(canonical[1]["usage"].is_object());
            let other = if provider == Provider::Claude {
                Provider::Codex
            } else {
                Provider::Claude
            };
            let generation = store.generation;
            store
                .set_model(Some(id), other.id(), &chat::models(other)[0].0)
                .unwrap();
            assert_eq!(store.generation, generation);
            assert_eq!(
                store.local.as_ref().unwrap().conversations[&id].history,
                canonical
            );
            assert_eq!(store.session(id).unwrap().turns.len(), 3);
            store
                .send(id, "follow up using the other provider".into(), vec![])
                .unwrap();
            drain(&mut store);
            assert_eq!(store.session(id).unwrap().state, State::Done);
            assert_eq!(
                store.local.as_ref().unwrap().conversations[&id].history[7]["provider"],
                other.id()
            );
            let canonical = store.local.as_ref().unwrap().conversations[&id]
                .history
                .clone();
            store.local.as_mut().unwrap().mode = Mode::Stall;
            store.send(id, "stop fixture".into(), vec![]).unwrap();
            store.stop(id).unwrap();
            assert_eq!(
                store.session(id).unwrap().state,
                State::Working,
                "must drain before allowing another request"
            );
            drain(&mut store);
            assert_eq!(store.session(id).unwrap().state, State::Stopped);
            assert_eq!(
                store.local.as_ref().unwrap().conversations[&id].history,
                canonical
            );
            store.local.as_mut().unwrap().mode = Mode::Reject;
            store.send(id, "reject fixture".into(), vec![]).unwrap();
            drain(&mut store);
            assert_eq!(store.session(id).unwrap().state, State::Failed);
            assert_eq!(
                store.local.as_ref().unwrap().conversations[&id].history,
                canonical
            );
            store.local.as_mut().unwrap().mode = Mode::Reply;
            for text in ["old facts ".repeat(10000), "recent facts ".repeat(10000)] {
                store.send(id, text.as_str().into(), vec![]).unwrap();
                drain(&mut store);
            }
            let before = store.session(id).unwrap().clone();
            let graph = store.local.as_ref().unwrap().conversations[&id]
                .checkpoint
                .as_ref()
                .unwrap()["fileEntries"]
                .as_array()
                .unwrap()
                .clone();
            assert!(store.supports_compact(id));
            store.compact(id, Some("retain facts".into())).unwrap();
            assert_eq!(store.session(id).unwrap().state, State::Working);
            assert!(store.session(id).unwrap().activity.contains("Compacting"));
            assert!(store.send(id, "during compaction".into(), vec![]).is_err());
            let mut compact_events = Vec::new();
            let until = std::time::Instant::now() + Duration::from_secs(10);
            while store.running().next().is_some() {
                assert!(std::time::Instant::now() < until);
                compact_events.extend(store.tick(Duration::from_millis(10)));
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(compact_events.contains(&Event::Compacted(id, None)));
            let after = store.session(id).unwrap();
            assert_eq!(after.id, before.id);
            assert_eq!(after.turns.len(), before.turns.len());
            assert_eq!(after.details.context_percent, None);
            assert!(after.details.auto_compaction_enabled);
            assert!(after.turns.iter().any(|t| {
                t.flow
                    .iter()
                    .any(|f| matches!(f,Flow::Compacted(summary) if summary.contains("Hello")))
            }));
            for (old, new) in before.turns.iter().zip(&after.turns) {
                assert_eq!(old.attachments, new.attachments);
                assert_eq!(old.excluded, new.excluded);
            }
            let cp = store.local.as_ref().unwrap().conversations[&id]
                .checkpoint
                .as_ref()
                .unwrap();
            assert_eq!(
                &cp["fileEntries"].as_array().unwrap()[..graph.len()],
                graph.as_slice()
            );
            store.send(id, "after compaction".into(), vec![]).unwrap();
            drain(&mut store);
            assert_eq!(store.session(id).unwrap().state, State::Done);
            // Byte admission counts visible failures/metadata too, not only active LLM context.
            let c = store
                .local
                .as_mut()
                .unwrap()
                .conversations
                .get_mut(&id)
                .unwrap();
            c.turns.last_mut().unwrap().excluded = Some("x".repeat(SESSION_BYTES));
            assert!(store.send(id, "budget check".into(), vec![]).is_err());
            store.remove(id);
            assert!(store.local.as_ref().unwrap().conversations.is_empty());
            // Maximum supported attachment payloads must survive native settlement,
            // checkpoint restoration and a subsequent prompt, not just the JS VM.
            use base64::Engine;
            let image = pi_core::protocol::ImageContent::new(
                base64::engine::general_purpose::STANDARD.encode(vec![1; 1024 * 1024]),
                "image/png",
            );
            let session = store.local.as_mut().unwrap().create().unwrap();
            let id = session.id;
            store.sessions.push(session);
            store
                .send(
                    id,
                    Prompt::new("four images".into(), vec![image; 4]),
                    vec!["image.png".into(); 4],
                )
                .unwrap();
            drain(&mut store);
            assert_eq!(store.session(id).unwrap().state, State::Done);
            assert!(!store.local.as_ref().unwrap().conversations[&id].poisoned);
            store.send(id, "image follow-up".into(), vec![]).unwrap();
            drain(&mut store);
            assert_eq!(store.session(id).unwrap().state, State::Done);
            store.remove(id);
        }
    }
    #[gpui::test]
    fn local_uses_shared_thread_model_picker_preferences_and_account_boundaries(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(px(384.), px(854.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.visible = true;
                app.window_active = true;
                assert!(app.open_local_chat(Provider::Claude, Memory::new(), true, window, cx));
                assert!(matches!(app.route(), Route::Thread(_)));
                assert!(!app.store.as_ref().unwrap().is_sample());
            })
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("composer-model").is_some());
        for name in ["composer-attach", "composer-thinking"] {
            assert!(cx.debug_bounds(name).is_some(), "{name}");
        }
        for name in ["local-chat-composer", "backend-picker"] {
            assert!(cx.debug_bounds(name).is_none(), "{name}");
        }
        assert!(
            cx.debug_bounds("composer-command").is_some(),
            "Local exposes the shared Compact action"
        );
        let point = cx.debug_bounds("composer-model").unwrap().center();
        cx.simulate_click(point, Modifiers::none());
        cx.run_until_parked();
        let point = cx.debug_bounds("model-choice-1").unwrap().center();
        cx.simulate_click(point, Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let id = app.model_session().unwrap();
                assert_eq!(
                    app.model_settings(cx).0,
                    chat::models(Provider::Claude)[1].1
                );
                assert_eq!(
                    app.prefs(cx).model,
                    "Opus 5.5",
                    "local picker must not change SSH defaults"
                );
                let composer = app.thread_composer(id, window, cx);
                assert!(!composer.read(cx).area.read(cx).enter_sends);
                composer.update(cx, |c, cx| c.set_text("private draft", cx));
                app.open_url("pi://unknown/not-navigation", window, cx);
                assert_eq!(app.route(), Route::Thread(id));
                assert_eq!(composer.read(cx).area.read(cx).text(), "private draft");
                let old = app.store.as_ref().unwrap().generation;
                assert!(app.open_local_chat(Provider::Claude, Memory::new(), true, window, cx));
                assert_eq!(app.store.as_ref().unwrap().generation, old);
                assert_eq!(composer.read(cx).area.read(cx).text(), "private draft");
                app.select_local_backend(window, cx);
                assert_eq!(app.route(), Route::Thread(id));
                assert!(app.open_local_chat(Provider::Codex, Memory::new(), true, window, cx));
                assert_eq!(app.store.as_ref().unwrap().generation, old);
                assert_eq!(app.model_session(), Some(id));
                assert_eq!(composer.read(cx).area.read(cx).text(), "private draft");
                app.choose_model(
                    "GPT".into(),
                    Provider::Codex.id().into(),
                    chat::models(Provider::Codex)[0].0.clone(),
                    cx,
                );
                assert_eq!(app.model_session(), Some(id));
                assert_eq!(app.store.as_ref().unwrap().generation, old);
                assert_eq!(
                    app.store.as_ref().unwrap().session(id).unwrap().project,
                    "Local"
                );
                assert_eq!(composer.read(cx).area.read(cx).text(), "private draft");
                app.invalidate_local_account(Provider::Claude, true, cx);
                assert!(app.store.as_ref().unwrap().session(id).is_some());
                app.invalidate_local_account(Provider::Codex, false, cx);
                assert!(app.store.is_some());
                app.invalidate_local_account(Provider::Codex, true, cx);
                assert!(app.store.as_ref().unwrap().session(id).is_some());
                assert_eq!(composer.read(cx).area.read(cx).text(), "private draft");
                assert!(!<PhoneApp as pi_visual_ui::Host>::visual_persist(app));
                let import = composer.update(cx, |c, cx| c.begin_import(cx));
                assert!(!composer.read(cx).can_send(cx));
                app.start.update(cx, |c, cx| {
                    c.set_text("new session draft", cx);
                    c.attach(
                        crate::composer::Attachment::File {
                            name: "note.txt".into(),
                            size: "4 bytes".into(),
                            contents: "note".into(),
                        },
                        cx,
                    );
                });
                app.open_sample(window, cx);
                assert!(app.parked_local.as_ref().unwrap().session(id).is_some());
                app.select_local_backend(window, cx);
                assert!(app.store.as_ref().unwrap().session(id).is_some());
                assert_ne!(
                    app.store.as_ref().unwrap().generation,
                    old,
                    "retire callbacks admitted before parking"
                );
                assert!(composer.read(cx).can_send(cx));
                assert!(!composer.update(cx, |c, cx| c.finish_import(import, cx)));
                assert_eq!(app.start.read(cx).area.read(cx).text(), "new session draft");
                assert_eq!(app.start.read(cx).attachments().len(), 1);
                assert_eq!(
                    app.threads[&id].read(cx).area.read(cx).text(),
                    "private draft"
                );
            })
        });
    }
}
