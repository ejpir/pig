use pi_core::{remote_files::Client, ssh::SshTarget, transport::Launch};
use std::{path::Path, time::Duration};
fn client(root: &Path) -> Client {
    let target = SshTarget::new("test".into(), root.to_str().unwrap().into()).unwrap();
    Client::from_launch(
        &target,
        Launch {
            program: env!("CARGO_BIN_EXE_pi-desktop-remote").into(),
            args: vec!["files".into(), "--stdio".into()],
            cwd: root.into(),
            env: vec![],
            request_timeout: Duration::from_secs(5),
            extension: None,
        },
    )
    .unwrap()
}
#[test]
fn independent_file_channel_survives_reconnect_without_starting_pi() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    let path = root.path().join("src/quotes ' héllo.rs");
    std::fs::write(&path, "fn main() {}\r\n").unwrap();
    let first = client(root.path());
    assert!(
        first
            .list()
            .unwrap()
            .entries
            .iter()
            .any(|e| e.path == "src/quotes ' héllo.rs")
    );
    let doc = first.read("src/quotes ' héllo.rs".into()).unwrap();
    let second = client(root.path());
    let saved = second
        .save(
            doc.path.clone(),
            "fn main() { println!(\"hello\"); }\r\n".into(),
            doc.revision.clone(),
        )
        .unwrap();
    assert!(first.save(doc.path, "stale".into(), doc.revision).is_err());
    drop(first);
    drop(second);
    let third = client(root.path());
    assert_eq!(third.read(saved.path).unwrap().text, saved.text);
    assert!(third.read("../not-my-project".into()).is_err());
    assert!(
        !root.path().join(".pi").exists(),
        "file service never starts an agent or creates its session storage"
    );
}
