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
    },
    thread,
    time::Duration,
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
    fn validate(&self) -> Result<()> {
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
        ensure!(
            !self.messages.is_empty() && self.messages.len() <= 64,
            "conversation limit"
        );
        ensure!(
            self.messages.last().is_some_and(|m| m["role"] == "user"),
            "user turn required"
        );
        ensure!(
            self.messages
                .iter()
                .all(|m| m["role"] == "user" || m["role"] == "assistant"),
            "unsupported message role"
        );
        ensure!(
            serde_json::to_vec(&self.messages)?.len() <= 8 * 1024 * 1024,
            "conversation too large"
        );
        Ok(())
    }
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
}
pub struct ChatTask {
    cancel: Cancellation,
    state: Arc<Mutex<State>>,
    worker: thread::JoinHandle<()>,
}
impl ChatTask {
    pub fn start(request: Request, vault: Arc<dyn Vault>) -> Result<Self> {
        ensure!(
            vault.scope() == VaultScope::Accounts,
            "real chat requires accounts vault"
        );
        let provider = request.provider;
        Self::start_with_hosts(
            request,
            vault,
            HttpHost::auth(provider),
            HttpHost::inference(provider),
        )
    }
    pub(crate) fn start_with_hosts(
        request: Request,
        vault: Arc<dyn Vault>,
        auth_host: HttpHost,
        mut host: HttpHost,
    ) -> Result<Self> {
        request.validate()?;
        let account = AccountLease::reserve(request.provider, vault.scope())?;
        let cancel = Cancellation::default();
        let stopped = cancel.clone();
        let state = Arc::new(Mutex::new(State {
            snapshot: Snapshot {
                status: "preparing",
                text: String::new(),
            },
            message: None,
            records: Vec::new(),
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
                vm::execute(vm::Config {
                    host, revoked: Some(revoked.clone()), timeout: Duration::from_secs(185), max_requests: 1,
                    source: include_str!("../generated/chat.js"), entry: "startChat",
                    input: json!({"provider":request.provider.id(), "model":request.model,"thinkingLevel":request.thinking_level, "messages":request.messages}),
                }, stopped.0.clone(), None, move |ctx| {
                    ctx.globals().set("__resolvedAuth", Function::new(ctx.clone(), move || resolved.clone())?)?;
                    Ok(())
                }, move |event| {
                    if event["kind"]=="record" {
                        let record=event["record"].clone();
                        let mut state=updates.lock().unwrap_or_else(|e|e.into_inner());
                        let delta=&record["assistantMessageEvent"];
                        if delta["type"]=="text_delta" && let Some(text)=delta["delta"].as_str()
                            && state.snapshot.text.len().saturating_add(text.len())<=256*1024 {state.snapshot.text.push_str(text);}
                        // JS also bounds the aggregate serialized event stream to 4 MiB.
                        if state.records.len()<20000 {state.records.push(record);}
                    }
                })
            }));
            let mut state = output.lock().unwrap_or_else(|e| e.into_inner());
            state.snapshot.status = if stopped.is_cancelled() { "stopped" }
            else if revoked() { "account_changed" }
            else if let Ok(Ok(result)) = result && result["ok"] == true {
                state.message = Some(result["message"].clone());
                "done"
            } else { match status.load(Ordering::Relaxed) {
                401 | 403 => "auth_error", 429 => "rate_limited", 400 | 404 => "request_rejected", _ => "failed"
            }};
            // Account lease drops only after both VMs/HTTP hosts have torn down.
        })?;
        Ok(Self {
            cancel,
            state,
            worker,
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
        self.cancel.cancel();
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
