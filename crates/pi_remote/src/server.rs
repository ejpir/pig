use anyhow::{Context, Result, bail, ensure};
use async_channel::{Receiver, Sender, bounded};
use pi_core::{
    protocol::{Command, MAX_RECORD_BYTES, read_record},
    session::Session,
    ssh::{PROTOCOL_VERSION, RemoteBackend, SshTarget},
    transport::{Launch, RpcClient, TransportEvent},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, BufRead as _, BufReader, Cursor, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::PathBuf,
    process::{Command as ProcessCommand, Stdio},
    thread,
    time::{Duration, Instant},
};

// Keep lock placement stable across wire-protocol upgrades: never open a second Pi
// against the same session file just because a new helper speaks a different protocol.
const STATE_LAYOUT_VERSION: &str = "1";

#[derive(Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    token: String,
    cwd: String,
    version: u32,
    #[serde(default)]
    backend: RemoteBackend,
    #[serde(default)]
    pid: u32,
}
pub(crate) fn root() -> Result<PathBuf> {
    let path = std::env::var_os("PI_DESKTOP_REMOTE_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".pi/desktop/run")))
        .context("No remote home directory")?
        .join(STATE_LAYOUT_VERSION);
    fs::create_dir_all(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}
pub(crate) fn private_file(path: &std::path::Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
const LEGACY_PROTOCOL_VERSION: u32 = 1;
const LEGACY_MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
const MAX_PENDING_HANDSHAKES: usize = 32;
// A valid 4096-byte cwd can expand to nearly 24 KiB through JSON escaping.
const MAX_HANDSHAKE_BYTES: usize = 32 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Admission happens before spawning: silent, unauthenticated clients cannot
/// allocate an unlimited number of workers. Authenticated attachments do not
/// retain a handshake slot.
struct HandshakePool {
    available: Receiver<()>,
    returned: Sender<()>,
}
impl HandshakePool {
    fn new(capacity: usize) -> Self {
        let (returned, available) = bounded(capacity);
        for _ in 0..capacity {
            returned.try_send(()).expect("Empty handshake pool");
        }
        Self {
            available,
            returned,
        }
    }

    fn try_acquire(&self) -> Option<HandshakePermit> {
        self.available.try_recv().ok()?;
        Some(HandshakePermit(self.returned.clone()))
    }
}
struct HandshakePermit(Sender<()>);
impl Drop for HandshakePermit {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

fn handshake_remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .context("Daemon attachment handshake timed out")
}

/// Bound accumulation before parsing and use a single deadline across hello
/// and ready. Per-read timeouts alone let trickling clients retain a worker.
fn read_handshake_record(
    reader: &mut BufReader<TcpStream>,
    deadline: Instant,
) -> Result<Option<Value>> {
    let mut bytes = Vec::new();
    loop {
        reader
            .get_ref()
            .set_read_timeout(Some(handshake_remaining(deadline)?))?;
        let buffer = reader
            .fill_buf()
            .context("reading daemon attachment handshake")?;
        handshake_remaining(deadline)?;
        if buffer.is_empty() {
            // Reuse the strict RPC parser, including its unterminated-record check.
            return read_record(&mut Cursor::new(bytes));
        }
        let count = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |index| index + 1);
        ensure!(
            bytes.len() + count <= MAX_HANDSHAKE_BYTES,
            "Daemon attachment handshake record exceeds {MAX_HANDSHAKE_BYTES} bytes"
        );
        bytes.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if bytes.last() == Some(&b'\n') {
            return read_record(&mut Cursor::new(bytes));
        }
        // Reject a full unterminated record without waiting for another byte.
        ensure!(
            bytes.len() < MAX_HANDSHAKE_BYTES,
            "Daemon attachment handshake record exceeds {MAX_HANDSHAKE_BYTES} bytes"
        );
    }
}

fn write_record_with_limit(
    writer: &mut impl Write,
    record: &Value,
    max_record_bytes: usize,
) -> Result<()> {
    let bytes = serde_json::to_vec(record)?;
    ensure!(
        bytes.len() < max_record_bytes,
        "Remote record exceeds {} MiB",
        max_record_bytes / (1024 * 1024)
    );
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
fn write_record(writer: &mut impl Write, record: &Value) -> Result<()> {
    write_record_with_limit(writer, record, MAX_RECORD_BYTES)
}
fn spawn_daemon(target: &SshTarget) -> Result<()> {
    let command = |breakaway: bool| -> Result<ProcessCommand> {
        let mut command = ProcessCommand::new(std::env::current_exe()?);
        command
            .arg("daemon")
            .arg(serde_json::to_string(target)?)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(private_file(
                &root()?.join(format!("{}.log", target.key)),
            )?));
        #[cfg(unix)]
        {
            let _ = breakaway;
            use std::os::unix::process::CommandExt;
            // SAFETY: setsid is async-signal-safe and touches no Rust state after fork.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{
                CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
            };
            let mut flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
            if breakaway {
                flags |= CREATE_BREAKAWAY_FROM_JOB;
            }
            command.creation_flags(flags);
        }
        Ok(command)
    };
    let spawned = command(true)?.spawn();
    // A job that forbids breaking away (as CI runners and some SSH services
    // use) refuses the first try; the daemon then lives in that job.
    #[cfg(windows)]
    let spawned = match spawned {
        Err(error) if error.raw_os_error() == Some(5) => command(false)?.spawn(),
        spawned => spawned,
    };
    let mut child = spawned.context("Could not detach remote daemon")?;
    // Reap if it exits before the bridge. The child owns no inherited SSH pipes.
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
struct Attachment {
    reader: BufReader<TcpStream>,
    prefetched: Vec<Value>,
    daemon_version: u32,
    max_record_bytes: usize,
}

fn read_endpoint(path: &std::path::Path, target: &SshTarget) -> Result<Endpoint> {
    let endpoint: Endpoint = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        endpoint.cwd == target.cwd && endpoint.backend == target.backend,
        "Remote session identity mismatch"
    );
    Ok(endpoint)
}

fn session_lock_is_held(endpoint_path: &std::path::Path) -> Result<bool> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(endpoint_path.with_extension("lock"))?;
    match lock.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn connect_endpoint(target: &SshTarget, endpoint: &Endpoint) -> Result<BufReader<TcpStream>> {
    let stream = TcpStream::connect_timeout(
        &([127, 0, 0, 1], endpoint.port).into(),
        Duration::from_secs(2),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream);
    write_record_with_limit(
        reader.get_mut(),
        &json!({"type":"hello", "token":endpoint.token, "version":endpoint.version, "cwd":target.cwd, "backend":target.backend}),
        if endpoint.version == LEGACY_PROTOCOL_VERSION {
            LEGACY_MAX_RECORD_BYTES
        } else {
            MAX_RECORD_BYTES
        },
    )?;
    let hello = read_record(&mut reader)?.context("Daemon closed before hello")?;
    ensure!(
        hello["type"] == "remote_hello"
            && hello["version"] == endpoint.version
            && hello["key"] == target.key,
        "Remote daemon handshake mismatch"
    );
    // Keep this reader: replacing it could discard bytes it buffered after the hello.
    write_record_with_limit(
        reader.get_mut(),
        &json!({"type":"ready"}),
        if endpoint.version == LEGACY_PROTOCOL_VERSION {
            LEGACY_MAX_RECORD_BYTES
        } else {
            MAX_RECORD_BYTES
        },
    )?;
    Ok(reader)
}

fn attach_legacy(endpoint: &Endpoint, mut reader: BufReader<TcpStream>) -> Result<Attachment> {
    let snapshot = read_record(&mut reader)?.context(
        "Protocol-1 daemon closed before its snapshot; its session may exceed the legacy 16 MiB limit",
    )?;
    reader.get_mut().set_read_timeout(None)?;
    // An old daemon cannot be retired atomically: another attached client could
    // submit work between its idle snapshot and its v1 shutdown check. Keep it
    // attached at its negotiated limit until it exits on its own.
    Ok(Attachment {
        reader,
        prefetched: vec![snapshot],
        daemon_version: endpoint.version,
        max_record_bytes: LEGACY_MAX_RECORD_BYTES,
    })
}

fn current_attachment(target: &SshTarget, path: &std::path::Path) -> Result<Attachment> {
    let endpoint = read_endpoint(path, target)?;
    ensure!(
        endpoint.version == PROTOCOL_VERSION,
        "Remote daemon protocol mismatch"
    );
    let reader = connect_endpoint(target, &endpoint)?;
    reader.get_ref().set_read_timeout(None)?;
    Ok(Attachment {
        reader,
        prefetched: Vec::new(),
        daemon_version: endpoint.version,
        max_record_bytes: MAX_RECORD_BYTES,
    })
}

fn attach(target: &SshTarget) -> Result<Attachment> {
    ensure!(
        !root()?
            .join(format!("{}.deleted.json", target.key))
            .exists(),
        "This session was permanently deleted"
    );
    let path = root()?.join(format!("{}.json", target.key));
    if let Ok(endpoint) = read_endpoint(&path, target) {
        if endpoint.version == PROTOCOL_VERSION {
            if let Ok(attachment) = current_attachment(target, &path) {
                return Ok(attachment);
            }
        } else if endpoint.version == LEGACY_PROTOCOL_VERSION {
            if let Ok(reader) = connect_endpoint(target, &endpoint) {
                return attach_legacy(&endpoint, reader);
            }
        } else if session_lock_is_held(&path)? {
            bail!("Unsupported remote daemon protocol {}", endpoint.version);
        }
    }
    spawn_daemon(target)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match current_attachment(target, &path) {
            Ok(attachment) => return Ok(attachment),
            Err(error) if Instant::now() >= deadline => {
                use std::io::Read as _;
                let mut detail = String::new();
                if let Ok(file) = File::open(root()?.join(format!("{}.log", target.key))) {
                    let _ = file.take(8192).read_to_string(&mut detail);
                }
                return Err(error).context(format!("Remote daemon did not start; check the project directory and selected backend installation. {detail}"));
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// A scheduler is an ordinary helper client, never a runner owner or UI
/// controller. Reattachment uses the same endpoint checks, daemon locks and
/// durable worker ownership as desktop/phone attachment.
pub(crate) struct SchedulerAttachment {
    reader: BufReader<TcpStream>,
}
impl SchedulerAttachment {
    pub(crate) fn open(target: &SshTarget) -> Result<Self> {
        target.validate()?;
        ensure!(
            target.backend == RemoteBackend::Durable,
            "Scheduler requires a durable session"
        );
        let attachment = attach(target)?;
        ensure!(
            attachment.daemon_version == PROTOCOL_VERSION,
            "Scheduler requires the current helper protocol"
        );
        attachment
            .reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(10)))?;
        Ok(Self {
            reader: attachment.reader,
        })
    }
    /// Cleanup is never permission to spawn/recover a runner. If its endpoint
    /// is absent the owner already retired (or is unavailable), retain history.
    pub(crate) fn open_existing(target: &SshTarget) -> Result<Option<Self>> {
        target.validate()?;
        ensure!(
            target.backend == RemoteBackend::Durable,
            "Scheduler requires a durable session"
        );
        let path = root()?.join(format!("{}.json", target.key));
        if !path.exists() {
            return Ok(None);
        }
        let attachment = current_attachment(target, &path)?;
        attachment
            .reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(10)))?;
        Ok(Some(Self {
            reader: attachment.reader,
        }))
    }
    pub(crate) fn call(&mut self, mut record: Value) -> Result<(Value, Option<Value>)> {
        let id = format!("scheduler-read-{:032x}", rand::random::<u128>());
        record["id"] = json!(id);
        write_record(self.reader.get_mut(), &record)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut snapshot = None;
        loop {
            ensure!(
                Instant::now() < deadline,
                "Scheduler helper request timed out"
            );
            let value = read_record(&mut self.reader)?
                .context("Scheduler attachment closed before acknowledgement")?;
            if value["type"] == "remote_snapshot" {
                snapshot = Some(value["data"].clone());
            }
            if value["type"] == "response" && value["id"] == id {
                ensure!(
                    value["success"] == true,
                    "{}",
                    value["error"].as_str().unwrap_or("Helper request failed")
                );
                return Ok((value["data"].clone(), snapshot));
            }
            if value["type"] == "remote_error" {
                bail!("{}", value["error"]);
            }
        }
    }
}
impl Drop for SchedulerAttachment {
    fn drop(&mut self) {
        let _ = self.reader.get_ref().shutdown(Shutdown::Both);
    }
}

/// Session-wide ownership safety, NOT an occurrence outcome. Unknown subagent
/// details are not evidence that its background work has finished.
/// Unlike the UI handoff fallback, a runtime safety check must never infer
/// that children ended merely because their parent tool returned.
pub(crate) fn scheduler_handoff(
    tool: &pi_core::session::Tool,
) -> Option<pi_core::subagent::Handoff> {
    if tool.details["version"] != 1 || !tool.details["background"].is_boolean() {
        return None;
    }
    pi_core::subagent::Handoff::from_details(&tool.details)
}
/// Background detail updates do not reopen a tool whose result was already
/// committed. Use the existing terminal tool record for both helper and UI
/// projections; an actually-running tool continues to receive update records.
fn normalize_subagent_progress(record: &mut Value, model: &Session) {
    if record["type"] == "tool_execution_update"
        && let Some(tool) = model
            .tools
            .iter()
            .find(|tool| tool.id == record["toolCallId"].as_str().unwrap_or("") && tool.finished)
    {
        record["type"] = json!("tool_execution_end");
        record["result"] = record["partialResult"].clone();
        record["isError"] = json!(tool.is_error);
    }
}
pub(crate) fn scheduler_idle(model: &Session) -> bool {
    !model.busy()
        && model.steering.is_empty()
        && model.follow_up.is_empty()
        && model.queued_submissions.is_empty()
        && model.tools.iter().all(|tool| {
            tool.finished
                && (tool.name != "subagent"
                    || scheduler_handoff(tool).is_some_and(|handoff| {
                        handoff
                            .subagents
                            .iter()
                            .all(|agent| agent.status.finished())
                    }))
        })
}

/// Keep durable entry identities privately in the helper. The UI's flattened
/// Session messages deliberately do not retain them. A receipt's `answer` is
/// the ONLY authority for which assistant answered a scheduled submission.
#[derive(Default)]
struct SubmissionTranscript {
    entries: BTreeMap<u64, Value>,
    submissions: HashMap<String, Value>,
}
impl SubmissionTranscript {
    fn apply(&mut self, record: &Value) {
        if record["type"] == "durable_state" {
            self.entries.clear();
            for entry in record["data"]["entries"].as_array().into_iter().flatten() {
                self.insert(entry);
            }
        } else if record["type"] == "durable_events" {
            for event in record["events"].as_array().into_iter().flatten() {
                if event["type"] == "message_end" || event["type"] == "entry_appended" {
                    self.insert(&event["entry"]);
                } else if event["type"] == "submission"
                    && let Some(id) = event["record"]["requestId"].as_str()
                    && id.starts_with("subagent-report-")
                {
                    self.submissions.insert(id.into(), event["record"].clone());
                }
            }
        }
    }
    fn insert(&mut self, entry: &Value) {
        if let Some(id) = entry["id"].as_u64() {
            self.entries.insert(id, entry.clone());
        }
    }
    fn answer_span(&self, submission: &Value) -> Option<Value> {
        let start = submission["entry"].as_u64()?;
        let end = submission["answer"].as_u64()?;
        if submission["type"] != "input" || submission["status"] != "done" || end < start {
            return None;
        }
        let input = self.entries.get(&start)?["model"].as_array()?;
        let answer = self.entries.get(&end)?["model"].as_array()?;
        if !input.iter().any(|m| m["role"] == "user")
            || answer.len() != 1
            || answer[0]["role"] != "assistant"
        {
            return None;
        }
        let mut messages = Vec::new();
        for entry in self.entries.range(start..=end).map(|(_, entry)| entry) {
            messages.extend(entry["model"].as_array()?.iter().cloned());
        }
        Some(json!({"entry":start,"answer":end,"messages":messages}))
    }
    fn correlated(&self, submission: &Value) -> Option<Value> {
        let mut scoped = self.answer_span(submission)?;
        let reports = self.settled_reports();
        let mut descendants = serde_json::Map::new();
        let mut pending = vec![scoped["messages"].as_array()?];
        // Follow exact report receipts, not the whole transcript after A. A
        // report may itself delegate and finish before that delegate reports.
        // Missing/ambiguous receipts stay missing so the worker retains A.
        while let Some(messages) = pending.pop() {
            for id in messages
                .iter()
                .flat_map(|m| m["content"].as_array().into_iter().flatten())
                .filter(|b| b["type"] == "toolCall")
                .filter_map(|b| b["id"].as_str())
            {
                if descendants.contains_key(id) {
                    continue;
                }
                if let Some(report) = reports.get(id) {
                    descendants.insert(id.into(), report.clone());
                    if let Some(messages) = report["messages"].as_array() {
                        pending.push(messages);
                    }
                }
            }
        }
        scoped["reports"] = Value::Object(descendants);
        Some(scoped)
    }
    /// A child's terminal status can precede its crew's queued report. Require
    /// the exact internal report's entire answer span, not just its last answer.
    /// Cold helpers lacking receipt events conservatively retain owners. An
    /// unanswered report has no end boundary: its continuation remains unknown.
    fn settled_reports(&self) -> Value {
        let mut reports = serde_json::Map::new();
        for submission in self.submissions.values() {
            let Some(entry) = submission["entry"]
                .as_u64()
                .and_then(|id| self.entries.get(&id))
            else {
                continue;
            };
            let Some(content) = entry["model"]
                .as_array()
                .and_then(|messages| messages.first())
                .filter(|m| m["role"] == "user")
                .and_then(|m| m["content"].as_str())
            else {
                continue;
            };
            let Some(call) = content
                .strip_prefix("<subagent_report call=\"")
                .and_then(|s| s.split_once("\">\n").map(|(call, _)| call))
            else {
                continue;
            };
            let settled = self.answer_span(submission).and_then(|span| {
                let answer = span["messages"].as_array()?.last()?;
                if !matches!(answer["stopReason"].as_str(), Some("stop" | "length" | "aborted" | "error")) {
                    return None;
                }
                Some(json!({"status":"done","receipt":submission,"messages":span["messages"],"answer":answer}))
            }).unwrap_or(Value::Null);
            // Never choose an arbitrary report if call IDs/receipts collide.
            if let Some(previous) = reports.get_mut(call) {
                *previous = Value::Null;
            } else {
                reports.insert(call.into(), settled);
            }
        }
        Value::Object(reports)
    }
    fn background_settled(&self, model: &Session) -> bool {
        let reports = self.settled_reports();
        model
            .tools
            .iter()
            .filter(|tool| tool.name == "subagent")
            .all(|tool| {
                scheduler_handoff(tool).is_some_and(|handoff| {
                    !handoff.background
                        || reports
                            .get(&tool.id)
                            .is_some_and(|report| !report.is_null())
                })
            })
    }
}

fn can_retire_owner(
    model: &Session,
    clients: usize,
    requests: usize,
    bootstrap: usize,
    dialogs: usize,
    transcript: &SubmissionTranscript,
) -> bool {
    clients == 1
        && requests == 0
        && bootstrap == 0
        && dialogs == 0
        && scheduler_idle(model)
        && transcript.background_settled(model)
}

fn normalize_daemon_record(mut record: Value, daemon_version: u32) -> Value {
    if daemon_version != PROTOCOL_VERSION && record["type"] == "remote_snapshot" {
        record["version"] = json!(PROTOCOL_VERSION);
    }
    record
}

pub fn connect() -> Result<()> {
    let mut input = BufReader::new(std::io::stdin());
    let request = read_record(&mut input)?.context("Missing remote attachment")?;
    ensure!(
        request["type"] == "remote_attach" && request["version"] == PROTOCOL_VERSION,
        "Unsupported remote protocol"
    );
    let target: SshTarget = serde_json::from_value(request["target"].clone())?;
    target.validate()?;
    let attachment = attach(&target)?;
    let daemon_version = attachment.daemon_version;
    let max_record_bytes = attachment.max_record_bytes;
    let mut upstream = attachment.reader.get_ref().try_clone()?;
    thread::spawn(move || {
        while let Ok(Some(record)) = read_record(&mut input) {
            if let Err(error) = write_record_with_limit(&mut upstream, &record, max_record_bytes) {
                eprintln!("Remote bridge write failed: {error:#}");
                break;
            }
        }
        // EOF detaches the bridge, never aborts the daemon's Pi. Keep the read
        // half open so Windows receives the daemon's orderly close instead of
        // reporting this local detach as WSAECONNABORTED.
        let _ = upstream.shutdown(Shutdown::Write);
    });
    let mut reader = attachment.reader;
    let mut output = std::io::stdout().lock();
    let mut received = false;
    for record in attachment.prefetched {
        received = true;
        write_record(
            &mut output,
            &normalize_daemon_record(record, daemon_version),
        )?;
    }
    while let Some(record) = read_record(&mut reader)? {
        received = true;
        write_record(
            &mut output,
            &normalize_daemon_record(record, daemon_version),
        )?;
    }
    if !received {
        use std::io::Read as _;
        let mut detail = String::new();
        if let Ok(file) = File::open(root()?.join(format!("{}.log", target.key))) {
            let _ = file.take(8192).read_to_string(&mut detail);
        }
        bail!("Remote connection closed before its initial snapshot. {detail}");
    }
    Ok(())
}

enum Event {
    Attach {
        connection: u64,
        output: Sender<Value>,
        /// Disconnects once everything sent to `output` is written.
        written: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    },
    Request {
        connection: u64,
        record: Value,
    },
    Detach(u64),
    Backend(TransportEvent),
}
/// One attached app. Several may watch a session at once, a desktop and a phone:
/// every event goes to all of them, and each response to the app that asked.
struct Client {
    output: Sender<Value>,
    written: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
/// Closes every connection once what was sent to it is written, so the last
/// replies reach the desktop before the daemon exits.
fn close_all(clients: impl IntoIterator<Item = Client>) {
    let clients: Vec<Client> = clients.into_iter().collect();
    for client in &clients {
        client.output.close();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    for client in clients {
        let Ok(written) = client.written.into_inner() else {
            continue;
        };
        let _ = written.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()));
    }
}
impl Client {
    fn send(&self, record: Value) -> bool {
        if record["type"] == "remote_snapshot"
            && serde_json::to_vec(&record).map_or(true, |bytes| bytes.len() >= MAX_RECORD_BYTES)
        {
            let _ = self.output.try_send(json!({"type":"remote_error", "error":"Remote snapshot exceeds the 64 MiB transport limit"}));
            self.output.close();
            return false;
        }
        if self.output.try_send(record).is_err() {
            // Never block agent execution on a slow or disconnected desktop.
            self.output.close();
            return false;
        }
        true
    }
}
struct Request {
    connection: u64,
    original: String,
    command: String,
}

fn spawn_attachment(
    stream: TcpStream,
    token: &str,
    target: &SshTarget,
    events: &Sender<Event>,
    handshakes: &HandshakePool,
) -> io::Result<bool> {
    let Some(permit) = handshakes.try_acquire() else {
        return Ok(false); // Drop the socket, rather than queueing it or spawning.
    };
    let token = token.to_owned();
    let target = target.clone();
    let events = events.clone();
    // A failed spawn drops the closure (and its permit) instead of panicking.
    thread::Builder::new()
        .name("pi-remote-attachment".into())
        .spawn(move || {
            let _ = serve(stream, token, target, events, permit);
        })?;
    Ok(true)
}

fn serve(
    stream: TcpStream,
    token: String,
    target: SshTarget,
    events: Sender<Event>,
    permit: HandshakePermit,
) -> Result<()> {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut reader = BufReader::new(stream.try_clone()?);
    let hello = read_handshake_record(&mut reader, deadline)?.context("No hello")?;
    ensure!(
        hello["type"] == "hello"
            && hello["token"] == token
            && hello["version"] == PROTOCOL_VERSION
            && hello["cwd"] == target.cwd
            && hello
                .get("backend")
                .map(|value| serde_json::from_value::<RemoteBackend>(value.clone()))
                .transpose()?
                .unwrap_or_default()
                == target.backend,
        "Invalid daemon attachment"
    );
    let mut writer = stream;
    writer.set_write_timeout(Some(handshake_remaining(deadline)?))?;
    write_record(
        &mut writer,
        &json!({"type":"remote_hello", "version":PROTOCOL_VERSION, "key":target.key}),
    )?;
    ensure!(
        read_handshake_record(&mut reader, deadline)?.is_some_and(|r| r["type"] == "ready"),
        "No ready message"
    );
    reader.get_ref().set_read_timeout(None)?;
    writer.set_write_timeout(Some(Duration::from_secs(10)))?;
    drop(permit);
    let connection = rand::random();
    let (output, records) = bounded::<Value>(256);
    let (done, written) = std::sync::mpsc::channel::<()>();
    let written = std::sync::Mutex::new(written);
    events.send_blocking(Event::Attach {
        connection,
        output,
        written,
    })?;
    thread::spawn(move || {
        while let Ok(record) = records.recv_blocking() {
            if write_record(&mut writer, &record).is_err() {
                break;
            }
        }
        let _ = writer.shutdown(Shutdown::Both);
        drop(done);
    });
    let result = (|| -> Result<()> {
        while let Some(record) = read_record(&mut reader)? {
            events.send_blocking(Event::Request { connection, record })?;
        }
        Ok(())
    })();
    let _ = events.send_blocking(Event::Detach(connection));
    result
}
fn snapshot(model: &Session, target: &SshTarget) -> Result<Value> {
    Ok(
        json!({"type":"remote_snapshot", "version":PROTOCOL_VERSION, "key":target.key, "data":serde_json::to_value(model)?}),
    )
}
fn unsolicited_read_update(
    internal: bool,
    bootstrapping: bool,
    record: &Value,
    model: &Session,
    target: &SshTarget,
) -> Result<Option<Value>> {
    if !internal || bootstrapping {
        return Ok(None);
    }
    let command = record["command"].as_str();
    if !matches!(command, Some("get_state" | "get_session_stats")) {
        return Ok(None);
    }
    if target.backend != RemoteBackend::Durable {
        return snapshot(model, target).map(Some);
    }
    match command {
        Some("get_state") => crate::durable::state_update(model).map(Some),
        Some("get_session_stats") => crate::durable::stats_update(model).map(Some),
        _ => unreachable!(),
    }
}
fn compaction_count(model: &Session) -> usize {
    model
        .messages
        .iter()
        .filter(|message| message["role"] == "compactionSummary")
        .count()
}
fn durable_stats_need_refresh(
    was_busy: bool,
    model: &Session,
    previous_compactions: usize,
) -> bool {
    (was_busy && !model.busy()) || compaction_count(model) > previous_compactions
}
// Refresh once per completed answer/model change, never on streaming token updates.
fn durable_context_changed(record: &Value) -> bool {
    record["events"].as_array().is_some_and(|events| {
        events.iter().any(|event| {
            event["type"] == "agent_changed"
                || (event["type"] == "message_end"
                    && event["entry"]["model"][0]["role"] == "assistant")
        })
    })
}

fn reply(id: &str, command: &str, data: Value) -> Value {
    json!({"type":"response", "id":id, "command":command, "success":true, "data":data})
}
fn failure(id: &str, command: &str, error: &str) -> Value {
    json!({"type":"response", "id":id, "command":command, "success":false, "error":error})
}

pub fn daemon(target: SshTarget) -> Result<()> {
    target.validate()?;
    let root = root()?;
    // Before the lock too: a deleted session's daemon may still hold it while exiting.
    ensure!(
        !root.join(format!("{}.deleted.json", target.key)).exists(),
        "This session was permanently deleted"
    );
    let lock_path = root.join(format!("{}.lock", target.key));
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    // The OS releases this lock on crash. Never unlink it while another process could hold it.
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    ensure!(
        !root.join(format!("{}.deleted.json", target.key)).exists(),
        "This session was permanently deleted"
    );
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let endpoint = Endpoint {
        port: listener.local_addr()?.port(),
        token: format!("{:032x}", rand::random::<u128>()),
        cwd: target.cwd.clone(),
        version: PROTOCOL_VERSION,
        backend: target.backend,
        pid: std::process::id(),
    };
    let path = root.join(format!("{}.json", target.key));
    let _ = fs::remove_file(&path); // Only the lock owner can replace a stale endpoint.
    // Start Pi before publishing the endpoint so a failed launch is not mistaken for a daemon.
    let cwd = PathBuf::from(&target.cwd)
        .canonicalize()
        .context("Remote project must exist")?;
    ensure!(cwd.is_dir(), "Remote project must be a directory");
    let identity_path = root.join(format!("{}.identity.json", target.key));
    let identity = json!({"cwd":target.cwd,"backend":target.backend});
    if identity_path.exists() {
        ensure!(
            serde_json::from_slice::<Value>(&fs::read(&identity_path)?)? == identity,
            "Remote session backend/directory cannot be changed; create a new session instead"
        );
    } else {
        let mut file = private_file(&identity_path)?;
        serde_json::to_writer(&mut file, &identity)?;
        file.sync_all()?;
    }
    let mut launch = match target.backend {
        RemoteBackend::Pi => Launch::pi_with(
            cwd.clone(),
            target.session_file.as_deref(),
            &crate::backend()?,
        ),
        RemoteBackend::Durable => crate::durable::launch(&target, cwd.clone())?,
    };
    launch.env.extend([
        ("PI_DESKTOP_REMOTE".into(), "1".into()),
        // Never inherit a bridge belonging to a different desktop/session.
        ("PI_DESKTOP_LSP".into(), "".into()),
        ("PI_DESKTOP_LSP_TOKEN".into(), "".into()),
        ("PI_DESKTOP_JJ_TOOLS".into(), "0".into()),
    ]);
    let backend = match target.backend {
        RemoteBackend::Pi => RpcClient::spawn(launch)?,
        RemoteBackend::Durable => RpcClient::spawn_forwarded(launch)?,
    };
    let mut history = crate::history::Recorder::open(&cwd);
    let result = run(
        listener,
        &endpoint,
        &target,
        backend,
        Session::new(cwd),
        &path,
        &mut history,
    );
    let _ = fs::remove_file(path);
    drop(lock);
    result
}
fn run(
    listener: TcpListener,
    endpoint: &Endpoint,
    target: &SshTarget,
    backend: RpcClient,
    mut model: Session,
    path: &std::path::Path,
    history: &mut crate::history::Recorder,
) -> Result<()> {
    let (events, incoming) = bounded::<Event>(512);
    let pi_events = backend.events();
    let sink = events.clone();
    thread::spawn(move || {
        while let Ok(event) = pi_events.recv_blocking() {
            if sink.send_blocking(Event::Backend(event)).is_err() {
                break;
            }
        }
    });
    let sink = events.clone();
    let token = endpoint.token.clone();
    let identity = target.clone();
    thread::spawn(move || {
        let handshakes = HandshakePool::new(MAX_PENDING_HANDSHAKES);
        for stream in listener.incoming().flatten() {
            let _ = spawn_attachment(stream, &token, &identity, &sink, &handshakes);
        }
    });
    let mut bootstrap = HashSet::new();
    for command in [
        Command::GetState,
        Command::GetMessages,
        Command::GetSessionStats,
    ] {
        bootstrap.insert(backend.send(command)?);
    }
    let mut staging = private_file(&path.with_extension("partial"))?;
    serde_json::to_writer(&mut staging, endpoint)?;
    staging.flush()?;
    fs::rename(path.with_extension("partial"), path)?;
    let mut clients = HashMap::<u64, Client>::new();
    let mut requests = HashMap::<String, Request>::new();
    let mut dialogs = HashMap::<String, Value>::new();
    let mut transcript = SubmissionTranscript::default();
    let mut summary = None;
    while let Ok(event) = incoming.recv_blocking() {
        match event {
            Event::Attach {
                connection,
                output,
                written,
            } => {
                let attached = Client { output, written };
                if bootstrap.is_empty() && !attached.send(snapshot(&model, target)?) {
                    attached.output.close();
                    continue;
                }
                if bootstrap.is_empty() {
                    for dialog in dialogs.values() {
                        attached.send(dialog.clone());
                    }
                }
                clients.insert(connection, attached);
            }
            Event::Detach(connection) => {
                if let Some(old) = clients.remove(&connection) {
                    old.output.close();
                }
            }
            Event::Request { connection, record } => {
                let Some(attached) = clients.get(&connection) else {
                    continue;
                };
                if record["type"] == "remote_delete_session" {
                    let id = record["id"].as_str().unwrap_or("");
                    if record["confirmKey"].as_str() != Some(&target.key) {
                        attached.send(failure(
                            id,
                            "remote_delete_session",
                            "Session confirmation does not match",
                        ));
                        continue;
                    }
                    if model.busy() || !requests.is_empty() || !bootstrap.is_empty() {
                        attached.send(failure(
                            id,
                            "remote_delete_session",
                            "Stop the session and wait for pending operations before deleting it",
                        ));
                        continue;
                    }
                    #[cfg(unix)]
                    {
                        let run = root()?;
                        let storage = crate::durable::storage_dir(target)?;
                        if let Err(error) = crate::deletion::validate(target, &run, &storage) {
                            attached.send(failure(
                                id,
                                "remote_delete_session",
                                &format!("{error:#}"),
                            ));
                            continue;
                        }
                        drop(backend); // Waits for the whole writer process tree to exit.
                        let result = crate::deletion::remove(target, &run, &storage);
                        match &result {
                            Ok(()) => {
                                attached.send(reply(
                                    id,
                                    "remote_delete_session",
                                    json!({"key":target.key}),
                                ));
                                for other in clients.values() {
                                    other.send(
                                        json!({"type":"remote_session_deleted", "key":target.key}),
                                    );
                                }
                            }
                            Err(error) => {
                                attached.send(failure(
                                    id,
                                    "remote_delete_session",
                                    &format!("{error:#}"),
                                ));
                            }
                        }
                        close_all(clients.into_values());
                        return result;
                    }
                    #[cfg(not(unix))]
                    {
                        attached.send(failure(
                            id,
                            "remote_delete_session",
                            "Permanent deletion requires a Unix SSH host",
                        ));
                        continue;
                    }
                }
                if record["type"] == "remote_shutdown" || record["type"] == "remote_retire" {
                    let id = record["id"].as_str().unwrap_or("");
                    if record["type"] == "remote_retire" {
                        // Serialized with Attach/Request/backend events. No client
                        // can submit between this live recheck and owner teardown.
                        if target.backend != RemoteBackend::Durable
                            || !can_retire_owner(
                                &model,
                                clients.len(),
                                requests.len(),
                                bootstrap.len(),
                                dialogs.len(),
                                &transcript,
                            )
                        {
                            attached.send(reply(id, "remote_retire", json!({"retired":false})));
                            continue;
                        }
                        attached.send(reply(id, "remote_retire", json!({"retired":true})));
                        close_all(clients.drain().map(|(_, client)| client));
                        break; // normal RpcClient Drop reaps the owned runner tree
                    }
                    if model.busy() {
                        attached.send(failure(
                            id,
                            "remote_shutdown",
                            "Stop the agent before shutting down its daemon",
                        ));
                    } else {
                        attached.send(reply(id, "remote_shutdown", json!({})));
                        close_all(clients.drain().map(|(_, client)| client));
                        break;
                    }
                    continue;
                }
                if record["type"] == "extension_ui_response" {
                    let id = record["id"].as_str().unwrap_or("");
                    if !dialogs.contains_key(id) {
                        continue;
                    }
                    let id = id.to_owned();
                    match backend.send_record(record) {
                        Ok(()) => {
                            dialogs.remove(&id);
                        }
                        Err(error) => {
                            attached.send(json!({"type":"remote_error","error":error.to_string()}));
                        }
                    }
                    continue;
                }
                let Some(original) = record["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                else {
                    attached.output.close();
                    clients.remove(&connection);
                    continue;
                };
                if target.backend == RemoteBackend::Durable && record["type"] == "get_submission" {
                    if requests.len() >= 256 {
                        attached.send(failure(
                            original,
                            "get_submission",
                            "Remote request queue is full",
                        ));
                        continue;
                    }
                    match backend.send_custom("get_submission", record.clone()) {
                        Ok(id) => {
                            requests.insert(
                                id,
                                Request {
                                    connection,
                                    original: original.into(),
                                    command: "get_submission".into(),
                                },
                            );
                        }
                        Err(error) => {
                            attached.send(failure(original, "get_submission", &error.to_string()));
                        }
                    }
                    continue;
                }
                let command: Command = match serde_json::from_value(record.clone()) {
                    Ok(command) => command,
                    Err(error) => {
                        attached.send(failure(
                            original,
                            record["type"].as_str().unwrap_or("unknown"),
                            &error.to_string(),
                        ));
                        continue;
                    }
                };
                if let Some(error) = backend_command_error(target.backend, &command) {
                    attached.send(failure(original, command.name(), error));
                    continue;
                }
                // Reads use the live projection, including any unfinished assistant block.
                // A snapshot resets the projection before the correlated read acknowledgement.
                let local = match &command {
                    Command::GetMessages => {
                        attached.send(snapshot(&model, target)?);
                        Some(json!({"remoteSnapshot":true}))
                    }
                    Command::GetState => Some(serde_json::to_value(&model.state)?),
                    // Durable context is measured by its runner on demand. Lifetime
                    // counters alone cannot reveal that a compaction shrank context.
                    Command::GetSessionStats if target.backend != RemoteBackend::Durable => {
                        Some(serde_json::to_value(&model.stats)?)
                    }
                    Command::GetSessionStats => None,
                    // Remote saved files must not enter the desktop's local file catalog.
                    Command::ListSessions { .. } => Some(json!({"sessions":[]})),
                    Command::Bash { .. } => {
                        attached.send(failure(original, command.name(), "Explicit shell submissions are not supported over SSH yet; agent bash tools still run remotely"));
                        continue;
                    }
                    _ => None,
                };
                if let Some(data) = local {
                    attached.send(reply(original, command.name(), data));
                    continue;
                }
                if requests.len() >= 256 {
                    attached.send(failure(
                        original,
                        command.name(),
                        "Remote request queue is full",
                    ));
                    continue;
                }
                let starts_turn = !model.busy() && matches!(&command, Command::Prompt { .. });
                if starts_turn && let Command::Prompt { message, .. } = &command {
                    history.begin(message);
                }
                let sent = if target.backend == RemoteBackend::Durable
                    && matches!(
                        command,
                        Command::Prompt { .. } | Command::PromptSubagent { .. }
                    ) {
                    backend.send_custom(command.name(), record.clone())
                } else {
                    backend.send(command.clone())
                };
                match sent {
                    Ok(id) => {
                        requests.insert(
                            id,
                            Request {
                                connection,
                                original: original.into(),
                                command: command.name().into(),
                            },
                        );
                    }
                    Err(error) => {
                        if starts_turn {
                            history.cancel();
                        }
                        attached.send(failure(original, command.name(), &error.to_string()));
                    }
                }
            }
            Event::Backend(TransportEvent::Record(mut record)) => {
                if target.backend == RemoteBackend::Durable {
                    transcript.apply(&record);
                }
                let mut batch_was_busy = model.busy();
                let previous_compactions = compaction_count(&model);
                let durable_state =
                    target.backend == RemoteBackend::Durable && record["type"] == "durable_state";
                let durable_events =
                    target.backend == RemoteBackend::Durable && record["type"] == "durable_events";
                let durable_metadata = target.backend == RemoteBackend::Durable
                    && record["type"] == "durable_metadata";
                let durable_subagents = target.backend == RemoteBackend::Durable
                    && record["type"] == "durable_subagents";
                let context_changed = durable_events && durable_context_changed(&record);
                let durable_incremental = durable_events || durable_metadata || durable_subagents;
                let durable_projection = durable_state || durable_incremental;
                let records = if durable_state {
                    model = crate::durable::project(&record, &model, target)?;
                    vec![snapshot(&model, target)?]
                } else if durable_events {
                    crate::durable::events(&record, &model, target)?
                } else if durable_metadata {
                    crate::durable::metadata(&record, &model, target)?
                } else if durable_subagents {
                    crate::durable::subagents(&record, &model, target)?
                } else {
                    if target.backend == RemoteBackend::Durable {
                        crate::durable::discard_stale_stats(&mut record, &model)?;
                    }
                    if target.backend == RemoteBackend::Durable
                        && record["type"] == "response"
                        && record["success"] == true
                        && record["command"] == "get_state"
                    {
                        // The committed projection owns state. Session stats are different:
                        // the durable runner measures current context on demand, which is
                        // not a lifetime counter in the committed usage document.
                        record["data"] = serde_json::to_value(&model.state)?;
                    }
                    vec![record]
                };
                for mut record in records {
                    if durable_subagents {
                        normalize_subagent_progress(&mut record, &model);
                    }
                    let was_busy = model.busy();
                    let id = record["id"].as_str().unwrap_or("").to_owned();
                    let was_bootstrap = bootstrap.remove(&id);
                    if was_bootstrap && record["success"] == false {
                        bail!("Pi bootstrap failed: {}", record["error"]);
                    }
                    // Never replace the live partial with a later Pi history refresh.
                    model.apply(&record)?;
                    batch_was_busy |= model.busy();
                    if !durable_projection && was_busy && !model.busy() {
                        history.finish();
                    }
                    if record["type"] == "extension_ui_request"
                        && !id.is_empty()
                        && matches!(
                            record["method"].as_str(),
                            Some("confirm" | "select" | "input" | "editor")
                        )
                    {
                        dialogs.insert(id.clone(), record.clone());
                    }
                    if record["type"] == "extension_ui_cancel" {
                        dialogs.remove(&id);
                    }
                    if record["type"] == "response"
                        && let Some(request) = requests.get(&id)
                    {
                        record["id"] = json!(request.original);
                    }
                    if record["type"] == "response" {
                        let internal = !was_bootstrap && !requests.contains_key(&id);
                        if let Some(update) = unsolicited_read_update(
                            internal,
                            !bootstrap.is_empty(),
                            &record,
                            &model,
                            target,
                        )? {
                            clients.retain(|_, attached| attached.send(update.clone()));
                        }
                        let failed_prompt = record["success"] == false
                            && requests
                                .get(&id)
                                .is_some_and(|request| request.command == "prompt");
                        if failed_prompt && !model.busy() {
                            history.cancel();
                        }
                        if let Some(request) = requests.remove(&id)
                            && let Some(attached) = clients.get(&request.connection)
                        {
                            if request.command == "get_submission" && record["success"] == true {
                                // Optional helper-local metadata; no shared command
                                // or Session protocol changes, no last-message guess.
                                record["data"]["correlated"] = transcript
                                    .correlated(&record["data"]["submission"])
                                    .unwrap_or(Value::Null);
                            }
                            attached.send(record.clone());
                        }
                        if matches!(
                            record["command"].as_str(),
                            Some(
                                "set_model"
                                    | "set_thinking_level"
                                    | "cycle_model"
                                    | "cycle_thinking_level"
                                    | "set_session_name"
                                    | "set_auto_compaction"
                                    | "set_compaction_threshold"
                                    | "reload"
                            )
                        ) {
                            let _ = backend.send(Command::GetState);
                            if target.backend == RemoteBackend::Durable
                                && matches!(
                                    record["command"].as_str(),
                                    Some(
                                        "set_model"
                                            | "set_auto_compaction"
                                            | "set_compaction_threshold"
                                    )
                                )
                            {
                                let _ = backend.send(Command::GetSessionStats);
                            }
                        }
                        if target.backend == RemoteBackend::Durable
                            && record["command"] == "compact"
                            && record["success"] == true
                        {
                            let _ = backend.send(Command::GetSessionStats);
                        }
                    } else if bootstrap.is_empty() {
                        clients.retain(|_, attached| attached.send(record.clone()));
                    }
                    if was_bootstrap && bootstrap.is_empty() {
                        let update = snapshot(&model, target)?;
                        for attached in clients.values() {
                            attached.send(update.clone());
                            for dialog in dialogs.values() {
                                attached.send(dialog.clone());
                            }
                        }
                    }
                    if !durable_incremental && record["type"] == "agent_start" {
                        let _ = backend.send(Command::GetState);
                    }
                    if !durable_incremental && record["type"] == "agent_settled" {
                        let _ = backend.send(Command::GetState);
                        let _ = backend.send(Command::GetSessionStats);
                    }
                }
                if durable_projection && batch_was_busy && !model.busy() {
                    history.finish();
                }
                let listed = path.with_extension("summary.json");
                if let Err(error) = crate::sessions::record(&listed, &model, target, &mut summary) {
                    eprintln!("Could not update the session summary: {error}");
                }
                if (durable_state || durable_events || durable_metadata)
                    && bootstrap.is_empty()
                    && (durable_metadata
                        || durable_stats_need_refresh(batch_was_busy, &model, previous_compactions)
                        || context_changed)
                {
                    let _ = backend.send(Command::GetSessionStats);
                }
            }
            Event::Backend(TransportEvent::RequestFailed { id, command, error }) => {
                if target.backend == RemoteBackend::Durable && command == "set_compaction_threshold"
                {
                    // A timeout is uncertain, not proof that the commit failed. Read
                    // authoritative policy; never replay the mutation. A later metadata
                    // publication also refreshes stats if this read arrives too early.
                    let _ = backend.send(Command::GetSessionStats);
                }
                if bootstrap.contains(&id) && error.starts_with("RPC process") {
                    // Exit emits outstanding failures before its stderr-bearing event. Keep the
                    // bootstrap gate closed until that event so startup logs retain the real cause.
                    eprintln!("Backend bootstrap failed: {error}");
                    continue;
                }
                ensure!(
                    !bootstrap.contains(&id),
                    "Backend bootstrap failed: {error}"
                );
                if let Some(request) = requests.remove(&id)
                    && let Some(attached) = clients.get(&request.connection)
                {
                    attached.send(failure(&request.original, &request.command, &error));
                }
            }
            Event::Backend(TransportEvent::Exited {
                description,
                stderr,
            }) => {
                eprintln!("{description}\n{stderr}");
                let error =
                    json!({"type":"remote_error", "error":format!("{description}\n{stderr}")});
                for (_, attached) in clients.drain() {
                    attached.send(error.clone());
                    attached.output.close();
                }
                break;
            }
            Event::Backend(TransportEvent::ProtocolError(error)) => {
                bail!("Pi protocol error: {error}")
            }
        }
    }
    Ok(())
}

fn backend_command_error(backend: RemoteBackend, command: &Command) -> Option<&'static str> {
    (backend != RemoteBackend::Durable && matches!(command, Command::PromptSubagent { .. }))
        .then_some("Direct subagent input requires a durable session")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    fn wait_for_handshake_slots(pool: &HandshakePool, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while pool.available.len() != count && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(pool.available.len(), count);
    }

    fn hello(target: &SshTarget) -> Value {
        json!({"type":"hello","token":"secret","version":PROTOCOL_VERSION,"cwd":target.cwd,"backend":target.backend})
    }

    #[test]
    fn handshake_admission_bounds_silent_clients_and_recovers_after_eof() {
        let pool = HandshakePool::new(2);
        let target = SshTarget::new("dev".into(), "/work".into()).unwrap();
        let (events, _incoming) = bounded(8);
        let mut clients = Vec::new();
        for _ in 0..2 {
            let (client, server) = socket_pair();
            assert!(spawn_attachment(server, "secret", &target, &events, &pool).unwrap());
            clients.push(client);
        }
        assert_eq!(pool.available.len(), 0);
        let (mut rejected, server) = socket_pair();
        assert!(!spawn_attachment(server, "secret", &target, &events, &pool).unwrap());
        rejected
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert_eq!(rejected.read(&mut [0]).unwrap(), 0);
        drop(clients);
        wait_for_handshake_slots(&pool, 2);
    }

    #[test]
    fn handshake_permits_recover_after_malformed_or_unauthorized_records() {
        let pool = HandshakePool::new(1);
        let target = SshTarget::new("dev".into(), "/work".into()).unwrap();
        let (events, incoming) = bounded(8);
        let mut wrong_token = hello(&target);
        wrong_token["token"] = json!("wrong");
        let mut unauthorized = serde_json::to_vec(&wrong_token).unwrap();
        unauthorized.push(b'\n');
        for bytes in [b"not JSON\n".to_vec(), unauthorized] {
            let (mut client, server) = socket_pair();
            client.write_all(&bytes).unwrap();
            assert!(spawn_attachment(server, "secret", &target, &events, &pool).unwrap());
            wait_for_handshake_slots(&pool, 1);
            assert!(
                incoming.try_recv().is_err(),
                "Invalid hello must not attach"
            );
        }
        let (mut client, server) = socket_pair();
        write_record(&mut client, &hello(&target)).unwrap();
        write_record(&mut client, &json!({"type":"not-ready"})).unwrap();
        assert!(spawn_attachment(server, "secret", &target, &events, &pool).unwrap());
        wait_for_handshake_slots(&pool, 1);
        assert!(
            incoming.try_recv().is_err(),
            "Invalid ready must not attach"
        );
    }

    #[test]
    fn handshake_record_limit_includes_lf_and_rejects_full_unterminated_input() {
        for kind in ["hello", "ready"] {
            let base = format!("{{\"type\":\"{kind}\",\"padding\":\"\"}}\n");
            for (extra, terminated, accepted) in
                [(0, true, true), (1, true, false), (1, false, false)]
            {
                let padding = "x".repeat(MAX_HANDSHAKE_BYTES - base.len() + extra);
                let mut bytes =
                    format!("{{\"type\":\"{kind}\",\"padding\":\"{padding}\"}}\n").into_bytes();
                if !terminated {
                    bytes.pop();
                }
                let (mut client, server) = socket_pair();
                client.write_all(&bytes).unwrap();
                // Keep the peer open: a full unframed record must fail immediately.
                let result = read_handshake_record(
                    &mut BufReader::new(server),
                    Instant::now() + Duration::from_secs(1),
                );
                if accepted {
                    assert_eq!(result.unwrap().unwrap()["type"], kind);
                } else {
                    assert!(result.unwrap_err().to_string().contains("exceeds"));
                }
            }
        }
    }

    #[test]
    fn handshake_framing_preserves_prefetched_ready_and_large_normal_records() {
        let (mut client, server) = socket_pair();
        let request = json!({"type":"prompt","message":"x".repeat(2 * MAX_HANDSHAKE_BYTES)});
        for record in [
            json!({"type":"hello"}),
            json!({"type":"ready"}),
            request.clone(),
        ] {
            write_record(&mut client, &record).unwrap();
        }
        let mut reader = BufReader::with_capacity(4 * MAX_HANDSHAKE_BYTES, server);
        let deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            read_handshake_record(&mut reader, deadline)
                .unwrap()
                .unwrap()["type"],
            "hello"
        );
        assert_eq!(
            read_handshake_record(&mut reader, deadline)
                .unwrap()
                .unwrap()["type"],
            "ready"
        );
        assert_eq!(read_record(&mut reader).unwrap().unwrap(), request);
    }

    #[test]
    fn handshake_accepts_valid_maximum_length_json_escaped_cwd() {
        let target = SshTarget::new("dev".into(), format!("/{}", "\u{0001}".repeat(4095))).unwrap();
        let record = hello(&target);
        let (mut client, server) = socket_pair();
        write_record(&mut client, &record).unwrap();
        assert_eq!(
            read_handshake_record(
                &mut BufReader::new(server),
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap()
            .unwrap(),
            record
        );
    }

    #[test]
    fn handshake_deadline_cannot_be_extended_by_trickling_and_returns_its_slot() {
        let pool = HandshakePool::new(1);
        let permit = pool.try_acquire().unwrap();
        let (mut client, server) = socket_pair();
        let peer = thread::spawn(move || {
            let _permit = permit;
            read_handshake_record(
                &mut BufReader::new(server),
                Instant::now() + Duration::from_millis(100),
            )
        });
        for byte in b"{\"type\":\"hello\"}\n" {
            if client.write_all(&[*byte]).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(peer.join().unwrap().is_err());
        assert_eq!(pool.available.len(), 1);
    }

    #[test]
    fn handshake_ready_shares_the_original_deadline_even_when_buffered() {
        let (mut client, server) = socket_pair();
        client
            .write_all(b"{\"type\":\"hello\"}\n{\"type\":\"ready\"}\n")
            .unwrap();
        let mut reader = BufReader::new(server);
        let deadline = Instant::now() + Duration::from_millis(50);
        assert!(
            read_handshake_record(&mut reader, deadline)
                .unwrap()
                .is_some()
        );
        thread::sleep(Duration::from_millis(60));
        assert!(
            read_handshake_record(&mut reader, deadline)
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
    }

    #[test]
    fn handshake_permits_return_when_unspawned_closures_are_dropped() {
        let pool = HandshakePool::new(1);
        let permit = pool.try_acquire().unwrap();
        let never_spawned = move || drop(permit);
        assert!(pool.try_acquire().is_none());
        drop(never_spawned);
        assert_eq!(pool.available.len(), 1);
    }

    #[test]
    fn authenticated_attachment_releases_slot_and_accepts_large_normal_requests() {
        let pool = HandshakePool::new(1);
        let target = SshTarget::new("dev".into(), "/work".into()).unwrap();
        let (events, incoming) = bounded(8);
        let (mut client, server) = socket_pair();
        assert!(spawn_attachment(server, "secret", &target, &events, &pool).unwrap());
        let request =
            json!({"type":"prompt","id":"large","message":"x".repeat(2 * MAX_HANDSHAKE_BYTES)});
        for record in [hello(&target), json!({"type":"ready"}), request.clone()] {
            write_record(&mut client, &record).unwrap();
        }
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        assert_eq!(
            read_record(&mut reader).unwrap().unwrap()["type"],
            "remote_hello"
        );
        let Event::Attach { output, .. } = incoming.recv_blocking().unwrap() else {
            panic!("Missing attachment")
        };
        wait_for_handshake_slots(&pool, 1);
        let Event::Request { record, .. } = incoming.recv_blocking().unwrap() else {
            panic!("Missing request")
        };
        assert_eq!(record, request);
        // An authenticated client still connected must not occupy a handshake slot.
        let (silent, server) = socket_pair();
        assert!(spawn_attachment(server, "secret", &target, &events, &pool).unwrap());
        drop(silent);
        wait_for_handshake_slots(&pool, 1);
        drop(output);
        drop(client);
    }

    #[test]
    fn durable_receipt_answer_identity_excludes_later_work_and_missing_identity_is_unknown() {
        let mut transcript = SubmissionTranscript::default();
        transcript.apply(&json!({"type":"durable_state","data":{"entries":[
            {"id":1,"model":[{"role":"user","content":"A"}]},
            {"id":2,"model":[{"role":"assistant","stopReason":"stop","content":"A done"}]},
            {"id":3,"model":[{"role":"user","content":"B"}]}
        ]}}));
        transcript.apply(&json!({"type":"durable_events","events":[{"type":"message_end","entry":
            {"id":4,"model":[{"role":"assistant","stopReason":"error","errorMessage":"B failed"}]}
        }]}));
        let receipt = json!({"type":"input","status":"done","entry":1,"answer":2});
        let correlated = transcript.correlated(&receipt).unwrap();
        assert_eq!(correlated["messages"].as_array().unwrap().len(), 2);
        assert_eq!(correlated["messages"][1]["content"], "A done");
        assert!(
            transcript
                .correlated(&json!({"type":"input","status":"done","entry":1,"answer":99}))
                .is_none()
        );
        assert!(
            transcript
                .correlated(&json!({"type":"input","status":"done","entry":1}))
                .is_none()
        );
        assert!(
            transcript
                .correlated(&json!({"type":"write","status":"done","entry":1,"answer":2}))
                .is_none()
        );
    }

    #[test]
    fn background_detail_progress_cannot_reopen_a_committed_terminal_tool() {
        let mut model = Session::new("/test".into());
        model.apply(&json!({"type":"tool_execution_start","toolCallId":"crew","toolName":"subagent","args":{}})).unwrap();
        let update = json!({"type":"tool_execution_update","toolCallId":"crew","toolName":"subagent","partialResult":{"content":[{"type":"text","text":"Delegated"}],"details":{"version":1,"background":true,"results":[]}}});
        let mut running = update.clone();
        normalize_subagent_progress(&mut running, &model);
        assert_eq!(running["type"], "tool_execution_update");
        model.apply(&json!({"type":"tool_execution_end","toolCallId":"crew","toolName":"subagent","result":{"content":[]},"isError":true})).unwrap();
        let mut terminal = update;
        normalize_subagent_progress(&mut terminal, &model);
        model.apply(&terminal).unwrap();
        assert!(model.tools[0].finished);
        assert!(model.tools[0].is_error);
        assert_eq!(model.tools[0].details["background"], true);
    }

    #[test]
    fn background_report_requires_its_own_terminal_submission_not_later_user_work() {
        let mut transcript = SubmissionTranscript::default();
        transcript.apply(&json!({"type":"durable_state","data":{"entries":[
            {"id":3,"model":[{"role":"user","content":"<subagent_report call=\"crew\">\nreport"}]},
            {"id":4,"model":[{"role":"assistant","stopReason":"stop"}]},
            {"id":5,"model":[{"role":"user","content":"B"}]},
            {"id":6,"model":[{"role":"assistant","stopReason":"error","errorMessage":"B failed"}]}
        ]}}));
        assert!(transcript.settled_reports()["crew"].is_null());
        transcript.apply(
            &json!({"type":"durable_events","events":[{"type":"submission","record":
                {"id":7,"requestId":"subagent-report-42","type":"input","status":"placed","entry":3}
            }]}),
        );
        assert!(transcript.settled_reports()["crew"].is_null());
        transcript.apply(&json!({"type":"durable_events","events":[{"type":"submission","record":
            {"id":7,"requestId":"subagent-report-42","type":"input","status":"done","entry":3,"answer":4}
        }]}));
        assert_eq!(
            transcript.settled_reports()["crew"]["answer"]["stopReason"],
            "stop"
        );
        assert!(transcript.settled_reports()["crew"]["answer"]["errorMessage"].is_null());
    }

    #[test]
    fn correlated_report_continuation_tree_includes_nested_calls_and_only_exact_descendant_receipts()
     {
        let mut transcript = SubmissionTranscript::default();
        transcript.apply(&json!({"type":"durable_state","data":{"entries":[
            {"id":1,"model":[{"role":"user","content":"A"}]},
            {"id":2,"model":[{"role":"assistant","content":[{"type":"toolCall","id":"A-crew","name":"subagent"}],"stopReason":"toolUse"}]},
            {"id":3,"model":[{"role":"assistant","stopReason":"stop","content":"A done"}]},
            {"id":4,"model":[{"role":"user","content":"B"}]},
            {"id":5,"model":[{"role":"assistant","content":[{"type":"toolCall","id":"B-crew","name":"subagent"}],"stopReason":"toolUse"}]},
            {"id":6,"model":[{"role":"assistant","stopReason":"error","errorMessage":"B failed"}]},
            {"id":7,"model":[{"role":"user","content":"<subagent_report call=\"A-crew\">\nreport"}]},
            {"id":8,"model":[{"role":"assistant","content":[{"type":"toolCall","id":"nested-crew","name":"subagent"}],"stopReason":"toolUse"}]},
            {"id":9,"model":[{"role":"assistant","stopReason":"stop","content":"A report done"}]},
            {"id":10,"model":[{"role":"user","content":"<subagent_report call=\"nested-crew\">\nreport"}]},
            {"id":11,"model":[{"role":"assistant","content":[{"type":"toolCall","id":"third-crew","name":"subagent"}],"stopReason":"toolUse"}]},
            {"id":12,"model":[{"role":"assistant","stopReason":"stop","content":"Nested report done"}]},
            {"id":13,"model":[{"role":"user","content":"<subagent_report call=\"third-crew\">\nreport"}]},
            {"id":14,"model":[{"role":"assistant","stopReason":"stop","content":"Third report done"}]},
            {"id":15,"model":[{"role":"user","content":"<subagent_report call=\"B-crew\">\nreport"}]},
            {"id":16,"model":[{"role":"assistant","stopReason":"error","errorMessage":"B report failed"}]}
        ]}}));
        let a = json!({"type":"input","status":"done","entry":1,"answer":3});
        assert!(
            transcript.correlated(&a).unwrap()["reports"]
                .as_object()
                .unwrap()
                .is_empty(),
            "messages alone do not prove receipt correlation"
        );
        transcript.apply(&json!({"type":"durable_events","events":[
            {"type":"submission","record":{"type":"input","requestId":"subagent-report-A","status":"done","entry":7,"answer":9}},
            {"type":"submission","record":{"type":"input","requestId":"subagent-report-nested","status":"placed","entry":10}},
            {"type":"submission","record":{"type":"input","requestId":"subagent-report-B","status":"done","entry":15,"answer":16}}
        ]}));
        let scoped = transcript.correlated(&a).unwrap();
        assert_eq!(
            scoped["reports"]["A-crew"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            scoped["reports"]["A-crew"]["messages"][1]["content"][0]["id"],
            "nested-crew"
        );
        assert!(
            scoped["reports"]["nested-crew"].is_null(),
            "placed report is still outstanding"
        );
        assert!(
            scoped["reports"]["B-crew"].is_null(),
            "unrelated B is not a descendant"
        );
        transcript.apply(&json!({"type":"durable_events","events":[
            {"type":"submission","record":{"type":"input","requestId":"subagent-report-nested","status":"done","entry":10,"answer":12}},
            {"type":"submission","record":{"type":"input","requestId":"subagent-report-third","status":"done","entry":13,"answer":14}}
        ]}));
        let scoped = transcript.correlated(&a).unwrap();
        assert_eq!(scoped["reports"].as_object().unwrap().len(), 3);
        assert_eq!(
            scoped["reports"]["nested-crew"]["messages"][1]["content"][0]["id"],
            "third-crew"
        );
        assert_eq!(scoped["reports"]["third-crew"]["receipt"]["answer"], 14);
        assert!(!scoped.to_string().contains("B failed"));
        assert!(!scoped.to_string().contains("B report failed"));
        // Lost or ambiguous identity is never replaced by a nearby assistant.
        transcript.entries.remove(&14);
        assert!(transcript.correlated(&a).unwrap()["reports"]["third-crew"].is_null());
        transcript.apply(&json!({"type":"durable_events","events":[
            {"type":"submission","record":{"type":"input","requestId":"subagent-report-duplicate","status":"done","entry":7,"answer":9}}
        ]}));
        assert!(transcript.correlated(&a).unwrap()["reports"]["A-crew"].is_null());
    }

    #[test]
    fn retirement_rechecks_live_work_queues_background_and_other_clients() {
        let mut model = Session::new("/test".into());
        let transcript = SubmissionTranscript::default();
        assert!(can_retire_owner(&model, 1, 0, 0, 0, &transcript));
        for counts in [(2, 0, 0, 0), (1, 1, 0, 0), (1, 0, 1, 0), (1, 0, 0, 1)] {
            assert!(!can_retire_owner(
                &model,
                counts.0,
                counts.1,
                counts.2,
                counts.3,
                &transcript
            ));
        }
        model.apply(&json!({"type":"agent_start"})).unwrap();
        assert!(!can_retire_owner(&model, 1, 0, 0, 0, &transcript));
        model.apply(&json!({"type":"agent_settled"})).unwrap();
        model.follow_up.push("Unrelated UI input".into());
        assert!(!can_retire_owner(&model, 1, 0, 0, 0, &transcript));
        model.follow_up.clear();
        model.apply(&json!({"type":"tool_execution_start","toolCallId":"crew","toolName":"subagent","args":{}})).unwrap();
        model.apply(&json!({"type":"tool_execution_end","toolCallId":"crew","toolName":"subagent","result":{"content":[{"type":"text","text":"Delegated"}],"details":{"version":1,"mode":"parallel","background":true,"results":[{"index":0,"agent":"scout","task":"work","status":"running"}]}},"isError":false})).unwrap();
        assert!(
            !model.busy(),
            "main turn is idle but background work is not"
        );
        assert!(!can_retire_owner(&model, 1, 0, 0, 0, &transcript));
        model.tools.last_mut().unwrap().details["results"][0]["status"] = json!("done");
        assert!(
            !can_retire_owner(&model, 1, 0, 0, 0, &transcript),
            "terminal children do not prove their background report settled"
        );
        model.tools.last_mut().unwrap().details["background"] = json!(false);
        assert!(can_retire_owner(&model, 1, 0, 0, 0, &transcript));
        model.tools.last_mut().unwrap().details = Value::Null;
        assert!(!can_retire_owner(&model, 1, 0, 0, 0, &transcript));
    }

    #[test]
    fn scheduler_rpc_preserves_request_key_and_reads_committed_snapshots_without_ui_takeover() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let peer = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let request = read_record(&mut reader).unwrap().unwrap();
            assert_eq!(request["type"], "prompt");
            assert_eq!(request["requestId"], "persisted_request_key");
            assert_eq!(request["message"], "Exact saved instruction");
            assert!(request.get("controller").is_none());
            write_record(reader.get_mut(), &json!({"type":"state_update","state":{}})).unwrap();
            write_record(
                reader.get_mut(),
                &json!({"type":"response","id":"other-client","success":true}),
            )
            .unwrap();
            write_record(
                reader.get_mut(),
                &json!({"type":"remote_snapshot","data":{"committed":true}}),
            )
            .unwrap();
            write_record(reader.get_mut(), &json!({"type":"response","id":request["id"],"success":true,"data":{"requestId":request["requestId"],"submissionId":"7"}})).unwrap();
        });
        // This is the same ordinary RPC stream obtained by normal attach; the
        // test uses a socket peer instead of spawning any runner or user session.
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut client = SchedulerAttachment {
            reader: BufReader::new(stream),
        };
        let (receipt, snapshot) = client.call(json!({"type":"prompt","message":"Exact saved instruction","requestId":"persisted_request_key"})).unwrap();
        assert_eq!(receipt["requestId"], "persisted_request_key");
        assert_eq!(snapshot.unwrap()["committed"], true);
        peer.join().unwrap();
    }

    #[test]
    fn direct_child_input_is_rejected_before_reaching_a_non_durable_backend() {
        let command = Command::PromptSubagent {
            conversation_id: "child-1".into(),
            message: "change course".into(),
            request_id: "request-1".into(),
            streaming_behavior: pi_core::protocol::StreamingBehavior::Steer,
            expected_run_id: Some("run-7".into()),
        };
        assert_eq!(
            backend_command_error(RemoteBackend::Pi, &command),
            Some("Direct subagent input requires a durable session")
        );
        assert_eq!(
            backend_command_error(RemoteBackend::Durable, &command),
            None
        );
    }

    #[test]
    fn snapshot_preserves_live_message_and_tool_state() {
        let target = SshTarget::new("dev".into(), "/work".into()).unwrap();
        let mut model = Session::new("/work".into());
        model.apply(&json!({"type":"agent_start"})).unwrap();
        model.apply(&json!({"type":"message_start","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}})).unwrap();
        model.apply(&json!({"type":"tool_execution_start","toolCallId":"t","toolName":"bash","args":{"command":"sleep 5"}})).unwrap();
        let mut desktop = Session::new(target.identity());
        desktop.apply(&snapshot(&model, &target).unwrap()).unwrap();
        assert!(desktop.busy());
        assert_eq!(desktop.streaming_message_index(), Some(0));
        assert_eq!(desktop.tools.len(), 1);
        assert!(!desktop.tools[0].finished);
        assert_eq!(desktop.cwd, target.identity());
        desktop.apply(&snapshot(&model, &target).unwrap()).unwrap();
        assert_eq!(
            desktop.messages.len(),
            1,
            "reconnection replaces instead of appending"
        );
    }
    #[test]
    fn durable_unsolicited_reads_become_events_but_correlated_responses_stay_responses() {
        let mut target = SshTarget::new("dev".into(), "/work".into()).unwrap();
        target.backend = RemoteBackend::Durable;
        let mut model = Session::new(target.identity());
        model.apply(&json!({"type":"agent_start"})).unwrap();
        model.stats.tokens.input = 7;
        let state = json!({"type":"response","id":"backend-1","command":"get_state","success":true,"data":{}});
        assert!(
            unsolicited_read_update(false, false, &state, &model, &target)
                .unwrap()
                .is_none(),
            "a phone-owned response remains correlated instead of being broadcast"
        );
        assert_eq!(state["type"], "response");
        assert_eq!(state["id"], "backend-1");

        let update = unsolicited_read_update(true, false, &state, &model, &target)
            .unwrap()
            .unwrap();
        assert_eq!(update["type"], "state_update");
        assert!(update.get("id").is_none());
        let stats = unsolicited_read_update(
            true,
            false,
            &json!({"type":"response","command":"get_session_stats"}),
            &model,
            &target,
        )
        .unwrap()
        .unwrap();
        assert_eq!(stats["type"], "stats_update");
        assert_eq!(stats["stats"]["tokens"]["input"], 7);
    }
    #[test]
    fn durable_stats_refresh_when_a_turn_settles_or_compaction_appears() {
        let mut model = Session::new("/work".into());
        assert!(!durable_stats_need_refresh(false, &model, 0));
        assert!(durable_stats_need_refresh(true, &model, 0));
        model
            .messages
            .push(json!({"role":"compactionSummary","summary":"shorter"}));
        assert!(durable_stats_need_refresh(false, &model, 0));
        assert!(!durable_stats_need_refresh(false, &model, 1));
    }
    #[test]
    fn durable_context_refreshes_after_answers_not_each_stream_delta() {
        assert!(durable_context_changed(&json!({"events":[
            {"type":"message_end","entry":{"model":[{"role":"assistant"}]}}
        ]})));
        assert!(durable_context_changed(
            &json!({"events":[{"type":"agent_changed"}]})
        ));
        assert!(!durable_context_changed(&json!({"events":[
            {"type":"message_update"}, {"type":"message_end","entry":{"model":[{"role":"user"}]}}
        ]})));
    }

    #[test]
    fn records_are_bounded() {
        assert!(
            write_record(
                &mut Vec::new(),
                &json!({"text":"x".repeat(MAX_RECORD_BYTES)})
            )
            .is_err()
        );
    }

    #[test]
    fn stale_unknown_protocol_endpoints_do_not_block_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = directory.path().join("unknown.json");
        assert!(!session_lock_is_held(&endpoint).unwrap());
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .open(endpoint.with_extension("lock"))
            .unwrap();
        owner.try_lock().unwrap();
        assert!(session_lock_is_held(&endpoint).unwrap());
        drop(owner);
        assert!(!session_lock_is_held(&endpoint).unwrap());
    }

    #[test]
    fn busy_protocol_one_daemon_remains_attached_with_its_limit() {
        let directory = tempfile::tempdir().unwrap();
        let target = SshTarget::new(
            "legacy-busy".into(),
            directory.path().to_string_lossy().into_owned(),
        )
        .unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint = Endpoint {
            port: listener.local_addr().unwrap().port(),
            token: "legacy-token".into(),
            cwd: target.cwd.clone(),
            version: LEGACY_PROTOCOL_VERSION,
            backend: target.backend,
            pid: 1,
        };
        let server_target = target.clone();
        let server_token = endpoint.token.clone();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let hello = read_record(&mut reader).unwrap().unwrap();
            assert_eq!(hello["version"], LEGACY_PROTOCOL_VERSION);
            assert_eq!(hello["token"], server_token);
            write_record_with_limit(
                reader.get_mut(),
                &json!({"type":"remote_hello", "version":LEGACY_PROTOCOL_VERSION, "key":server_target.key}),
                LEGACY_MAX_RECORD_BYTES,
            )
            .unwrap();
            assert_eq!(read_record(&mut reader).unwrap().unwrap()["type"], "ready");
            let mut model = Session::new(server_target.cwd.clone().into());
            model.apply(&json!({"type":"agent_start"})).unwrap();
            let mut initial = snapshot(&model, &server_target).unwrap();
            initial["version"] = json!(LEGACY_PROTOCOL_VERSION);
            write_record_with_limit(reader.get_mut(), &initial, LEGACY_MAX_RECORD_BYTES).unwrap();
        });

        let reader = connect_endpoint(&target, &endpoint).unwrap();
        let attachment = attach_legacy(&endpoint, reader).unwrap();
        assert_eq!(attachment.daemon_version, LEGACY_PROTOCOL_VERSION);
        assert_eq!(attachment.max_record_bytes, LEGACY_MAX_RECORD_BYTES);
        assert_eq!(attachment.prefetched.len(), 1);
        let session: Session =
            serde_json::from_value(attachment.prefetched[0]["data"].clone()).unwrap();
        assert!(session.busy());
        drop(attachment);
        server.join().unwrap();
    }

    #[test]
    fn idle_protocol_one_daemon_is_not_raced_by_automatic_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let target = SshTarget::new(
            "legacy-idle".into(),
            directory.path().to_string_lossy().into_owned(),
        )
        .unwrap();
        let lock_path = directory.path().join("legacy-idle.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        lock.try_lock().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint = Endpoint {
            port: listener.local_addr().unwrap().port(),
            token: "legacy-token".into(),
            cwd: target.cwd.clone(),
            version: LEGACY_PROTOCOL_VERSION,
            backend: target.backend,
            pid: 1,
        };
        let server_target = target.clone();
        let server_token = endpoint.token.clone();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let hello = read_record(&mut reader).unwrap().unwrap();
            assert_eq!(hello["version"], LEGACY_PROTOCOL_VERSION);
            assert_eq!(hello["token"], server_token);
            write_record_with_limit(
                reader.get_mut(),
                &json!({"type":"remote_hello", "version":LEGACY_PROTOCOL_VERSION, "key":server_target.key}),
                LEGACY_MAX_RECORD_BYTES,
            )
            .unwrap();
            assert_eq!(read_record(&mut reader).unwrap().unwrap()["type"], "ready");
            let mut model = Session::new(server_target.cwd.clone().into());
            model.state.session_file = Some("/remote/original-session.jsonl".into());
            let mut initial = snapshot(&model, &server_target).unwrap();
            initial["version"] = json!(LEGACY_PROTOCOL_VERSION);
            write_record_with_limit(reader.get_mut(), &initial, LEGACY_MAX_RECORD_BYTES).unwrap();
            assert!(
                read_record(&mut reader).unwrap().is_none(),
                "compatibility attach must not send automatic shutdown"
            );
            drop(lock);
        });

        let reader = connect_endpoint(&target, &endpoint).unwrap();
        let attachment = attach_legacy(&endpoint, reader).unwrap();
        let session: Session =
            serde_json::from_value(attachment.prefetched[0]["data"].clone()).unwrap();
        assert_eq!(
            session.state.session_file.as_deref(),
            Some("/remote/original-session.jsonl")
        );
        let replacement_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        assert!(matches!(
            replacement_lock.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(attachment);
        server.join().unwrap();
        replacement_lock.try_lock().unwrap();
    }

    #[test]
    fn legacy_snapshots_are_normalized_for_protocol_two_clients() {
        let normalized = normalize_daemon_record(
            json!({"type":"remote_snapshot", "version":LEGACY_PROTOCOL_VERSION}),
            LEGACY_PROTOCOL_VERSION,
        );
        assert_eq!(normalized["version"], PROTOCOL_VERSION);
    }
}
