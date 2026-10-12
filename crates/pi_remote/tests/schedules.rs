//! Real CLI/gateway processes, with isolated HOME/state and no runner or credentials.
use pi_core::schedules::{Data, MissedPolicy, Request, Response, ScheduleSpec};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
fn helper(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"));
    command
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("PI_DESKTOP_REMOTE_STATE_DIR", root.join("state"))
        .env(
            "PI_DESKTOP_DURABLE_RUNNER",
            root.join("NEVER-START-A-PROVIDER"),
        )
        .env("PI_DESKTOP_PI", root.join("NEVER-START-STOCK-PI"))
        .env_remove("PI_DESKTOP_RPC_ENTRY");
    command
}
fn request(root: &Path, request: &Request) -> Response {
    let mut child = helper(root)
        .args(["schedules", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}",
        serde_json::to_string(request).unwrap()
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.iter().filter(|b| **b == b'\n').count(), 1);
    let response: Response = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response.version, 1);
    response
}
#[test]
fn cli_is_one_shot_and_gateway_never_exposes_local_service_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let response = request(temp.path(), &Request::List);
    assert!(response.success);
    match response.data.unwrap() {
        Data::List { jobs, health } => {
            assert!(jobs.is_empty());
            assert!(!health.running);
            assert!(!health.autostart_installed);
        }
        _ => panic!(),
    }
    assert!(!temp.path().join("state/1").exists());
    assert!(!temp.path().join("state/durable").exists());
    let mut child = helper(temp.path())
        .args(["gateway", "test-device"])
        .env(
            "SSH_ORIGINAL_COMMAND",
            "pi-desktop-remote schedules --stdio",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(),"{{\"type\":\"preview\",\"cron\":\"0 9 * * 1-5\",\"timezone\":\"Europe/Berlin\",\"count\":3}}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let response: Response = serde_json::from_slice(&output.stdout).unwrap();
    assert!(response.success);
    for args in [
        "scheduler start",
        "scheduler stop",
        "scheduler status",
        "scheduler install --confirm",
        "schedules --stdio extra",
        "schedules",
    ] {
        let output = helper(temp.path())
            .args(["gateway", "test-device"])
            .env("SSH_ORIGINAL_COMMAND", format!("pi-desktop-remote {args}"))
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args}");
        assert!(output.stdout.is_empty());
    }
    let output = helper(temp.path())
        .args(["schedules", "--stdio", "extra"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cli_persists_paused_jobs_without_starting_any_worker_or_service() {
    let temp = tempfile::tempdir().unwrap();
    let spec = ScheduleSpec {
        name: "Offline test".into(),
        cwd: temp.path().to_str().unwrap().into(),
        prompt: "Never sent".into(),
        provider: "fake".into(),
        model_id: "offline".into(),
        cron: "0 9 * * *".into(),
        timezone: "UTC".into(),
        enabled: false,
        missed_policy: MissedPolicy::Skip,
        timeout_minutes: 5,
    };
    let response = request(
        temp.path(),
        &Request::Create {
            spec,
            consent: false,
        },
    );
    assert!(response.success, "{:?}", response.error);
    let job = match response.data.unwrap() {
        Data::Job { job } => job,
        _ => panic!(),
    };
    assert!(!job.spec.enabled);
    let response = request(
        temp.path(),
        &Request::RunNow {
            job_id: job.id.clone(),
            expected_revision: job.revision,
            request_id: "confirmed-click".into(),
            consent: false,
        },
    );
    assert!(!response.success);
    let response = request(
        temp.path(),
        &Request::RunNow {
            job_id: job.id.clone(),
            expected_revision: job.revision,
            request_id: "confirmed-click".into(),
            consent: true,
        },
    );
    assert!(response.success);
    let occurrence = match response.data.unwrap() {
        Data::Run { occurrence } => occurrence,
        _ => panic!(),
    };
    assert_eq!(occurrence.status, pi_core::schedules::RunStatus::Pending);
    let response = request(temp.path(), &Request::History { job_id: job.id });
    match response.data.unwrap() {
        Data::History { occurrences } => assert_eq!(occurrences, vec![occurrence]),
        _ => panic!(),
    }
    assert!(!temp.path().join("state/1").exists());
    assert!(!temp.path().join("state/durable").exists());
    assert!(
        !temp
            .path()
            .join("config/systemd/user/pi-desktop-scheduler.service")
            .exists()
    );
}

#[cfg(unix)]
struct FixtureSandbox {
    directory: tempfile::TempDir,
    scheduler: Option<std::process::Child>,
    session_key: Option<String>,
}
#[cfg(unix)]
impl Drop for FixtureSandbox {
    fn drop(&mut self) {
        use std::{
            io::BufReader,
            net::TcpStream,
            time::{Duration, Instant},
        };
        let root = self.directory.path().join("state");
        let _ = std::fs::write(root.join("schedules/stop.request"), b"");
        if let Some(child) = self.scheduler.as_mut() {
            let deadline = Instant::now() + Duration::from_secs(5);
            while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            // Only the exact child started by this test, never a process search.
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        if let Some(key) = &self.session_key {
            let cleanup = (|| -> anyhow::Result<()> {
                let path = root.join(format!("1/{key}.json"));
                if !path.exists() {
                    return Ok(());
                } // normally already retired
                let endpoint: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
                let stream = TcpStream::connect((
                    "127.0.0.1",
                    u16::try_from(endpoint["port"].as_u64().unwrap())?,
                ))?;
                stream.set_read_timeout(Some(Duration::from_secs(10)))?;
                let mut stream = BufReader::new(stream);
                writeln!(
                    stream.get_mut(),
                    "{}",
                    serde_json::json!({"type":"hello","token":endpoint["token"],"version":endpoint["version"],"cwd":endpoint["cwd"],"backend":"durable"})
                )?;
                let _ = pi_core::protocol::read_record(&mut stream)?;
                writeln!(stream.get_mut(), "{{\"type\":\"ready\"}}")?;
                for (command, id) in [
                    ("abort", "cleanup-stop"),
                    ("remote_shutdown", "cleanup-shutdown"),
                ] {
                    writeln!(
                        stream.get_mut(),
                        "{}",
                        serde_json::json!({"type":command,"id":id})
                    )?;
                    loop {
                        let value = pi_core::protocol::read_record(&mut stream)?
                            .ok_or_else(|| anyhow::anyhow!("Cleanup attachment closed"))?;
                        if value["type"] == "response" && value["id"] == id {
                            anyhow::ensure!(value["success"] == true, "Cleanup failed: {value}");
                            break;
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = cleanup {
                eprintln!("Isolated fixture cleanup: {error:#}");
            }
        }
    }
}
#[test]
#[cfg(unix)]
#[ignore = "Requires explicitly supplied faux-only PI_DESKTOP_TEST_DURABLE_RUNNER; never a production runner"]
fn explicit_scheduler_executes_fresh_faux_session_and_settles_committed_history() {
    use std::time::{Duration, Instant};
    let runner = std::env::var_os("PI_DESKTOP_TEST_DURABLE_RUNNER").expect(
        "Set the standalone backend/durable/test/fixture.ts binary, not a production runner",
    );
    let mut sandbox = FixtureSandbox {
        directory: tempfile::tempdir().unwrap(),
        scheduler: None,
        session_key: None,
    };
    let root = sandbox.directory.path();
    let spec = ScheduleSpec {
        name: "Faux-only occurrence".into(),
        cwd: root.to_str().unwrap().into(),
        prompt: "find scheduler".into(),
        provider: "faux".into(),
        model_id: "faux-1".into(),
        cron: "0 9 * * *".into(),
        timezone: "UTC".into(),
        enabled: false,
        missed_policy: MissedPolicy::Skip,
        timeout_minutes: 1,
    };
    let response = request(
        root,
        &Request::Create {
            spec,
            consent: false,
        },
    );
    assert!(response.success, "{:?}", response.error);
    let mut job = match response.data.unwrap() {
        Data::Job { job } => job,
        _ => panic!(),
    };
    let response = request(
        root,
        &Request::RunNow {
            job_id: job.id.clone(),
            expected_revision: job.revision,
            request_id: "faux-confirmation".into(),
            consent: true,
        },
    );
    assert!(response.success);
    let occurrence = match response.data.unwrap() {
        Data::Run { occurrence } => occurrence,
        _ => panic!(),
    };
    sandbox.session_key = occurrence.session_key.clone();
    sandbox.scheduler = Some(
        helper(root)
            .args(["scheduler", "serve"])
            .env("PI_DESKTOP_DURABLE_RUNNER", &runner)
            .env("PI_DESKTOP_DURABLE_PROVIDER", "faux")
            .env("PI_DESKTOP_DURABLE_MODEL", "faux-1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let first = occurrence;
    let mut keys = Vec::new();
    for index in 0..8 {
        if index == 7 {
            let mut spec = job.spec.clone();
            spec.prompt = "delegate".into(); // final run includes real faux background children/report
            let updated = request(
                root,
                &Request::Update {
                    job_id: job.id.clone(),
                    expected_revision: job.revision,
                    spec,
                    consent: true,
                },
            );
            assert!(updated.success, "{:?}", updated.error);
            job = match updated.data.unwrap() {
                Data::Job { job } => job,
                _ => panic!(),
            };
        }
        let occurrence = if index == 0 {
            first.clone()
        } else {
            let response = request(
                root,
                &Request::RunNow {
                    job_id: job.id.clone(),
                    expected_revision: job.revision,
                    request_id: format!("faux-confirmation-{index}"),
                    consent: true,
                },
            );
            assert!(response.success, "{:?}", response.error);
            match response.data.unwrap() {
                Data::Run { occurrence } => occurrence,
                _ => panic!(),
            }
        };
        assert_eq!(occurrence.status, pi_core::schedules::RunStatus::Pending);
        let key = occurrence.session_key.clone().unwrap();
        sandbox.session_key = Some(key.clone());
        keys.push(key.clone());
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let response = request(
                root,
                &Request::History {
                    job_id: job.id.clone(),
                },
            );
            assert!(response.success);
            let settled = match response.data.unwrap() {
                Data::History { occurrences } => occurrences
                    .into_iter()
                    .find(|o| o.id == occurrence.id)
                    .unwrap(),
                _ => panic!(),
            };
            let listed = helper(root).arg("sessions").output().unwrap();
            assert!(listed.status.success());
            let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
            let sessions = listed["sessions"].as_array().unwrap();
            let owners = sessions.iter().filter(|s| s["running"] == true).count();
            assert!(owners <= 1, "completed occurrence owners leaked: {listed}");
            let owner_path = root.join(format!("state/durable/{key}/owner.lock"));
            let owner_idle = std::fs::File::open(&owner_path)
                .ok()
                .is_some_and(|lock| lock.try_lock().is_ok());
            if settled.status == pi_core::schedules::RunStatus::Completed
                && owners == 0
                && owner_idle
            {
                assert!(settled.started_ms.is_some());
                assert!(settled.finished_ms.is_some());
                assert_eq!(
                    sessions.len(),
                    index + 1,
                    "retirement must retain saved sessions"
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Faux scheduler did not settle/retire: {settled:?}; {listed}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let history = request(
        root,
        &Request::History {
            job_id: job.id.clone(),
        },
    );
    assert!(matches!(history.data, Some(Data::History { occurrences }) if occurrences.len() == 8));
    for key in &keys {
        assert!(
            root.join(format!("state/durable/{key}/identity.json"))
                .is_file()
        );
        assert!(root.join(format!("state/1/{key}.summary.json")).is_file());
    }
    // Reopen the first retired session through normal attach, then look up its
    // original receipt and committed messages. No replacement prompt is sent.
    use pi_core::ssh::{RemoteBackend, SshTarget};
    use std::{io::BufReader, sync::mpsc};
    let target = SshTarget {
        host: "localhost".into(),
        // Validation canonicalizes the project's identity (/var vs /private/var on macOS).
        cwd: first.spec.cwd.clone(),
        key: keys[0].clone(),
        backend: RemoteBackend::Durable,
        session_file: None,
    };
    sandbox.session_key = Some(target.key.clone());
    let mut bridge = helper(root)
        .args(["connect", "--stdio"])
        .env("PI_DESKTOP_DURABLE_RUNNER", &runner)
        .env("PI_DESKTOP_DURABLE_PROVIDER", "faux")
        .env("PI_DESKTOP_DURABLE_MODEL", "faux-1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = bridge.stdin.take().unwrap();
    let stdout = bridge.stdout.take().unwrap();
    let (sender, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(record)) = pi_core::protocol::read_record(&mut reader) {
            if sender.send(record).is_err() {
                break;
            }
        }
    });
    writeln!(input, "{}", target.attach_record()).unwrap();
    let mut committed = false;
    loop {
        let record = receive.recv_timeout(Duration::from_secs(10)).unwrap();
        if record["type"] == "remote_snapshot" {
            committed |= record["data"]["messages"]
                .to_string()
                .contains("found scheduler");
            if committed {
                break;
            }
        }
    }
    writeln!(
        input,
        "{}",
        serde_json::json!({"type":"get_submission","id":"original","requestId":first.request_id})
    )
    .unwrap();
    let original = loop {
        let record = receive.recv_timeout(Duration::from_secs(10)).unwrap();
        if record["id"] == "original" {
            assert_eq!(record["data"]["submission"]["status"], "done");
            assert!(record["data"]["correlated"]["messages"].is_array());
            break record["data"]["submission"].clone();
        }
    };
    {
        let mut serial = 0;
        let mut rpc = |mut record: serde_json::Value| {
            serial += 1;
            let id = format!("review-{serial}");
            record["id"] = serde_json::json!(id);
            writeln!(input, "{record}").unwrap();
            loop {
                let response = receive.recv_timeout(Duration::from_secs(10)).unwrap();
                if response["id"] == id {
                    assert_eq!(response["success"], true, "{response}");
                    break response["data"].clone();
                }
            }
        };
        // A is terminal; B is a separately admitted, gated faux turn. A stale
        // exact cancellation must NOT act like session-global abort on B.
        rpc(serde_json::json!({"type":"prompt","message":"queued hold","requestId":"unrelated-B"}));
        let cancel = rpc(
            serde_json::json!({"type":"cancel_submission","submissionId":original["id"],"requestId":first.request_id}),
        );
        assert_eq!(cancel["outcome"], "settled");
        let b = rpc(serde_json::json!({"type":"get_submission","requestId":"unrelated-B"}));
        assert_eq!(
            b["submission"]["status"], "placed",
            "A cancellation must preserve B"
        );
        assert_eq!(
            rpc(serde_json::json!({"type":"remote_retire"}))["retired"],
            false,
            "active UI work must preserve its owner"
        );
        std::fs::write(root.join("release-queued"), b"").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let b = rpc(serde_json::json!({"type":"get_submission","requestId":"unrelated-B"}));
            if b["submission"]["status"] == "done" {
                break;
            }
            assert!(Instant::now() < deadline, "B was displaced: {b}");
            std::thread::sleep(Duration::from_millis(50));
        }
        let a = rpc(serde_json::json!({"type":"get_submission","requestId":first.request_id}));
        assert_eq!(
            a["submission"], original,
            "B must not change A's original result"
        );
        assert!(
            a["correlated"]["messages"]
                .to_string()
                .contains("found scheduler")
        );
        assert!(
            !a["correlated"]["messages"]
                .to_string()
                .contains("queued hold")
        );
        assert_eq!(
            rpc(serde_json::json!({"type":"remote_retire"}))["retired"],
            true
        );
    }
    drop(input);
    assert!(bridge.wait().unwrap().success());
    assert!(root.join("state/durable").is_dir());
    assert!(
        !root
            .join("config/systemd/user/pi-desktop-scheduler.service")
            .exists()
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn concurrent_cli_mutations_have_one_revision_winner() {
    let temp = tempfile::tempdir().unwrap();
    let spec = ScheduleSpec {
        name: "Concurrent offline test".into(),
        cwd: temp.path().to_str().unwrap().into(),
        prompt: "Original snapshot".into(),
        provider: "fake".into(),
        model_id: "offline".into(),
        cron: "0 9 * * *".into(),
        timezone: "UTC".into(),
        enabled: false,
        missed_policy: MissedPolicy::Skip,
        timeout_minutes: 5,
    };
    let created = request(
        temp.path(),
        &Request::Create {
            spec,
            consent: false,
        },
    );
    let job = match created.data.unwrap() {
        Data::Job { job } => job,
        _ => panic!(),
    };
    let threads: Vec<_> = (0..12)
        .map(|index| {
            let root = temp.path().to_owned();
            let mut spec = job.spec.clone();
            spec.prompt = format!("Reviewed candidate {index}");
            let job_id = job.id.clone();
            std::thread::spawn(move || {
                request(
                    &root,
                    &Request::Update {
                        job_id,
                        expected_revision: 1,
                        spec,
                        consent: true,
                    },
                )
            })
        })
        .collect();
    let responses: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(responses.iter().filter(|r| r.success).count(), 1);
    assert!(
        responses.iter().filter(|r| !r.success).all(|r| r
            .error
            .as_deref()
            .unwrap()
            .contains("revision conflict"))
    );
    match request(temp.path(), &Request::List).data.unwrap() {
        Data::List { jobs, .. } => {
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].revision, 2);
        }
        _ => panic!(),
    }
}
