//! Subscription chat, on a privileged worker VM with no tool/workspace capabilities.
//! Account admission spans independently owned refresh and inference/socket teardown.
use crate::{
    Cancellation,
    auth::{AccountLease, Provider, Vault, VaultScope},
    http::HttpHost,
    vm,
};
use anyhow::{Result, ensure};
use rquickjs::Function;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU16, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub fn models(provider: Provider) -> &'static [(String, String)] {
    static CLAUDE: OnceLock<Vec<(String, String)>> = OnceLock::new();
    static CODEX: OnceLock<Vec<(String, String)>> = OnceLock::new();
    let cell = match provider {
        Provider::Claude => &CLAUDE,
        Provider::Codex => &CODEX,
    };
    cell.get_or_init(|| {
        let data: Value = serde_json::from_str(include_str!("../generated/chat-models.json"))
            .expect("bundled catalog");
        let mut models: Vec<_> = data[provider.id()]
            .as_array()
            .expect("provider catalog")
            .iter()
            .map(|m| {
                (
                    m["id"].as_str().unwrap().to_owned(),
                    m["name"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let default = match provider {
            Provider::Claude => "claude-sonnet-4-6",
            Provider::Codex => "gpt-5.5",
        };
        models.sort_by_key(|m| m.0 != default);
        models
    })
}

/// Pinned upstream model metadata for native pickers; no account lookup or network.
pub fn catalog(provider: Provider) -> Value {
    let data: Value = serde_json::from_str(include_str!("../generated/chat-models.json"))
        .expect("bundled catalog");
    data[provider.id()].clone()
}

pub struct Request {
    pub provider: Provider,
    pub model: String,
    pub thinking_level: String,
    /// Canonical upstream user/assistant messages. Never credentials or tool results.
    pub messages: Vec<Value>,
}
impl Request {
    fn validate(&self, compact: bool) -> Result<()> {
        ensure!(
            models(self.provider).iter().any(|m| m.0 == self.model),
            "unknown model"
        );
        let model = catalog(self.provider)
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == self.model)
            .unwrap()
            .clone();
        ensure!(
            model["supportedThinkingLevels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|level| level == &self.thinking_level),
            "unsupported thinking level"
        );
        let mut images = 0;
        for message in &self.messages {
            if message["role"] == "user"
                && let Some(content) = message["content"].as_array()
            {
                let mut count = 0;
                for block in content {
                    match block["type"].as_str() {
                        Some("text") => ensure!(block["text"].is_string(), "invalid text"),
                        Some("image") => {
                            ensure!(
                                model["input"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .any(|v| v == "image"),
                                "model does not support images"
                            );
                            ensure!(
                                matches!(
                                    block["mimeType"].as_str(),
                                    Some("image/png" | "image/jpeg" | "image/webp" | "image/gif")
                                ),
                                "invalid image type"
                            );
                            let data = block["data"]
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("image data required"))?;
                            ensure!(
                                data.len() <= 1_398_104 && !data.is_empty(),
                                "image exceeds native import limit"
                            );
                            use base64::Engine;
                            ensure!(
                                base64::engine::general_purpose::STANDARD
                                    .decode(data)?
                                    .len()
                                    <= 1024 * 1024,
                                "image too large"
                            );
                            count += 1;
                            images += 1;
                        }
                        _ => anyhow::bail!("unsupported user content"),
                    }
                }
                ensure!(count <= 4, "too many images per message");
            }
        }
        if let Some(max) = model["inputLimits"]["images"]["maxPerRequest"].as_u64() {
            ensure!(images <= max, "model image limit");
        }
        ensure!(compact || !self.messages.is_empty(), "conversation limit");
        ensure!(
            compact || self.messages.last().is_some_and(|m| m["role"] == "user"),
            "user turn required"
        );
        ensure!(
            self.messages.iter().all(|m| matches!(
                m["role"].as_str(),
                Some("user" | "assistant" | "system" | "compactionSummary")
            )),
            "unsupported message role"
        );
        ensure!(
            serde_json::to_vec(&self.messages)?.len() <= 8 * 1024 * 1024,
            "conversation too large"
        );
        Ok(())
    }
}
#[derive(Clone, Default)]
pub struct SessionInput {
    pub session_id: Option<String>,
    pub checkpoint: Option<Value>,
    pub compact: bool,
    pub custom_instructions: Option<String>,
    #[cfg(test)]
    pub stop_after_compaction: bool,
}

/// Bounded, canonical upstream graph. Never reconstruct entries from projected messages.
pub fn validate_checkpoint(checkpoint: &Value, expected: Option<&str>) -> Result<()> {
    use std::collections::HashSet;
    ensure!(
        serde_json::to_vec(&checkpoint["fileEntries"])?.len() <= 8 * 1024 * 1024
            && serde_json::to_vec(&checkpoint["messages"])?.len() <= 8 * 1024 * 1024,
        "session state limit"
    );
    let id = checkpoint["sessionId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("session identity"))?;
    ensure!(
        !id.is_empty() && id.len() <= 128 && expected.is_none_or(|e| e == id),
        "session identity changed"
    );
    ensure!(
        checkpoint["cwd"] == "/workspace",
        "session namespace changed"
    );
    let entries = checkpoint["fileEntries"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("session entries"))?;
    ensure!(
        !entries.is_empty() && entries.len() <= 20000,
        "session entry limit"
    );
    let header = &entries[0];
    ensure!(
        header["type"] == "session"
            && header["version"] == 3
            && header["id"] == id
            && header["cwd"] == "/workspace",
        "invalid session header"
    );
    let mut ids = HashSet::new();
    for entry in &entries[1..] {
        ensure!(entry["type"] != "session", "duplicate header");
        let key = entry["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("entry identity"))?;
        ensure!(
            !key.is_empty() && key.len() <= 128 && !ids.contains(key),
            "duplicate entry identity"
        );
        if !entry["parentId"].is_null() {
            ensure!(
                entry["parentId"].as_str().is_some_and(|p| ids.contains(p)),
                "invalid entry parent"
            );
        }
        if entry["type"] == "compaction" && !entry["firstKeptEntryId"].is_null() {
            ensure!(
                entry["firstKeptEntryId"]
                    .as_str()
                    .is_some_and(|p| p == key || ids.contains(p)),
                "invalid compaction boundary"
            );
        }
        if entry["type"] == "message" && entry["message"]["role"] == "assistant" {
            ensure!(
                serde_json::to_vec(&entry["message"])?.len() <= 512 * 1024,
                "assistant state limit"
            );
        }
        if entry["type"] == "context_edit" {
            ensure!(
                entry["targetId"].as_str().is_some_and(|p| ids.contains(p)),
                "invalid edit target"
            );
        }
        ids.insert(key);
    }
    ensure!(
        checkpoint["leafId"].is_null()
            || checkpoint["leafId"]
                .as_str()
                .is_some_and(|p| ids.contains(p)),
        "invalid session leaf"
    );
    ensure!(
        checkpoint["messages"].is_array(),
        "missing projected context"
    );
    Ok(())
}
#[derive(Clone)]
pub struct Snapshot {
    pub status: &'static str,
    pub text: String,
}
struct State {
    snapshot: Snapshot,
    message: Option<Value>,
    records: Vec<Value>,
    checkpoint: Option<Value>,
    projected_context: Option<Value>,
    stats: Option<Value>,
    compaction_result: Option<Value>,
}
pub struct ChatTask {
    cancel: Cancellation,
    state: Arc<Mutex<State>>,
    worker: thread::JoinHandle<()>,
    input: mpsc::SyncSender<Value>,
    stop_deadline: Arc<Mutex<Option<Instant>>>,
}
impl ChatTask {
    pub fn start(request: Request, vault: Arc<dyn Vault>) -> Result<Self> {
        ensure!(
            vault.scope() == VaultScope::Accounts,
            "real chat requires accounts vault"
        );
        Self::start_session(request, SessionInput::default(), vault)
    }
    pub fn start_session(
        request: Request,
        session: SessionInput,
        vault: Arc<dyn Vault>,
    ) -> Result<Self> {
        ensure!(
            vault.scope() == VaultScope::Accounts,
            "real chat requires accounts vault"
        );
        let provider = request.provider;
        Self::start_session_with_hosts(
            request,
            session,
            vault,
            HttpHost::auth(provider),
            HttpHost::inference(provider),
        )
    }
    #[cfg(test)]
    pub(crate) fn start_with_hosts(
        request: Request,
        vault: Arc<dyn Vault>,
        auth_host: HttpHost,
        host: HttpHost,
    ) -> Result<Self> {
        Self::start_session_with_hosts(request, SessionInput::default(), vault, auth_host, host)
    }
    pub(crate) fn start_session_with_hosts(
        request: Request,
        session: SessionInput,
        vault: Arc<dyn Vault>,
        auth_host: HttpHost,
        mut host: HttpHost,
    ) -> Result<Self> {
        request.validate(session.compact)?;
        if let Some(cp) = &session.checkpoint {
            validate_checkpoint(cp, session.session_id.as_deref())?;
        }
        ensure!(
            session
                .custom_instructions
                .as_ref()
                .is_none_or(|s| s.len() <= 64 * 1024),
            "compaction instruction limit"
        );
        let (input, inbound) = mpsc::sync_channel(4);
        let account = AccountLease::reserve(request.provider, vault.scope())?;
        let cancel = Cancellation::default();
        let stopped = cancel.clone();
        let stop_deadline = Arc::new(Mutex::new(None::<Instant>));
        let deadline = stop_deadline.clone();
        let stop_http = cancel.clone();
        host.cancelled = Some(Arc::new(move || stop_http.is_cancelled()));
        let state = Arc::new(Mutex::new(State {
            snapshot: Snapshot {
                status: "preparing",
                text: String::new(),
            },
            message: None,
            records: Vec::new(),
            checkpoint: None,
            projected_context: None,
            stats: None,
            compaction_result: None,
        }));
        let output = state.clone();
        let status = Arc::new(AtomicU16::new(0));
        host.last_status = Some(status.clone());
        let worker = thread::Builder::new().name("pi-local-chat".into()).spawn(move || {
            let revoked = account.revoked();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Value> {
                ensure!(!stopped.is_cancelled(), "cancelled");
                // Once started, refresh is allowed to save a rotated credential even if
                // the UI stops/leaves. No inference request may start after cancellation.
                let resolved = account.prepare(request.provider, vault, auth_host)?;
                ensure!(!stopped.is_cancelled() && !revoked(), "cancelled");
                output.lock().unwrap_or_else(|e| e.into_inner()).snapshot.status = "streaming";
                let updates = output.clone();
                let context_output=output.clone();
                let operation_stop=stopped.clone();
                #[cfg(test)]
                let boundary_stop=stopped.clone();
                #[cfg(test)]
                let boundary_deadline=deadline.clone();
                let revoked_vm=revoked.clone();
                let stop_vm=stopped.clone();
                let hard_stop=Arc::new(move || revoked_vm() || (stop_vm.is_cancelled() && deadline.lock().unwrap_or_else(|e|e.into_inner()).is_some_and(|t|t.elapsed()>=Duration::from_secs(2))));
                vm::execute(vm::Config {
                    host, revoked: Some(hard_stop), timeout: Duration::from_secs(185), max_requests: 8,
                    source: include_str!("../generated/session.js"), entry: "startSessionOperation",
                    input: json!({"provider":request.provider.id(), "model":request.model,"thinkingLevel":request.thinking_level,
                        "messages":if session.checkpoint.is_some() {if session.compact {vec![]} else {request.messages.last().map(|m|vec![m.clone()]).unwrap_or_default()}} else {request.messages},
                        "sessionId":session.session_id,"checkpoint":session.checkpoint.as_ref().map(|cp| {let mut cp=cp.clone();cp.as_object_mut().unwrap().remove("messages");cp}),"operation":if session.compact {"compact"} else {"prompt"},"customInstructions":session.custom_instructions}),
                }, Arc::new(std::sync::atomic::AtomicBool::new(false)), Some(inbound), move |ctx| {
                    crate::sdk_platform::install(ctx)?;
                    ctx.globals().set("__operationCancelled",Function::new(ctx.clone(),move ||operation_stop.is_cancelled())?)?;
                    ctx.globals().set("__sessionContext", Function::new(ctx.clone(), move |ctx: rquickjs::Ctx<'_>, data:String| -> rquickjs::Result<()> {
                        if data.len()>8*1024*1024 {return Err(rquickjs::Exception::throw_message(&ctx,"Context limit"));}
                        let messages:Value=serde_json::from_str(&data).map_err(|_|rquickjs::Exception::throw_message(&ctx,"Invalid context"))?;
                        let mut state=context_output.lock().unwrap_or_else(|e|e.into_inner());
                        if state.projected_context.is_some() {return Err(rquickjs::Exception::throw_message(&ctx,"Duplicate context"));}
                        state.projected_context=Some(messages);Ok(())
                    })?)?;
                    ctx.globals().set("__resolvedAuth", Function::new(ctx.clone(), move || resolved.clone())?)?;
                    Ok(())
                }, move |event| {
                    if event["kind"]=="record" {
                        let record=event["record"].clone();
                        #[cfg(test)]
                        if session.stop_after_compaction && record["type"]=="compaction_end" && record["result"].is_object() {
                            boundary_deadline.lock().unwrap_or_else(|e|e.into_inner()).get_or_insert_with(Instant::now);
                            boundary_stop.cancel();
                        }
                        let mut state=updates.lock().unwrap_or_else(|e|e.into_inner());
                        if record["type"]=="compaction_start" {state.snapshot.status="compacting";}
                        else if record["type"]=="compaction_end" {state.snapshot.status="streaming";}
                        let delta=&record["assistantMessageEvent"];
                        if delta["type"]=="text_delta" && let Some(text)=delta["delta"].as_str()
                            && state.snapshot.text.len().saturating_add(text.len())<=256*1024 {state.snapshot.text.push_str(text);}
                        // JS also bounds the aggregate serialized event stream to 4 MiB.
                        if state.records.len()<20000 {state.records.push(record);}
                    }
                })
            }));
            let mut state = output.lock().unwrap_or_else(|e| e.into_inner());
            let mut outcome="failed";
            if let Ok(Ok(mut result))=result {
                if let Some(cp)=result.get_mut("checkpoint") && let Some(context)=state.projected_context.take() {cp["messages"]=context;}
                let valid=result.get("checkpoint").is_some_and(|cp|validate_checkpoint(cp,session.session_id.as_deref()).is_ok());
                if valid {
                    let cp=result["checkpoint"].clone();
                    if result["ok"]==true && !session.compact {state.message=cp["messages"].as_array().and_then(|messages|messages.last()).filter(|m|m["role"]=="assistant").cloned();}
                    state.checkpoint=Some(cp);state.stats=result.get("stats").cloned();state.compaction_result=result.get("compactionResult").cloned();
                    outcome=if result["ok"]==true {"done"} else {match result["safeError"].as_str(){Some("context_full")=>"context_full",Some("nothing_to_compact")=>"nothing_to_compact",Some("compaction_failed")=>"compaction_failed",_=>"failed"}};
                } else {outcome="sync_failed";}
            }
            state.snapshot.status = if stopped.is_cancelled() {"stopped"} else if revoked() {"account_changed"} else if outcome=="done"||outcome=="sync_failed"||outcome=="context_full"||outcome=="nothing_to_compact"||outcome=="compaction_failed" {outcome}
            else {match status.load(Ordering::Relaxed){401|403=>"auth_error",429=>"rate_limited",400|404=>"request_rejected",_=>"failed"}};
            // Account lease drops only after both VMs/HTTP hosts have torn down.
        })?;
        Ok(Self {
            cancel,
            state,
            worker,
            input,
            stop_deadline,
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut value = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone();
        if self.cancel.is_cancelled() && !self.is_finished() {
            value.status = "stopping";
        }
        value
    }
    pub fn is_finished(&self) -> bool {
        self.worker.is_finished()
    }
    pub fn cancel(&self) {
        self.stop_deadline
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(Instant::now);
        self.cancel.cancel();
        let _ = self
            .input
            .try_send(json!({"kind":"input","action":"abort"}));
    }
    pub fn take_session_result(&self) -> (Option<Value>, Option<Value>, Option<Value>) {
        if !self.is_finished() {
            return (None, None, None);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (
            state.checkpoint.take(),
            state.stats.take(),
            state.compaction_result.take(),
        )
    }
    pub fn take_records(&self) -> Vec<Value> {
        std::mem::take(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()).records)
    }
    pub fn take_message(&self) -> Option<Value> {
        if !self.is_finished() {
            return None;
        }
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .message
            .take()
    }
}
impl Drop for ChatTask {
    fn drop(&mut self) {
        self.cancel();
    }
}
