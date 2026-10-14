//! Offline-only APK harness. No provider selection, API keys, prompt input, or real workspace.
//! Own it in a view; cancellation and cleanup happen off the UI thread.
use crate::{
    Cancellation, Run,
    fixture::{Fixture, Scenario},
    spawn_with_cancel,
};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

static BUSY: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
struct Admission;
impl Drop for Admission {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
pub enum Mode {
    RoundTrip,
    WaitForCancel,
}

type FixtureExecute = fn(&Path, Mode, &Cancellation, &Arc<Mutex<Value>>) -> Result<()>;

pub struct FixtureRun {
    snapshot: Arc<Mutex<Value>>,
    cancel: Cancellation,
    worker: Option<thread::JoinHandle<()>>,
}
impl FixtureRun {
    pub fn start(directory: PathBuf, mode: Mode) -> Result<Self> {
        Self::start_using(directory, mode, execute)
    }
    fn start_using(directory: PathBuf, mode: Mode, execute: FixtureExecute) -> Result<Self> {
        ensure!(
            BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "a fixture is still running or cleaning up"
        );
        let admission = Admission;
        let id = format!(
            "{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let snapshot = Arc::new(Mutex::new(json!({
            "run_id": id, "status": "preparing", "mode": match mode { Mode::RoundTrip => "round-trip", Mode::WaitForCancel => "wait-for-cancel" },
            "runtime": "QuickJS", "provider": "loopback-fixture", "live_provider": false,
            "pid": std::process::id(), "uid": unsafe { libc::geteuid() },
            "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
            "events": 0, "tools": 0, "agent_ends": 0, "text": "", "error": "",
            "request_seen": false, "transport_closed": false, "fixture_files_cleaned": false,
            "cancel_reason": ""
        })));
        let cancel = Cancellation::default();
        let worker_cancel = cancel.clone();
        let worker_snapshot = snapshot.clone();
        let worker = thread::Builder::new()
            .name("pi-quickjs-fixture".into())
            .spawn(move || {
                let _admission = admission;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    execute(&directory, mode, &worker_cancel, &worker_snapshot)
                }))
                .unwrap_or_else(|_| Err(anyhow!("fixture harness panicked")));
                let mut final_snapshot = worker_snapshot
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                match result {
                    Ok(()) => final_snapshot["status"] = json!("passed"),
                    Err(error) => {
                        final_snapshot["status"] = json!(if final_snapshot["cancel_reason"]
                            .as_str()
                            .is_some_and(|reason| !reason.is_empty())
                        {
                            "cancelled"
                        } else {
                            "failed"
                        });
                        final_snapshot["error"] =
                            json!(error.to_string().chars().take(2048).collect::<String>());
                    }
                }
                if let Err(error) = publish(&directory, &final_snapshot) {
                    final_snapshot["status"] = json!("failed");
                    final_snapshot["error"] =
                        json!(format!("Cannot write fixture report: {error}"));
                }
                *worker_snapshot
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = final_snapshot;
            })?;
        Ok(Self {
            snapshot,
            cancel,
            worker: Some(worker),
        })
    }
    pub fn snapshot(&self) -> Value {
        self.snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
    pub fn is_running(&self) -> bool {
        matches!(
            self.snapshot()["status"].as_str(),
            Some("preparing" | "running")
        )
    }
    pub fn cancel(&self, reason: &str) {
        let mut state = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(state["status"].as_str(), Some("preparing" | "running")) {
            if !self.cancel.is_cancelled() {
                state["cancel_reason"] = json!(reason.chars().take(80).collect::<String>());
            }
            self.cancel.cancel();
        }
    }
    /// For tests/non-UI callers only. UI owners just drop/cancel and poll `snapshot`.
    pub fn join(mut self) -> Result<Value> {
        self.worker
            .take()
            .unwrap()
            .join()
            .map_err(|_| anyhow!("fixture worker panicked"))?;
        Ok(self.snapshot())
    }
}
impl Drop for FixtureRun {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.cancel("view dropped");
        }
    }
}

fn publish(directory: &Path, snapshot: &Value) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let temporary = directory.join("report.json.new");
    std::fs::write(&temporary, serde_json::to_vec_pretty(snapshot)?)?;
    std::fs::rename(temporary, directory.join("report.json"))?;
    Ok(())
}
// Unlike the UI-owned Run handle, a harness scope must join even during unwinding.
struct JoinedRun(Option<Run>);
impl JoinedRun {
    fn cancel(&self) {
        self.0.as_ref().unwrap().cancel();
    }
    fn join(mut self) -> Result<Value> {
        self.0.take().unwrap().join()
    }
}
impl Drop for JoinedRun {
    fn drop(&mut self) {
        if let Some(run) = self.0.take() {
            run.cancel();
            let _ = run.join();
        }
    }
}
fn execute(
    directory: &Path,
    mode: Mode,
    cancel: &Cancellation,
    state: &Arc<Mutex<Value>>,
) -> Result<()> {
    execute_with_publisher(directory, mode, cancel, state, publish)
}
fn execute_with_publisher(
    directory: &Path,
    mode: Mode,
    cancel: &Cancellation,
    state: &Arc<Mutex<Value>>,
    publish: impl Fn(&Path, &Value) -> Result<()>,
) -> Result<()> {
    publish(
        directory,
        &state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )?;
    ensure!(!cancel.is_cancelled(), "cancelled before fixture started");
    let scenario = match mode {
        Mode::RoundTrip => Scenario::Normal,
        Mode::WaitForCancel => Scenario::StallBody,
    };
    let mut fixture = Fixture::start_in(scenario, directory)?;
    let fixture_path = fixture.workspace.path().to_owned();
    let events = state.clone();
    let mut options = fixture.options();
    if matches!(mode, Mode::WaitForCancel) {
        options.timeout = Duration::from_secs(60);
    }
    let run = JoinedRun(Some(spawn_with_cancel(
        options,
        cancel.clone(),
        move |event| {
            let mut snapshot = events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let n = snapshot["events"].as_u64().unwrap();
            snapshot["events"] = json!(n + 1);
            if event["type"] == "tool_execution_end" {
                let n = snapshot["tools"].as_u64().unwrap();
                snapshot["tools"] = json!(n + 1);
            }
            if event["type"] == "agent_end" {
                let n = snapshot["agent_ends"].as_u64().unwrap();
                snapshot["agent_ends"] = json!(n + 1);
            }
            if event["message"]["role"] == "assistant" {
                let text: String = event["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|part| part["text"].as_str())
                    .collect();
                snapshot["text"] = json!(text.chars().take(4096).collect::<String>());
            }
        },
    )));
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())["status"] = json!("running");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.received_request() && !cancel.is_cancelled() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())["request_seen"] =
        json!(fixture.received_request());
    let publication = publish(
        directory,
        &state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    if publication.is_err() {
        run.cancel();
    }
    let result = run.join();
    let checked_transport =
        result.is_ok() || (matches!(mode, Mode::WaitForCancel) && fixture.received_request());
    let verification = if let Ok(result) = &result {
        fixture
            .finish()
            .and_then(|()| fixture.verify_result(result))
    } else if matches!(mode, Mode::WaitForCancel) && fixture.received_request() {
        // The stalled fixture only finishes on socket EOF/reset. Prove transport teardown.
        fixture.finish()
    } else {
        Ok(())
    };
    if verification.is_ok() && checked_transport && fixture.received_request() {
        state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())["transport_closed"] = json!(true);
    }
    drop(fixture);
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())["fixture_files_cleaned"] =
        json!(!fixture_path.exists());
    verification?;
    publication?;
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_app_fixture_lifecycle_and_single_admission() -> Result<()> {
        let root = tempfile::tempdir()?;
        let report = FixtureRun::start(root.path().to_owned(), Mode::RoundTrip)?.join()?;
        assert_eq!(report["status"], "passed");
        assert_eq!(report["text"], "Saved 🐷");
        assert_eq!(report["tools"], 3);
        assert_eq!(report["agent_ends"], 1);
        assert_eq!(report["fixture_files_cleaned"], true);
        let original_id = report["run_id"].clone();
        let run = FixtureRun::start(root.path().to_owned(), Mode::WaitForCancel)?;
        assert!(FixtureRun::start(root.path().to_owned(), Mode::RoundTrip).is_err());
        let deadline = Instant::now() + Duration::from_secs(5);
        while run.snapshot()["request_seen"] != true {
            ensure!(Instant::now() < deadline, "no fixture request");
            thread::sleep(Duration::from_millis(5));
        }
        run.cancel("background");
        let report = run.join()?;
        assert_eq!(report["status"], "cancelled");
        assert_eq!(report["cancel_reason"], "background");
        assert_eq!(report["transport_closed"], true);
        assert_eq!(report["fixture_files_cleaned"], true);
        assert_ne!(report["run_id"], original_id);
        let bytes = std::fs::read(root.path().join("report.json"))?;
        assert_eq!(serde_json::from_slice::<Value>(&bytes)?, report);
        assert_eq!(std::fs::read_dir(root.path())?.count(), 1);
        fn fail_second_report(
            directory: &Path,
            mode: Mode,
            cancel: &Cancellation,
            state: &Arc<Mutex<Value>>,
        ) -> Result<()> {
            let calls = std::cell::Cell::new(0);
            execute_with_publisher(directory, mode, cancel, state, |directory, snapshot| {
                calls.set(calls.get() + 1);
                ensure!(calls.get() != 2, "injected second publication failure");
                publish(directory, snapshot)
            })
        }
        let failed = FixtureRun::start_using(
            root.path().to_owned(),
            Mode::WaitForCancel,
            fail_second_report,
        )?
        .join()?;
        assert_eq!(failed["status"], "failed");
        assert!(
            failed["error"]
                .as_str()
                .unwrap()
                .contains("publication failure")
        );
        assert_eq!(failed["transport_closed"], true);
        assert_eq!(failed["fixture_files_cleaned"], true);
        fn panic_harness(_: &Path, _: Mode, _: &Cancellation, _: &Arc<Mutex<Value>>) -> Result<()> {
            panic!("injected harness panic");
        }
        let failed =
            FixtureRun::start_using(root.path().to_owned(), Mode::RoundTrip, panic_harness)?
                .join()?;
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["error"], "fixture harness panicked");
        assert_eq!(
            FixtureRun::start(root.path().to_owned(), Mode::RoundTrip)?.join()?["status"],
            "passed"
        );
        Ok(())
    }
}
