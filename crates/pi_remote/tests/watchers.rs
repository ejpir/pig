//! A desktop and a phone watching one session: both see the run, each gets its
//! own answers, and `sessions` lists it without attaching.
use pi_core::{protocol::read_record, session::Session, ssh::SshTarget};
use serde_json::{Value, json};
use std::{
    io::{BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

fn helper(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"));
    command
        .env("PI_DESKTOP_REMOTE_STATE_DIR", root)
        .env(
            "PI_DESKTOP_RPC_ENTRY",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_rpc.py"),
        )
        .env(
            "PI_DESKTOP_NODE",
            if cfg!(windows) { "python" } else { "python3" },
        )
        .env_remove("PI_DESKTOP_PI");
    command
}

struct Bridge {
    process: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}
impl Bridge {
    fn new(root: &Path, target: &SshTarget) -> Self {
        let mut process = helper(root)
            .args(["connect", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = process.stdin.take();
        let output = BufReader::new(process.stdout.take().unwrap());
        let mut bridge = Self {
            process,
            input,
            output,
        };
        bridge.send(target.attach_record());
        bridge
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input.as_mut().unwrap(), "{value}").unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
    }
    fn next(&mut self) -> Value {
        read_record(&mut self.output)
            .unwrap()
            .expect("bridge ended unexpectedly")
    }
    fn until(&mut self, kind: &str) -> Value {
        loop {
            let record = self.next();
            if record["type"] == kind {
                return record;
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

fn listed(root: &Path) -> Value {
    let output = helper(root).arg("sessions").output().unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn two_apps_watch_one_session() {
    let directory = tempfile::tempdir().unwrap();
    let target = SshTarget::new(
        "test".into(),
        directory.path().to_string_lossy().into_owned(),
    )
    .unwrap();
    let mut desktop = Bridge::new(directory.path(), &target);
    desktop.until("remote_snapshot");
    let mut phone = Bridge::new(directory.path(), &target);
    let mut model = Session::new(target.identity());
    model.apply(&phone.until("remote_snapshot")).unwrap();

    phone.send(json!({"type":"prompt", "id":"from-phone", "message":"run"}));
    // The desktop stays attached and follows the phone's run.
    let started = desktop.until("message_end");
    assert_eq!(started["message"]["role"], "user");
    loop {
        let record = phone.next();
        if record["type"] == "response" {
            assert_eq!(record["id"], "from-phone", "the phone gets its own answer");
        }
        model.apply(&record).unwrap();
        if record["type"] == "agent_settled" {
            break;
        }
    }
    assert_eq!(
        model.messages[1]["content"][0]["text"],
        "hello remote world"
    );
    desktop.until("agent_settled");

    // An answer goes only to the app that asked.
    desktop.send(json!({"type":"get_state", "id":"desktop-state"}));
    assert_eq!(desktop.until("response")["id"], "desktop-state");
    phone.send(json!({"type":"get_state", "id":"phone-state"}));
    assert_eq!(phone.until("response")["id"], "phone-state");

    let sessions = listed(directory.path());
    let session = &sessions["sessions"][0];
    assert_eq!(session["key"], target.key.as_str());
    assert_eq!(session["title"], "run");
    assert_eq!(session["busy"], false);
    assert_eq!(session["running"], true);

    phone.detach();
    desktop.send(json!({"type":"remote_shutdown", "id":"done"}));
    assert_eq!(desktop.until("response")["command"], "remote_shutdown");
    desktop.detach();
}
