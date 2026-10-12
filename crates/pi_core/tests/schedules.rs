//! Wire-contract tests shared by host scheduler and phone clients.
use pi_core::schedules::*;

fn spec() -> ScheduleSpec {
    ScheduleSpec {
        name: "Morning review".into(),
        cwd: "/tmp/isolated-project".into(),
        prompt: "Inspect changes; do not edit files.".into(),
        provider: "fixture".into(),
        model_id: "offline".into(),
        cron: "0 9 * * 1-5".into(),
        timezone: "Europe/Stockholm".into(),
        enabled: true,
        missed_policy: MissedPolicy::Skip,
        timeout_minutes: 30,
    }
}

#[test]
fn every_host_request_round_trips_without_shell_encoding() {
    let requests = [
        Request::List,
        Request::Preview {
            cron: "0 9 * * 1-5".into(),
            timezone: "UTC".into(),
            count: 3,
        },
        Request::Create {
            spec: spec(),
            consent: true,
        },
        Request::Update {
            job_id: "job_1".into(),
            expected_revision: 4,
            spec: spec(),
            consent: true,
        },
        Request::Delete {
            job_id: "job_1".into(),
            expected_revision: 4,
        },
        Request::RunNow {
            job_id: "job_1".into(),
            expected_revision: 4,
            request_id: "manual_stable_key".into(),
            consent: true,
        },
        Request::History {
            job_id: "job_1".into(),
        },
    ];
    for request in requests {
        let bytes = serde_json::to_vec(&request).unwrap();
        assert_eq!(serde_json::from_slice::<Request>(&bytes).unwrap(), request);
    }
    let mut with_quotes = spec();
    with_quotes.cwd = "/tmp/it's a folder; $(echo not-executed)".into();
    with_quotes.prompt = "Inspect \"quoted\" text\nUnicode: π, 日本語; <script> is text.".into();
    let request = Request::Create {
        spec: with_quotes,
        consent: true,
    };
    assert_eq!(
        serde_json::from_str::<Request>(&serde_json::to_string(&request).unwrap()).unwrap(),
        request
    );
}

#[test]
fn consent_and_optimistic_revision_are_not_optional_wire_defaults() {
    assert!(
        serde_json::from_value::<Request>(serde_json::json!({"type":"create", "spec":spec()}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<Request>(serde_json::json!({"type":"delete", "job_id":"job_1"}))
            .is_err()
    );
    assert!(serde_json::from_value::<Request>(serde_json::json!({"type":"run_now", "job_id":"job_1", "expected_revision":1,"consent":true})).is_err());
    let request: Request = serde_json::from_value(
        serde_json::json!({"type":"create", "spec":spec(), "consent":false}),
    )
    .unwrap();
    assert!(matches!(request, Request::Create { consent: false, .. }));
}

#[test]
fn missed_runs_default_to_skip_and_unknown_policies_fail_closed() {
    let mut value = serde_json::to_value(spec()).unwrap();
    value.as_object_mut().unwrap().remove("missed_policy");
    assert_eq!(
        serde_json::from_value::<ScheduleSpec>(value.clone())
            .unwrap()
            .missed_policy,
        MissedPolicy::Skip
    );
    value["missed_policy"] = serde_json::json!("replay_everything");
    assert!(serde_json::from_value::<ScheduleSpec>(value).is_err());
    assert!(serde_json::from_str::<Request>(r#"{"type":"install_service"}"#).is_err());
}

#[test]
fn admission_is_distinct_from_running_and_completion() {
    let mut occurrence = Occurrence {
        id: "occurrence_1".into(),
        job_id: "job_1".into(),
        job_revision: 4,
        scheduled_ms: 1_792_000_000_000,
        session_key: Some("sched_session_1".into()),
        request_id: "stable_occurrence_1".into(),
        spec: spec(),
        status: RunStatus::Accepted,
        error: None,
        started_ms: None,
        finished_ms: None,
    };
    let key = occurrence.request_id.clone();
    let encoded = serde_json::to_value(&occurrence).unwrap();
    assert_eq!(encoded["status"], "accepted");
    for status in [
        RunStatus::Pending,
        RunStatus::Accepted,
        RunStatus::Running,
        RunStatus::Completed,
        RunStatus::Failed,
        RunStatus::Interrupted,
        RunStatus::Skipped,
        RunStatus::Missed,
        RunStatus::Unknown,
    ] {
        occurrence.status = status;
        assert_eq!(
            serde_json::from_value::<Occurrence>(serde_json::to_value(&occurrence).unwrap())
                .unwrap(),
            occurrence
        );
        assert_eq!(occurrence.request_id, key);
    }
}

#[test]
fn versioned_responses_preserve_job_snapshot_and_health() {
    let job = Job {
        id: "job_1".into(),
        revision: 4,
        spec: spec(),
        next_due_ms: Some(1_792_000_000_000),
        created_ms: 1,
        updated_ms: 2,
    };
    let health = Health {
        running: false,
        autostart_installed: false,
        last_tick_ms: None,
        detail: "Set up the host scheduler explicitly.".into(),
    };
    let response = Response {
        version: VERSION,
        success: true,
        data: Some(Data::List {
            jobs: vec![job],
            health,
        }),
        error: None,
    };
    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["data"]["type"], "list");
    assert_eq!(value["data"]["health"]["running"], false);
    assert_eq!(serde_json::from_value::<Response>(value).unwrap(), response);
    let failure = Response {
        version: VERSION,
        success: false,
        data: None,
        error: Some("Revision conflict".into()),
    };
    assert_eq!(
        serde_json::from_str::<Response>(&serde_json::to_string(&failure).unwrap()).unwrap(),
        failure
    );
}
