//! Local UI adapter for the shared Durable runner; SQLite, not this cache, owns execution.
#[cfg(all(test, target_os = "android", feature = "ui-test"))]
use crate::store::Store;
use crate::{app::PhoneApp, model::*, prompt::Prompt, store::Event};
use gpui::{Context, Window};
use pi_local::{
    auth::{Provider, Vault},
    bun::{Engine, FixtureMode as Mode},
    models,
};
use serde_json::{Value, json};
#[cfg(feature = "ui-test")]
use std::sync::{Mutex, mpsc};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Duration,
};
const FEATURES: &[&str] = &[
    "models",
    "thinking",
    "attachments",
    "streaming",
    "prompt_queue",
    "stop",
    "visualize",
    "manual_compaction",
    "auto_compaction",
];
static ROOT: OnceLock<PathBuf> = OnceLock::new();
pub fn configure(data: &Path) {
    let _ = ROOT.set(data.parent().unwrap_or(data).join("no_backup/local-agent"));
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Meta {
    id: u32,
    provider: String,
    model: String,
    thinking: String,
    #[serde(default)]
    pending: Option<Value>,
    #[serde(default = "starts_turn_default")]
    pending_starts_turn: bool,
    #[serde(default)]
    attachments: Vec<Vec<String>>,
    #[serde(default)]
    attachments_by_request: HashMap<String, AttachedPrompt>,
    #[serde(default)]
    legacy_attachment_entries: HashMap<u64, Vec<String>>,
    #[serde(default)]
    legacy_attachments_checked: bool,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct AttachedPrompt {
    labels: Vec<String>,
    #[serde(default)]
    submission: Option<String>,
    #[serde(default)]
    entry: Option<u64>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Index {
    version: u32,
    next: u32,
    sessions: Vec<Meta>,
}
fn starts_turn_default() -> bool {
    true
}

struct Conversation {
    pi: pi_core::session::Session,
    engine: Engine,
    meta: Meta,
    key: String,
    cwd: String,
    status: &'static str,
    loaded: bool,
    /// Only a newly allocated, nonexistent state may queue its first prompt before the initial snapshot.
    fresh: bool,
    admitting: bool,
    setting: Option<String>,
    compact: Option<String>,
    stopped: std::cell::Cell<bool>,
    error: Option<String>,
    history: Vec<Value>,
    cancelling: HashMap<String, String>,
    attachment_recovery: AttachmentRecovery,
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
    root: PathBuf,
    next: u32,
    index_error: Option<String>,
}
fn response(pi: &mut pi_core::session::Session, command: &str, data: Value) -> Result<(), String> {
    pi.apply(&json!({"type":"response","command":command,"success":true,"data":data}))
        .map_err(|_| "Invalid Local protocol record".into())
}
/// One correlated read at a time, at most three attempts per unresolved
/// association per resync. A queued/failed/missing read is not a permanent cache.
#[derive(Default)]
struct AttachmentRecovery {
    reads: HashMap<String, (u8, std::time::Instant)>,
    in_flight: Option<(String, String, std::time::Instant)>,
}
impl AttachmentRecovery {
    fn resync(&mut self) {
        self.reads.clear();
    }
    fn next(&self, meta: &Meta, now: std::time::Instant) -> Option<String> {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|(_, _, sent)| now.duration_since(*sent).as_secs() < 2)
        {
            return None;
        }
        meta.attachments_by_request
            .iter()
            .find(|(id, association)| {
                association.entry.is_none()
                    && self.reads.get(*id).is_none_or(|(attempts, sent)| {
                        *attempts < 3 && now.duration_since(*sent).as_secs() >= 1
                    })
            })
            .map(|(id, _)| id.clone())
    }
    fn begin(&mut self, id: String, request: String, now: std::time::Instant) {
        let attempt = self.reads.entry(request.clone()).or_insert((0, now));
        *attempt = (attempt.0 + 1, now);
        self.in_flight = Some((id, request, now));
    }
    fn complete(&mut self, record: &Value, meta: &mut Meta) -> bool {
        if !self
            .in_flight
            .as_ref()
            .is_some_and(|(id, _, _)| record["id"] == *id)
        {
            return false;
        }
        let (_, request, _) = self.in_flight.take().unwrap();
        record["success"] == true
            && record["data"]["submission"]["requestId"] == request
            && observe_submission(meta, &record["data"]["submission"])
    }
}

/// Associate labels only through the SDK's durable request/submission/entry IDs.
/// Queue cancellation/reordering and identical text must never shift attachments.
fn observe_submission(meta: &mut Meta, record: &Value) -> bool {
    let Some(association) = record["requestId"]
        .as_str()
        .and_then(|id| meta.attachments_by_request.get_mut(id))
    else {
        return false;
    };
    let id = record["id"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| record["id"].as_u64().map(|id| id.to_string()));
    let Some(id) = id else {
        return false;
    };
    if association
        .submission
        .as_ref()
        .is_some_and(|previous| previous != &id)
    {
        return false;
    }
    let mut changed = association.submission.is_none();
    association.submission = Some(id);
    if let Some(entry) = record["entry"].as_u64() {
        if association.entry.is_some_and(|previous| previous != entry) {
            return changed;
        }
        changed |= association.entry != Some(entry);
        association.entry = Some(entry);
    }
    changed
}

fn prompt_record(prompt: &Prompt, follow_up: bool) -> Value {
    // Freeze the user's choice before admission so a reconnect or a run
    // settling cannot turn a review follow-up into steering (or vice versa).
    json!({"type":"prompt","id":format!("admit-{}",prompt.request_id),"requestId":prompt.request_id,
        "message":prompt.message,"images":prompt.images,
        "streamingBehavior":if follow_up { "followUp" } else { "steer" }})
}

fn settle_admission(
    meta: &mut Meta,
    admitting: &mut bool,
    error: &mut Option<String>,
    record: &Value,
) -> bool {
    let Some(pending) = &meta.pending else {
        return false;
    };
    if pending["id"] != record["id"] {
        return false;
    }
    *admitting = false;
    if record["success"] != true {
        *error = Some(
            record["error"]
                .as_str()
                .unwrap_or("Prompt was not admitted; original request retained")
                .into(),
        );
        return false;
    }
    if record["data"]["requestId"] != pending["requestId"] {
        *error = Some("Prompt receipt identity did not match; original request retained".into());
        return false;
    }
    let receipt = json!({"requestId":pending["requestId"], "id":record["data"]["submissionId"]});
    observe_submission(meta, &receipt);
    meta.pending = None;
    *error = None;
    true
}

fn attach_labels(meta: &Meta, session: &mut Session) {
    for turn in &mut session.turns {
        if let Some(labels) = turn.durable_entry_id.and_then(|entry| {
            meta.attachments_by_request
                .values()
                .find(|association| association.entry == Some(entry))
                .map(|association| &association.labels)
                .or_else(|| meta.legacy_attachment_entries.get(&entry))
        }) {
            turn.attachments = labels.clone();
        }
    }
}

/// Durable configuration events own model/thinking state. Read/configuration
/// responses are acknowledgements, not stock Pi snapshots (get_state is {}).
fn apply_local_response(
    pi: &mut pi_core::session::Session,
    setting: &mut Option<String>,
    error: &mut Option<String>,
    record: &Value,
) -> anyhow::Result<()> {
    let command = record["command"].as_str().unwrap_or("");
    let configuration = matches!(command, "set_model" | "set_thinking_level");
    if configuration && setting.is_some() && setting.as_deref() == record["id"].as_str() {
        *setting = None;
        *error = (record["success"] != true).then(|| {
            "Model settings were not changed; wait for the chat to become idle and retry".into()
        });
    }
    if !configuration && !matches!(command, "get_state" | "get_messages") {
        pi.apply(record)?;
    }
    Ok(())
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
        let root = ROOT
            .get()
            .cloned()
            .unwrap_or_else(|| std::env::temp_dir().join("pi-bun-host"));
        let mut metadata = Vec::new();
        for p in [
            provider,
            if provider == Provider::Claude {
                Provider::Codex
            } else {
                Provider::Claude
            },
        ] {
            let mut models = models::catalog(p).as_array().unwrap().clone();
            models.sort_by_key(|m| m["id"] != models::models(p)[0].0);
            metadata.extend(models);
        }
        let catalog = serde_json::from_value(json!(metadata)).expect("pinned public catalog");
        let mut result = Self {
            default_provider: provider,
            synthetic,
            vault,
            default_model: models::models(provider)[0].0.clone(),
            default_thinking: "high".into(),
            catalog,
            metadata,
            conversations: HashMap::new(),
            mode: Mode::Reply,
            root,
            next: 0x8000_0000,
            index_error: None,
        };
        match std::fs::read(result.index_path()) {
            Ok(bytes) => match serde_json::from_slice::<Index>(&bytes) {
                Ok(index)
                    if index.version == 1
                        && index.sessions.len() <= 8
                        && index.next >= 0x8000_0000
                        && index
                            .sessions
                            .iter()
                            .all(|meta| meta.id >= 0x8000_0000 && meta.id < index.next)
                        && index
                            .sessions
                            .iter()
                            .map(|meta| meta.id)
                            .collect::<std::collections::HashSet<_>>()
                            .len()
                            == index.sessions.len() =>
                {
                    result.next = index.next;
                    for meta in index.sessions {
                        if let Err(error) = result.open(meta, false) {
                            result.index_error = Some(error);
                        }
                    }
                }
                _ => {
                    result.index_error =
                        Some("Saved Local index could not be opened; it was not overwritten".into())
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                result.index_error =
                    Some("Saved Local index is unavailable; it was not overwritten".into())
            }
        }
        result
    }
    fn namespace(&self) -> PathBuf {
        self.root.join(if self.synthetic {
            "synthetic"
        } else {
            "accounts"
        })
    }
    fn index_path(&self) -> PathBuf {
        self.namespace().join("index.json")
    }
    fn save(&self) -> Result<(), String> {
        self.save_extra(None)
    }
    fn save_extra(&self, extra: Option<&Meta>) -> Result<(), String> {
        if let Some(error) = &self.index_error {
            return Err(error.clone());
        }
        let save = || -> anyhow::Result<()> {
            use std::io::Write;
            std::fs::create_dir_all(self.namespace())?;
            let mut sessions: Vec<_> = self
                .conversations
                .values()
                .map(|c| c.meta.clone())
                .collect();
            if let Some(meta) = extra {
                sessions.push(meta.clone());
            }
            sessions.sort_by_key(|m| m.id);
            let bytes = serde_json::to_vec(&Index {
                version: 1,
                next: self.next,
                sessions,
            })?;
            let path = self.index_path();
            let temp = path.with_extension("new");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(temp, path)?;
            std::fs::File::open(self.namespace())?.sync_all()?;
            Ok(())
        };
        save().map_err(|_| "Could not save Local session index; your draft was kept".into())
    }
    fn open(&mut self, meta: Meta, fresh: bool) -> Result<(), String> {
        let provider = provider_id(&meta.provider).ok_or("Unsupported saved provider")?;
        if !self
            .catalog
            .iter()
            .any(|m| m.provider == meta.provider && m.id == meta.model)
        {
            return Err("Saved model is not in this build's catalog".into());
        }
        let key = format!("local-{}", meta.id);
        let state = self.namespace().join(&key);
        if fresh && state.exists() {
            return Err("New Local identity already exists; saved state was retained".into());
        }
        let cwd = self.namespace().join("workspace");
        std::fs::create_dir_all(&cwd).map_err(|_| "Local workspace unavailable")?;
        let engine=Engine::open(self.root.clone(),json!({"state":state,"cwd":cwd,"key":key,"synthetic":self.synthetic,"provider":provider.id(),"model":meta.model,"thinking":meta.thinking,"home":self.root.join("home"),"temp":self.root.join("tmp"),"mode":format!("{:?}",self.mode)}),self.vault.clone()).map_err(|_|"Could not start embedded Bun")?;
        let mut pi = pi_core::session::Session::new(cwd.clone());
        let model = self
            .catalog
            .iter()
            .find(|m| m.provider == meta.provider && m.id == meta.model)
            .unwrap();
        response(
            &mut pi,
            "get_state",
            json!({"sessionId":key,"model":model,"thinkingLevel":meta.thinking,"isStreaming":false,"isCompacting":false,"isBashRunning":false,"autoCompactionEnabled":true}),
        )?;
        response(
            &mut pi,
            "get_available_models",
            json!({"models":self.catalog}),
        )?;
        response(&mut pi, "get_commands", json!({"commands":[]}))?;
        if let Some(pending) = &meta.pending {
            engine
                .send(pending.clone())
                .map_err(|_| "Saved prompt could not be checked")?;
        }
        let admitting = meta.pending.is_some();
        self.conversations.insert(
            SessionId(meta.id),
            Conversation {
                pi,
                engine,
                meta,
                key,
                cwd: cwd.to_string_lossy().into_owned(),
                status: "loading",
                loaded: false,
                fresh,
                admitting,
                setting: None,
                compact: None,
                stopped: std::cell::Cell::new(false),
                error: None,
                history: Vec::new(),
                cancelling: HashMap::new(),
                attachment_recovery: AttachmentRecovery::default(),
            },
        );
        Ok(())
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
            .unwrap_or("high")
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
    pub fn restored(&self) -> Vec<Session> {
        let mut ids: Vec<_> = self.conversations.keys().copied().collect();
        ids.sort_by_key(|id| id.0);
        ids.into_iter().map(|id| self.project(id)).collect()
    }
    pub fn create(&mut self) -> Result<Session, String> {
        if self.conversations.len() >= 8 {
            return Err("Delete an old Local chat first (eight open sessions maximum)".into());
        }
        if let Some(error) = &self.index_error {
            return Err(error.clone());
        }
        let id = self.next;
        self.next = self.next.checked_add(1).ok_or("Session identity limit")?;
        let meta = Meta {
            id,
            provider: self.default_provider.id().into(),
            model: self.default_model.clone(),
            thinking: self.default_thinking.clone(),
            pending: None,
            pending_starts_turn: true,
            attachments: Vec::new(),
            attachments_by_request: HashMap::new(),
            legacy_attachment_entries: HashMap::new(),
            legacy_attachments_checked: true,
        };
        self.save_extra(Some(&meta))?;
        self.open(meta, true)?;
        Ok(self.project(SessionId(id)))
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
        self.current_model(id)
            .and_then(|m| provider_id(&m.provider))
            .unwrap_or(self.default_provider)
    }
    pub fn model(&self, id: Option<SessionId>) -> String {
        self.current_model(id)
            .map_or_else(|| self.default_model.clone(), |m| m.id.clone())
    }
    pub fn model_settings(&self, id: Option<SessionId>) -> (String, String) {
        let Some(model) = self.current_model(id) else {
            return ("Loading model…".into(), "…".into());
        };
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
            .map(|l| thinking_label(l).into())
            .collect()
    }
    pub fn set_thinking(&mut self, id: Option<SessionId>, label: &str) -> Result<(), String> {
        let level = self
            .raw_levels(self.provider(id), &self.model(id))
            .into_iter()
            .find(|l| thinking_label(l) == label)
            .ok_or("Unsupported thinking level")?;
        if let Some(id) = id {
            let c = self
                .conversations
                .get_mut(&id)
                .ok_or("Session unavailable")?;
            if !c.loaded || c.pi.busy() || c.admitting || c.compact.is_some() || c.setting.is_some()
            {
                return Err("Stop the reply before changing thinking".into());
            }
            let request = crate::prompt::request_id();
            c.engine
                .send(json!({"type":"set_thinking_level","id":request,"level":level}))
                .map_err(|_| "Local command queue unavailable")?;
            c.setting = Some(request);
            return Ok(());
        } else {
            self.default_thinking = level;
        }
        self.save()
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
            .ok_or("Model unavailable")?;
        let chosen = provider_id(provider).ok_or("Provider unavailable")?;
        let thinking = self.clamp(
            chosen,
            model,
            id.and_then(|id| self.conversations.get(&id))
                .map_or(&self.default_thinking, |c| &c.pi.state.thinking_level),
        );
        if let Some(id) = id {
            let c = self
                .conversations
                .get_mut(&id)
                .ok_or("Session unavailable")?;
            if !c.loaded || c.pi.busy() || c.admitting || c.compact.is_some() || c.setting.is_some()
            {
                return Err("Stop the reply before changing models".into());
            }
            if !selected.input.iter().any(|s| s == "image")
                && c.pi.messages.iter().any(|m| {
                    m["role"] == "user"
                        && m["content"]
                            .as_array()
                            .is_some_and(|a| a.iter().any(|b| b["type"] == "image"))
                })
            {
                return Err("Choose an image-capable model for this chat".into());
            }
            let request = crate::prompt::request_id();
            c.engine
                .send(json!({"type":"set_model","id":request,"provider":provider,"modelId":model}))
                .map_err(|_| "Local command queue unavailable")?;
            c.setting = Some(request);
            return Ok(());
        } else {
            self.default_provider = chosen;
            self.default_model = model.into();
            self.default_thinking = thinking;
        }
        self.save()
    }
    pub fn supports(&self, feature: &str) -> bool {
        FEATURES.contains(&feature)
    }
    pub fn commands(&self, id: SessionId) -> Option<&[pi_core::protocol::SlashCommand]> {
        self.conversations.get(&id).map(|_| &[][..])
    }
    pub fn running(&self) -> bool {
        self.conversations.values().any(|c| {
            !c.loaded || c.pi.busy() || c.admitting || c.compact.is_some() || c.setting.is_some()
        })
    }
    pub fn stopping(&self, id: SessionId) -> bool {
        self.conversations.get(&id).is_some_and(|c| c.stopped.get())
    }
    /// Explicit application-lifecycle cancellation; never used for backend navigation.
    pub fn cancel(&self) {
        for (id, c) in &self.conversations {
            if !c.loaded || c.pi.busy() || c.admitting || c.compact.is_some() || c.setting.is_some()
            {
                let _ = self.stop(*id);
            }
        }
    }
    pub fn stop(&self, id: SessionId) -> Result<(), String> {
        let c = self.conversations.get(&id).ok_or("Session unavailable")?;
        c.engine
            .send(json!({"type":"abort","id":crate::prompt::request_id()}))
            .map_err(|_| "Local stop command unavailable")?;
        c.stopped.set(true);
        Ok(())
    }
    pub fn remove(&mut self, id: SessionId) {
        if let Some(c) = self.conversations.remove(&id) {
            let _ = c
                .engine
                .send(json!({"type":"abort","id":crate::prompt::request_id()}));
            drop(c);
        }
        if self.save().is_err() {
            self.index_error = Some("Could not save Local session removal".into());
        }
    }
    pub fn send(
        &mut self,
        session: &mut Session,
        prompt: Prompt,
        attachments: Vec<String>,
        follow_up: bool,
    ) -> Result<(), String> {
        if prompt.message.trim().is_empty() && prompt.images.is_empty() {
            return Err("Write a message first".into());
        }
        if prompt.message.len() > 2 * 1024 * 1024
            || attachments.len() > 8
            || serde_json::to_vec(&prompt.images).map_or(true, |v| v.len() > 6 * 1024 * 1024)
        {
            return Err("Local prompt/attachment limit reached; your draft was kept".into());
        }
        let c = self
            .conversations
            .get_mut(&session.id)
            .ok_or("Session unavailable")?;
        if (!c.loaded && !c.fresh)
            || c.stopped.get()
            || c.admitting
            || c.compact.is_some()
            || c.setting.is_some()
            || c.meta.pending.is_some()
        {
            return Err(
                "Wait for admission/reply or restart to check the retained request identity".into(),
            );
        }
        if !prompt.images.is_empty()
            && !c
                .pi
                .state
                .model
                .as_ref()
                .is_some_and(|m| m.input.iter().any(|i| i == "image"))
        {
            return Err("Choose an image-capable model first".into());
        }
        let record = prompt_record(&prompt, follow_up);
        c.engine
            .validate(&record)
            .map_err(|_| "Prompt exceeds the native bridge limit; your draft was kept")?;
        let previous = c.meta.clone();
        c.meta.pending = Some(record.clone());
        c.meta.pending_starts_turn = !c.pi.busy();
        if !attachments.is_empty() {
            c.meta.attachments_by_request.insert(
                prompt.request_id.clone(),
                AttachedPrompt {
                    labels: attachments,
                    submission: None,
                    entry: None,
                },
            );
        }
        if let Err(error) = self.save() {
            self.conversations.get_mut(&session.id).unwrap().meta = previous;
            return Err(error);
        }
        let c = self.conversations.get_mut(&session.id).unwrap();
        if self.synthetic {
            c.engine
                .send(json!({"type":"phone_fixture_mode","mode":format!("{:?}",self.mode)}))
                .map_err(|_| "Local command queue unavailable")?;
        }
        c.engine.send(record).map_err(
            |_| "Prompt admission uncertain; it retains the same request identity on reopen",
        )?;
        c.admitting = true;
        c.error = None;
        c.status = "preparing";
        *session = self.project(session.id);
        Ok(())
    }
    pub fn unqueue_key(&mut self, id: SessionId, key: &QueueKey) -> Result<(), String> {
        let QueueKey::Durable(submission) = key else {
            return Err("Wait for the durable queue receipt before cancelling this message".into());
        };
        let c = self
            .conversations
            .get_mut(&id)
            .ok_or("Session unavailable")?;
        if c.cancelling.values().any(|pending| pending == submission) {
            return Ok(());
        }
        if !c.pi.queued_submissions.contains(submission) {
            return Err("This message is no longer queued. Nothing was cancelled.".into());
        }
        let request = crate::prompt::request_id();
        c.engine
            .send(json!({"type":"cancel_submission","id":request,"submissionId":submission}))
            .map_err(|_| "Local queue cancellation could not be sent")?;
        c.cancelling.insert(request, submission.clone());
        Ok(())
    }

    pub fn compact(
        &mut self,
        session: &mut Session,
        instructions: Option<String>,
    ) -> Result<(), String> {
        let c = self
            .conversations
            .get_mut(&session.id)
            .ok_or("Session unavailable")?;
        if !c.loaded || c.pi.busy() || c.admitting || c.compact.is_some() || c.setting.is_some() {
            return Err("Wait for the chat to become idle first".into());
        }
        let id = crate::prompt::request_id();
        let mut record = json!({"type":"compact","id":id});
        if let Some(instructions) = instructions {
            record["customInstructions"] = json!(instructions);
        }
        c.engine
            .send(record)
            .map_err(|_| "Local compaction command unavailable")?;
        c.compact = Some(id);
        c.error = None;
        c.status = "compacting";
        *session = self.project(session.id);
        Ok(())
    }
    pub fn project(&self, id: SessionId) -> Session {
        let c = &self.conversations[&id];
        let pending: Vec<_> = c
            .meta
            .pending
            .as_ref()
            .filter(|_| c.admitting || c.error.is_some())
            .map(|record| crate::projection::PendingPrompt {
                text: record["message"].as_str().unwrap_or("").into(),
                key: None,
                starts_turn: c.meta.pending_starts_turn,
            })
            .into_iter()
            .collect();
        let mut session = crate::projection::project(
            &c.pi,
            crate::projection::Facts {
                id,
                cwd: &c.cwd,
                folder: c.cwd.clone(),
                question: None,
                outbox: &pending,
                key: &c.key,
            },
        );
        session.local = true;
        session.project = "Local".into();
        session.backend_features = Some(FEATURES.iter().map(|s| (*s).into()).collect());
        session.details.session_file = "Saved on this phone · SQLite".into();
        if !c.loaded {
            session.activity = "Opening saved chat…".into();
            session.state = State::Working;
        } else if c.setting.is_some() {
            session.activity = "Updating model settings…".into();
            session.state = State::Working;
        } else if c.stopped.get() && c.pi.busy() {
            session.activity = "Stopping…".into();
        } else if c.compact.is_some() {
            session.activity = "Compacting…".into();
            session.state = State::Working;
        } else if c.admitting {
            session.activity = "Admitting prompt…".into();
            session.state = State::Working;
        } else if !c.pi.busy() && session.turns.is_empty() {
            session.activity = "Ready · Android sh tools available".into();
        }
        if let Some(error) = c.error.as_ref().or(self.index_error.as_ref()) {
            session.state = State::Failed;
            session.failure = Some(error.clone());
            session.activity = error.clone();
        }
        attach_labels(&c.meta, &mut session);
        session
    }
    pub fn tick(&mut self, sessions: &mut [Session], _elapsed: Duration) -> Vec<Event> {
        let mut events = Vec::new();
        let mut save = false;
        for session in sessions {
            let Some(c) = self.conversations.get_mut(&session.id) else {
                continue;
            };
            let before = session.state;
            let mut refresh = false;
            for mut record in c.engine.records() {
                let result: anyhow::Result<()> = (|| {
                    match record["type"].as_str() {
                        Some("durable_state") => {
                            c.pi = pi_core::durable::project(&record, &c.pi, &c.key)?;
                            c.attachment_recovery.resync();
                            c.loaded = true;
                        }
                        Some("durable_events") => {
                            let projected = pi_core::durable::events(&record, &c.pi, &c.key)?;
                            for event in record["events"].as_array().into_iter().flatten() {
                                if event["type"] == "submission" {
                                    save |= observe_submission(&mut c.meta, &event["record"]);
                                }
                            }
                            for event in projected {
                                c.pi.apply(&event)?;
                            }
                            let state = pi_core::durable::state_update(&c.pi)?;
                            c.pi.apply(&state)?;
                        }
                        Some("durable_metadata") => {
                            for event in pi_core::durable::metadata(&record, &c.pi, &c.key)? {
                                c.pi.apply(&event)?;
                            }
                        }
                        Some("durable_subagents") => {
                            for event in pi_core::durable::subagents(&record, &c.pi, &c.key)? {
                                c.pi.apply(&event)?;
                            }
                        }
                        Some("response") => {
                            if record["command"] == "prompt" {
                                save |= settle_admission(
                                    &mut c.meta,
                                    &mut c.admitting,
                                    &mut c.error,
                                    &record,
                                );
                                return Ok(());
                            }
                            if record["command"] == "get_submission" {
                                save |= c.attachment_recovery.complete(&record, &mut c.meta);
                                return Ok(());
                            }
                            if record["command"] == "cancel_submission" {
                                let Some(request) = record["id"].as_str() else {
                                    return Ok(());
                                };
                                if c.cancelling.remove(request).is_none() {
                                    return Ok(());
                                }
                                if record["success"] != true {
                                    c.error = Some("Queued message cancellation was not confirmed; check the queue before retrying".into());
                                }
                                // Inbox events, not an optimistic list edit, remove the row.
                                return Ok(());
                            }
                            if record["command"] == "compact"
                                && c.compact.as_deref() == record["id"].as_str()
                            {
                                c.compact = None;
                                events.push(Event::Compacted(
                                    session.id,
                                    if record["success"] == true {
                                        None
                                    } else {
                                        Some(
                                            record["error"]
                                                .as_str()
                                                .unwrap_or("Compaction did not complete")
                                                .into(),
                                        )
                                    },
                                ));
                                refresh = true;
                            }
                            if record["command"] == "abort" {
                                c.stopped.set(false);
                            }
                            if record["command"] == "get_session_stats" {
                                pi_core::durable::discard_stale_stats(&mut record, &c.pi)?;
                            }
                            apply_local_response(&mut c.pi, &mut c.setting, &mut c.error, &record)?;
                        }
                        Some("local_error") => {
                            c.loaded = true;
                            c.admitting = false;
                            c.setting = None;
                            c.error = Some(
                                record["error"]
                                    .as_str()
                                    .unwrap_or("Local runtime unavailable")
                                    .into(),
                            );
                            if c.compact.take().is_some() {
                                events.push(Event::Compacted(session.id, c.error.clone()));
                            }
                        }
                        _ => {}
                    }
                    Ok(())
                })();
                if result.is_err() {
                    c.error = Some(
                        "Local presentation could not synchronize; saved SDK state was retained"
                            .into(),
                    );
                    c.loaded = true;
                    c.admitting = false;
                    c.setting = None;
                }
            }
            if c.loaded && !c.meta.legacy_attachments_checked && c.meta.pending.is_none() {
                // Legacy versions admitted only idle prompts. Migrate once, only
                // with an exact complete user-entry sequence; never zip new rows.
                c.meta.legacy_attachments_checked = true;
                let users: Vec<_> =
                    c.pi.messages
                        .iter()
                        .filter(|message| message["role"] == "user")
                        .collect();
                if c.meta.pending.is_none()
                    && users.len() == c.meta.attachments.len()
                    && users.iter().all(|message| {
                        message["__piDurableEntryId"].is_u64()
                            && pi_core::subagent::report(&pi_core::session::content_text(
                                &message["content"],
                            ))
                            .is_none()
                    })
                {
                    for (message, labels) in users.iter().zip(&c.meta.attachments) {
                        c.meta.legacy_attachment_entries.insert(
                            message["__piDurableEntryId"].as_u64().unwrap(),
                            labels.clone(),
                        );
                    }
                }
                save = true;
            }
            let now = std::time::Instant::now();
            if c.loaded
                && let Some(request_id) = c.attachment_recovery.next(&c.meta, now)
            {
                let id = crate::prompt::request_id();
                if c.engine
                    .send(json!({"type":"get_submission", "id":id, "requestId":request_id}))
                    .is_ok()
                {
                    c.attachment_recovery.begin(id, request_id, now);
                }
            }
            if c.loaded
                && let Some(model) = &c.pi.state.model
                && (c.meta.provider != model.provider
                    || c.meta.model != model.id
                    || c.meta.thinking != c.pi.state.thinking_level)
            {
                c.meta.provider = model.provider.clone();
                c.meta.model = model.id.clone();
                c.meta.thinking = c.pi.state.thinking_level.clone();
                save = true;
            }
            if c.loaded
                && !c.pi.busy()
                && !c.admitting
                && c.compact.is_none()
                && c.setting.is_none()
            {
                c.status = "idle";
                if before == State::Working {
                    refresh = true;
                    events.push(Event::Finished(session.id));
                }
            } else if c.compact.is_some() {
                c.status = "compacting";
            } else if c.pi.busy() {
                c.status = "streaming";
            }
            if refresh {
                let _ = c
                    .engine
                    .send(json!({"type":"get_session_stats","id":crate::prompt::request_id()}));
            }
            c.history = c.pi.messages.clone();
            *session = self.project(session.id);
        }
        if save && self.save().is_err() {
            self.index_error = Some(
                "Could not update Local admission index; original request IDs were retained".into(),
            );
        }
        events
    }
}

// Independent coalescing writer: synthetic test geometry/status only, never chat text.
#[cfg(feature = "ui-test")]
type Publication = (PathBuf, Option<Value>);
#[cfg(feature = "ui-test")]
struct Telemetry {
    pending: Arc<Mutex<Option<Publication>>>,
    wake: mpsc::SyncSender<()>,
}
#[cfg(feature = "ui-test")]
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
    #[cfg(all(test, target_os = "android", feature = "ui-test"))]
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
        if !self.install_store(store, cx) {
            return false;
        }
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
                    if c.pi
                        .state
                        .model
                        .as_ref()
                        .and_then(|model| provider_id(&model.provider))
                        .or_else(|| provider_id(&c.meta.provider))
                        == Some(provider)
                    {
                        let _ = c
                            .engine
                            .send(json!({"type":"abort","id":crate::prompt::request_id()}));
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
    #[cfg(feature = "ui-test")]
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
            "tool_verified":current.is_some_and(|c|c.pi.tools.iter().any(|tool|tool.finished&&!tool.is_error&&tool.output.contains("phone-tool-ok"))),
            "bounds":if visible {json!(bounds)} else {json!({})},"scale":geometry.scale,"mode":format!("{:?}",local.mode)}),
            ),
        );
        if visible {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoCredentials;
    impl Vault for NoCredentials {
        fn scope(&self) -> pi_local::auth::VaultScope {
            pi_local::auth::VaultScope::Synthetic
        }
        fn read(&self, _: Provider) -> anyhow::Result<Value> {
            panic!("presentation test must not read accounts")
        }
        fn write(&self, _: Provider, _: &str, _: &str) -> anyhow::Result<Option<String>> {
            panic!("presentation test must not write accounts")
        }
        fn clear(&self, _: Provider) -> anyhow::Result<()> {
            panic!("presentation test must not clear accounts")
        }
    }

    #[gpui::test]
    fn working_local_chat_keeps_send_enabled_for_a_typed_steering_message(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let metadata = models::catalog(Provider::Claude)
                    .as_array()
                    .unwrap()
                    .clone();
                let backend = Local {
                    default_provider: Provider::Claude,
                    synthetic: true,
                    vault: Arc::new(NoCredentials),
                    default_model: metadata[0]["id"].as_str().unwrap().into(),
                    default_thinking: "high".into(),
                    catalog: serde_json::from_value(json!(metadata)).unwrap(),
                    metadata,
                    conversations: HashMap::new(),
                    mode: Mode::Reply,
                    root: PathBuf::from("/unused-presentation-test"),
                    next: 0x8000_0000,
                    index_error: None,
                };
                let mut store = crate::store::Store::local(backend);
                let mut session =
                    crate::demo::new_session(SessionId(1), 0, "working".into(), Vec::new());
                session.local = true;
                session.state = State::Working;
                session.backend_features =
                    Some(FEATURES.iter().map(|feature| (*feature).into()).collect());
                store.sessions.push(session);
                app.install_store(store, cx);
                app.show_session(SessionId(1), window, cx);
                let composer = app.thread_composer(SessionId(1), window, cx);
                composer.update(cx, |composer, cx| {
                    composer.set_text("Steer the current run", cx)
                });
                cx.notify();
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            let composer = app.threads.get(&SessionId(1)).unwrap().read(cx);
            assert!(
                composer.can_send(cx),
                "Local prompt_queue must enable Send while Working"
            );
        });
    }

    fn metadata() -> Meta {
        serde_json::from_value(
            json!({"id":1,"provider":"anthropic","model":"model","thinking":"high"}),
        )
        .unwrap()
    }

    #[test]
    fn busy_send_policy_and_original_pending_identity_survive_reopen() {
        assert!(FEATURES.contains(&"prompt_queue"));
        let prompt = Prompt {
            request_id: "stable".into(),
            message: "steer".into(),
            images: Vec::new(),
        };
        let record = prompt_record(&prompt, false);
        assert_eq!(record["streamingBehavior"], "steer");
        assert_eq!(
            prompt_record(&prompt, true)["streamingBehavior"],
            "followUp"
        );
        let mut meta = metadata();
        meta.pending = Some(record.clone());
        meta.pending_starts_turn = false;
        let reopened: Meta = serde_json::from_slice(&serde_json::to_vec(&meta).unwrap()).unwrap();
        assert_eq!(reopened.pending, Some(record));
        assert!(!reopened.pending_starts_turn);
        // A legacy request is replayed verbatim, never upgraded to another hash.
        meta.pending = Some(json!({"type":"prompt","id":"old","requestId":"old","message":"kept"}));
        let reopened: Meta = serde_json::from_slice(&serde_json::to_vec(&meta).unwrap()).unwrap();
        assert_eq!(reopened.pending, meta.pending);
        assert!(reopened.pending.unwrap().get("streamingBehavior").is_none());
    }

    #[test]
    fn queued_attachment_labels_follow_entries_not_send_order_or_duplicate_text() {
        let mut meta = metadata();
        for (id, name) in [("cancelled", "wrong.png"), ("a", "a.png"), ("b", "b.png")] {
            meta.attachments_by_request.insert(
                id.into(),
                AttachedPrompt {
                    labels: vec![name.into()],
                    submission: None,
                    entry: None,
                },
            );
        }
        observe_submission(
            &mut meta,
            &json!({"requestId":"cancelled","id":10,"status":"unanswered"}),
        );
        observe_submission(&mut meta, &json!({"requestId":"b","id":12,"entry":100}));
        observe_submission(&mut meta, &json!({"requestId":"a","id":11,"entry":101}));
        // A late pre-placement receipt must not clear a known entry association.
        observe_submission(&mut meta, &json!({"requestId":"b","id":"12"}));
        assert!(!observe_submission(
            &mut meta,
            &json!({"requestId":"a","id":999,"entry":999})
        ));
        let mut session = crate::demo::new_session(SessionId(1), 0, "duplicate".into(), Vec::new());
        session.turns[0].durable_entry_id = Some(100);
        let mut second = session.turns[0].clone();
        second.durable_entry_id = Some(101);
        session.turns.push(second);
        attach_labels(&meta, &mut session);
        assert_eq!(session.turns[0].attachments, ["b.png"]);
        assert_eq!(session.turns[1].attachments, ["a.png"]);
        // Placement missed across a crash is recovered with get_submission data.
        let mut reopened: Meta =
            serde_json::from_slice(&serde_json::to_vec(&meta).unwrap()).unwrap();
        assert!(observe_submission(
            &mut reopened,
            &json!({"requestId":"cancelled","id":10,"entry":102})
        ));
        assert_eq!(
            reopened.attachments_by_request["cancelled"].entry,
            Some(102)
        );
    }

    #[test]
    fn attachment_recovery_requeries_queued_or_failed_reads_and_resync_without_submission_events() {
        let mut meta = metadata();
        meta.attachments_by_request.insert(
            "queued".into(),
            AttachedPrompt {
                labels: vec!["image.png".into()],
                submission: None,
                entry: None,
            },
        );
        let now = std::time::Instant::now();
        let mut recovery = AttachmentRecovery::default();
        assert_eq!(recovery.next(&meta, now).as_deref(), Some("queued"));
        recovery.begin("read-1".into(), "queued".into(), now);
        assert!(recovery.next(&meta, now).is_none());
        recovery.complete(&json!({"id":"read-1","success":true,"data":{"submission":{"requestId":"queued","id":10,"status":"queued"}}}),&mut meta);
        assert!(recovery.next(&meta, now).is_none());
        // A placement event can be replaced by a full snapshot. It contains the
        // entry but no request mapping; resync must invalidate the early lookup.
        recovery.resync();
        assert_eq!(recovery.next(&meta, now).as_deref(), Some("queued"));
        recovery.begin("read-2".into(), "queued".into(), now);
        assert!(recovery.complete(&json!({"id":"read-2","success":true,"data":{"submission":{"requestId":"queued","id":10,"entry":42}}}),&mut meta));
        let mut session = crate::demo::new_session(SessionId(1), 0, "placed".into(), Vec::new());
        session.turns[0].durable_entry_id = Some(42);
        attach_labels(&meta, &mut session);
        assert_eq!(session.turns[0].attachments, ["image.png"]);
        assert!(recovery.next(&meta, now).is_none());
        meta.attachments_by_request.get_mut("queued").unwrap().entry = None;
        recovery.resync();
        for attempt in 0..3 {
            let at = now + std::time::Duration::from_secs(attempt);
            assert_eq!(recovery.next(&meta, at).as_deref(), Some("queued"));
            recovery.begin(format!("retry-{attempt}"), "queued".into(), at);
            assert!(!recovery.complete(
                &json!({"id":format!("retry-{attempt}"),"success":false}),
                &mut meta
            ));
        }
        assert!(
            recovery
                .next(&meta, now + std::time::Duration::from_secs(10))
                .is_none()
        );
    }

    #[test]
    fn only_matching_receipts_settle_admission_and_rejection_keeps_the_payload() {
        let mut meta = metadata();
        let original = prompt_record(
            &Prompt {
                request_id: "current".into(),
                message: "steer".into(),
                images: Vec::new(),
            },
            false,
        );
        meta.pending = Some(original.clone());
        let mut admitting = true;
        let mut error = None;
        assert!(!settle_admission(
            &mut meta,
            &mut admitting,
            &mut error,
            &json!({"id":"stale","success":true})
        ));
        assert!(admitting);
        assert_eq!(meta.pending, Some(original.clone()));
        assert!(!settle_admission(
            &mut meta,
            &mut admitting,
            &mut error,
            &json!({"id":"admit-current","success":false,"error":"rejected"})
        ));
        assert_eq!(meta.pending, Some(original));
        assert!(error.is_some());
        assert!(settle_admission(
            &mut meta,
            &mut admitting,
            &mut error,
            &json!({"id":"admit-current","success":true,"data":{"requestId":"current","submissionId":"7"}})
        ));
        assert!(meta.pending.is_none());
        assert!(error.is_none());
    }

    #[test]
    fn configuration_acknowledgements_never_replace_committed_state() {
        let mut pi = pi_core::session::Session::new("/offline".into());
        response(
            &mut pi,
            "get_state",
            json!({
                "sessionId":"local-test", "model":{"provider":"anthropic","id":"before"},
                "thinkingLevel":"high"
            }),
        )
        .unwrap();
        response(
            &mut pi,
            "get_messages",
            json!({"messages":[{"role":"user","content":"kept"}]}),
        )
        .unwrap();
        for (provider, model) in [("openai-codex", "codex"), ("anthropic", "claude")] {
            let events = json!({"key":"local-test","events":[{"type":"agent_changed","agent":{
                "model":{"provider":provider,"modelId":model},"thinkingLevel":"low"
            }}]});
            for event in pi_core::durable::events(&events, &pi, "local-test").unwrap() {
                pi.apply(&event).unwrap();
            }
            let committed = serde_json::to_value(&pi.state).unwrap();
            let mut pending = Some("selection".into());
            let mut error = Some("previous error".into());
            for command in [
                "set_model",
                "set_thinking_level",
                "get_state",
                "get_messages",
            ] {
                apply_local_response(&mut pi, &mut pending, &mut error, &json!({
                    "type":"response","id":"selection","command":command,"success":true,"data":{}
                })).unwrap();
                assert_eq!(serde_json::to_value(&pi.state).unwrap(), committed);
                assert_eq!(pi.messages.len(), 1);
            }
            assert!(pending.is_none());
            assert!(error.is_none());
            assert_eq!(pi.state.model.as_ref().unwrap().provider, provider);
        }
    }

    #[test]
    fn settings_failure_is_correlated_and_keeps_the_selected_model() {
        let mut pi = pi_core::session::Session::new("/offline".into());
        response(
            &mut pi,
            "get_state",
            json!({"model":{"provider":"anthropic","id":"kept"}}),
        )
        .unwrap();
        let mut pending = Some("current".into());
        let mut error = None;
        for id in ["old", "current"] {
            apply_local_response(
                &mut pi,
                &mut pending,
                &mut error,
                &json!({
                    "type":"response","id":id,"command":"set_model","success":false,
                    "error":"must not expose arbitrary native details"
                }),
            )
            .unwrap();
            assert_eq!(pending.is_none(), id == "current");
            assert_eq!(error.is_some(), id == "current");
            assert_eq!(pi.state.model.as_ref().unwrap().id, "kept");
        }
    }
}
