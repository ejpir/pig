use anyhow::{Context, Result, bail, ensure};
use async_channel::{Sender, bounded};
use pi_core::{
    protocol::{Command, MAX_RECORD_BYTES, read_record},
    session::Session,
    ssh::{PROTOCOL_VERSION, RemoteBackend, SshTarget},
    transport::{Launch, RpcClient, TransportEvent},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufReader, Write},
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

fn serve(stream: TcpStream, token: String, target: SshTarget, events: Sender<Event>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let hello = read_record(&mut reader)?.context("No hello")?;
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
    write_record(
        &mut writer,
        &json!({"type":"remote_hello", "version":PROTOCOL_VERSION, "key":target.key}),
    )?;
    ensure!(
        read_record(&mut reader)?.is_some_and(|r| r["type"] == "ready"),
        "No ready message"
    );
    reader.get_ref().set_read_timeout(None)?;
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
        for stream in listener.incoming().flatten() {
            let sink = sink.clone();
            let token = token.clone();
            let target = identity.clone();
            thread::spawn(move || {
                let _ = serve(stream, token, target, sink);
            });
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
                if record["type"] == "remote_shutdown" {
                    let id = record["id"].as_str().unwrap_or("");
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
                    && matches!(command, Command::Prompt { .. })
                {
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
                let was_busy = model.busy();
                let durable_state =
                    target.backend == RemoteBackend::Durable && record["type"] == "durable_state";
                let previous_compactions = compaction_count(&model);
                if target.backend == RemoteBackend::Durable {
                    if durable_state {
                        model = crate::durable::project(&record, &model, target)?;
                        record = snapshot(&model, target)?;
                    } else if record["type"] == "response" && record["success"] == true {
                        // The committed projection owns state. Session stats are different:
                        // the durable runner measures current context on demand, which is
                        // not a lifetime counter in the committed usage document.
                        if record["command"] == "get_state" {
                            record["data"] = serde_json::to_value(&model.state)?;
                        }
                    }
                }
                let id = record["id"].as_str().unwrap_or("").to_owned();
                let was_bootstrap = bootstrap.remove(&id);
                if was_bootstrap && record["success"] == false {
                    bail!("Pi bootstrap failed: {}", record["error"]);
                }
                // Never replace the live partial with a later Pi history refresh.
                model.apply(&record)?;
                if was_busy && !model.busy() {
                    history.finish();
                }
                let listed = path.with_extension("summary.json");
                if let Err(error) = crate::sessions::record(&listed, &model, target, &mut summary) {
                    eprintln!("Could not update the session summary: {error}");
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
                    if internal
                        && bootstrap.is_empty()
                        && matches!(
                            record["command"].as_str(),
                            Some("get_state" | "get_session_stats")
                        )
                    {
                        let update = snapshot(&model, target)?;
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
                                | "reload"
                        )
                    ) {
                        let _ = backend.send(Command::GetState);
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
                if record["type"] == "agent_start" {
                    let _ = backend.send(Command::GetState);
                }
                if record["type"] == "agent_settled" {
                    let _ = backend.send(Command::GetState);
                    let _ = backend.send(Command::GetSessionStats);
                }
                if durable_state
                    && bootstrap.is_empty()
                    && durable_stats_need_refresh(was_busy, &model, previous_compactions)
                {
                    let _ = backend.send(Command::GetSessionStats);
                }
            }
            Event::Backend(TransportEvent::RequestFailed {
                id,
                command: _,
                error,
            }) => {
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

#[cfg(test)]
mod tests {
    use super::*;
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
