//! The browser command is available before creating or attaching any session.
use std::{fs, process::Command};

#[test]
fn browse_is_read_only_and_needs_no_session_or_model() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("café's project with spaces");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("Cargo.toml"), "[package]").unwrap();
    fs::create_dir(root.path().join(".hidden")).unwrap();
    let run = |path: &std::path::Path, hidden: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"));
        command.arg("directories").arg("--path").arg(path);
        if hidden {
            command.arg("--show-hidden");
        }
        command.output().unwrap()
    };
    let output = run(root.path(), false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
    assert_eq!(listed["entries"][0]["name"], "café's project with spaces");
    assert_eq!(listed["entries"][0]["project"], true);
    let listed: serde_json::Value = serde_json::from_slice(&run(root.path(), true).stdout).unwrap();
    assert_eq!(listed["entries"].as_array().unwrap().len(), 2);
    assert!(!run(&project.join("missing"), false).status.success());
    assert!(!run(&project.join("Cargo.toml"), false).status.success());
    assert_eq!(
        fs::read_to_string(project.join("Cargo.toml")).unwrap(),
        "[package]"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}
