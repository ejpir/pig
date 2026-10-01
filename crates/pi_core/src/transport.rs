use std::{
    collections::{HashMap, VecDeque},
    ffi::OsString,
    io::{BufReader, Read, Write},
    path::PathBuf,
    process::{Command as ProcessCommand, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use async_channel::{Receiver, Sender};
use serde_json::Value;

use crate::{
    channel::{self, Channel},
    process_tree::ProcessTree,
    protocol::{Command, MAX_RECORD_BYTES, Route, read_record},
};

#[derive(Clone, Debug)]
pub struct Launch {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub request_timeout: Duration,
    /// Where to install the desktop extension, which pi then loads (`-e`) and which
    /// answers [`Route::Extension`] commands. `None` runs a program without it.
    pub extension: Option<PathBuf>,
}

/// What runs sessions when no environment variable says otherwise: the pi that
/// release builds embed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Backend {
    pub program: Option<PathBuf>,
}

/// The program and its first arguments. For development, `PI_DESKTOP_PI` (an
/// executable) and `PI_DESKTOP_RPC_ENTRY` (a JavaScript entry run with
/// `PI_DESKTOP_NODE`, or `node`) win over `backend`.
fn program(env: impl Fn(&str) -> Option<OsString>, backend: &Backend) -> (OsString, Vec<OsString>) {
    if let Some(entry) = env("PI_DESKTOP_RPC_ENTRY") {
        return (
            env("PI_DESKTOP_NODE").unwrap_or_else(|| "node".into()),
            vec![entry],
        );
    }
    let program = env("PI_DESKTOP_PI")
        .or_else(|| backend.program.clone().map(OsString::from))
        .unwrap_or_else(|| if cfg!(windows) { "pi.cmd" } else { "pi" }.into());
    (program, vec![])
}

/// What the backend gets on top of the desktop's own environment.
pub(crate) fn environment(env: impl Fn(&str) -> Option<OsString>) -> Vec<(OsString, OsString)> {
    let mut vars = Vec::new();
    if cfg!(target_os = "macos") {
        // Finder does not inherit a terminal's PATH. Include standard native package-manager bins.
        let mut paths = vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ];
        paths.extend(std::env::split_paths(&env("PATH").unwrap_or_default()));
        if let Ok(path) = std::env::join_paths(paths) {
            vars.push(("PATH".into(), path));
        }
    }
    // Trust the certificates the system trusts, as native apps do. Bun and Node
    // otherwise trust only their own list, so behind a proxy that re-signs HTTPS
    // (common on company networks) every model request fails with "Connection
    // error." Node.js 22.19+ reads this too. An explicit value wins.
    if env("NODE_USE_SYSTEM_CA").is_none() {
        vars.push(("NODE_USE_SYSTEM_CA".into(), "1".into()));
    }
    vars
}

impl Launch {
    pub fn pi(cwd: PathBuf, session: Option<&str>) -> Self {
        Self::pi_with(cwd, session, &Backend::default())
    }

    pub fn pi_with(cwd: PathBuf, session: Option<&str>, backend: &Backend) -> Self {
        let (program, mut args) = program(|name| std::env::var_os(name), backend);
        args.extend(["--mode".into(), "rpc".into()]);
        if let Some(path) = session {
            args.extend(["--session".into(), path.into()]);
        }
        Self {
            program,
            args,
            cwd,
            env: environment(|name| std::env::var_os(name)),
            request_timeout: Duration::from_secs(30),
            extension: Some(crate::extension::default_dir()),
        }
    }
}

/// Correlates a response with its request before it joins the event stream; late
/// replies to timed-out requests are dropped. `false` once nobody listens.
fn deliver(record: Value, pending: &PendingMap, events: &Sender<TransportEvent>) -> bool {
    if record["type"] == "response" {
        let id = record["id"].as_str().unwrap_or("");
        let request = pending.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        if let Some(request) = request {
            if record["command"].as_str() != Some(&request.command) {
                return events
                    .send_blocking(TransportEvent::RequestFailed {
                        id: id.into(),
                        command: request.command,
                        error: "RPC response command mismatch".into(),
                    })
                    .is_ok();
            }
        } else if !id.is_empty() {
            // Late replies to timed-out requests must not mutate newer state.
            return true;
        }
    }
    events.send_blocking(TransportEvent::Record(record)).is_ok()
}

#[derive(Debug)]
pub enum TransportEvent {
    Record(Value),
    RequestFailed {
        id: String,
        command: String,
        error: String,
    },
    Exited {
        description: String,
        stderr: String,
    },
    ProtocolError(String),
}

struct Pending {
    command: String,
    /// `None` for commands that reply only after long-running work finishes.
    deadline: Option<Instant>,
}

type PendingMap = Arc<Mutex<HashMap<String, Pending>>>;

/// Pipe I/O never runs on GPUI's foreground executor. One client owns one child.
/// All queues are bounded; backpressure cannot block the UI's send path.
pub struct RpcClient {
    writes: Sender<Vec<u8>>,
    events: Receiver<TransportEvent>,
    pending: PendingMap,
    stopped: Arc<AtomicBool>,
    next_id: AtomicU64,
    timeout: Duration,
    pid: u32,
    channel: Option<Channel>,
    supervisor: Option<thread::JoinHandle<()>>,
}

impl RpcClient {
    pub fn spawn(launch: Launch) -> Result<Self> {
        let (event_sender, events) = async_channel::bounded(256);
        let stopped = Arc::new(AtomicBool::new(false));
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let mut args = launch.args;
        let mut env = launch.env;
        let channel = match &launch.extension {
            Some(dir) => {
                let extension = crate::extension::install(dir)
                    .context("Could not install Pi Desktop's extension")?;
                let channel = Channel::open(channel::TRANSPORT, stopped.clone(), {
                    let pending = pending.clone();
                    let events = event_sender.clone();
                    move |record| deliver(record, &pending, &events)
                })
                .context("Could not open the extension channel")?;
                args.extend(["-e".into(), extension.into_os_string()]);
                env.extend(channel.env().iter().cloned());
                Some(channel)
            }
            None => None,
        };
        let mut command = ProcessCommand::new(&launch.program);
        command
            .args(&args)
            .current_dir(&launch.cwd)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().with_context(|| {
            format!(
                "Could not start {:?} in {}",
                launch.program,
                launch.cwd.display()
            )
        })?;
        let pid = child.id();
        let process_tree = match ProcessTree::new(&child) {
            Ok(tree) => tree,
            Err(error) => {
                if let Err(kill_error) = child.kill() {
                    eprintln!("RPC startup cleanup: {kill_error}");
                }
                if let Err(wait_error) = child.wait() {
                    eprintln!("RPC startup wait: {wait_error}");
                }
                return Err(error).context("Could not isolate RPC process tree");
            }
        };
        let mut stdin = child.stdin.take().context("child stdin unavailable")?;
        let stdout = child.stdout.take().context("child stdout unavailable")?;
        let mut stderr = child.stderr.take().context("child stderr unavailable")?;
        let (writes, write_receiver) = async_channel::bounded::<Vec<u8>>(64);
        let diagnostics = Arc::new(Mutex::new(VecDeque::<u8>::new()));

        thread::spawn({
            let stopped = stopped.clone();
            let events = event_sender.clone();
            move || {
                while let Ok(bytes) = write_receiver.recv_blocking() {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    if let Err(error) = stdin.write_all(&bytes).and_then(|_| stdin.flush()) {
                        stopped.store(true, Ordering::Release);
                        if events
                            .send_blocking(TransportEvent::ProtocolError(format!(
                                "RPC stdin: {error}"
                            )))
                            .is_err()
                        {
                            break;
                        }
                        break;
                    }
                }
                // Closing stdin is pi's orderly shutdown request.
                drop(stdin);
            }
        });
        let stderr_reader = thread::spawn({
            let diagnostics = diagnostics.clone();
            move || {
                let mut buffer = [0; 4096];
                loop {
                    match stderr.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            let mut tail = diagnostics.lock().unwrap_or_else(|e| e.into_inner());
                            tail.extend(&buffer[..count]);
                            while tail.len() > 8192 {
                                tail.pop_front();
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            }
        });
        let reader = thread::spawn({
            let events = event_sender.clone();
            let pending = pending.clone();
            let stopped = stopped.clone();
            move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_record(&mut reader) {
                        Ok(Some(record)) => {
                            if !deliver(record, &pending, &events) {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            stopped.store(true, Ordering::Release);
                            if events
                                .send_blocking(TransportEvent::ProtocolError(error.to_string()))
                                .is_err()
                            {
                                break;
                            }
                            break;
                        }
                    }
                }
                stopped.store(true, Ordering::Release);
            }
        });
        let supervisor = thread::spawn({
            let stopped = stopped.clone();
            let pending = pending.clone();
            let writes = writes.clone();
            move || {
                let mut shutdown_at = None;
                let description = loop {
                    if stopped.load(Ordering::Acquire) {
                        writes.close();
                        let began = shutdown_at.get_or_insert_with(Instant::now);
                        if began.elapsed() >= Duration::from_secs(2) {
                            if let Err(error) = child.kill() {
                                eprintln!("Could not stop RPC child {pid}: {error}");
                            }
                            break match child.wait() {
                                Ok(status) => format!("RPC process exited: {status}"),
                                Err(error) => format!("RPC wait failed: {error}"),
                            };
                        }
                    }
                    match child.try_wait() {
                        Ok(Some(status)) => break format!("RPC process exited: {status}"),
                        Err(error) => break format!("RPC process failed: {error}"),
                        Ok(None) => {}
                    }
                    let expired: Vec<_> = {
                        let mut pending = pending.lock().unwrap_or_else(|e| e.into_inner());
                        let ids: Vec<_> = pending
                            .iter()
                            .filter(|(_, request)| {
                                request
                                    .deadline
                                    .is_some_and(|deadline| deadline <= Instant::now())
                            })
                            .map(|(id, _)| id.clone())
                            .collect();
                        ids.into_iter()
                            .filter_map(|id| pending.remove(&id).map(|request| (id, request)))
                            .collect()
                    };
                    for (id, request) in expired {
                        if event_sender
                            .send_blocking(TransportEvent::RequestFailed {
                                id,
                                command: request.command,
                                error: "RPC request timed out; its outcome is unknown".into(),
                            })
                            .is_err()
                        {
                            stopped.store(true, Ordering::Release);
                        }
                    }
                    thread::sleep(Duration::from_millis(20));
                };
                stopped.store(true, Ordering::Release);
                writes.close();
                // Descendants can inherit the pipes. Close the process tree before joining readers.
                drop(process_tree);
                if reader.join().is_err() {
                    eprintln!("RPC reader panicked");
                }
                if stderr_reader.join().is_err() {
                    eprintln!("RPC stderr reader panicked");
                }
                let outstanding =
                    std::mem::take(&mut *pending.lock().unwrap_or_else(|e| e.into_inner()));
                for (id, request) in outstanding {
                    if event_sender
                        .send_blocking(TransportEvent::RequestFailed {
                            id,
                            command: request.command,
                            error: description.clone(),
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                let stderr = String::from_utf8_lossy(
                    &diagnostics
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .iter()
                        .copied()
                        .collect::<Vec<_>>(),
                )
                .into_owned();
                // The view may already be gone during orderly shutdown.
                let _receiver_closed = event_sender.send_blocking(TransportEvent::Exited {
                    description,
                    stderr,
                });
            }
        });
        Ok(Self {
            writes,
            events,
            pending,
            stopped,
            next_id: AtomicU64::new(1),
            timeout: launch.request_timeout,
            pid,
            channel,
            supervisor: Some(supervisor),
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }
    pub fn events(&self) -> Receiver<TransportEvent> {
        self.events.clone()
    }

    pub fn send(&self, command: Command) -> Result<String> {
        let id = format!("desktop-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let record = command.record(&id)?;
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id.clone(),
                Pending {
                    command: command.name().into(),
                    deadline: (!command.replies_when_finished())
                        .then(|| Instant::now() + self.timeout),
                },
            );
        let sent = match command.route() {
            Route::Pi => self.send_record(record),
            Route::Extension => self.send_to_extension(record),
        };
        if let Err(error) = sent {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(error);
        }
        Ok(id)
    }

    pub fn send_record(&self, record: Value) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            bail!("RPC process is disconnected");
        }
        let mut bytes = serde_json::to_vec(&record)?;
        if bytes.len() >= MAX_RECORD_BYTES {
            bail!("RPC command is too large");
        }
        bytes.push(b'\n');
        self.writes
            .try_send(bytes)
            .context("RPC write queue full or closed")
    }

    fn send_to_extension(&self, record: Value) -> Result<()> {
        let channel = self
            .channel
            .as_ref()
            .context("This program runs without Pi Desktop's extension")?;
        if self.stopped.load(Ordering::Acquire) {
            bail!("RPC process is disconnected");
        }
        let mut bytes = serde_json::to_vec(&record)?;
        if bytes.len() >= MAX_RECORD_BYTES {
            bail!("Extension command is too large");
        }
        bytes.push(b'\n');
        channel.send(bytes)
    }
}

impl Drop for RpcClient {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.writes.close();
        self.events.close();
        // At app exit, detached threads would be terminated before their kill/reap fallback ran.
        if let Some(supervisor) = self.supervisor.take()
            && supervisor.join().is_err()
        {
            eprintln!("RPC supervisor panicked during shutdown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_pi_runs_unless_the_environment_names_another() {
        let none = |_: &str| None;
        let default = if cfg!(windows) { "pi.cmd" } else { "pi" };
        assert_eq!(program(none, &Backend::default()), (default.into(), vec![]));
        let embedded = Backend {
            program: Some("/cache/pi-desktop/backend/1/pi".into()),
        };
        assert_eq!(
            program(none, &embedded),
            ("/cache/pi-desktop/backend/1/pi".into(), vec![])
        );
        let env = |name: &str| (name == "PI_DESKTOP_PI").then(|| OsString::from("/dev/pi"));
        assert_eq!(program(env, &embedded), ("/dev/pi".into(), vec![]));
        let env = |name: &str| match name {
            "PI_DESKTOP_RPC_ENTRY" => Some(OsString::from("/dev/cli.js")),
            "PI_DESKTOP_NODE" => Some(OsString::from("/opt/node/bin/node")),
            _ => None,
        };
        assert_eq!(
            program(env, &embedded),
            ("/opt/node/bin/node".into(), vec!["/dev/cli.js".into()])
        );
    }

    #[test]
    fn the_backend_trusts_the_system_certificates_unless_told_otherwise() {
        let system_ca = |vars: Vec<(OsString, OsString)>| {
            vars.into_iter()
                .find(|(name, _)| name == "NODE_USE_SYSTEM_CA")
                .map(|(_, value)| value)
        };
        assert_eq!(system_ca(environment(|_| None)), Some("1".into()));
        let explicit = |name: &str| (name == "NODE_USE_SYSTEM_CA").then(|| OsString::from("0"));
        assert_eq!(system_ca(environment(explicit)), None, "inherited as set");
    }
}
