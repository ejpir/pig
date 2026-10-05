//! Runs the REAL pinned durable harness/SQLite in a standalone faux-only executable, never a provider.
#![cfg(unix)]
use pi_core::{
    protocol::read_record,
    ssh::{RemoteBackend, SshTarget},
};
use serde_json::{Value, json};
use std::{
    io::{BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[test]
fn wrong_platform_runner_is_rejected_before_publishing_an_endpoint_or_starting_pi() {
    let directory = tempfile::tempdir().unwrap();
    let runner = directory.path().join("wrong-platform-runner");
    let mut header = [0; 64];
    if std::env::consts::OS == "macos" {
        header[..6].copy_from_slice(b"\x7fELF\x02\x01");
        header[18..20].copy_from_slice(&183u16.to_le_bytes());
    } else {
        header[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        header[4..8].copy_from_slice(&0x0100_000cu32.to_le_bytes());
    }
    std::fs::write(&runner, header).unwrap();
    let mut target = SshTarget::new(
        "test".into(),
        directory.path().to_string_lossy().into_owned(),
    )
    .unwrap();
    target.backend = RemoteBackend::Durable;
    let output = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"))
        .args(["daemon", &serde_json::to_string(&target).unwrap()])
        .env("PI_DESKTOP_REMOTE_STATE_DIR", directory.path())
        .env("PI_DESKTOP_DURABLE_RUNNER", &runner)
        .env(
            "PI_DESKTOP_PI",
            directory.path().join("DO-NOT-START-STOCK-PI"),
        )
        .env_remove("PI_DESKTOP_RPC_ENTRY")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("not a native"), "{stderr}");
    assert!(stderr.contains("PRODUCTION"), "{stderr}");
    assert!(
        !directory
            .path()
            .join("1")
            .join(format!("{}.json", target.key))
            .exists()
    );
    assert!(
        !directory.path().join("durable").exists(),
        "No runner, database or stock fallback should start"
    );
}

struct Bridge {
    process: Child,
    input: Option<ChildStdin>,
    records: mpsc::Receiver<Value>,
}
impl Bridge {
    fn new(root: &Path, target: &SshTarget) -> Self {
        let runner = std::env::var_os("PI_DESKTOP_TEST_DURABLE_RUNNER")
            .expect("Build backend/durable/test/fixture.ts and set PI_DESKTOP_TEST_DURABLE_RUNNER");
        let mut process = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"))
            .args(["connect", "--stdio"])
            .env("PI_DESKTOP_REMOTE_STATE_DIR", root)
            .env("PI_DESKTOP_DURABLE_RUNNER", runner)
            .env("PI_DESKTOP_DURABLE_PROVIDER", "faux")
            .env("PI_DESKTOP_DURABLE_MODEL", "faux-1")
            // A regression to stock Pi must fail; it must never launch a real provider/session.
            .env("PI_DESKTOP_PI", root.join("DO-NOT-START-STOCK-PI"))
            .env_remove("PI_DESKTOP_RPC_ENTRY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = process.stdin.take();
        let stdout = process.stdout.take().unwrap();
        let (sender, records) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(record)) = read_record(&mut reader) {
                if sender.send(record).is_err() {
                    break;
                }
            }
        });
        let mut bridge = Self {
            process,
            input,
            records,
        };
        bridge.send(target.attach_record());
        bridge
    }
    fn send(&mut self, record: Value) {
        writeln!(self.input.as_mut().unwrap(), "{record}").unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
    }
    fn until(&self, condition: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let record = self
                .records
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("Timed out or durable bridge closed");
            if condition(&record) {
                return record;
            }
        }
    }
    fn snapshot(&self) -> Value {
        self.until(|record| record["type"] == "remote_snapshot")
    }
    fn response(&self, id: &str) -> Value {
        self.until(|record| record["type"] == "response" && record["id"] == id)
    }
    fn closed(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match self
                .records
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(error) => panic!("Durable bridge did not close: {error}"),
            }
        }
    }
    fn detach(mut self) {
        self.input.take();
        assert!(self.process.wait().unwrap().success());
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
struct Cleanup(PathBuf, String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Ok(bytes) = std::fs::read(self.0.join("1").join(format!("{}.json", self.1)))
            && let Ok(endpoint) = serde_json::from_slice::<Value>(&bytes)
            && let Some(pid) = endpoint["pid"].as_i64()
        {
            // SAFETY: this PID is the detached daemon created for this isolated test directory.
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
    }
}
fn kill(pid: &Value) {
    // SAFETY: the PID comes from this test's private endpoint/runner metadata.
    assert_eq!(
        unsafe { libc::kill(pid.as_i64().unwrap() as i32, libc::SIGKILL) },
        0
    );
}
fn executions(root: &Path, name: &str) -> usize {
    std::fs::read_to_string(root.join(format!("{name}_work.log")))
        .unwrap()
        .lines()
        .count()
}
fn recovery(name: &str, kill_daemon: bool) {
    let directory = tempfile::tempdir().unwrap();
    let mut target = SshTarget::new(
        "test".into(),
        directory.path().to_string_lossy().into_owned(),
    )
    .unwrap();
    target.backend = RemoteBackend::Durable;
    let _cleanup = Cleanup(directory.path().into(), target.key.clone());
    let request = json!({"type":"prompt","id":"original","message":name,"requestId":"persistent-input","streamingBehavior":"followUp"});
    let mut first = Bridge::new(directory.path(), &target);
    first.snapshot();
    first.send(request.clone());
    let ack = first.response("original");
    assert_eq!(ack["success"], true, "{ack}");
    let running = first.until(|r| {
        r["type"] == "remote_snapshot"
            && r["data"]["tools"].as_array().is_some_and(|tools| {
                tools
                    .iter()
                    .any(|tool| tool["output"].as_str().unwrap_or("").contains("waiting"))
            })
    });
    assert_eq!(executions(directory.path(), name), 1);
    first.send(json!({"type":"prompt","id":"queue","message":"queued follow-up","requestId":"persistent-follow-up","streamingBehavior":"followUp"}));
    assert_eq!(first.response("queue")["success"], true);
    first.detach();

    let second = Bridge::new(directory.path(), &target);
    let reattached = second.until(|r| {
        r["type"] == "remote_snapshot"
            && r["data"]["follow_up"]
                .as_array()
                .is_some_and(|queue| queue.len() == 1)
    });
    assert_eq!(reattached["data"]["run"], "Running");
    assert_eq!(
        executions(directory.path(), name),
        1,
        "A desktop disconnect must not restart the tool"
    );
    if kill_daemon {
        let endpoint: Value = serde_json::from_slice(
            &std::fs::read(
                directory
                    .path()
                    .join("1")
                    .join(format!("{}.json", target.key)),
            )
            .unwrap(),
        )
        .unwrap();
        kill(&endpoint["pid"]);
    } else {
        kill(&running["data"]["backend"]["Found"]["workerPid"]);
    }
    second.closed();
    // A forced kill can truncate an in-flight JSONL frame. That disconnect is expected, not a clean detach.
    let mut second = second;
    second.input.take();
    let _ = second.process.wait().unwrap();
    std::fs::write(directory.path().join("release"), "continue").unwrap();

    let mut third = Bridge::new(directory.path(), &target);
    let settled = third.until(|r| {
        r["type"] == "remote_snapshot"
            && r["data"]["run"] == "Idle"
            && r["data"]["messages"].as_array().is_some_and(|messages| {
                messages.iter().any(|m| {
                    m["role"] == "assistant"
                        && m["content"][0]["text"]
                            .as_str()
                            .unwrap_or("")
                            .contains("queued follow-up")
                })
            })
    });
    let messages = settled["data"]["messages"].as_array().unwrap();
    assert_eq!(
        messages.iter().filter(|m| m["role"] == "user").count(),
        2,
        "Restoring the inbox must not resubmit the original prompt"
    );
    assert_eq!(
        executions(directory.path(), name),
        if name == "safe" { 2 } else { 1 }
    );
    if name == "unsafe" {
        assert!(
            settled["data"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["is_error"] == true),
            "Unsafe interruption must be visible, not replayed or reported successful"
        );
    }
    third.send(json!({"type":"get_submission","id":"receipt","requestId":"persistent-input"}));
    let receipt = third.response("receipt");
    assert_eq!(receipt["data"]["submission"]["status"], "done");
    assert_eq!(
        receipt["data"]["submission"]["id"],
        ack["data"]["submissionId"]
    );
    // An explicit retry with the SAME durable key is safe. There is no automatic mutation replay.
    third.send(request);
    let duplicate = third.response("original");
    assert_eq!(
        duplicate["data"]["submissionId"],
        ack["data"]["submissionId"]
    );
    assert_eq!(duplicate["data"]["duplicate"], true);
    third.send(json!({"type":"prompt","id":"collision","message":"different payload","requestId":"persistent-input","streamingBehavior":"followUp"}));
    assert_eq!(third.response("collision")["success"], false);
    third.send(json!({"type":"remote_shutdown","id":"shutdown"}));
    assert_eq!(third.response("shutdown")["success"], true);
    third.detach();
    assert!(
        directory
            .path()
            .join("durable")
            .join(&target.key)
            .join("session.sqlite")
            .is_file()
    );
}

#[test]
#[ignore = "Build the standalone faux-only durable fixture and set PI_DESKTOP_TEST_DURABLE_RUNNER"]
fn durable_worker_crash_recovers_safe_tool_and_persisted_inbox() {
    recovery("safe", false);
}
#[test]
#[ignore = "Build the standalone faux-only durable fixture and set PI_DESKTOP_TEST_DURABLE_RUNNER"]
fn durable_worker_crash_does_not_repeat_unsafe_side_effects() {
    recovery("unsafe", false);
}
#[test]
#[ignore = "Build the standalone faux-only durable fixture and set PI_DESKTOP_TEST_DURABLE_RUNNER"]
fn durable_daemon_crash_recovers_pending_work() {
    recovery("safe", true);
}

#[test]
#[ignore = "Build the standalone faux-only durable fixture and set PI_DESKTOP_TEST_DURABLE_RUNNER"]
fn durable_runner_keeps_writer_lock_if_its_rust_owner_is_killed() {
    let directory = tempfile::tempdir().unwrap();
    let mut target = SshTarget::new(
        "test".into(),
        directory.path().to_string_lossy().into_owned(),
    )
    .unwrap();
    target.backend = RemoteBackend::Durable;
    let mut owner = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"))
        .args(["durable-worker", &serde_json::to_string(&target).unwrap()])
        .env("PI_DESKTOP_REMOTE_STATE_DIR", directory.path())
        .env(
            "PI_DESKTOP_DURABLE_RUNNER",
            std::env::var_os("PI_DESKTOP_TEST_DURABLE_RUNNER").expect("Build the durable fixture"),
        )
        .env("PI_DESKTOP_DURABLE_PROVIDER", "faux")
        .env("PI_DESKTOP_DURABLE_MODEL", "faux-1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let input = owner.stdin.take().unwrap();
    let stdout = owner.stdout.take().unwrap();
    let (sender, records) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(record)) = read_record(&mut reader) {
            if sender.send(record).is_err() {
                break;
            }
        }
    });
    let record = records.recv_timeout(Duration::from_secs(15)).unwrap();
    assert_eq!(record["type"], "durable_state");
    // Keep stdout open: an owner crash must not be confused with the runner losing its output pipe.
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(
            directory
                .path()
                .join("durable")
                .join(&target.key)
                .join("owner.lock"),
        )
        .unwrap();
    assert!(matches!(
        lock.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    kill(&json!(owner.id()));
    let _ = owner.wait().unwrap();
    assert!(
        matches!(lock.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        "A surviving runner must retain the inherited writer lock"
    );
    drop(input); // EOF tells the surviving runner to close, preserving any pending checkpoints.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            error => panic!("Orphan runner failed to release its storage lock: {error:?}"),
        }
    }
}
