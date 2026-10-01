use pi_core::{
    protocol::Command,
    transport::{Launch, RpcClient, TransportEvent},
};
use serde_json::json;
use std::{
    ffi::OsString,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn client(mode: &str, cwd: &std::path::Path) -> RpcClient {
    client_with(mode, cwd, None)
}

fn client_with(mode: &str, cwd: &std::path::Path, extension: Option<PathBuf>) -> RpcClient {
    RpcClient::spawn(Launch {
        program: if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        },
        args: vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fake_rpc.py")
                .into_os_string(),
            mode.into(),
        ],
        cwd: cwd.into(),
        env: vec![],
        request_timeout: Duration::from_millis(if mode == "timeout" { 80 } else { 3000 }),
        extension,
    })
    .unwrap()
}

fn next(events: &async_channel::Receiver<TransportEvent>) -> TransportEvent {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if let Ok(event) = events.try_recv() {
            return event;
        }
        assert!(Instant::now() < deadline, "RPC event deadline elapsed");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn extension_commands_take_the_channel_and_join_the_same_events() {
    let directory = tempfile::tempdir().unwrap();
    let client = client_with(
        "extension",
        directory.path(),
        Some(directory.path().join("extension")),
    );
    let events = client.events();
    // Sent before the extension connects: it waits for the extension's hello.
    let sessions = client
        .send(Command::ListSessions {
            scope: "all".into(),
        })
        .unwrap();
    let state = client.send(Command::GetState).unwrap();
    let mut answered = std::collections::HashMap::new();
    while answered.len() < 2 {
        if let TransportEvent::Record(record) = next(&events) {
            answered.insert(record["id"].as_str().unwrap().to_owned(), record);
        }
    }
    assert_eq!(answered[&sessions]["command"], "list_sessions");
    assert_eq!(answered[&sessions]["data"]["via"], "extension");
    assert_eq!(answered[&state]["command"], "get_state");
    assert_eq!(
        answered[&state]["data"]["cwd"],
        directory.path().to_str().unwrap()
    );
    assert!(
        directory.path().join("extension/pi-desktop.ts").is_file(),
        "pi loads the installed extension"
    );
}

#[test]
fn extension_commands_fail_at_once_without_the_extension() {
    let directory = tempfile::tempdir().unwrap();
    let client = client("ok", directory.path());
    let error = client.send(Command::GetSettings).unwrap_err().to_string();
    assert!(error.contains("without Pi Desktop's extension"), "{error}");
}

#[test]
fn responses_correlate_by_id_not_order_and_events_are_delivered() {
    let directory = tempfile::tempdir().unwrap();
    let client = client("reverse", directory.path());
    let events = client.events();
    let first = client.send(Command::GetState).unwrap();
    let second = client.send(Command::GetMessages).unwrap();
    assert!(
        matches!(next(&events), TransportEvent::Record(record) if record["type"] == "agent_start")
    );
    assert!(
        matches!(next(&events), TransportEvent::Record(record) if record["id"] == second && record["data"]["cwd"] == directory.path().to_str().unwrap())
    );
    assert!(matches!(next(&events), TransportEvent::Record(record) if record["id"] == first));
}

#[test]
fn exit_fails_pending_requests_and_captures_stderr_separately() {
    let directory = tempfile::tempdir().unwrap();
    let client = client("exit", directory.path());
    let events = client.events();
    let id = client.send(Command::GetState).unwrap();
    assert!(
        matches!(next(&events), TransportEvent::RequestFailed { id: failed, .. } if failed == id)
    );
    assert!(
        matches!(next(&events), TransportEvent::Exited { stderr, .. } if stderr.contains("deliberate fixture failure"))
    );
    assert!(client.send(Command::GetState).is_err());
}

#[test]
fn timeout_is_reported_and_late_response_is_not_applied() {
    let directory = tempfile::tempdir().unwrap();
    let client = client("timeout", directory.path());
    let events = client.events();
    let id = client.send(Command::GetState).unwrap();
    assert!(
        matches!(next(&events), TransportEvent::RequestFailed { id: failed, error, .. } if failed == id && error.contains("timed out"))
    );
    thread::sleep(Duration::from_millis(350));
    assert!(events.try_recv().is_err());
}

#[test]
fn commands_that_reply_when_finished_have_no_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let client = client("timeout", directory.path());
    let events = client.events();
    let id = client
        .send(Command::Compact {
            custom_instructions: Some("keep the test plan".into()),
        })
        .unwrap();
    // The fixture replies after 250 ms, well past the 80 ms request timeout.
    assert!(
        matches!(next(&events), TransportEvent::Record(record) if record["id"] == id && record["command"] == "compact")
    );
}

#[test]
fn mismatched_command_does_not_update_state() {
    let directory = tempfile::tempdir().unwrap();
    let client = client("mismatch", directory.path());
    let events = client.events();
    client.send(Command::GetState).unwrap();
    assert!(
        matches!(next(&events), TransportEvent::RequestFailed { error, .. } if error.contains("mismatch"))
    );
}

#[test]
fn malformed_stdout_is_visible_and_terminates_child() {
    let directory = tempfile::tempdir().unwrap();
    let client = client("malformed", directory.path());
    let events = client.events();
    client.send(Command::GetState).unwrap();
    assert!(matches!(next(&events), TransportEvent::ProtocolError(_)));
    assert!(matches!(
        next(&events),
        TransportEvent::RequestFailed { .. }
    ));
    assert!(matches!(next(&events), TransportEvent::Exited { .. }));
}

#[test]
#[cfg(unix)]
fn dropping_client_eventually_kills_and_reaps_an_uncooperative_child() {
    let directory = tempfile::tempdir().unwrap();
    let client = client("hang", directory.path());
    let pid = client.pid();
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if unsafe { libc::kill(pid as i32, 0) } == -1 {
            break;
        }
        assert!(Instant::now() < deadline, "child {pid} was not reaped");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn startup_errors_are_actionable() {
    let launch = Launch {
        program: "/nonexistent/pi-desktop-test".into(),
        args: vec![],
        cwd: std::env::temp_dir(),
        env: vec![],
        request_timeout: Duration::from_secs(1),
        extension: None,
    };
    assert!(
        RpcClient::spawn(launch)
            .err()
            .unwrap()
            .to_string()
            .contains("Could not start")
    );
}

#[test]
#[ignore = "set PI_DESKTOP_TEST_PI to pi's release binary (see docs/validation.md)"]
fn local_pi_metadata_handshake_without_model_calls() {
    let pi = std::env::var_os("PI_DESKTOP_TEST_PI").expect("PI_DESKTOP_TEST_PI required");
    let directory = tempfile::tempdir().unwrap();
    let session_file = directory.path().join("resume.jsonl");
    std::fs::write(&session_file, format!("{}\n{}\n",
        json!({"type":"session","version":3,"id":"resume-fixture","timestamp":"2026-09-28T09:41:00Z","cwd":directory.path()}),
        json!({"type":"message","id":"a1b2c3d4","parentId":null,"timestamp":"2026-09-28T09:41:00Z","message":{"role":"user","content":[{"type":"text","text":"hello"}],"timestamp":1790588460000_u64}})
    )).unwrap();
    let mut args = vec!["--session".into(), session_file.clone().into_os_string()];
    args.extend(
        [
            "--mode",
            "rpc",
            "--offline",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-context-files",
        ]
        .into_iter()
        .map(OsString::from),
    );
    let client = RpcClient::spawn(Launch {
        program: pi,
        args,
        cwd: directory.path().into(),
        env: vec![(
            "PI_CODING_AGENT_DIR".into(),
            directory.path().join("config").into_os_string(),
        )],
        request_timeout: Duration::from_secs(30),
        extension: Some(directory.path().join("extension")),
    })
    .unwrap();
    let events = client.events();
    let mut ids = std::collections::HashSet::new();
    let mut model = pi_core::session::Session::new(directory.path().into());
    for command in [
        Command::GetState,
        Command::GetMessages,
        Command::GetSessionStats,
        Command::GetAvailableModels,
        Command::GetAvailableThinkingLevels,
        Command::GetCommands,
        Command::ListSessions {
            scope: "all".into(),
        },
        Command::GetActiveTools,
        Command::GetSettings,
        Command::GetBackendInfo,
    ] {
        ids.insert(client.send(command).unwrap());
    }
    while !ids.is_empty() {
        match next(&events) {
            TransportEvent::Record(record) if record["type"] == "response" => {
                assert_eq!(record["success"], true, "{record}");
                model.apply(&record).unwrap();
                ids.remove(record["id"].as_str().unwrap());
            }
            TransportEvent::Record(_) => {}
            event => panic!("Unexpected metadata handshake event: {event:?}"),
        }
    }
    assert_eq!(model.state.session_id.as_deref(), Some("resume-fixture"));
    assert_eq!(model.state.session_file.as_deref(), session_file.to_str());
    assert_eq!(model.title(), "hello");
    assert!(
        model
            .state
            .active_tools
            .as_ref()
            .is_some_and(|tools| !tools.is_empty())
    );
    assert!(model.settings.is_some());
    assert!(
        matches!(&model.backend, pi_core::session::BackendInfo::Found(info) if info["piVersion"] == pi_core::extension::PI_VERSION),
        "{:?}",
        model.backend
    );
    assert!(
        model
            .commands
            .iter()
            .all(|command| command.name != "pi-desktop"),
        "the extension's own command is not the user's"
    );
}
