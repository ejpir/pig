//! Optional live OpenSSH bootstrap test. No prompt or model request is sent.
use pi_core::protocol::StreamingBehavior;
use pi_core::session::Session;
use pi_core::{
    protocol::Command,
    ssh::{SshTarget, install},
    transport::{RpcClient, TransportEvent},
};
use serde_json::{Value, json};
use std::{
    thread,
    time::{Duration, Instant},
};

fn next(client: &RpcClient) -> Value {
    let events = client.events();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        match events.try_recv() {
            Ok(TransportEvent::Record(record)) => return record,
            Ok(event) => panic!("SSH failed: {event:?}"),
            Err(_) => {
                assert!(Instant::now() < deadline, "SSH event deadline");
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
fn target() -> SshTarget {
    SshTarget::new(
        std::env::var("PI_DESKTOP_TEST_SSH_HOST").unwrap(),
        std::env::var("PI_DESKTOP_TEST_SSH_PROJECT").unwrap(),
    )
    .unwrap()
}
#[test]
#[ignore = "read-only live file smoke test; set SSH fixture variables and PI_DESKTOP_TEST_SSH_FILE"]
fn remote_files_use_an_independent_verified_ssh_channel() {
    let target = target();
    let client = pi_core::remote_files::Client::connect(&target).unwrap();
    let file = std::env::var("PI_DESKTOP_TEST_SSH_FILE").unwrap();
    assert!(
        client
            .list()
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.path == file)
    );
    let doc = client.read(file.clone()).unwrap();
    drop(client);
    let again = pi_core::remote_files::Client::connect(&target).unwrap();
    assert_eq!(again.read(file).unwrap().revision, doc.revision);
}

fn connect(target: &SshTarget) -> RpcClient {
    let client = RpcClient::spawn_forwarded(install(target).unwrap()).unwrap();
    client.send_record(target.attach_record()).unwrap();
    client
}
#[test]
#[ignore = "loopback fake backend only; set PI_DESKTOP_TEST_SSH_FAKE=1 and SSH fixture variables"]
fn streaming_fake_run_survives_an_actual_ssh_disconnect() {
    assert_eq!(
        std::env::var("PI_DESKTOP_TEST_SSH_FAKE").as_deref(),
        Ok("1")
    );
    let target = target();
    let client = connect(&target);
    let initial = next(&client);
    assert_eq!(
        initial["data"]["state"]["sessionId"], "fake",
        "This test must never target a real Pi backend"
    );
    client
        .send(Command::Prompt {
            message: "run".into(),
            images: vec![],
            streaming_behavior: Some(StreamingBehavior::FollowUp),
        })
        .unwrap();
    while next(&client)["type"] != "message_update" {}
    drop(client);
    let again = connect(&target);
    let mut model = Session::new(target.identity());
    model.apply(&next(&again)).unwrap();
    assert!(
        model.busy(),
        "the SSH disconnect did not stop or restart the agent"
    );
    assert!(model.streaming_message_index().is_some());
    while model.busy() {
        model.apply(&next(&again)).unwrap();
    }
    assert_eq!(model.messages.len(), 2);
    assert_eq!(
        model.messages[1]["content"][0]["text"],
        "hello remote world"
    );
    again
        .send_record(json!({"type":"remote_shutdown"}))
        .unwrap();
    while next(&again)["command"] != "remote_shutdown" {}
}

#[test]
#[ignore = "set PI_DESKTOP_TEST_SSH_HOST, PI_DESKTOP_TEST_SSH_PROJECT and PI_DESKTOP_REMOTE_HELPER"]
fn installs_over_openssh_and_routes_metadata_without_model_calls() {
    let target = SshTarget::new(
        std::env::var("PI_DESKTOP_TEST_SSH_HOST").unwrap(),
        std::env::var("PI_DESKTOP_TEST_SSH_PROJECT").unwrap(),
    )
    .unwrap();
    let client = RpcClient::spawn_forwarded(install(&target).unwrap()).unwrap();
    client.send_record(target.attach_record()).unwrap();
    assert_eq!(next(&client)["type"], "remote_snapshot");
    let id = client.send(Command::GetState).unwrap();
    loop {
        let record = next(&client);
        if record["type"] == "response" && record["id"] == id {
            assert_eq!(record["success"], true);
            break;
        }
    }
    drop(client); // Only the SSH connection is stopped.
    let again = RpcClient::spawn_forwarded(install(&target).unwrap()).unwrap();
    again.send_record(target.attach_record()).unwrap();
    assert_eq!(next(&again)["key"], target.key);
    again
        .send_record(json!({"type":"remote_shutdown"}))
        .unwrap();
    loop {
        let record = next(&again);
        if record["type"] == "response" && record["command"] == "remote_shutdown" {
            assert_eq!(record["success"], true);
            break;
        }
    }
}
