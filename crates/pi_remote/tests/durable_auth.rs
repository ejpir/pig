//! Standalone production metadata and compiled auth derivation with synthetic credentials; no inference.
#![cfg(unix)]
use pi_core::{
    protocol::read_record,
    ssh::{RemoteBackend, SshTarget},
};
use serde_json::{Value, json};
use std::{
    io::{BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Bridge {
    process: Child,
    input: Option<ChildStdin>,
    records: mpsc::Receiver<Value>,
    root: PathBuf,
    key: String,
}
impl Bridge {
    fn send(&mut self, value: Value) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
    }
    fn until(&self, condition: impl Fn(&Value) -> bool) -> Value {
        loop {
            let record = self
                .records
                .recv_timeout(Duration::from_secs(20))
                .expect("Production bridge closed or timed out");
            let wire = record.to_string();
            assert!(!wire.contains("FAKE-ACCESS-NEVER-SEND"));
            assert!(!wire.contains("FAKE-REFRESH-NEVER-SEND"));
            if condition(&record) {
                return record;
            }
        }
    }
    fn response(&self, id: &str) -> Value {
        let response = self.until(|record| record["id"] == id);
        assert_eq!(response["success"], true, "{response}");
        response
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.process.kill();
        let _ = self.process.wait();
        if let Ok(bytes) = std::fs::read(self.root.join("1").join(format!("{}.json", self.key)))
            && let Ok(endpoint) = serde_json::from_slice::<Value>(&bytes)
            && let Some(pid) = endpoint["pid"].as_i64()
        {
            // SAFETY: the daemon PID comes from this test's private endpoint, never a user session.
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
    }
}

#[test]
#[ignore = "Compile test/standalone-auth.ts and set PI_DESKTOP_TEST_DURABLE_AUTH_PROBE"]
fn standalone_oauth_derivation_needs_no_package_files_runtime_or_network() {
    let probe = std::env::var_os("PI_DESKTOP_TEST_DURABLE_AUTH_PROBE")
        .expect("Compile backend/durable/test/standalone-auth.ts");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        * 1000
        + 3_600_000;
    let providers = [
        "openai-codex",
        "openai",
        "anthropic",
        "kimi-coding",
        "github-copilot",
    ];
    let credential = json!({"type":"oauth","access":"FAKE-ACCESS-NEVER-SEND","refresh":"FAKE-REFRESH-NEVER-SEND","expires":expires});
    let credentials: serde_json::Map<String, Value> = providers
        .iter()
        .map(|provider| ((*provider).to_string(), credential.clone()))
        .collect();
    let before = serde_json::to_vec(&credentials).unwrap();
    std::fs::write(&path, &before).unwrap();
    for provider in providers {
        let output = Command::new(&probe)
            .arg(&path)
            .arg(provider)
            .current_dir(directory.path())
            .env_clear()
            .env("HOME", directory.path())
            .env("PATH", "/no-node-or-bun")
            .env("PI_CODING_AGENT_DIR", directory.path())
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{provider}: {stderr}");
        assert!(!stdout.contains("FAKE-ACCESS-NEVER-SEND"));
        assert!(!stderr.contains("FAKE-ACCESS-NEVER-SEND"));
        assert!(!stdout.contains("FAKE-REFRESH-NEVER-SEND"));
        assert!(!stderr.contains("FAKE-REFRESH-NEVER-SEND"));
        assert_eq!(
            serde_json::from_str::<Value>(&stdout).unwrap(),
            json!({"provider":provider,"derived":true,"networkRequests":0})
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

#[test]
#[ignore = "Build the PRODUCTION durable runner and set PI_DESKTOP_TEST_DURABLE_PRODUCTION"]
fn production_runner_reuses_remote_oauth_and_all_builtin_providers_without_inference() {
    let runner = std::env::var_os("PI_DESKTOP_TEST_DURABLE_PRODUCTION")
        .expect("Build backend/durable/src/main.ts, not the faux fixture");
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("state");
    let agent = directory.path().join("remote-agent");
    std::fs::create_dir(&agent).unwrap();
    std::fs::write(agent.join("auth.json"), "{}").unwrap();
    let mut target = SshTarget::new(
        "test".into(),
        directory.path().to_string_lossy().into_owned(),
    )
    .unwrap();
    target.backend = RemoteBackend::Durable;
    let mut process = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"))
        .args(["connect", "--stdio"])
        // No real credentials, Node/Bun executable, API keys, or cloud profiles from the test host.
        .env_clear()
        .env("HOME", directory.path())
        .env("PATH", "/no-node-or-bun")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_DESKTOP_REMOTE_STATE_DIR", &root)
        .env("PI_DESKTOP_DURABLE_RUNNER", runner)
        .env("PI_DESKTOP_PI", directory.path().join("DO-NOT-START-STOCK"))
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
    let mut bridge = Bridge {
        process,
        input,
        records,
        root,
        key: target.key.clone(),
    };
    bridge.send(target.attach_record());
    bridge.until(|record| record["type"] == "remote_snapshot");
    // Emulate a new login on the host after the durable process has already started.
    let credential = json!({"kimi-coding":{"type":"oauth","access":"FAKE-ACCESS-NEVER-SEND","refresh":"FAKE-REFRESH-NEVER-SEND","expires":1}});
    std::fs::write(agent.join("auth.json"), credential.to_string()).unwrap();
    bridge.send(json!({"type":"get_available_models","id":"models"}));
    let models = bridge.response("models")["data"]["models"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        models
            .iter()
            .any(|model| model["provider"] == "kimi-coding")
    );
    assert!(
        models
            .iter()
            .all(|model| model.get("headers").is_none() && model.get("baseUrl").is_none())
    );
    bridge.send(json!({"type":"get_auth_providers","id":"auth"}));
    let response = bridge.response("auth");
    let providers = response["data"]["providers"].as_array().unwrap();
    assert!(providers.len() > 30);
    for id in [
        "openai-codex",
        "github-copilot",
        "openrouter",
        "amazon-bedrock",
    ] {
        assert!(providers.iter().any(|provider| provider["id"] == id));
    }
    let kimi = providers
        .iter()
        .find(|provider| provider["id"] == "kimi-coding")
        .unwrap();
    assert_eq!(kimi["status"]["type"], "oauth");
    // Even an expired synthetic token must not be refreshed merely to populate the GUI.
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(agent.join("auth.json")).unwrap()).unwrap(),
        credential
    );
    bridge.send(json!({"type":"remote_shutdown","id":"shutdown"}));
    bridge.response("shutdown");
    bridge.input.take();
    assert!(bridge.process.wait().unwrap().success());
}
