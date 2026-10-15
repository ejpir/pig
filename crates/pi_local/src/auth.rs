//! Supervised OAuth owner, separate from agent/visual VMs. No workspace capability.
//! Only the trusted packaged JS sees credentials. Public snapshots never contain them.
use crate::{Cancellation, http::HttpHost, vm};
use anyhow::{Result, anyhow, ensure};
use rquickjs::{Ctx, Exception, Function};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provider {
    Claude,
    Codex,
}
impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "anthropic",
            Self::Codex => "openai-codex",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

/// Stable storage namespaces; revocation/admission must never cross this boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultScope {
    Accounts,
    Synthetic,
}

/// Native storage returns `{generation: string, credential: object | null}`.
/// Writes atomically compare the generation and return a fresh generation on success.
/// Clear MUST change generation even for an empty account. User actions must use
/// `auth::logout` (also revokes admitted workers), not the raw clear primitive.
/// Implementations must never include credential contents in errors or diagnostics.
pub trait Vault: Send + Sync + 'static {
    fn scope(&self) -> VaultScope;
    fn read(&self, provider: Provider) -> Result<Value>;
    fn write(&self, provider: Provider, expected: &str, credential: &str)
    -> Result<Option<String>>;
    fn clear(&self, provider: Provider) -> Result<()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Login,
    Refresh,
}

static EPOCH: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static REVOKING: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
struct Revoking(usize);
impl Drop for Revoking {
    fn drop(&mut self) {
        REVOKING[self.0].fetch_sub(1, Ordering::AcqRel);
    }
}
static COMMITS: [Mutex<()>; 4] = [const { Mutex::new(()) }; 4];
fn index(provider: Provider, scope: VaultScope) -> usize {
    (if provider == Provider::Claude { 0 } else { 1 })
        + if scope == VaultScope::Synthetic { 2 } else { 0 }
}

/// Invoke on a worker: revocation is linearized at admission, even if a login worker
/// has not yet read the vault. Commit exclusion plus the native CAS cover both races.
pub fn logout(provider: Provider, vault: &dyn Vault) -> Result<()> {
    let index = index(provider, vault.scope());
    REVOKING[index].fetch_add(1, Ordering::AcqRel);
    let _revoking = Revoking(index);
    EPOCH[index].fetch_add(1, Ordering::AcqRel);
    let _commit = COMMITS[index].lock().unwrap_or_else(|e| e.into_inner());
    vault.clear(provider)
}
struct Attempt {
    provider: Provider,
    operation: Operation,
    handoff: bool,
    vault: Arc<dyn Vault>,
    host: HttpHost,
    epoch: u64,
    slot: usize,
}

#[cfg(test)]
pub(crate) static TEST_ACCOUNT_LOCK: Mutex<()> = Mutex::new(());

static BUSY: [AtomicBool; 4] = [const { AtomicBool::new(false) }; 4];
struct Admission(usize);
impl Drop for Admission {
    fn drop(&mut self) {
        BUSY[self.0].store(false, Ordering::Release);
    }
}

/// Holds provider admission from before refresh through the final inference socket cleanup.
/// Refresh owns its cancellation independently: stopping chat must not lose rotated tokens.
pub(crate) struct AccountLease {
    _admission: Admission,
    slot: usize,
    epoch: u64,
}
impl AccountLease {
    pub(crate) fn reserve(provider: Provider, scope: VaultScope) -> Result<Self> {
        let slot = index(provider, scope);
        let epoch = EPOCH[slot].load(Ordering::Acquire);
        ensure!(
            REVOKING[slot].load(Ordering::Acquire) == 0,
            "account signing out"
        );
        ensure!(
            BUSY[slot]
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "account busy"
        );
        let admission = Admission(slot);
        ensure!(
            REVOKING[slot].load(Ordering::Acquire) == 0
                && EPOCH[slot].load(Ordering::Acquire) == epoch,
            "account changed during admission"
        );
        Ok(Self {
            _admission: admission,
            slot,
            epoch,
        })
    }
    pub(crate) fn revoked(&self) -> Arc<dyn Fn() -> bool + Send + Sync> {
        let slot = self.slot;
        let epoch = self.epoch;
        Arc::new(move || EPOCH[slot].load(Ordering::Acquire) != epoch)
    }
    pub(crate) fn prepare(
        &self,
        provider: Provider,
        vault: Arc<dyn Vault>,
        host: HttpHost,
    ) -> Result<String> {
        ensure!(!(self.revoked())(), "account changed");
        let (_, rx) = mpsc::sync_channel(1);
        let result = execute(
            Attempt {
                provider,
                operation: Operation::Refresh,
                handoff: true,
                vault: vault.clone(),
                host,
                epoch: self.epoch,
                slot: self.slot,
            },
            Cancellation::default(),
            rx,
            Arc::new(Mutex::new(json!({}))),
            Arc::new(AtomicBool::new(false)),
        )?;
        ensure!(!(self.revoked())(), "account changed");
        ensure!(
            result["resolved"]["source"] == "OAuth",
            "missing resolved auth"
        );
        Ok(result["resolved"].to_string())
    }
}

/// Only an explicit empty, valid Vault record means the user is signed out.
/// Storage, refresh, revocation and malformed-record errors must stay generic.
#[derive(Debug)]
pub(crate) struct MissingAccount;
impl std::fmt::Display for MissingAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("No Local account signed in")
    }
}
impl std::error::Error for MissingAccount {}

fn account_generation(record: &Value, handoff: bool) -> Result<String> {
    let generation = record["generation"]
        .as_str()
        .ok_or_else(|| anyhow!("invalid vault record"))?;
    if handoff {
        match record.get("credential") {
            Some(Value::Null) => return Err(MissingAccount.into()),
            Some(Value::Object(_)) => {}
            _ => return Err(anyhow!("invalid vault credential")),
        }
    }
    Ok(generation.to_owned())
}

pub struct AuthTask {
    cancel: Cancellation,
    operation: Operation,
    state: Arc<Mutex<Value>>,
    input: mpsc::SyncSender<Value>,
    worker: Option<thread::JoinHandle<()>>,
}
impl AuthTask {
    pub fn start(provider: Provider, operation: Operation, vault: Arc<dyn Vault>) -> Result<Self> {
        Self::start_with_host(provider, operation, vault, HttpHost::auth(provider))
    }
    pub(crate) fn start_with_host(
        provider: Provider,
        operation: Operation,
        vault: Arc<dyn Vault>,
        host: HttpHost,
    ) -> Result<Self> {
        let admission = AccountLease::reserve(provider, vault.scope())?;
        let index = admission.slot;
        let epoch = admission.epoch;
        let attempt = Attempt {
            provider,
            operation,
            handoff: false,
            vault,
            host,
            epoch,
            slot: index,
        };
        let cancel = Cancellation::default();
        let state = Arc::new(Mutex::new(
            json!({"provider": provider.id(), "status": "running"}),
        ));
        let (input, rx) = mpsc::sync_channel(2);
        let output = state.clone();
        let stopped = cancel.clone();
        let worker = thread::Builder::new()
            .name("pi-provider-auth".into())
            .spawn(move || {
                let _admission = admission;
                let saved = Arc::new(AtomicBool::new(false));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    execute(attempt, stopped.clone(), rx, output.clone(), saved.clone())
                }));
                let mut state = output.lock().unwrap_or_else(|e| e.into_inner());
                // A completed store commit wins a simultaneous cancel; never conceal a saved account.
                let status = if saved.load(Ordering::Relaxed) || matches!(result, Ok(Ok(_))) {
                    "connected"
                } else if stopped.is_cancelled() {
                    "cancelled"
                } else {
                    "failed"
                };
                // Do not expose upstream exceptions (HTTP/token/code details may be in their text).
                *state = json!({"provider": provider.id(), "status": status});
            })?;
        Ok(Self {
            cancel,
            operation,
            state,
            input,
            worker: Some(worker),
        })
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
    pub fn snapshot(&self) -> Value {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(|w| w.is_finished())
    }
    pub fn cancel(&self) {
        // Refresh has its own bounded owner, not the lifetime of an agent/UI caller.
        // Logout invalidates its storage generation instead of destroying rotated credentials.
        if self.operation == Operation::Login {
            self.cancel.cancel();
        }
    }
    pub fn respond(&self, id: &str, code: String) -> Result<()> {
        let state = self.snapshot();
        ensure!(
            state["prompt_id"].as_str() == Some(id) && state["status"] == "prompt",
            "prompt is no longer active"
        );
        ensure!(
            !code.trim().is_empty() && code.len() <= 16 * 1024,
            "invalid authorization code length"
        );
        self.input
            .try_send(json!({"kind": "input", "id": id, "value": code}))
            .map_err(|_| anyhow!("authorization input unavailable"))
    }
    /// Worker/tests only. The UI polls is_finished and never joins a running task.
    pub fn join(mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for AuthTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn execute(
    attempt: Attempt,
    cancel: Cancellation,
    input: mpsc::Receiver<Value>,
    output: Arc<Mutex<Value>>,
    saved: Arc<AtomicBool>,
) -> Result<Value> {
    let Attempt {
        provider,
        operation,
        handoff,
        vault,
        host,
        epoch,
        slot,
    } = attempt;
    let record = vault.read(provider)?;
    ensure!(
        EPOCH[slot].load(Ordering::Acquire) == epoch,
        "auth attempt revoked"
    );
    let generation = Arc::new(Mutex::new(account_generation(&record, handoff)?));
    drop(record); // do not retain an unnecessary plaintext credential copy across the operation
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| anyhow!("OS randomness unavailable"))?;
    let session: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let worker_cancel = cancel.clone();
    let result = vm::execute(
        vm::Config {
            host,
            revoked: None,
            timeout: if operation == Operation::Login {
                Duration::from_secs(15 * 60 + 30)
            } else {
                Duration::from_secs(25)
            },
            max_requests: 1024,
            source: include_str!("../generated/auth.js"),
            entry: "startAuth",
            input: json!({"provider": provider.id(), "operation": if operation == Operation::Login { "login" } else { "refresh" }, "session": session, "handoff": handoff}),
        },
        cancel.0.clone(),
        Some(input),
        move |ctx| {
            let read_vault = vault.clone();
            let read_generation = generation.clone();
            ctx.globals().set(
                "__vaultRead",
                Function::new(
                    ctx.clone(),
                    move |ctx: Ctx<'_>| -> rquickjs::Result<String> {
                        let record = read_vault.read(provider).map_err(|_| {
                            Exception::throw_message(&ctx, "credential storage unavailable")
                        })?;
                        if record["generation"].as_str()
                            != Some(
                                read_generation
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .as_str(),
                            )
                        {
                            return Err(Exception::throw_message(&ctx, "account changed"));
                        }
                        Ok(record["credential"].to_string())
                    },
                )?,
            )?;
            let generation = RefCell::new(generation);
            ctx.globals().set(
                "__vaultWrite",
                Function::new(
                    ctx.clone(),
                    move |ctx: Ctx<'_>, credential: String| -> rquickjs::Result<()> {
                        let _commit = COMMITS[slot].lock().unwrap_or_else(|e| e.into_inner());
                        if EPOCH[slot].load(Ordering::Acquire) != epoch
                            || worker_cancel.is_cancelled()
                            || credential.len() > 64 * 1024
                        {
                            return Err(Exception::throw_message(
                                &ctx,
                                "credential write cancelled or oversized",
                            ));
                        }
                        let current = generation.borrow();
                        let mut expected = current.lock().unwrap_or_else(|e| e.into_inner());
                        let next = vault
                            .write(provider, &expected, &credential)
                            .map_err(|_| Exception::throw_message(&ctx, "credential save failed"))?
                            .ok_or_else(|| Exception::throw_message(&ctx, "account changed"))?;
                        *expected = next;
                        saved.store(true, Ordering::Relaxed);
                        Ok(())
                    },
                )?,
            )?;
            Ok(())
        },
        move |event| {
            let mut state = output.lock().unwrap_or_else(|e| e.into_inner());
            // Whitelist fields; no raw credential, exception, HTTP body, or manual-code echo.
            match event["kind"].as_str() {
                Some("browser") => {
                    if let Some(url) = event["url"]
                        .as_str()
                        .filter(|url| allowed_browser_url(provider, url))
                    {
                        state["url"] = json!(url);
                    }
                }
                Some("device") => {
                    if let Some(code) = event["code"].as_str().filter(|s| {
                        s.len() <= 64
                            && s.chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ' ')
                    }) {
                        state["device_code"] = json!(code);
                    }
                }
                Some("prompt") => {
                    if let Some(id) = event["id"].as_str().filter(|s| s.len() <= 64) {
                        state["status"] = json!("prompt");
                        state["prompt_id"] = json!(id);
                    }
                }
                Some("submitted") => {
                    state["status"] = json!("running");
                    state.as_object_mut().unwrap().remove("prompt_id");
                }
                _ => (),
            }
        },
    )?;
    ensure!(result["ok"] == true, "provider authorization failed");
    Ok(result)
}

pub fn allowed_browser_url(provider: Provider, url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    match provider {
        Provider::Claude => url.host_str() == Some("claude.ai") && url.path() == "/oauth/authorize",
        Provider::Codex => {
            url.host_str() == Some("auth.openai.com")
                && url.path() == "/codex/device"
                && url.query().is_none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct OfflineVault(Option<Value>);
    impl Vault for OfflineVault {
        fn scope(&self) -> VaultScope {
            VaultScope::Synthetic
        }
        fn read(&self, _: Provider) -> Result<Value> {
            self.0
                .clone()
                .ok_or_else(|| anyhow!("offline storage unavailable"))
        }
        fn write(&self, _: Provider, _: &str, _: &str) -> Result<Option<String>> {
            panic!("offline failure must not write credentials")
        }
        fn clear(&self, _: Provider) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn only_explicit_absence_is_missing_account_and_failed_prepare_releases_lease() {
        let _serial = TEST_ACCOUNT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for provider in [Provider::Claude, Provider::Codex] {
            for (record, missing) in [
                (None, false),
                (Some(json!({"credential":null})), false),
                (Some(json!({"generation":"0"})), false),
                (
                    Some(json!({"generation":"0","credential":"malformed"})),
                    false,
                ),
                (Some(json!({"generation":"0","credential":null})), true),
            ] {
                let lease = AccountLease::reserve(provider, VaultScope::Synthetic).unwrap();
                let error = lease
                    .prepare(
                        provider,
                        Arc::new(OfflineVault(record)),
                        HttpHost::auth(provider),
                    )
                    .unwrap_err();
                assert_eq!(error.is::<MissingAccount>(), missing);
                drop(lease);
                drop(AccountLease::reserve(provider, VaultScope::Synthetic).unwrap());
            }
            let vault: Arc<dyn Vault> = Arc::new(OfflineVault(Some(
                json!({"generation":"0","credential":null}),
            )));
            let lease = AccountLease::reserve(provider, vault.scope()).unwrap();
            assert!(
                !AccountLease::reserve(provider, vault.scope())
                    .err()
                    .unwrap()
                    .is::<MissingAccount>()
            );
            logout(provider, vault.as_ref()).unwrap();
            assert!(
                !lease
                    .prepare(provider, vault, HttpHost::auth(provider))
                    .unwrap_err()
                    .is::<MissingAccount>()
            );
        }
    }
}
