use pi_core::{protocol::read_record, session::Session, ssh::SshTarget};
use serde_json::{Value, json};
use std::{
    io::{BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread,
    time::Duration,
};

struct Bridge {
    process: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}
impl Bridge {
    fn new(root: &std::path::Path, target: &SshTarget) -> Self {
        let mut process = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"))
            .args(["connect", "--stdio"])
            .env("PI_DESKTOP_REMOTE_STATE_DIR", root)
            .env(
                "PI_DESKTOP_RPC_ENTRY",
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_rpc.py"),
            )
            .env(
                "PI_DESKTOP_NODE",
                if cfg!(windows) { "python" } else { "python3" },
            )
            .env_remove("PI_DESKTOP_PI")
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

#[test]
fn disconnect_preserves_agent_and_reconnect_reconstructs_live_state() {
    let directory = tempfile::tempdir().unwrap();
    let target = SshTarget::new(
        "test".into(),
        directory.path().to_string_lossy().into_owned(),
    )
    .unwrap();
    let mut first = Bridge::new(directory.path(), &target);
    let initial = first.until("remote_snapshot");
    assert_eq!(initial["data"]["state"]["sessionId"], "fake");
    first.send(json!({"type":"prompt", "id":"first", "message":"run"}));
    first.until("message_update");
    first.detach();

    let mut second = Bridge::new(directory.path(), &target);
    let snapshot = second.until("remote_snapshot");
    let mut model = Session::new(target.identity());
    model.apply(&snapshot).unwrap();
    assert!(model.busy(), "detaching did not stop the agent");
    assert!(
        model.streaming_message_index().is_some(),
        "the partial answer is in the snapshot"
    );
    assert_eq!(model.tools.len(), 1);
    assert!(!model.tools[0].finished);
    while model.busy() {
        model.apply(&second.next()).unwrap();
    }
    assert_eq!(
        model.messages.len(),
        2,
        "no duplicate submission or assistant message"
    );
    assert_eq!(
        model.messages[1]["content"][0]["text"],
        "hello remote world"
    );

    // Reattachment to a settled run starts with a complete snapshot, not a replay.
    second.detach();
    let mut third = Bridge::new(directory.path(), &target);
    model.apply(&third.until("remote_snapshot")).unwrap();
    assert!(!model.busy());
    assert_eq!(model.messages.len(), 2);
    third.send(json!({"type":"remote_shutdown"}));
    assert_eq!(third.until("response")["command"], "remote_shutdown");
    third.detach();
}

#[test]
fn reconnect_does_not_deliver_responses_to_an_obsolete_attachment() {
    let directory = tempfile::tempdir().unwrap();
    let target = SshTarget::new(
        "test".into(),
        directory.path().to_string_lossy().into_owned(),
    )
    .unwrap();
    let mut first = Bridge::new(directory.path(), &target);
    first.until("remote_snapshot");
    let mut second = Bridge::new(directory.path(), &target);
    second.until("remote_snapshot");
    second.send(json!({"type":"get_state", "id":"current"}));
    assert_eq!(second.until("response")["id"], "current");
    first.detach();
    // Closing the old bridge does not detach its replacement.
    thread::sleep(Duration::from_millis(30));
    second.send(json!({"type":"get_messages", "id":"messages"}));
    second.until("remote_snapshot");
    assert_eq!(second.until("response")["id"], "messages");
    second.send(json!({"type":"remote_shutdown"}));
    second.until("response");
    second.detach();
}
