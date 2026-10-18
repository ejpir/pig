//! Helper-owned PTY relay for restricted Android SSH connections.

use anyhow::{Context as _, Result, anyhow, bail};
use pi_core::terminal_transport::{ClientDecoder, ClientMessage, Dimensions, ServerMessage};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    collections::HashSet,
    io::{self, Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const IO_CHUNK: usize = 32 * 1024;
const INPUT_QUEUE_DEPTH: usize = 32;
const OUTPUT_QUEUE_DEPTH: usize = 64;
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
const QUEUE_PROGRESS_TIMEOUT: Duration = Duration::from_secs(30);
const QUEUE_WAIT_SLICE: Duration = Duration::from_millis(5);

type SharedMaster = Arc<Mutex<Box<dyn MasterPty + Send>>>;

enum OutputCommand {
    Frame(Vec<u8>),
    Final(Vec<u8>),
}

/// A bounded, serialized path to stdout. Producers never own the pipe write;
/// a full queue applies backpressure until the writer advances or disconnects.
struct ProtocolOutput {
    sender: mpsc::SyncSender<OutputCommand>,
    finished: Mutex<bool>,
}

type SharedOutput = Arc<ProtocolOutput>;

enum PtyCommand {
    Input(Vec<u8>),
    Resize(Dimensions),
}

struct RelayTermination<'a> {
    started: &'a AtomicBool,
    killer: &'a mut dyn ChildKiller,
    master: &'a SharedMaster,
    session: i32,
}

impl RelayTermination<'_> {
    fn stop(&mut self) {
        terminate_once(self.started, self.killer, Some(self.master), self.session);
    }
}

struct TerminationGuard {
    killer: Box<dyn ChildKiller + Send + Sync>,
    master: SharedMaster,
    session: i32,
    started: Arc<AtomicBool>,
    armed: bool,
}

impl Drop for TerminationGuard {
    fn drop(&mut self) {
        if self.armed {
            terminate_once(
                &self.started,
                &mut *self.killer,
                Some(&self.master),
                self.session,
            );
        }
    }
}

/// Runs one terminal over stdin/stdout until its shell exits.
/// Stdout contains protocol frames only.
pub fn stdio() -> Result<()> {
    match run() {
        Ok(()) => Ok(()),
        Err(error) => Err(error),
    }
}

fn run() -> Result<()> {
    configure_process_containment()?;
    let mut stdin = io::stdin();
    let mut decoder = ClientDecoder::new();
    let mut pending = Vec::new();
    let (dimensions, cwd) = 'open: loop {
        let mut bytes = [0; IO_CHUNK];
        let read = stdin
            .read(&mut bytes)
            .context("Could not read the terminal open frame")?;
        if read == 0 {
            decoder.finish().context("Invalid terminal open frame")?;
            bail!("Terminal transport closed before its open frame");
        }
        let messages = decoder
            .push(&bytes[..read])
            .context("Invalid terminal open frame")?;
        let mut messages = messages.into_iter();
        let Some(first) = messages.next() else {
            continue;
        };
        let ClientMessage::Open { dimensions, cwd } = first else {
            bail!("The first terminal frame must open the terminal");
        };
        pending.extend(messages);
        break 'open (dimensions, working_directory(cwd)?);
    };

    let pair = native_pty_system()
        .openpty(pty_size(dimensions))
        .context("Could not create a terminal")?;

    let mut command = CommandBuilder::new(default_shell());
    command.cwd(cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.arg("-l");

    let mut child = pair
        .slave
        .spawn_command(command)
        .context("Could not start the shell")?;
    drop(pair.slave);
    let session = child
        .process_id()
        .and_then(|pid| i32::try_from(pid).ok())
        .ok_or_else(|| anyhow!("Could not identify the terminal session"))?;
    let master = Arc::new(Mutex::new(pair.master));
    let termination = Arc::new(AtomicBool::new(false));
    let mut guard = TerminationGuard {
        killer: child.clone_killer(),
        master: master.clone(),
        session,
        started: termination.clone(),
        armed: true,
    };
    let reader = master
        .lock()
        .map_err(|_| anyhow!("Terminal PTY lock was poisoned"))?
        .try_clone_reader()
        .context("Could not read from the terminal")?;
    let writer = master
        .lock()
        .map_err(|_| anyhow!("Terminal PTY lock was poisoned"))?
        .take_writer()
        .context("Could not write to the terminal")?;

    let (output_tx, output_rx) = mpsc::sync_channel(OUTPUT_QUEUE_DEPTH);
    let output = Arc::new(ProtocolOutput {
        sender: output_tx,
        finished: Mutex::new(false),
    });
    let (output_actor_done_tx, output_actor_done_rx) = mpsc::sync_channel(1);
    let output_master = master.clone();
    let output_actor_termination = termination.clone();
    let mut output_actor_killer = child.clone_killer();
    thread::Builder::new()
        .name("remote-terminal-stdout".into())
        .spawn(move || {
            let result = write_protocol_output(
                output_rx,
                io::BufWriter::new(io::stdout()),
                &output_actor_termination,
                &mut *output_actor_killer,
                &output_master,
                session,
            );
            let _ = output_actor_done_tx.send(result);
        })
        .context("Could not start the terminal stdout writer")?;
    send(&output, &ServerMessage::Opened)?;

    let (pty_tx, pty_rx) = mpsc::sync_channel(INPUT_QUEUE_DEPTH);
    let writer_output = output.clone();
    let writer_master = master.clone();
    let writer_termination = termination.clone();
    let mut writer_killer = child.clone_killer();
    thread::Builder::new()
        .name("remote-terminal-pty-writer".into())
        .spawn(move || {
            if let Err(error) = write_pty_input(
                pty_rx,
                writer,
                &writer_termination,
                &writer_master,
                &mut *writer_killer,
                session,
            ) {
                let _ = finish(
                    &writer_output,
                    &ServerMessage::Error(user_error(&error.to_string())),
                );
                terminate_once(
                    &writer_termination,
                    &mut *writer_killer,
                    Some(&writer_master),
                    session,
                );
            }
        })
        .context("Could not start the terminal input writer")?;

    let relay_output_channel = output.clone();
    let relay_output_master = master.clone();
    let relay_output_termination = termination.clone();
    let mut relay_output_killer = child.clone_killer();
    let (relay_output_done_tx, relay_output_done_rx) = mpsc::sync_channel(1);
    let relay_output_thread = thread::Builder::new()
        .name("remote-terminal-output".into())
        .spawn(move || {
            let result = relay_output(
                reader,
                relay_output_channel.clone(),
                &relay_output_termination,
                &mut *relay_output_killer,
                &relay_output_master,
                session,
            );
            if let Err(error) = &result {
                let _ = finish(
                    &relay_output_channel,
                    &ServerMessage::Error(user_error(&error.to_string())),
                );
                terminate_once(
                    &relay_output_termination,
                    &mut *relay_output_killer,
                    Some(&relay_output_master),
                    session,
                );
            }
            let _ = relay_output_done_tx.send(result);
        })
        .context("Could not start the terminal output relay")?;

    let relay_input_output = output.clone();
    let relay_input_master = master.clone();
    let relay_input_termination = termination.clone();
    let mut relay_input_killer = child.clone_killer();
    thread::Builder::new()
        .name("remote-terminal-input".into())
        .spawn(move || {
            if let Err(error) = relay_input(
                stdin,
                decoder,
                pending,
                pty_tx,
                RelayTermination {
                    started: &relay_input_termination,
                    killer: &mut *relay_input_killer,
                    master: &relay_input_master,
                    session,
                },
            ) {
                let _ = finish(
                    &relay_input_output,
                    &ServerMessage::Error(user_error(&error.to_string())),
                );
                terminate_once(
                    &relay_input_termination,
                    &mut *relay_input_killer,
                    Some(&relay_input_master),
                    session,
                );
            }
        })
        .context("Could not start the terminal input relay")?;

    let status = child.wait().context("Could not wait for the shell")?;
    // A shell exit ends the whole terminal ownership domain. This also closes
    // a slave retained by a background job so the output relay cannot linger.
    terminate_once(&termination, &mut *guard.killer, Some(&master), session);

    let relay_result = relay_output_done_rx
        .recv_timeout(OUTPUT_DRAIN_TIMEOUT)
        .map_err(|_| anyhow!("Terminal output did not close after the shell exited"))?;
    relay_output_thread
        .join()
        .map_err(|_| anyhow!("The terminal output relay panicked"))?;
    relay_result?;

    finish(
        &output,
        &ServerMessage::Exit {
            code: Some(status.exit_code()),
        },
    )?;
    output_actor_done_rx
        .recv_timeout(OUTPUT_DRAIN_TIMEOUT)
        .map_err(|_| anyhow!("Terminal stdout did not drain"))??;
    guard.armed = false;
    Ok(())
}

fn write_protocol_output(
    receiver: mpsc::Receiver<OutputCommand>,
    mut writer: impl Write,
    termination: &AtomicBool,
    killer: &mut dyn ChildKiller,
    master: &SharedMaster,
    session: i32,
) -> Result<()> {
    while let Ok(command) = receiver.recv() {
        let (frame, final_frame) = match command {
            OutputCommand::Frame(frame) => (frame, false),
            OutputCommand::Final(frame) => (frame, true),
        };
        if let Err(error) = writer.write_all(&frame).and_then(|()| writer.flush()) {
            terminate_once(termination, killer, Some(master), session);
            return Err(error).context("Could not write terminal protocol output");
        }
        if final_frame {
            return Ok(());
        }
    }
    terminate_once(termination, killer, Some(master), session);
    bail!("Terminal protocol output queue closed without a final frame")
}

fn relay_output(
    mut reader: Box<dyn Read + Send>,
    output: SharedOutput,
    termination: &AtomicBool,
    killer: &mut dyn ChildKiller,
    master: &SharedMaster,
    session: i32,
) -> Result<()> {
    let mut bytes = [0; IO_CHUNK];
    loop {
        match reader.read(&mut bytes) {
            Ok(0) => {
                terminate_once(termination, killer, Some(master), session);
                return Ok(());
            }
            Ok(read) => match send(&output, &ServerMessage::Output(bytes[..read].to_vec())) {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(error) => return Err(error).context("Could not queue terminal output"),
            },
            Err(error) if error.raw_os_error() == Some(libc::EIO) => {
                terminate_once(termination, killer, Some(master), session);
                return Ok(());
            }
            Err(error) => return Err(error).context("Could not read terminal output"),
        }
    }
}

fn write_pty_input(
    receiver: mpsc::Receiver<PtyCommand>,
    mut writer: Box<dyn Write + Send>,
    termination: &AtomicBool,
    master: &SharedMaster,
    killer: &mut dyn ChildKiller,
    session: i32,
) -> Result<()> {
    while let Ok(command) = receiver.recv() {
        match command {
            PtyCommand::Input(bytes) => writer
                .write_all(&bytes)
                .context("Could not write terminal input")?,
            PtyCommand::Resize(dimensions) => master
                .lock()
                .map_err(|_| anyhow!("Terminal PTY lock was poisoned"))?
                .resize(pty_size(dimensions))
                .context("Could not resize the terminal")?,
        }
    }
    // The reader normally disappears only after EOF, Close, or a parser
    // failure, all of which must terminate rather than leave a shell behind.
    terminate_once(termination, killer, Some(master), session);
    Ok(())
}

fn relay_input(
    mut stdin: io::Stdin,
    mut decoder: ClientDecoder,
    pending: Vec<ClientMessage>,
    writer: mpsc::SyncSender<PtyCommand>,
    mut termination: RelayTermination<'_>,
) -> Result<()> {
    for message in pending {
        if !handle_input(message, &writer, &mut termination)? {
            return Ok(());
        }
    }
    let mut bytes = [0; IO_CHUNK];
    loop {
        let read = stdin
            .read(&mut bytes)
            .context("Could not read terminal input")?;
        if read == 0 {
            decoder.finish().context("Invalid terminal input frame")?;
            termination.stop();
            return Ok(());
        }
        for message in decoder
            .push(&bytes[..read])
            .context("Invalid terminal input frame")?
        {
            if !handle_input(message, &writer, &mut termination)? {
                return Ok(());
            }
        }
    }
}

fn handle_input(
    message: ClientMessage,
    writer: &mpsc::SyncSender<PtyCommand>,
    termination: &mut RelayTermination<'_>,
) -> Result<bool> {
    let command = match message {
        ClientMessage::Open { .. } => bail!("A terminal can only be opened once"),
        ClientMessage::Input(bytes) => PtyCommand::Input(bytes),
        ClientMessage::Resize(dimensions) => PtyCommand::Resize(dimensions),
        ClientMessage::Close => {
            termination.stop();
            return Ok(false);
        }
    };
    queue_pty_command(writer, command, termination.started, QUEUE_PROGRESS_TIMEOUT)?;
    Ok(true)
}

fn queue_pty_command(
    writer: &mpsc::SyncSender<PtyCommand>,
    mut command: PtyCommand,
    termination: &AtomicBool,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if termination.load(Ordering::Acquire) {
            bail!("Terminal stopped while input was waiting for backpressure");
        }
        match writer.try_send(command) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                bail!("Terminal input writer stopped");
            }
            Err(mpsc::TrySendError::Full(returned)) => command = returned,
        }
        if Instant::now() >= deadline {
            bail!("Terminal input queue made no progress before its deadline");
        }
        thread::sleep(QUEUE_WAIT_SLICE);
    }
}

/// Queues a non-final message. Returns false once a final Error/Exit was queued.
fn send(output: &SharedOutput, message: &ServerMessage) -> Result<bool> {
    queue_output(output, message, false)
}

/// Atomically closes the server side of the protocol. The first final message
/// wins, so an input error cannot later be overwritten by a normal exit.
fn finish(output: &SharedOutput, message: &ServerMessage) -> Result<bool> {
    queue_output(output, message, true)
}

fn queue_output(output: &SharedOutput, message: &ServerMessage, final_frame: bool) -> Result<bool> {
    queue_output_with_timeout(output, message, final_frame, QUEUE_PROGRESS_TIMEOUT)
}

fn queue_output_with_timeout(
    output: &SharedOutput,
    message: &ServerMessage,
    final_frame: bool,
    timeout: Duration,
) -> Result<bool> {
    let frame = message.encode()?;
    let mut finished = output
        .finished
        .lock()
        .map_err(|_| anyhow!("Terminal output state lock was poisoned"))?;
    if *finished {
        return Ok(false);
    }
    let mut command = if final_frame {
        OutputCommand::Final(frame)
    } else {
        OutputCommand::Frame(frame)
    };
    let deadline = Instant::now() + timeout;
    loop {
        match output.sender.try_send(command) {
            Ok(()) => break,
            Err(mpsc::TrySendError::Disconnected(_)) => {
                bail!("Terminal stdout writer stopped");
            }
            Err(mpsc::TrySendError::Full(returned)) => command = returned,
        }
        if Instant::now() >= deadline {
            bail!("Terminal output queue made no progress before its deadline");
        }
        thread::sleep(QUEUE_WAIT_SLICE);
    }
    *finished = final_frame;
    Ok(true)
}

fn terminate_once(
    started: &AtomicBool,
    killer: &mut dyn ChildKiller,
    master: Option<&SharedMaster>,
    session: i32,
) {
    if !started.swap(true, Ordering::AcqRel) {
        terminate(killer, master, session);
    }
}

fn terminate(killer: &mut dyn ChildKiller, master: Option<&SharedMaster>, session: i32) {
    if session > 1 {
        kill_terminal_session(master, session);
    }
    let _ = killer.kill();
}

#[derive(Clone, Copy)]
struct ProcessRecord {
    pid: i32,
    parent: i32,
    session: i32,
    group: i32,
}

fn kill_terminal_session(master: Option<&SharedMaster>, session: i32) {
    // portable-pty makes the shell a session leader. Stop a transitive fixed
    // point before killing it, so no member can fork or create a new process
    // group between discovery and teardown. Linux's subreaper setting below
    // keeps double-forked descendants parented here.
    let foreground = master
        .and_then(|master| master.lock().ok())
        .and_then(|master| master.process_group_leader());
    signal_process_group(foreground, libc::SIGSTOP);
    let own_pid = i32::try_from(std::process::id()).unwrap_or_default();
    let own_group = unsafe { libc::getpgrp() };
    let mut contained = HashSet::new();
    for _ in 0..4 {
        let records = process_records();
        let before = contained.len();
        let roots = records
            .iter()
            .filter(|record| {
                record.pid == session
                    || record.session == session
                    || (cfg!(target_os = "linux") && record.parent == own_pid)
                    || contained.contains(&record.parent)
            })
            .map(|record| record.pid)
            .collect::<Vec<_>>();
        contained.extend(roots);
        loop {
            let size = contained.len();
            for record in &records {
                if contained.contains(&record.parent) {
                    contained.insert(record.pid);
                }
            }
            if contained.len() == size {
                break;
            }
        }
        for pid in contained
            .iter()
            .copied()
            .filter(|pid| *pid > 1 && *pid != own_pid)
        {
            unsafe {
                libc::kill(pid, libc::SIGSTOP);
            }
        }
        if contained.len() == before {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let records = process_records();
    for record in records
        .iter()
        .filter(|record| contained.contains(&record.pid))
    {
        if record.group != own_group {
            kill_process_group(Some(record.group));
        }
    }
    for pid in contained
        .into_iter()
        .filter(|pid| *pid > 1 && *pid != own_pid)
    {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    kill_process_group(Some(session));
}

fn signal_process_group(group: Option<i32>, signal: i32) {
    if let Some(group) = group.filter(|group| *group > 1) {
        unsafe {
            libc::kill(-group, signal);
        }
    }
}

fn kill_process_group(group: Option<i32>) {
    signal_process_group(group, libc::SIGKILL);
}

fn process_records() -> Vec<ProcessRecord> {
    // /bin/ps and these columns are available on both supported terminal
    // targets (Linux and macOS). A failed snapshot still leaves the known
    // foreground and shell groups to kill.
    let Ok(child) = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,sess=,pgid="])
        .stdout(std::process::Stdio::piped())
        .spawn()
    else {
        return Vec::new();
    };
    let ps_pid = i32::try_from(child.id()).ok();
    let Ok(output) = child.wait_with_output() else {
        return Vec::new();
    };
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let mut fields = line
                .split(|byte| byte.is_ascii_whitespace())
                .filter(|field| !field.is_empty());
            let mut number = || {
                std::str::from_utf8(fields.next()?)
                    .ok()?
                    .parse::<i32>()
                    .ok()
            };
            let record = ProcessRecord {
                pid: number()?,
                parent: number()?,
                session: number()?,
                group: number()?,
            };
            (Some(record.pid) != ps_pid).then_some(record)
        })
        .collect()
}

#[cfg(test)]
fn session_process_groups(session: i32) -> Vec<i32> {
    let mut groups = process_records()
        .into_iter()
        .filter_map(|record| {
            (record.session == session && record.group > 1).then_some(record.group)
        })
        .collect::<Vec<_>>();
    groups.sort_unstable();
    groups.dedup();
    groups
}

fn configure_process_containment() -> Result<()> {
    #[cfg(target_os = "linux")]
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } == -1 {
        return Err(io::Error::last_os_error()).context("Could not contain terminal descendants");
    }
    Ok(())
}

fn pty_size(dimensions: Dimensions) -> PtySize {
    PtySize {
        rows: dimensions.rows,
        cols: dimensions.columns,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn working_directory(cwd: String) -> Result<PathBuf> {
    let cwd = if cwd.is_empty() {
        dirs::home_dir().ok_or_else(|| anyhow!("The account has no home directory"))?
    } else {
        PathBuf::from(cwd)
    };
    if !cwd.is_dir() {
        bail!("The terminal working directory does not exist");
    }
    Ok(cwd)
}

fn default_shell() -> PathBuf {
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

fn user_error(message: &str) -> String {
    const LIMIT: usize = pi_core::terminal_transport::MAX_ERROR_BYTES;
    if message.len() <= LIMIT {
        return message.to_owned();
    }
    let mut end = LIMIT;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead as _, BufReader},
        os::unix::process::CommandExt as _,
        process::Stdio,
    };

    #[test]
    fn output_queue_applies_backpressure_and_final_is_unique() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let output = Arc::new(ProtocolOutput {
            sender,
            finished: Mutex::new(false),
        });
        assert!(send(&output, &ServerMessage::Opened).unwrap());
        let blocked = output.clone();
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let _ = done_tx.send(send(&blocked, &ServerMessage::Opened));
        });
        assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
        assert!(matches!(receiver.recv().unwrap(), OutputCommand::Frame(_)));
        assert!(
            done_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap()
        );
        drop(receiver);

        let (sender, _receiver) = mpsc::sync_channel(2);
        let output = Arc::new(ProtocolOutput {
            sender,
            finished: Mutex::new(false),
        });
        assert!(finish(&output, &ServerMessage::Exit { code: None }).unwrap());
        assert!(!finish(&output, &ServerMessage::Error("late".into())).unwrap());
        assert!(!send(&output, &ServerMessage::Opened).unwrap());
    }

    #[test]
    fn stalled_queues_time_out_or_cancel_without_unbounded_waits() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let output = Arc::new(ProtocolOutput {
            sender,
            finished: Mutex::new(false),
        });
        assert!(send(&output, &ServerMessage::Opened).unwrap());
        let started = Instant::now();
        assert!(
            queue_output_with_timeout(
                &output,
                &ServerMessage::Opened,
                false,
                Duration::from_millis(15),
            )
            .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(1));

        let (sender, _receiver) = mpsc::sync_channel(1);
        sender.send(PtyCommand::Input(vec![1])).unwrap();
        let cancelled = AtomicBool::new(true);
        let started = Instant::now();
        assert!(
            queue_pty_command(
                &sender,
                PtyCommand::Input(vec![2]),
                &cancelled,
                Duration::from_secs(1),
            )
            .is_err()
        );
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn closing_a_terminal_kills_a_sighup_ignoring_descendant_group() {
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args([
                "-c",
                "set -m; (trap '' HUP TERM; while :; do sleep 1; done) & echo $!; wait",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut shell = command.spawn().expect("spawn isolated shell session");
        let session = i32::try_from(shell.id()).unwrap();
        let stdout = shell.stdout.take().unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout)
                .read_line(&mut line)
                .map(|_| line.trim().parse::<i32>());
            let _ = sender.send(result);
        });
        let descendant = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("descendant pid was not reported")
            .expect("read descendant pid")
            .expect("parse descendant pid");

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut found = false;
        while Instant::now() < deadline {
            let group = unsafe { libc::getpgid(descendant) };
            if group > 1 && session_process_groups(session).contains(&group) {
                found = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        kill_terminal_session(None, session);
        let _ = shell.kill();
        let _ = shell.wait();
        assert!(
            found,
            "the descendant process group was not in the PTY session"
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && unsafe { libc::kill(descendant, 0) } == 0 {
            thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(
            unsafe { libc::kill(descendant, 0) },
            0,
            "SIGHUP-ignoring descendant survived terminal close"
        );
    }
}
