//! Runs the REAL pinned durable harness/SQLite in a standalone faux-only executable, never a provider.
#![cfg(unix)]
use pi_core::{
    protocol::read_record,
    ssh::{RemoteBackend, SshTarget},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
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
    records: RecordInbox,
}

/// Responses and committed snapshots are independently scheduled. Waiting for
/// an acknowledgement must not discard the snapshot that arrived just before
/// it, especially when no further state changes until the test releases a tool.
struct RecordInbox {
    received: mpsc::Receiver<Value>,
    pending: RefCell<Vec<Value>>,
}
impl RecordInbox {
    fn new(received: mpsc::Receiver<Value>) -> Self {
        Self {
            received,
            pending: RefCell::new(Vec::new()),
        }
    }

    #[track_caller]
    fn until(&self, condition: impl Fn(&Value) -> bool) -> Value {
        let found = self.pending.borrow().iter().position(&condition);
        if let Some(index) = found {
            return self.pending.borrow_mut().remove(index);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let record = self
                .received
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("Timed out or durable bridge closed");
            if condition(&record) {
                return record;
            }
            self.pending.borrow_mut().push(record);
        }
    }
}

#[test]
fn an_acknowledgement_wait_keeps_the_committed_snapshot_that_preceded_it() {
    let (sender, receiver) = mpsc::channel();
    let inbox = RecordInbox::new(receiver);
    let snapshot = json!({"type":"remote_snapshot","data":{"queued_submissions":["1","2"]}});
    sender.send(snapshot.clone()).unwrap();
    sender
        .send(json!({"type":"response","id":"enqueue","success":true}))
        .unwrap();
    drop(sender);
    assert_eq!(inbox.until(|r| r["id"] == "enqueue")["success"], true);
    assert_eq!(inbox.until(|r| r["type"] == "remote_snapshot"), snapshot);
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
            records: RecordInbox::new(records),
        };
        bridge.send(target.attach_record());
        bridge
    }
    fn send(&mut self, record: Value) {
        writeln!(self.input.as_mut().unwrap(), "{record}").unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
    }
    #[track_caller]
    fn until(&self, condition: impl Fn(&Value) -> bool) -> Value {
        self.records.until(condition)
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
                .received
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

#[test]
#[ignore = "Build the standalone faux-only durable fixture and set PI_DESKTOP_TEST_DURABLE_RUNNER"]
fn images_survive_queue_cancellation_completion_and_reconnection() {
    let directory = tempfile::tempdir().unwrap();
    let mut target =
        SshTarget::new("test".into(), directory.path().to_string_lossy().into()).unwrap();
    target.backend = RemoteBackend::Durable;
    let _cleanup = Cleanup(directory.path().into(), target.key.clone());
    let mut bridge = Bridge::new(directory.path(), &target);
    bridge.snapshot();
    bridge
        .send(json!({"type":"prompt","id":"work","message":"safe","requestId":"image-test-work"}));
    assert_eq!(bridge.response("work")["success"], true);
    bridge.until(|r| r["type"] == "remote_snapshot" && r["data"]["run"] == "Running");
    let image = json!({"type":"image","mimeType":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg=="});
    let image_prompt = json!({"type":"prompt","id":"image","message":"inspect","images":[image],"requestId":"image-test-photo","streamingBehavior":"followUp"});
    bridge.send(image_prompt.clone());
    assert_eq!(bridge.response("image")["success"], true);
    bridge.send(json!({"type":"prompt","id":"remove","message":"cancel only this","requestId":"image-test-remove","streamingBehavior":"followUp"}));
    assert_eq!(bridge.response("remove")["success"], true);
    let queued = bridge.until(|r| {
        r["type"] == "remote_snapshot"
            && r["data"]["queued_submissions"]
                .as_array()
                .is_some_and(|items| items.len() == 2)
    });
    let cancel = queued["data"]["queued_submissions"][1].clone();
    bridge.send(json!({"type":"cancel_submission","id":"cancel","submissionId":cancel}));
    let cancelled = bridge.response("cancel");
    assert_eq!(cancelled["success"], true, "{cancelled}");
    std::fs::write(directory.path().join("release"), "go").unwrap();
    let finished = bridge.until(|r| {
        r["type"] == "remote_snapshot"
            && r["data"]["run"] == "Idle"
            && r["data"]["messages"]
                .to_string()
                .contains("Received 1 image(s)")
    });
    let messages = finished["data"]["messages"].as_array().unwrap();
    assert!(
        !messages
            .iter()
            .any(|m| m["role"] == "user" && m["content"] == "cancel only this")
    );
    let reference = messages
        .iter()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .find(|part| part["type"] == "image")
        .unwrap();
    assert_eq!(
        reference["data"], "",
        "streamed frames don't duplicate image bytes"
    );
    let image_id = reference["imageId"].clone();
    bridge.send(json!({"type":"get_image","id":"original","imageId":image_id}));
    assert_eq!(bridge.response("original")["data"]["image"], image);
    bridge.detach();
    let mut reconnected = Bridge::new(directory.path(), &target);
    let restored = reconnected.snapshot();
    assert!(
        restored["data"]["messages"]
            .to_string()
            .contains("Received 1 image(s)")
    );
    reconnected.send(image_prompt.clone());
    assert_eq!(reconnected.response("image")["data"]["duplicate"], true);
    let mut collision = image_prompt;
    collision["images"] = json!([]);
    collision["id"] = json!("collision");
    reconnected.send(collision);
    assert_eq!(reconnected.response("collision")["success"], false);
}

#[test]
#[ignore = "Build the standalone faux-only durable fixture and set PI_DESKTOP_TEST_DURABLE_RUNNER"]
fn confirmed_deletion_removes_history_but_keeps_project_files() {
    let directory = tempfile::tempdir().unwrap();
    let mut target =
        SshTarget::new("test".into(), directory.path().to_string_lossy().into()).unwrap();
    target.backend = RemoteBackend::Durable;
    let _cleanup = Cleanup(directory.path().into(), target.key.clone());
    let keep = directory.path().join("project.txt");
    std::fs::write(&keep, "project survives").unwrap();
    let mut bridge = Bridge::new(directory.path(), &target);
    bridge.snapshot();
    bridge
        .send(json!({"type":"remote_delete_session","id":"wrong","confirmKey":"another-session"}));
    assert_eq!(bridge.response("wrong")["success"], false);
    bridge.send(json!({"type":"prompt","id":"run","message":"safe","requestId":"delete-test-run"}));
    assert_eq!(bridge.response("run")["success"], true);
    bridge.until(|r| r["type"] == "remote_snapshot" && r["data"]["run"] == "Running");
    bridge.send(json!({"type":"remote_delete_session","id":"busy","confirmKey":target.key}));
    assert_eq!(bridge.response("busy")["success"], false);
    assert!(directory.path().join("durable").join(&target.key).exists());
    std::fs::write(directory.path().join("release"), "go").unwrap();
    bridge.until(|r| {
        r["type"] == "remote_snapshot"
            && r["data"]["run"] == "Idle"
            && r["data"]["messages"]
                .as_array()
                .is_some_and(|messages| messages.len() >= 2)
    });
    bridge.send(json!({"type":"remote_delete_session","id":"delete","confirmKey":target.key}));
    let response = bridge.response("delete");
    assert_eq!(response["success"], true, "{response}");
    bridge.closed();
    assert!(!directory.path().join("durable").join(&target.key).exists());
    assert_eq!(std::fs::read_to_string(keep).unwrap(), "project survives");
    let listing = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"))
        .arg("sessions")
        .env("PI_DESKTOP_REMOTE_STATE_DIR", directory.path())
        .output()
        .unwrap();
    let listing: Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert!(listing["sessions"].as_array().unwrap().is_empty());
    let restart = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"))
        .args(["daemon", &serde_json::to_string(&target).unwrap()])
        .env("PI_DESKTOP_REMOTE_STATE_DIR", directory.path())
        .output()
        .unwrap();
    assert!(!restart.status.success());
    assert!(String::from_utf8_lossy(&restart.stderr).contains("permanently deleted"));
}
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

/// Whether Pi has said something starting with `start`.
fn said(snapshot: &Value, start: &str) -> bool {
    snapshot["data"]["messages"]
        .as_array()
        .is_some_and(|messages| {
            messages.iter().any(|m| {
                m["role"] == "assistant"
                    && m["content"][0]["text"]
                        .as_str()
                        .unwrap_or("")
                        .starts_with(start)
            })
        })
}

/// What Pi said last.
fn last_said(snapshot: &Value) -> String {
    snapshot["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|m| m["role"] == "assistant")
        .unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The `subagent` call in the latest snapshot that has one.
fn handoff(snapshot: &Value) -> Option<Value> {
    snapshot["data"]["tools"]
        .as_array()?
        .iter()
        .find(|tool| tool["name"] == "subagent" && tool["details"].is_object())
        .cloned()
}

fn durable_target(directory: &Path) -> SshTarget {
    let mut target =
        SshTarget::new("test".into(), directory.to_string_lossy().into_owned()).unwrap();
    target.backend = RemoteBackend::Durable;
    target
}

#[test]
#[ignore = "Build the standalone faux-only durable fixture and set PI_DESKTOP_TEST_DURABLE_RUNNER"]
fn subagents_answer_side_by_side_and_show_their_own_messages() {
    let directory = tempfile::tempdir().unwrap();
    let target = durable_target(directory.path());
    let _cleanup = Cleanup(directory.path().into(), target.key.clone());
    let mut bridge = Bridge::new(directory.path(), &target);
    bridge.snapshot();
    bridge
        .send(json!({"type":"prompt","id":"delegate","message":"delegate","requestId":"delegate"}));
    assert_eq!(bridge.response("delegate")["success"], true);
    // The call returns at once, and the answers come back to Pi when both finish.
    let settled = bridge.until(|r| {
        r["type"] == "remote_snapshot" && r["data"]["run"] == "Idle" && said(r, "Heard back:")
    });
    assert!(said(&settled, "Delegated: Started 2 subagents (scout)"));
    let answer = last_said(&settled);
    assert!(
        answer.contains("found alpha") && answer.contains("found beta"),
        "{answer}"
    );
    let call = handoff(&settled).expect("the call keeps its subagents' summary");
    let results = call["details"]["results"].as_array().unwrap();
    assert_eq!(call["details"]["mode"], "parallel");
    assert!(
        results.iter().all(|result| result["status"] == "done"),
        "{call}"
    );
    assert_eq!(results[0]["output"], "found alpha");
    assert_eq!(results[1]["agent"], "scout");
    assert!(call["details"]["resumed"].is_null());

    // A subagent's own messages, on request.
    let child = results[0]["conversationId"].as_str().unwrap();
    bridge.send(json!({"type":"get_subagent","id":"child","conversationId":child}));
    let reply = bridge.response("child");
    assert_eq!(reply["success"], true, "{reply}");
    let messages = reply["data"]["messages"].as_array().unwrap();
    assert_eq!(messages[0]["content"], "find alpha");
    assert_eq!(
        messages.last().unwrap()["content"][0]["text"],
        "found alpha"
    );
    assert_eq!(reply["data"]["busy"], false);
    // Only this session's subagents: not its own conversation, nor a made-up one.
    for other in ["0", "999"] {
        bridge.send(json!({"type":"get_subagent","id":other,"conversationId":other}));
        assert_eq!(bridge.response(other)["success"], false);
    }
    bridge.send(json!({"type":"remote_shutdown","id":"shutdown"}));
    assert_eq!(bridge.response("shutdown")["success"], true);
    bridge.detach();
}

#[test]
#[ignore = "Build the standalone faux-only durable fixture and set PI_DESKTOP_TEST_DURABLE_RUNNER"]
fn a_subagent_carries_on_after_a_crash_without_repeating_an_unsafe_command() {
    let directory = tempfile::tempdir().unwrap();
    let target = durable_target(directory.path());
    let _cleanup = Cleanup(directory.path().into(), target.key.clone());
    let mut first = Bridge::new(directory.path(), &target);
    first.snapshot();
    first.send(
        json!({"type":"prompt","id":"delegate","message":"delegate unsafe","requestId":"delegate"}),
    );
    assert_eq!(first.response("delegate")["success"], true);
    // The subagent's crash-test tool is running and waiting for release.
    let running = first.until(|r| {
        r["type"] == "remote_snapshot"
            && handoff(r).is_some_and(|call| {
                call["details"]["results"][0]["now"]
                    .as_str()
                    .is_some_and(|now| now.contains("unsafe_work"))
            })
    });
    assert_eq!(executions(directory.path(), "unsafe"), 1);
    kill(&running["data"]["backend"]["Found"]["workerPid"]);
    first.closed();
    let mut first = first;
    first.input.take();
    let _ = first.process.wait().unwrap();
    std::fs::write(directory.path().join("release"), "continue").unwrap();

    let mut second = Bridge::new(directory.path(), &target);
    let settled = second.until(|r| {
        r["type"] == "remote_snapshot" && r["data"]["run"] == "Idle" && said(r, "Heard back:")
    });
    assert_eq!(
        executions(directory.path(), "unsafe"),
        1,
        "An interrupted unsafe command must not run again by itself"
    );
    let call = handoff(&settled).unwrap();
    let scout = &call["details"]["results"][0];
    assert_eq!(call["details"]["resumed"], true, "{call}");
    assert_eq!(scout["interrupted"], json!(["unsafe_work"]), "{call}");
    assert_eq!(scout["output"], "Finished unsafe after recovery.");
    second.send(json!({"type":"remote_shutdown","id":"shutdown"}));
    assert_eq!(second.response("shutdown")["success"], true);
    second.detach();
}

#[test]
#[ignore = "Build the standalone faux-only durable fixture and set PI_DESKTOP_TEST_DURABLE_RUNNER"]
fn pi_carries_on_while_a_subagent_works_and_stopping_it_leaves_the_session_alone() {
    let directory = tempfile::tempdir().unwrap();
    let target = durable_target(directory.path());
    let _cleanup = Cleanup(directory.path().into(), target.key.clone());
    let mut bridge = Bridge::new(directory.path(), &target);
    bridge.snapshot();
    bridge.send(
        json!({"type":"prompt","id":"delegate","message":"delegate unsafe","requestId":"delegate"}),
    );
    assert_eq!(bridge.response("delegate")["success"], true);
    let running = bridge.until(|r| {
        r["type"] == "remote_snapshot"
            && handoff(r).is_some_and(|call| {
                call["details"]["results"][0]["now"]
                    .as_str()
                    .is_some_and(|now| now.contains("unsafe_work"))
            })
    });
    let child = handoff(&running).unwrap()["details"]["results"][0]["conversationId"]
        .as_str()
        .unwrap()
        .to_owned();
    // Pi isn't held up by its subagent: it answered, and answers again.
    bridge.until(|r| {
        r["type"] == "remote_snapshot" && r["data"]["run"] == "Idle" && said(r, "Delegated:")
    });
    bridge.send(json!({"type":"prompt","id":"chat","message":"hello","requestId":"chat"}));
    assert_eq!(bridge.response("chat")["success"], true);
    let chatted = bridge.until(|r| {
        r["type"] == "remote_snapshot" && r["data"]["run"] == "Idle" && said(r, "Finished: hello")
    });
    assert_eq!(
        handoff(&chatted).unwrap()["details"]["results"][0]["status"],
        "running"
    );
    bridge.send(json!({"type":"stop_subagent","id":"stop","conversationId":child}));
    assert_eq!(bridge.response("stop")["success"], true);
    // Pi heard back and answered: the session itself was not stopped.
    let settled = bridge.until(|r| {
        r["type"] == "remote_snapshot" && r["data"]["run"] == "Idle" && said(r, "Heard back:")
    });
    let call = handoff(&settled).unwrap();
    assert_eq!(call["details"]["results"][0]["status"], "stopped", "{call}");
    assert_eq!(executions(directory.path(), "unsafe"), 1);
    bridge.send(json!({"type":"remote_shutdown","id":"shutdown"}));
    assert_eq!(bridge.response("shutdown")["success"], true);
    bridge.detach();
}

#[test]
#[ignore = "Build the standalone faux-only durable fixture and set PI_DESKTOP_TEST_DURABLE_RUNNER"]
fn pi_can_define_the_agents_it_hands_work_to() {
    let directory = tempfile::tempdir().unwrap();
    let target = durable_target(directory.path());
    let _cleanup = Cleanup(directory.path().into(), target.key.clone());
    let mut bridge = Bridge::new(directory.path(), &target);
    bridge.snapshot();
    // No agent file names these: the call defines them.
    bridge.send(
        json!({"type":"prompt","id":"delegate","message":"delegate custom","requestId":"delegate"}),
    );
    assert_eq!(bridge.response("delegate")["success"], true);
    let settled = bridge.until(|r| {
        r["type"] == "remote_snapshot" && r["data"]["run"] == "Idle" && said(r, "Heard back:")
    });
    let call = handoff(&settled).unwrap();
    let results = call["details"]["results"].as_array().unwrap();
    assert_eq!(
        results
            .iter()
            .map(|result| (
                result["agent"].as_str().unwrap(),
                result["output"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            ("architecture", "found layers"),
            ("code-quality", "found smells")
        ]
    );
    bridge.send(json!({"type":"remote_shutdown","id":"shutdown"}));
    assert_eq!(bridge.response("shutdown")["success"], true);
    bridge.detach();
}
