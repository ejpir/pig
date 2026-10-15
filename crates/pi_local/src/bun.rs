//! Embedded Durable transport. Native OAuth and bounded HTTP retain their existing owners.
use crate::{
    auth::{AccountLease, MissingAccount, Provider, Vault, VaultScope},
    bun_vm::Vm,
    http::{self, HttpHost, HttpJob},
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

static NEXT: AtomicU64 = AtomicU64::new(1);
static WORKER: OnceLock<mpsc::SyncSender<Command>> = OnceLock::new();
const BUNDLE: &[u8] = include_bytes!("../generated/bun-local.mjs");
struct Shared {
    error: Mutex<Option<String>>,
    closed: AtomicBool,
}
struct Open {
    id: u64,
    root: PathBuf,
    options: Value,
    vault: Arc<dyn Vault>,
    output: mpsc::SyncSender<Value>,
    shared: Arc<Shared>,
}
enum Command {
    Open(Open),
    Send(u64, Value),
    Close(u64),
}
struct Session {
    output: mpsc::SyncSender<Value>,
    shared: Arc<Shared>,
    vault: Arc<dyn Vault>,
    _lock: File,
    closing: bool,
    js_closed: bool,
    quarantined: bool,
}
struct Operation {
    engine: u64,
    closing: bool,
    started: std::time::Instant,
    provider: Provider,
    lease: AccountLease,
    jobs: HashSet<u32>,
    release: Option<u64>,
}
struct Prepared {
    engine: u64,
    request: u64,
    result: Result<(AccountLease, Value)>,
    provider: Provider,
}

pub struct Engine {
    id: u64,
    input: mpsc::SyncSender<Command>,
    output: Mutex<mpsc::Receiver<Value>>,
    shared: Arc<Shared>,
}
impl Engine {
    pub fn open(root: PathBuf, options: Value, vault: Arc<dyn Vault>) -> Result<Self> {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        ensure!(id < 9_007_199_254_740_991, "Runtime identity limit");
        let (output, rx) = mpsc::sync_channel(128);
        let shared = Arc::new(Shared {
            error: Mutex::new(None),
            closed: AtomicBool::new(false),
        });
        let input = WORKER
            .get_or_init(|| {
                let (tx, rx) = mpsc::sync_channel(128);
                thread::Builder::new()
                    .name("pi-bun-local".into())
                    .stack_size(8 * 1024 * 1024)
                    .spawn(move || worker(rx))
                    .expect("Bun worker");
                tx
            })
            .clone();
        input
            .try_send(Command::Open(Open {
                id,
                root,
                options,
                vault,
                output,
                shared: shared.clone(),
            }))
            .map_err(|_| anyhow::anyhow!("Local runtime unavailable or busy"))?;
        Ok(Self {
            id,
            input,
            output: Mutex::new(rx),
            shared,
        })
    }
    pub fn validate(&self, record: &Value) -> Result<()> {
        ensure!(
            bridge_source("send", json!([self.id, record]))?.len() <= crate::bun_vm::INPUT_LIMIT,
            "Local command exceeds bridge limit"
        );
        Ok(())
    }
    pub fn send(&self, record: Value) -> Result<()> {
        self.validate(&record)?;
        ensure!(
            !self.shared.closed.load(Ordering::Relaxed),
            "Local session is closed"
        );
        self.input
            .try_send(Command::Send(self.id, record))
            .map_err(|_| anyhow::anyhow!("Local runtime unavailable or busy"))
    }
    pub fn records(&self) -> Vec<Value> {
        let receiver = self.output.lock().unwrap_or_else(|e| e.into_inner());
        let mut records = Vec::new();
        let mut disconnected = false;
        loop {
            match receiver.try_recv() {
                Ok(record) => records.push(record),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        let error = self
            .shared
            .error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(error) = error {
            self.shared.closed.store(true, Ordering::Relaxed);
            records.push(json!({"type":"local_error","error":error}));
        } else if disconnected && !self.shared.closed.swap(true, Ordering::Relaxed) {
            records.push(json!({"type":"local_error","error":"Embedded Bun could not start or stopped. Saved state was retained."}));
        }
        records
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
        let _ = self.input.try_send(Command::Close(self.id));
    }
}
fn fail(shared: &Shared) {
    shared.closed.store(true, Ordering::Relaxed);
    *shared.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(
        "Local runtime stopped. Saved state was retained; restart the app to reopen it.".into(),
    );
}
fn provider(value: &Value) -> Result<Provider> {
    match value.as_str() {
        Some("anthropic") => Ok(Provider::Claude),
        Some("openai-codex") => Ok(Provider::Codex),
        _ => anyhow::bail!("Unsupported native account"),
    }
}
fn bridge_source(method: &str, args: Value) -> Result<String> {
    Ok(format!(
        "globalThis.__piPhone.{method}(...{});'ok'",
        serde_json::to_string(&args)?
    ))
}
fn eval(vm: &mut Vm, method: &str, args: Value) -> Result<()> {
    vm.evaluate(&bridge_source(method, args)?)?;
    Ok(())
}
fn deliver(vm: &mut Vm, id: u64, value: Value, error: bool) -> Result<()> {
    eval(
        vm,
        "deliver",
        json!([
            id,
            if error { Value::Null } else { value },
            if error {
                json!("Native account operation unavailable; check Local accounts or try again")
            } else {
                Value::Null
            }
        ]),
    )
}
fn lock(path: PathBuf) -> Result<File> {
    fs::create_dir_all(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.join("writer.lock"))?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "Local state already owned"
        );
    }
    Ok(file)
}
fn boot(vm: &mut Vm, root: &std::path::Path) -> Result<()> {
    use sha2::{Digest, Sha256};
    fs::create_dir_all(root)?;
    let runtime = root.join("runtime");
    fs::create_dir_all(&runtime)?;
    let path = runtime.join(format!("local-{:x}.mjs", Sha256::digest(BUNDLE)));
    if !path.exists() {
        fs::write(&path, BUNDLE)?;
    }
    let uri = format!("file://{}", path.display());
    vm.evaluate(&format!(
        "void import({}).catch(()=>{{globalThis.__piPhoneBootFailed=true}});'started'",
        serde_json::to_string(&uri)?
    ))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while vm.evaluate("String(typeof globalThis.__piPhone==='object')")? != "true" {
        ensure!(
            std::time::Instant::now() < deadline
                && vm.evaluate("String(globalThis.__piPhoneBootFailed===true)")? != "true",
            "Local SDK could not load"
        );
        vm.tick()?;
        thread::sleep(Duration::from_millis(2));
    }
    let home = root.join("home");
    let temp = root.join("tmp");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&temp)?;
    eval(vm, "init", json!([{"home":home,"temp":temp}]))
}
struct Network {
    receiver: Option<mpsc::Receiver<Value>>,
    jobs: HashMap<u32, (u64, HttpJob)>,
}
impl Drop for Network {
    fn drop(&mut self) {
        for (_, job) in self.jobs.values() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        drop(self.receiver.take());
        self.jobs.clear();
    }
}

fn worker(input: mpsc::Receiver<Command>) {
    let mut sessions = HashMap::<u64, Session>::new();
    let result = (|| -> Result<()> {
        let mut vm = Vm::open()?;
        let mut booted = false;
        let (prepared_tx, prepared_rx) = mpsc::channel::<Prepared>();
        let (http_tx, http_rx) = mpsc::sync_channel::<Value>(128);
        let mut operations = HashMap::<u64, Operation>::new();
        let mut network = Network {
            receiver: Some(http_rx),
            jobs: HashMap::new(),
        };
        let mut preparing = HashSet::<(u64, u64)>::new();
        let mut next_operation = 0u64;
        loop {
            for command in input.try_iter() {
                match command {
                    Command::Open(open) => {
                        if !booted {
                            if let Err(error) = boot(&mut vm, &open.root) {
                                fail(&open.shared);
                                return Err(error);
                            }
                            booted = true;
                        }
                        let directory = PathBuf::from(
                            open.options["state"]
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("Missing local state"))?,
                        );
                        match lock(directory) {
                            Ok(owner) => {
                                sessions.insert(
                                    open.id,
                                    Session {
                                        output: open.output,
                                        shared: open.shared,
                                        vault: open.vault,
                                        _lock: owner,
                                        closing: false,
                                        js_closed: false,
                                        quarantined: false,
                                    },
                                );
                                eval(&mut vm, "open", json!([open.id, open.options]))?;
                            }
                            Err(_) => {
                                *open.shared.error.lock().unwrap_or_else(|e| e.into_inner()) =
                                    Some("Local session is already open in another owner".into());
                            }
                        }
                    }
                    Command::Send(id, record) => {
                        if sessions.get(&id).is_some_and(|s| !s.closing) {
                            eval(&mut vm, "send", json!([id, record]))?;
                        }
                    }
                    Command::Close(id) => {
                        if let Some(session) = sessions.get(&id) {
                            session.shared.closed.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
            for (id, session) in &mut sessions {
                if session.shared.closed.load(Ordering::Relaxed) && !session.closing {
                    session.closing = true;
                    eval(&mut vm, "close", json!([id]))?;
                }
            }
            for prepared in prepared_rx.try_iter() {
                preparing.remove(&(prepared.engine, prepared.request));
                match prepared.result {
                    Ok((lease, resolved))
                        if sessions.get(&prepared.engine).is_some_and(|s| !s.closing) =>
                    {
                        next_operation += 1;
                        let id = next_operation;
                        operations.insert(
                            id,
                            Operation {
                                engine: prepared.engine,
                                closing: false,
                                started: std::time::Instant::now(),
                                provider: prepared.provider,
                                lease,
                                jobs: HashSet::new(),
                                release: None,
                            },
                        );
                        deliver(
                            &mut vm,
                            prepared.request,
                            json!({"operation":id,"resolved":resolved}),
                            false,
                        )?;
                    }
                    Err(error) if error.is::<MissingAccount>() => {
                        // Only an allowlisted classification crosses this boundary;
                        // never serialize native errors, Vault records or credentials.
                        eval(
                            &mut vm,
                            "deliver",
                            json!([
                                prepared.request, null, {"code":"missing_account"}
                            ]),
                        )?;
                    }
                    _ => deliver(&mut vm, prepared.request, Value::Null, true)?,
                }
            }
            for op in operations.values_mut() {
                if !sessions.get(&op.engine).is_some_and(|s| !s.closing)
                    || op.started.elapsed() > Duration::from_secs(190)
                {
                    op.closing = true;
                }
                if op.closing || op.release.is_some() {
                    for id in &op.jobs {
                        if let Some((_, job)) = network.jobs.get(id) {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
            let mut finished = Vec::new();
            for mut event in network.receiver.as_ref().unwrap().try_iter() {
                let id = event["id"].as_u64().unwrap_or(0) as u32;
                if event["kind"] == "error" {
                    event["message"] = json!("Native provider transport failed");
                }
                eval(&mut vm, "http", json!([event]))?;
                if event["kind"] == "released" {
                    finished.push(id);
                }
            }
            for id in finished {
                if let Some((operation, job)) = network.jobs.remove(&id) {
                    drop(job);
                    if let Some(op) = operations.get_mut(&operation) {
                        op.jobs.remove(&id);
                    }
                }
            }
            let released: Vec<_> = operations
                .iter()
                .filter_map(|(id, op)| {
                    ((op.release.is_some() || op.closing) && op.jobs.is_empty()).then_some(*id)
                })
                .collect();
            for id in released {
                if let Some(op) = operations.remove(&id) {
                    let request = op.release;
                    drop(op);
                    if let Some(request) = request {
                        deliver(&mut vm, request, json!(true), false)?;
                    }
                }
            }
            if booted {
                vm.tick()?;
                let records: Vec<Value> =
                    serde_json::from_str(&vm.evaluate("globalThis.__piPhone.drain()")?)?;
                for record in records {
                    let id = record["engine"].as_u64().unwrap_or(0);
                    match record["channel"].as_str() {
                        Some("protocol") => {
                            if let Some(session) = sessions.get(&id)
                                && session.output.try_send(record["record"].clone()).is_err()
                            {
                                fail(&session.shared);
                            }
                        }
                        Some("closed") => {
                            if let Some(session) = sessions.get_mut(&id) {
                                session.quarantined = record["success"] == false;
                                session.js_closed = true;
                                session.shared.closed.store(true, Ordering::Relaxed);
                            }
                        }
                        Some("host") => {
                            let request = record["id"].as_u64().unwrap_or(0);
                            match record["type"].as_str() {
                                Some("assert_owner") => deliver(
                                    &mut vm,
                                    request,
                                    json!(sessions.contains_key(&id)),
                                    !sessions.contains_key(&id),
                                )?,
                                Some("account_status") => {
                                    let available = sessions
                                        .get(&id)
                                        .and_then(|s| {
                                            provider(&record["provider"])
                                                .ok()
                                                .and_then(|p| s.vault.read(p).ok())
                                        })
                                        .is_some_and(|v| v["credential"].is_object());
                                    deliver(&mut vm, request, json!(available), false)?;
                                }
                                Some("prepare") => {
                                    if let Some(session) = sessions.get(&id).filter(|s| !s.closing)
                                    {
                                        let vault = session.vault.clone();
                                        let provider = provider(&record["provider"])?;
                                        let tx = prepared_tx.clone();
                                        preparing.insert((id, request));
                                        thread::spawn(move || {
                                            let result = (|| {
                                                ensure!(
                                                    vault.scope() == VaultScope::Accounts,
                                                    "Production account namespace required"
                                                );
                                                let lease =
                                                    AccountLease::reserve(provider, vault.scope())?;
                                                let resolved = lease.prepare(
                                                    provider,
                                                    vault,
                                                    HttpHost::auth(provider),
                                                )?;
                                                Ok((lease, serde_json::from_str(&resolved)?))
                                            })(
                                            );
                                            let _ = tx.send(Prepared {
                                                engine: id,
                                                request,
                                                result,
                                                provider,
                                            });
                                        });
                                    } else {
                                        deliver(&mut vm, request, Value::Null, true)?;
                                    }
                                }
                                Some("release") => {
                                    let operation = record["operation"].as_u64().unwrap_or(0);
                                    if let Some(op) = operations.get_mut(&operation) {
                                        ensure!(
                                            op.engine == id,
                                            "Native operation ownership mismatch"
                                        );
                                        op.release = Some(request);
                                    } else {
                                        deliver(&mut vm, request, json!(true), false)?;
                                    }
                                }
                                Some("http") => {
                                    let operation = record["operation"].as_u64().unwrap_or(0);
                                    if let Some(op) = operations.get_mut(&operation) {
                                        let id = u32::try_from(request)?;
                                        let mut host = HttpHost::inference(op.provider);
                                        host.cancelled = Some(op.lease.revoked());
                                        let job = http::spawn_fetch(
                                            host,
                                            id,
                                            record["request"].clone(),
                                            http_tx.clone(),
                                        );
                                        op.jobs.insert(id);
                                        network.jobs.insert(id, (operation, job));
                                    } else {
                                        eval(
                                            &mut vm,
                                            "http",
                                            json!([{"id":request,"kind":"error"}]),
                                        )?;
                                        eval(
                                            &mut vm,
                                            "http",
                                            json!([{"id":request,"kind":"released"}]),
                                        )?;
                                    }
                                }
                                Some("http_cancel") => {
                                    if let Some((_, job)) = network.jobs.get(&(request as u32)) {
                                        job.cancel.store(true, Ordering::Relaxed);
                                    }
                                }
                                _ => deliver(&mut vm, request, Value::Null, true)?,
                            }
                        }
                        _ => {}
                    }
                }
            }
            sessions.retain(|id, s| {
                !(!s.quarantined
                    && s.js_closed
                    && !operations.values().any(|op| op.engine == *id)
                    && !preparing.iter().any(|(engine, _)| engine == id))
            });
            thread::sleep(Duration::from_millis(5));
        }
    })();
    if result.is_err() {
        for session in sessions.values() {
            fail(&session.shared);
        }
    }
}

/// Trusted synthetic provider controls; never used by real account inference.
#[derive(Clone, Copy, Debug, Default)]
pub enum FixtureMode {
    #[default]
    Reply,
    Stall,
    Reject,
}
