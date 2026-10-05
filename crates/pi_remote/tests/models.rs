use std::process::Command;

#[test]
#[ignore = "set PI_DESKTOP_TEST_DURABLE_RUNNER to the compiled faux fixture"]
fn model_discovery_needs_no_project_session_or_provider_request() {
    let directory = tempfile::tempdir().unwrap();
    let runner =
        std::env::var_os("PI_DESKTOP_TEST_DURABLE_RUNNER").expect("set the faux fixture runner");
    let output = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"))
        .arg("models")
        .env("PI_DESKTOP_DURABLE_RUNNER", runner)
        .env("PI_DESKTOP_REMOTE_STATE_DIR", directory.path())
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let catalog: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(catalog["version"], 1);
    let models = catalog["models"].as_array().unwrap();
    assert!(!models.is_empty());
    assert!(models.iter().all(|model| model["provider"] == "faux"));
    assert!(
        models
            .iter()
            .all(|model| model.get("headers").is_none() && model.get("apiKey").is_none())
    );
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        0,
        "discovery must not create a session or database"
    );
}
