//! Offline contract/engine tests. Every database/project is isolated; the
//! worker below has no executable, credentials, network or provider access.
use super::*;
use pi_core::schedules::{Job, MissedPolicy, Occurrence, RunStatus, ScheduleSpec};
use serde_json::{Value, json};
use store::{Ledger, Store};
use worker::{Observation, Worker};

fn spec(project: &Path) -> ScheduleSpec {
    ScheduleSpec {
        name: "Reviewed host work".into(),
        cwd: project.to_str().unwrap().into(),
        prompt: "Inspect effects before deciding whether to repeat an interrupted operation".into(),
        provider: "fake".into(),
        model_id: "offline".into(),
        cron: "* * * * *".into(),
        timezone: "UTC".into(),
        enabled: true,
        missed_policy: MissedPolicy::Skip,
        timeout_minutes: 1,
    }
}
fn create(store: &mut Store, spec: ScheduleSpec, now: u64) -> Job {
    match store
        .mutate(
            Request::Create {
                spec,
                consent: true,
            },
            now,
        )
        .unwrap()
    {
        Data::Job { job } => job,
        _ => panic!(),
    }
}
fn manual(store: &mut Store, job: &Job, key: &str, now: u64) -> Occurrence {
    match store
        .mutate(
            Request::RunNow {
                job_id: job.id.clone(),
                expected_revision: job.revision,
                request_id: key.into(),
                consent: true,
            },
            now,
        )
        .unwrap()
    {
        Data::Run { occurrence } => occurrence,
        _ => panic!(),
    }
}
fn rows(store: &Store, job: &Job) -> Vec<Occurrence> {
    store.history(&job.id).unwrap()
}

#[test]
fn missing_project_can_be_paused_but_not_reenabled_or_edited() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let root = temp.path().join("registry");
    let mut store = Store::open(&root).unwrap();
    let job = create(&mut store, spec(&project), 0);
    std::fs::remove_dir(&project).unwrap();

    let mut paused = job.spec.clone();
    paused.enabled = false;
    let paused_job = match store
        .mutate(
            Request::Update {
                job_id: job.id.clone(),
                expected_revision: job.revision,
                spec: paused.clone(),
                consent: false,
            },
            1,
        )
        .unwrap()
    {
        Data::Job { job } => job,
        _ => panic!(),
    };
    assert_eq!(paused_job.spec, paused);
    assert_eq!(paused_job.revision, job.revision + 1);
    assert!(paused_job.next_due_ms.is_none());

    let mut edited = paused.clone();
    edited.prompt = "Different instructions".into();
    for spec in [job.spec, edited] {
        assert!(
            store
                .mutate(
                    Request::Update {
                        job_id: job.id.clone(),
                        expected_revision: paused_job.revision,
                        spec,
                        consent: true,
                    },
                    2
                )
                .is_err()
        );
    }
    drop(store);
    let mut reopened = Store::open(&root).unwrap();
    reopened.claim_due(120_000).unwrap();
    assert_eq!(reopened.list().unwrap()[0], paused_job);
    assert!(reopened.history(&job.id).unwrap().is_empty());
}

#[test]
fn persistence_consent_revisions_pause_and_payload_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let mut store = Store::open(&root).unwrap();
    let initial = spec(temp.path());
    assert!(
        store
            .mutate(
                Request::Create {
                    spec: initial.clone(),
                    consent: false
                },
                0
            )
            .is_err()
    );
    let mut paused = initial.clone();
    paused.enabled = false;
    let job = match store
        .mutate(
            Request::Create {
                spec: paused.clone(),
                consent: false,
            },
            0,
        )
        .unwrap()
    {
        Data::Job { job } => job,
        _ => panic!(),
    };
    assert!(job.next_due_ms.is_none());
    for (revision, consent) in [(99, true), (1, false)] {
        assert!(
            store
                .mutate(
                    Request::Update {
                        job_id: job.id.clone(),
                        expected_revision: revision,
                        spec: initial.clone(),
                        consent
                    },
                    0
                )
                .is_err()
        );
    }
    let job = match store
        .mutate(
            Request::Update {
                job_id: job.id,
                expected_revision: 1,
                spec: initial,
                consent: true,
            },
            0,
        )
        .unwrap()
    {
        Data::Job { job } => job,
        _ => panic!(),
    };
    assert_eq!(job.revision, 2);
    assert_eq!(job.next_due_ms, Some(60_000));
    store.claim_due(60_000).unwrap();
    let ledger = store.outstanding().unwrap().pop().unwrap();
    assert_eq!(ledger.payload["message"], job.spec.prompt);
    assert_eq!(ledger.payload["requestId"], ledger.occurrence.request_id);
    assert!(ledger.occurrence.session_key.is_some());
    let mut edited = job.spec.clone();
    edited.prompt = "New future payload".into();
    assert!(
        store
            .mutate(
                Request::Update {
                    job_id: job.id.clone(),
                    expected_revision: 2,
                    spec: edited.clone(),
                    consent: false
                },
                61_000
            )
            .is_err()
    );
    let mut pause_only = job.spec.clone();
    pause_only.enabled = false;
    let paused_job = match store
        .mutate(
            Request::Update {
                job_id: job.id.clone(),
                expected_revision: 2,
                spec: pause_only,
                consent: false,
            },
            61_000,
        )
        .unwrap()
    {
        Data::Job { job } => job,
        _ => panic!(),
    };
    edited.enabled = false;
    let edited_job = match store
        .mutate(
            Request::Update {
                job_id: job.id.clone(),
                expected_revision: 3,
                spec: edited,
                consent: true,
            },
            62_000,
        )
        .unwrap()
    {
        Data::Job { job } => job,
        _ => panic!(),
    };
    assert!(!edited_job.spec.enabled);
    assert!(edited_job.next_due_ms.is_none());
    assert_eq!(
        store.outstanding().unwrap()[0].payload,
        ledger.payload,
        "Edits must not alter admitted/persisted payloads"
    );
    assert_eq!(store.outstanding().unwrap()[0].occurrence.spec, job.spec);
    assert!(
        store
            .mutate(
                Request::Delete {
                    job_id: job.id.clone(),
                    expected_revision: paused_job.revision
                },
                63_000
            )
            .is_err()
    );
    store
        .mutate(
            Request::Delete {
                job_id: job.id.clone(),
                expected_revision: 4,
            },
            63_000,
        )
        .unwrap();
    drop(store);
    let mut reopened = Store::open(&root).unwrap();
    assert!(reopened.list().unwrap().is_empty());
    reopened.claim_due(120_000).unwrap();
    assert_eq!(rows(&reopened, &job).len(), 1);
    assert_eq!(reopened.outstanding().unwrap()[0].payload, ledger.payload);
    assert!(
        temp.path().exists(),
        "Deleting a job must never delete a project or session"
    );
}
#[test]
fn manual_consent_stable_identity_revision_and_no_recurring_clock_change() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    assert!(
        store
            .mutate(
                Request::RunNow {
                    job_id: job.id.clone(),
                    expected_revision: 1,
                    request_id: "click".into(),
                    consent: false
                },
                10
            )
            .is_err()
    );
    let first = manual(&mut store, &job, "click", 10);
    let second = manual(&mut store, &job, "click", 999);
    assert_eq!(first, second);
    assert_eq!(store.list().unwrap()[0].next_due_ms, Some(60_000));
    assert_ne!(first.id, first.session_key.clone().unwrap());
    assert!(
        store
            .mutate(
                Request::RunNow {
                    job_id: job.id.clone(),
                    expected_revision: 2,
                    request_id: "click".into(),
                    consent: true
                },
                1000
            )
            .is_err()
    );
    assert_eq!(
        manual(&mut store, &job, "new-click", 10).status,
        RunStatus::Skipped
    );
    let mut ledger = store.outstanding().unwrap().pop().unwrap();
    ledger.occurrence.status = RunStatus::Completed;
    store.save(&ledger).unwrap();
    let third = manual(&mut store, &job, "third", 10);
    assert_eq!(third.status, RunStatus::Pending);
    assert_ne!(first.session_key, third.session_key);
    assert_ne!(first.request_id, third.request_id);
    // Recurring and confirmed manual actions can have the same UTC instant.
    store.claim_due(60_000).unwrap();
    assert_eq!(rows(&store, &job).len(), 4);
}
#[test]
fn transactional_parallel_claims_and_revision_independent_utc_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("db");
    let mut store = Store::open(&root).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    let threads: Vec<_> = (0..12)
        .map(|_| {
            let root = root.clone();
            std::thread::spawn(move || Store::open(&root).unwrap().claim_due(60_000).unwrap())
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(rows(&store, &job).len(), 1);
    // Simulate a revised definition pointing back at the same instant. The
    // unique UTC identity (not revision) remains authoritative.
    let mut revised = store.list().unwrap()[0].clone();
    revised.revision += 1;
    revised.next_due_ms = Some(60_000);
    store
        .connection
        .execute(
            "UPDATE jobs SET data=? WHERE id=?",
            rusqlite::params![serde_json::to_string(&revised).unwrap(), job.id],
        )
        .unwrap();
    store.claim_due(60_000).unwrap();
    assert_eq!(rows(&store, &job).len(), 1);
    assert_eq!(rows(&store, &job)[0].job_revision, 1);
}
#[test]
fn overlap_pause_missed_skip_once_bounded_and_clock_rewind() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    store.claim_due(60_000).unwrap();
    store.claim_due(120_000).unwrap();
    assert_eq!(rows(&store, &job)[0].status, RunStatus::Skipped);
    store.claim_due(30_000).unwrap();
    assert_eq!(rows(&store, &job).len(), 2);
    store.claim_due(10 * 60_000).unwrap();
    assert_eq!(rows(&store, &job)[0].status, RunStatus::Missed);
    assert_eq!(store.list().unwrap()[0].next_due_ms, Some(11 * 60_000));
    let mut once = spec(temp.path());
    once.missed_policy = MissedPolicy::Once;
    let once_job = create(&mut store, once, 0);
    store.claim_due(365 * 24 * 60 * 60_000).unwrap();
    let history = rows(&store, &once_job);
    assert_eq!(
        history.len(),
        2,
        "A year of missed minutes must not produce an unbounded backlog"
    );
    let run = history
        .iter()
        .find(|o| o.status == RunStatus::Pending)
        .unwrap();
    assert!(365 * 24 * 60 * 60_000 - run.scheduled_ms <= store::CATCHUP_MS);
    assert!(history.iter().any(|o| o.status == RunStatus::Missed));
    store.claim_due(365 * 24 * 60 * 60_000).unwrap();
    assert_eq!(rows(&store, &once_job).len(), 2);
    let mut small = spec(temp.path());
    small.missed_policy = MissedPolicy::Once;
    let small_job = create(&mut store, small, 365 * 24 * 60 * 60_000);
    store
        .claim_due(365 * 24 * 60 * 60_000 + 10 * 60_000)
        .unwrap();
    assert_eq!(rows(&store, &small_job).len(), 1);
    assert_eq!(rows(&store, &small_job)[0].status, RunStatus::Pending);
}

#[derive(Default)]
struct Fake {
    admitted: Option<Value>,
    busy: bool,
    has_messages: bool,
    settled: Option<RunStatus>,
    configure_calls: usize,
    prompts: Vec<Value>,
    stops: usize,
    lose_ack: bool,
    stop_fails: bool,
}
impl Worker for Fake {
    fn inspect(&mut self, _: &Ledger) -> Result<Observation> {
        Ok(Observation {
            submission: self.admitted.clone(),
            busy: self.busy,
            has_messages: self.has_messages,
            settled: self.settled.map(|s| (s, None)),
        })
    }
    fn configure(&mut self, _: &Ledger) -> Result<()> {
        self.configure_calls += 1;
        Ok(())
    }
    fn submit(&mut self, row: &Ledger) -> Result<Value> {
        self.prompts.push(row.payload.clone());
        self.busy = true;
        self.admitted = Some(
            json!({"id":"77","type":"input","status":"placed","requestId":row.occurrence.request_id}),
        );
        if self.lose_ack {
            anyhow::bail!("Committed, but socket/ack lost");
        }
        Ok(
            json!({"submissionId":"77","requestId":row.occurrence.request_id,"disposition":"started"}),
        )
    }
    fn stop(&mut self, row: &Ledger) -> Result<()> {
        assert!(
            row.receipt
                .as_ref()
                .is_some_and(|receipt| receipt["id"] == "77")
        );
        self.stops += 1;
        if self.stop_fails {
            anyhow::bail!("Stop connection lost");
        }
        Ok(())
    }
    fn retire(&mut self, _: &Ledger) -> Result<bool> {
        Ok(true)
    }
}
#[test]
fn admission_crash_lost_ack_lookup_recovery_and_completion_without_replay() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("db");
    let mut store = Store::open(&root).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "click", 10);
    let mut ledger = store.outstanding().unwrap().pop().unwrap();
    let original = ledger.payload.clone();
    let key = ledger.occurrence.session_key.clone();
    let mut fake = Fake {
        lose_ack: true,
        ..Default::default()
    };
    worker::progress(&store, &mut ledger, &mut fake, 10).unwrap();
    assert_eq!(ledger.occurrence.status, RunStatus::Unknown);
    assert_eq!(fake.prompts.len(), 1);
    drop(store);
    let store = Store::open(&root).unwrap();
    let mut recovered = store.outstanding().unwrap().pop().unwrap();
    worker::progress(&store, &mut recovered, &mut fake, 20).unwrap();
    assert_eq!(recovered.occurrence.status, RunStatus::Running);
    assert_eq!(fake.prompts.len(), 1);
    assert_eq!(fake.configure_calls, 1);
    assert_eq!(recovered.payload, original);
    assert_eq!(recovered.occurrence.session_key, key);
    fake.busy = false;
    fake.settled = Some(RunStatus::Interrupted);
    worker::progress(&store, &mut recovered, &mut fake, 30).unwrap();
    assert_eq!(recovered.occurrence.status, RunStatus::Interrupted);
    assert_eq!(recovered.occurrence.finished_ms, Some(30));
    assert!(store.outstanding().unwrap().is_empty());
    assert_eq!(fake.prompts.len(), 1);
}
#[test]
fn crash_before_dispatch_can_dispatch_but_intent_without_receipt_never_can() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("db");
    let mut store = Store::open(&root).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "a", 1);
    drop(store);
    let store = Store::open(&root).unwrap();
    let mut ledger = store.outstanding().unwrap().pop().unwrap();
    let mut fake = Fake::default();
    worker::progress(&store, &mut ledger, &mut fake, 2).unwrap();
    assert_eq!(ledger.occurrence.status, RunStatus::Accepted);
    assert_eq!(fake.prompts.len(), 1);
    // Crash after persisting dispatch intent but before writing to socket.
    // Conservatively lose liveness, never invent a new request key or replay.
    fake.admitted = None;
    fake.busy = false;
    worker::progress(&store, &mut ledger, &mut fake, 3).unwrap();
    worker::progress(&store, &mut ledger, &mut fake, 4).unwrap();
    assert_eq!(ledger.occurrence.status, RunStatus::Unknown);
    assert_eq!(fake.prompts.len(), 1);
    assert_eq!(
        store.outstanding().unwrap().len(),
        1,
        "Uncertainty must continue to block same-job overlap"
    );
}
#[test]
fn timeout_stop_is_best_effort_persisted_once_and_not_pretend_completion() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "a", 10);
    let mut row = store.outstanding().unwrap().pop().unwrap();
    let mut fake = Fake {
        stop_fails: true,
        ..Default::default()
    };
    worker::progress(&store, &mut row, &mut fake, 10).unwrap();
    worker::progress(&store, &mut row, &mut fake, 60_009).unwrap();
    assert_eq!(fake.stops, 0);
    assert!(worker::progress(&store, &mut row, &mut fake, 60_010).is_err());
    let mut row = store.outstanding().unwrap().pop().unwrap();
    assert_eq!(row.stop_requested_ms, Some(60_010));
    worker::progress(&store, &mut row, &mut fake, 90_000).unwrap();
    assert_eq!(fake.stops, 1);
    assert_eq!(row.occurrence.status, RunStatus::Running);
    assert!(row.occurrence.finished_ms.is_none());
    fake.busy = false;
    fake.settled = Some(RunStatus::Interrupted);
    worker::progress(&store, &mut row, &mut fake, 100_000).unwrap();
    assert_eq!(row.occurrence.status, RunStatus::Interrupted);
}
#[test]
fn unrelated_session_is_not_reconfigured_or_displaced() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "a", 10);
    let mut row = store.outstanding().unwrap().pop().unwrap();
    let mut fake = Fake {
        has_messages: true,
        ..Default::default()
    };
    worker::progress(&store, &mut row, &mut fake, 10).unwrap();
    assert_eq!(row.occurrence.status, RunStatus::Unknown);
    assert_eq!(fake.configure_calls, 0);
    assert!(fake.prompts.is_empty());
}
#[test]
fn one_line_versioned_protocol_errors_no_envelope_and_no_implicit_start() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("db");
    for line in [
        "not json\n",
        "{\"type\":\"list\",\"id\":\"not-an-envelope\"}\n",
        "{\"type\":\"run_now\",\"jobId\":\"wrong\"}\n",
        "{\"type\":\"preview\",\"cron\":\"@daily\",\"timezone\":\"UTC\",\"count\":3}\n",
    ] {
        let mut output = Vec::new();
        serve_at(&root, std::io::Cursor::new(line), &mut output, 0).unwrap();
        assert_eq!(output.iter().filter(|b| **b == b'\n').count(), 1);
        let response: Response = serde_json::from_slice(&output).unwrap();
        assert_eq!(response.version, 1);
        assert!(!response.success);
    }
    let mut output = Vec::new();
    serve_at(
        &root,
        std::io::Cursor::new("{\"type\":\"list\"}\nTHIS SECOND LINE MUST NOT BE READ\n"),
        &mut output,
        0,
    )
    .unwrap();
    let response: Response = serde_json::from_slice(&output).unwrap();
    match response.data.unwrap() {
        Data::List { jobs, health } => {
            assert!(jobs.is_empty());
            assert!(!health.running);
            if cfg!(any(target_os = "linux", target_os = "macos")) {
                assert!(health.detail.contains("setup"));
            } else {
                assert!(health.detail.contains("unsupported"));
                assert!(!health.autostart_installed);
            }
        }
        _ => panic!(),
    }
    let mut invented = serde_json::to_value(Request::Create {
        spec: spec(temp.path()),
        consent: true,
    })
    .unwrap();
    invented["spec"]["read_only"] = json!(true); // never silently imply a sandbox
    let mut output = Vec::new();
    serve_at(
        &root,
        std::io::Cursor::new(format!("{invented}\n")),
        &mut output,
        0,
    )
    .unwrap();
    assert!(!serde_json::from_slice::<Response>(&output).unwrap().success);
    assert!(!root.join("stop.request").exists());
    assert!(!service::locked(&root).unwrap());
    assert!(!root.join("1").exists());
    assert!(!root.join("durable").exists());
    let mut output = Vec::new();
    serve_at(
        &root,
        std::io::Cursor::new(
            "{\"type\":\"preview\",\"cron\":\"* * * * *\",\"timezone\":\"UTC\",\"count\":2}\n",
        ),
        &mut output,
        0,
    )
    .unwrap();
    match serde_json::from_slice::<Response>(&output)
        .unwrap()
        .data
        .unwrap()
    {
        Data::Preview { times_ms } => assert_eq!(times_ms, vec![60_000, 120_000]),
        _ => panic!(),
    }
}

#[test]
fn future_database_is_rejected_without_schema_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("db");
    let store = Store::open(&root).unwrap();
    store
        .connection
        .execute_batch("PRAGMA user_version=2; CREATE TABLE future_only(value TEXT);")
        .unwrap();
    drop(store);
    assert!(Store::open(&root).is_err());
    let connection = rusqlite::Connection::open(root.join("schedules.sqlite3")).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='future_only'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        1
    );
}
#[test]
#[cfg(unix)]
fn private_registry_and_lock_files_never_follow_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("db");
    service::private_dir(&root).unwrap();
    let unrelated = temp.path().join("unrelated");
    std::fs::write(&unrelated, b"must not change").unwrap();
    symlink(&unrelated, root.join("schedules.sqlite3")).unwrap();
    assert!(Store::open(&root).is_err());
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"must not change");
    symlink(&unrelated, root.join("scheduler.lock")).unwrap();
    assert!(service::locked(&root).is_err());
    symlink(&root, temp.path().join("linked-root")).unwrap();
    assert!(service::private_dir(&temp.path().join("linked-root")).is_err());
    std::fs::remove_file(root.join("schedules.sqlite3")).unwrap();
    let _store = Store::open(&root).unwrap();
    for name in [
        "schedules.sqlite3",
        "schedules.sqlite3-wal",
        "schedules.sqlite3-shm",
    ] {
        assert_eq!(
            std::fs::metadata(root.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "{name}"
        );
    }
}

#[test]
fn prepared_recurring_attachment_failure_expires_on_restart_but_manual_and_intent_do_not() {
    for policy in [MissedPolicy::Skip, MissedPolicy::Once] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("db");
        let mut store = Store::open(&root).unwrap();
        let mut settings = spec(temp.path());
        settings.missed_policy = policy;
        let job = create(&mut store, settings, 0);
        store.claim_due(60_000).unwrap();
        let before = store.outstanding().unwrap().pop().unwrap();
        // Attachment unavailable before any intent: exact row remains Pending.
        assert!(!store.expire_prepared(&mut before.clone(), 60_001).unwrap());
        drop(store);
        let mut store = Store::open(&root).unwrap();
        let mut row = store.outstanding().unwrap().pop().unwrap();
        let mut fake = Fake::default();
        worker::progress(&store, &mut row, &mut fake, 60_001 + store::CATCHUP_MS).unwrap();
        assert_eq!(row.occurrence.status, RunStatus::Missed);
        assert!(row.occurrence.started_ms.is_none());
        assert_eq!(fake.configure_calls, 0);
        assert!(fake.prompts.is_empty());
        assert!(store.outstanding().unwrap().is_empty());
        assert_eq!(
            rows(&store, &job)[0].request_id,
            before.occurrence.request_id
        );

        manual(&mut store, &job, "explicit", 60_000);
        let mut row = store.outstanding().unwrap().pop().unwrap();
        worker::progress(&store, &mut row, &mut fake, 2 * store::CATCHUP_MS).unwrap();
        assert_eq!(row.occurrence.status, RunStatus::Accepted);
        assert_eq!(fake.prompts.len(), 1);
        fake.admitted = None;
        fake.busy = true; // Later unrelated work does NOT authorize global abort.
        worker::progress(&store, &mut row, &mut fake, 3 * store::CATCHUP_MS).unwrap();
        assert_eq!(row.occurrence.status, RunStatus::Unknown);
        assert_eq!(fake.prompts.len(), 1);
        assert_eq!(fake.stops, 0);
    }
}

#[test]
fn grace_and_catchup_are_rechecked_immediately_before_dispatch() {
    for (policy, limit) in [
        (MissedPolicy::Skip, 60_000),
        (MissedPolicy::Once, store::CATCHUP_MS + 1),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().join("db")).unwrap();
        let mut settings = spec(temp.path());
        settings.missed_policy = policy;
        create(&mut store, settings, 0);
        store.claim_due(60_000).unwrap();
        let mut row = store.outstanding().unwrap().pop().unwrap();
        let mut fake = Fake::default();
        let mut times = [60_000 + limit - 1, 60_000 + limit - 1, 60_000 + limit].into_iter();
        worker::progress_with_clock(&store, &mut row, &mut fake, || times.next().unwrap()).unwrap();
        assert_eq!(fake.configure_calls, 1);
        assert!(fake.prompts.is_empty());
        assert_eq!(row.occurrence.status, RunStatus::Missed);
    }
}

#[test]
fn monotonic_loop_scans_new_job_after_backward_wall_jump() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let origin = Instant::now();
    let mut cadence = ClaimCadence::default();
    let mut scans = Vec::new();
    // This is the production scan gate with independently injected clocks.
    if cadence.due(origin) {
        scans.push(10_000_000);
        store.claim_due(10_000_000).unwrap();
    }
    let job = create(&mut store, spec(temp.path()), 0); // new job after wall rewind
    for (elapsed, wall) in [(2, 20_000), (4, 40_000), (6, 60_000)] {
        if cadence.due(origin + Duration::from_secs(elapsed)) {
            scans.push(wall);
            store.claim_due(wall).unwrap();
        }
    }
    assert_eq!(scans, vec![10_000_000, 60_000]);
    assert_eq!(rows(&store, &job)[0].status, RunStatus::Pending);
}

struct ProjectedWorker {
    receipt: Value,
    correlated: Option<Value>,
    model: pi_core::session::Session,
    cancellations: Vec<Value>,
    retirement_calls: usize,
}
impl Worker for ProjectedWorker {
    fn inspect(&mut self, _: &Ledger) -> Result<Observation> {
        Ok(worker::observe(
            Some(self.receipt.clone()),
            self.correlated.as_ref(),
            &self.model,
        ))
    }
    fn configure(&mut self, _: &Ledger) -> Result<()> {
        panic!("not fresh")
    }
    fn submit(&mut self, _: &Ledger) -> Result<Value> {
        panic!("never replay")
    }
    fn stop(&mut self, row: &Ledger) -> Result<()> {
        self.cancellations.push(worker::cancel_request(row)?);
        Ok(())
    }
    fn retire(&mut self, _: &Ledger) -> Result<bool> {
        self.retirement_calls += 1;
        Ok(crate::server::scheduler_idle(&self.model))
    }
}
fn projection_worker(row: &Ledger) -> ProjectedWorker {
    let a = json!([
        {"role":"user","content":"A"},
        {"role":"assistant","content":[{"type":"text","text":"A done"}],"stopReason":"stop"}
    ]);
    let mut model = pi_core::session::Session::new("/test".into());
    model.apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":a}})).unwrap();
    ProjectedWorker {
        receipt: json!({"id":"77","requestId":row.occurrence.request_id,"type":"input","status":"done","entry":1,"answer":2}),
        correlated: Some(json!({"entry":1,"answer":2,"messages":a})),
        model,
        cancellations: Vec::new(),
        retirement_calls: 0,
    }
}

#[test]
fn a_terminal_then_b_running_or_failed_never_aborts_or_attributes_b() {
    for running in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().join("db")).unwrap();
        let job = create(&mut store, spec(temp.path()), 0);
        manual(&mut store, &job, "A", 1);
        let mut row = store.outstanding().unwrap().pop().unwrap();
        row.phase = "dispatching".into();
        row.occurrence.started_ms = Some(1);
        let mut fake = projection_worker(&row);
        fake.model
            .messages
            .push(json!({"role":"user","content":"B"}));
        fake.model
            .messages
            .push(json!({"role":"assistant","stopReason":"error","errorMessage":"B failed"}));
        fake.model.apply(&json!({"type":"tool_execution_start","toolCallId":"B-tool","toolName":"bash","args":{}})).unwrap();
        fake.model.apply(&json!({"type":"tool_execution_end","toolCallId":"B-tool","toolName":"bash","result":{"content":[{"type":"text","text":"B interrupted"}]},"isError":true})).unwrap();
        if running {
            fake.model.apply(&json!({"type":"agent_start"})).unwrap();
        }
        worker::progress(&store, &mut row, &mut fake, 120_000).unwrap();
        assert_eq!(row.occurrence.status, RunStatus::Completed);
        assert!(row.occurrence.error.is_none());
        assert!(row.stop_requested_ms.is_none());
        assert!(fake.cancellations.is_empty());
        assert_eq!(fake.model.busy(), running, "B must not be aborted");
        assert_eq!(row.retirement_pending, running, "busy owner preserved");
    }
}

#[test]
fn ambiguous_terminal_receipt_stays_unknown_and_exact_cancel_cannot_hit_b() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "A", 1);
    let mut row = store.outstanding().unwrap().pop().unwrap();
    row.phase = "dispatching".into();
    row.occurrence.started_ms = Some(1);
    let mut fake = projection_worker(&row);
    fake.correlated = None; // entry identity not available: no last-message fallback
    fake.model.apply(&json!({"type":"agent_start"})).unwrap();
    worker::progress(&store, &mut row, &mut fake, 120_000).unwrap();
    assert_eq!(row.occurrence.status, RunStatus::Unknown);
    assert!(fake.cancellations.is_empty());
    assert_eq!(fake.retirement_calls, 0);
    // Simulate an inspection/timeout race: cached A placed, now B owns the run.
    fake.receipt["status"] = json!("placed");
    worker::progress(&store, &mut row, &mut fake, 120_001).unwrap();
    assert_eq!(
        fake.cancellations,
        vec![
            json!({"type":"cancel_submission","submissionId":"77","requestId":row.occurrence.request_id})
        ]
    );
    assert!(fake.model.busy()); // exact A cancellation is never global B abort
}

#[test]
fn correlated_background_handoff_blocks_completion_until_known_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "A", 1);
    let row = store.outstanding().unwrap().pop().unwrap();
    let mut fake = projection_worker(&row);
    let call = json!({"role":"assistant","content":[{"type":"toolCall","id":"crew","name":"subagent","arguments":{}}],"stopReason":"toolUse"});
    fake.model.messages.insert(1, call.clone());
    fake.correlated.as_mut().unwrap()["messages"]
        .as_array_mut()
        .unwrap()
        .insert(1, call);
    fake.model.apply(&json!({"type":"tool_execution_start","toolCallId":"crew","toolName":"subagent","args":{}})).unwrap();
    fake.model.apply(&json!({"type":"tool_execution_end","toolCallId":"crew","toolName":"subagent","result":{"content":[{"type":"text","text":"Delegated"}],"details":{"version":1,"mode":"parallel","background":true,"results":[{"index":0,"agent":"scout","task":"work","status":"running"}]}},"isError":false})).unwrap();
    assert!(fake.inspect(&row).unwrap().settled.is_none());
    fake.model.tools.last_mut().unwrap().details["results"][0]["status"] = json!("stopped");
    assert!(
        fake.inspect(&row).unwrap().settled.is_none(),
        "terminal children alone do not prove report work settled"
    );
    fake.correlated.as_mut().unwrap()["reports"] = json!({
        "crew": report_projection("crew", 3, 4, json!([
            report_input("crew"),
            {"role":"assistant","stopReason":"stop"}
        ]))
    });
    assert_eq!(
        fake.inspect(&row).unwrap().settled.unwrap().0,
        RunStatus::Interrupted
    );
    fake.model.tools.last_mut().unwrap().details = Value::Null;
    assert!(
        fake.inspect(&row).unwrap().settled.is_none(),
        "missing delegate state stays ambiguous"
    );
}

fn report_input(call: &str) -> Value {
    json!({"role":"user","content":format!("<subagent_report call=\"{call}\">\nreport")})
}
fn report_projection(call: &str, entry: u64, answer: u64, messages: Value) -> Value {
    json!({"status":"done","receipt":{
        "type":"input","status":"done","requestId":format!("subagent-report-{call}"),
        "entry":entry,"answer":answer
    },"answer":messages.as_array().unwrap().last().unwrap(),"messages":messages})
}
fn delegate_call(id: &str) -> Value {
    json!({"role":"assistant","stopReason":"toolUse","content":[
        {"type":"toolCall","id":id,"name":"subagent","arguments":{}}
    ]})
}
fn add_delegate(model: &mut pi_core::session::Session, id: &str, status: &str) {
    model
        .apply(
            &json!({"type":"tool_execution_start","toolCallId":id,"toolName":"subagent","args":{}}),
        )
        .unwrap();
    model
        .apply(
            &json!({"type":"tool_execution_end","toolCallId":id,"toolName":"subagent","result":{
        "content":[{"type":"text","text":"Delegated"}],
        "details":{"version":1,"mode":"parallel","background":true,"results":[
            {"index":0,"agent":"scout","task":"work","status":status}
        ]}
    },"isError":false}),
        )
        .unwrap();
}
fn nested_projection(row: &Ledger) -> ProjectedWorker {
    let mut fake = projection_worker(row);
    let a = delegate_call("A-crew");
    fake.model.messages.insert(1, a.clone());
    fake.correlated.as_mut().unwrap()["messages"]
        .as_array_mut()
        .unwrap()
        .insert(1, a);
    add_delegate(&mut fake.model, "A-crew", "done");
    let report = json!([report_input("A-crew"), delegate_call("nested-crew"),
        {"role":"assistant","stopReason":"stop","content":"A report done"}]);
    fake.model
        .messages
        .extend(report.as_array().unwrap().iter().cloned());
    fake.correlated.as_mut().unwrap()["reports"] = json!({
        "A-crew":report_projection("A-crew", 3, 5, report)
    });
    add_delegate(&mut fake.model, "nested-crew", "running");
    fake
}
fn assert_unresolved_overlap(
    store: &mut Store,
    job: &Job,
    row: &mut Ledger,
    fake: &mut ProjectedWorker,
    now: u64,
) {
    worker::progress(store, row, fake, now).unwrap();
    assert_eq!(row.occurrence.status, RunStatus::Unknown);
    assert!(row.occurrence.finished_ms.is_none());
    assert!(!row.retirement_pending);
    assert_eq!(fake.retirement_calls, 0);
    assert!(fake.cancellations.is_empty());
    let active: i64 = store
        .connection
        .query_row(
            "SELECT active FROM occurrences WHERE id=?",
            [&row.occurrence.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        active, 1,
        "unresolved descendant must keep the overlap lock"
    );
    assert_eq!(
        manual(store, job, &format!("overlap-{now}"), now).status,
        RunStatus::Skipped
    );
    store.claim_due(now).unwrap();
    let recurring: String = store
        .connection
        .query_row(
            "SELECT data FROM occurrences WHERE job_id=? AND scheduled_ms=? AND manual_key IS NULL",
            rusqlite::params![job.id, now],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Ledger>(&recurring)
            .unwrap()
            .occurrence
            .status,
        RunStatus::Skipped
    );
}

#[test]
fn nested_report_tree_keeps_a_active_and_blocks_manual_and_recurring_overlap_until_all_settle() {
    // Deterministic worker + real SQLite policy proof, not a provider simulation.
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "A", 1);
    let mut row = store.outstanding().unwrap().pop().unwrap();
    let mut fake = nested_projection(&row);
    assert_unresolved_overlap(&mut store, &job, &mut row, &mut fake, 60_000);
    fake.model.tools.last_mut().unwrap().details["results"][0]["status"] = json!("done");
    assert_unresolved_overlap(&mut store, &job, &mut row, &mut fake, 120_000);
    // Nested children done, but their report receipt/answer span is outstanding.
    let report = json!([report_input("nested-crew"), delegate_call("third-crew"),
        {"role":"assistant","stopReason":"stop","content":"nested report done"}]);
    fake.model
        .messages
        .extend(report.as_array().unwrap().iter().cloned());
    fake.correlated.as_mut().unwrap()["reports"]["nested-crew"] =
        report_projection("nested-crew", 6, 8, report);
    add_delegate(&mut fake.model, "third-crew", "done");
    assert_unresolved_overlap(&mut store, &job, &mut row, &mut fake, 180_000);
    let report = json!([report_input("third-crew"),
        {"role":"assistant","stopReason":"stop","content":"third report done"}]);
    fake.model
        .messages
        .extend(report.as_array().unwrap().iter().cloned());
    fake.correlated.as_mut().unwrap()["reports"]["third-crew"] =
        report_projection("third-crew", 9, 10, report);
    worker::progress(&store, &mut row, &mut fake, 180_001).unwrap();
    assert_eq!(row.occurrence.status, RunStatus::Completed);
    assert_eq!(row.occurrence.finished_ms, Some(180_001));
    assert!(store.outstanding().unwrap().is_empty());
    assert_eq!(
        manual(&mut store, &job, "after-descendants", 180_002).status,
        RunStatus::Pending
    );
}

#[test]
fn descendant_outcomes_are_scoped_to_a_not_unrelated_b_and_missing_correlation_is_unknown() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "A", 1);
    let row = store.outstanding().unwrap().pop().unwrap();
    let mut fake = nested_projection(&row);
    fake.model.tools.last_mut().unwrap().details["results"][0]["status"] = json!("done");
    let report = json!([report_input("nested-crew"),
        {"role":"assistant","stopReason":"stop","content":"A descendants done"}]);
    fake.model
        .messages
        .extend(report.as_array().unwrap().iter().cloned());
    fake.correlated.as_mut().unwrap()["reports"]["nested-crew"] =
        report_projection("nested-crew", 6, 7, report);
    // Even a report map supplied by an older helper may contain unrelated B.
    // Only report IDs reached from A's continuation tree can affect A.
    fake.model.messages.extend([
        json!({"role":"user","content":"B"}),
        delegate_call("B-crew"),
        json!({"role":"assistant","stopReason":"error","errorMessage":"B failed"}),
    ]);
    add_delegate(&mut fake.model, "B-crew", "running");
    fake.correlated.as_mut().unwrap()["reports"]["B-crew"] = report_projection(
        "B-crew",
        11,
        12,
        json!([report_input("B-crew"), {"role":"assistant","stopReason":"error","errorMessage":"B report failed"}]),
    );
    assert_eq!(
        fake.inspect(&row).unwrap().settled.unwrap(),
        (RunStatus::Completed, None)
    );
    let original = fake.correlated.clone().unwrap();
    for field in ["messages", "receipt"] {
        fake.correlated.as_mut().unwrap()["reports"]["nested-crew"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            fake.inspect(&row).unwrap().settled.is_none(),
            "missing {field} is not completion"
        );
        fake.correlated = Some(original.clone());
    }
    fake.model
        .tools
        .iter_mut()
        .find(|t| t.id == "nested-crew")
        .unwrap()
        .details = Value::Null;
    assert!(fake.inspect(&row).unwrap().settled.is_none());
    let details = fake.model.tools[0].details.clone();
    fake.model
        .tools
        .iter_mut()
        .find(|t| t.id == "nested-crew")
        .unwrap()
        .details = details;
    for (reason, status) in [
        ("error", RunStatus::Failed),
        ("aborted", RunStatus::Interrupted),
    ] {
        let report = &mut fake.correlated.as_mut().unwrap()["reports"]["nested-crew"];
        report["answer"]["stopReason"] = json!(reason);
        report["answer"]["errorMessage"] = json!("A descendant failed");
        report["messages"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["stopReason"] = json!(reason);
        report["messages"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["errorMessage"] = json!("A descendant failed");
        assert_eq!(fake.inspect(&row).unwrap().settled.unwrap().0, status);
    }
}

#[test]
fn report_continuation_tools_and_shared_steered_spans_are_not_skipped() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "A", 1);
    let row = store.outstanding().unwrap().pop().unwrap();
    let mut fake = nested_projection(&row);
    fake.model.tools.last_mut().unwrap().details["results"][0]["status"] = json!("done");
    let work = json!({"role":"assistant","stopReason":"toolUse","content":[
        {"type":"toolCall","id":"report-work","name":"bash","arguments":{}}
    ]});
    let report = json!([report_input("nested-crew"), work,
        {"role":"assistant","stopReason":"stop","content":"A descendants done"}]);
    fake.model
        .messages
        .extend(report.as_array().unwrap().iter().cloned());
    fake.correlated.as_mut().unwrap()["reports"]["nested-crew"] =
        report_projection("nested-crew", 6, 8, report.clone());
    assert!(
        fake.inspect(&row).unwrap().settled.is_none(),
        "missing descendant tool projection is unknown"
    );
    fake.model.apply(&json!({"type":"tool_execution_start","toolCallId":"report-work","toolName":"bash","args":{}})).unwrap();
    assert!(
        fake.inspect(&row).unwrap().settled.is_none(),
        "running tool in a nested report blocks A"
    );
    fake.model.apply(&json!({"type":"tool_execution_end","toolCallId":"report-work","toolName":"bash","result":{"content":[{"type":"text","text":"work done"}]},"isError":false})).unwrap();
    assert_eq!(
        fake.inspect(&row).unwrap().settled.unwrap().0,
        RunStatus::Completed
    );
    // A steer report and A's own answer can share the same durable entries.
    // Walk the union of calls, without treating that overlap as ID reuse.
    let outer_report = fake.correlated.as_ref().unwrap()["reports"]["A-crew"]["messages"]
        .as_array()
        .unwrap()
        .clone();
    fake.correlated.as_mut().unwrap()["messages"]
        .as_array_mut()
        .unwrap()
        .extend(outer_report);
    fake.correlated.as_mut().unwrap()["messages"]
        .as_array_mut()
        .unwrap()
        .extend(report.as_array().unwrap().iter().cloned());
    fake.correlated.as_mut().unwrap()["answer"] = json!(8);
    fake.receipt["answer"] = json!(8);
    assert_eq!(
        fake.inspect(&row).unwrap().settled.unwrap().0,
        RunStatus::Completed
    );
    fake.model.messages.push(work);
    assert!(
        fake.inspect(&row).unwrap().settled.is_none(),
        "genuinely reused tool IDs are ambiguous"
    );
}

#[test]
fn unanswered_or_old_answer_only_report_without_continuation_correlation_keeps_a_active() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("db")).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "A", 1);
    let mut row = store.outstanding().unwrap().pop().unwrap();
    let mut fake = nested_projection(&row);
    fake.receipt["status"] = json!("unanswered");
    fake.receipt["reason"] = json!("aborted");
    fake.correlated = None;
    fake.model.tools.clear(); // missing projection is not proof no delegates ran
    assert_unresolved_overlap(&mut store, &job, &mut row, &mut fake, 60_000);
    fake = nested_projection(&row);
    fake.model.tools.last_mut().unwrap().details["results"][0]["status"] = json!("done");
    fake.correlated.as_mut().unwrap()["reports"]["A-crew"] =
        json!({"status":"done","answer":{"role":"assistant","stopReason":"stop"}});
    assert_unresolved_overlap(&mut store, &job, &mut row, &mut fake, 120_000);
}

#[test]
fn many_terminal_owners_are_retired_while_history_and_sessions_remain_reopenable() {
    use std::{cell::RefCell, collections::HashMap, rc::Rc};
    #[derive(Default)]
    struct Factory {
        owners: usize,
        peak: usize,
        sessions: HashMap<String, Value>,
    }
    struct Owner {
        fake: Fake,
        factory: Rc<RefCell<Factory>>,
    }
    impl Owner {
        fn open(factory: Rc<RefCell<Factory>>) -> Self {
            let mut state = factory.borrow_mut();
            state.owners += 1;
            state.peak = state.peak.max(state.owners);
            drop(state);
            Self {
                fake: Fake::default(),
                factory,
            }
        }
    }
    impl Worker for Owner {
        fn inspect(&mut self, row: &Ledger) -> Result<Observation> {
            self.fake.inspect(row)
        }
        fn configure(&mut self, row: &Ledger) -> Result<()> {
            self.fake.configure(row)
        }
        fn submit(&mut self, row: &Ledger) -> Result<Value> {
            self.factory.borrow_mut().sessions.insert(
                row.occurrence.session_key.clone().unwrap(),
                row.payload.clone(),
            );
            self.fake.submit(row)
        }
        fn stop(&mut self, row: &Ledger) -> Result<()> {
            self.fake.stop(row)
        }
        fn retire(&mut self, _: &Ledger) -> Result<bool> {
            if self.fake.busy {
                return Ok(false);
            }
            self.factory.borrow_mut().owners -= 1;
            Ok(true)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("db");
    let mut store = Store::open(&root).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    let factory = Rc::new(RefCell::new(Factory::default()));
    for index in 0..100 {
        manual(&mut store, &job, &format!("run_{index}"), index + 1);
        let mut row = store.outstanding().unwrap().pop().unwrap();
        let mut owner = Owner::open(factory.clone());
        worker::progress(&store, &mut row, &mut owner, index + 1).unwrap();
        owner.fake.busy = false;
        owner.fake.settled = Some(RunStatus::Completed);
        worker::progress(&store, &mut row, &mut owner, index + 2).unwrap();
        assert!(!row.retirement_pending);
        assert!(store.outstanding().unwrap().is_empty());
        assert_eq!(factory.borrow().owners, 0);
    }
    assert_eq!(factory.borrow().peak, 1);
    drop(store);
    let store = Store::open(&root).unwrap();
    let history = rows(&store, &job);
    assert_eq!(history.len(), 100);
    for occurrence in history {
        assert_eq!(occurrence.status, RunStatus::Completed);
        let key = occurrence.session_key.unwrap();
        assert_eq!(
            factory.borrow().sessions[&key]["requestId"],
            occurrence.request_id
        );
    }
}

#[test]
fn cleanup_retry_survives_restart_without_reinspection_replay_or_outcome_change() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("db");
    let mut store = Store::open(&root).unwrap();
    let job = create(&mut store, spec(temp.path()), 0);
    manual(&mut store, &job, "A", 1);
    let mut row = store.outstanding().unwrap().pop().unwrap();
    let mut fake = projection_worker(&row);
    fake.model.apply(&json!({"type":"agent_start"})).unwrap(); // B prevents retirement
    worker::progress(&store, &mut row, &mut fake, 2).unwrap();
    assert!(row.retirement_pending);
    let outcome = row.occurrence.clone();
    drop(store);
    let store = Store::open(&root).unwrap();
    let mut row = store.outstanding().unwrap().pop().unwrap();
    fake.correlated = None;
    fake.receipt = Value::Null; // cleanup must not perform any receipt inspection
    worker::progress(&store, &mut row, &mut fake, 3).unwrap();
    assert_eq!(row.occurrence, outcome);
    assert!(row.retirement_pending);
    fake.model.apply(&json!({"type":"agent_settled"})).unwrap();
    worker::progress(&store, &mut row, &mut fake, 4).unwrap();
    assert_eq!(row.occurrence, outcome);
    assert!(store.outstanding().unwrap().is_empty());
    assert!(fake.cancellations.is_empty());
}

#[test]
fn recurring_intent_or_receipt_never_expires_replays_or_globally_stops() {
    for has_receipt in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().join("db")).unwrap();
        create(&mut store, spec(temp.path()), 0);
        store.claim_due(60_000).unwrap();
        let mut row = store.outstanding().unwrap().pop().unwrap();
        if has_receipt {
            // Even an inconsistent recovered phase is not permission to replay
            // an input whose stable admission identity was already persisted.
            row.receipt = Some(json!({"submissionId":"77","requestId":row.occurrence.request_id}));
        } else {
            row.phase = "dispatching".into();
            row.occurrence.started_ms = Some(60_000);
        }
        store.save(&row).unwrap();
        let mut fake = Fake {
            busy: true,
            ..Default::default()
        };
        worker::progress(&store, &mut row, &mut fake, 2 * store::CATCHUP_MS).unwrap();
        assert_eq!(row.occurrence.status, RunStatus::Unknown);
        assert!(row.occurrence.finished_ms.is_none());
        assert!(!row.retirement_pending);
        assert_eq!(fake.configure_calls, 0);
        assert!(fake.prompts.is_empty());
        assert_eq!(fake.stops, 0);
    }
}
