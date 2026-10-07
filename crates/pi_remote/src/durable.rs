//! Experimental durable backend. Rust owns the writer lock; the bundled runner owns execution state.
use anyhow::{Context, Result, ensure};
use pi_core::{
    session::{BackendInfo, RunState, Session},
    ssh::SshTarget,
    transport::Launch,
};
use serde_json::{Value, json};
#[cfg(any(unix, feature = "bundled-durable"))]
use std::fs;
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::{path::PathBuf, time::Duration};

#[cfg(unix)]
pub(crate) fn storage_dir(target: &SshTarget) -> Result<PathBuf> {
    let root = std::env::var_os("PI_DESKTOP_REMOTE_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".pi/desktop")))
        .context("No remote home directory")?;
    Ok(root.join("durable").join(&target.key))
}

fn program() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PI_DESKTOP_DURABLE_RUNNER") {
        let path = PathBuf::from(path);
        ensure!(
            path.is_file(),
            "PI_DESKTOP_DURABLE_RUNNER must name a standalone executable on the SSH host"
        );
        crate::executable::validate(&path, std::env::consts::OS, std::env::consts::ARCH)?;
        return Ok(path);
    }
    #[cfg(feature = "bundled-durable")]
    {
        use sha2::{Digest, Sha256};
        use std::io::Write;
        let bytes = include_bytes!(env!("PI_DESKTOP_DURABLE_BINARY"));
        let root = dirs::cache_dir()
            .context("No cache directory")?
            .join("pi-desktop-remote/durable")
            .join(env!("PI_DESKTOP_DURABLE_ID"));
        fs::create_dir_all(&root)?;
        let path = root.join("pi-desktop-durable");
        if !path.is_file() {
            let mut staging = tempfile::NamedTempFile::new_in(&root)?;
            staging.write_all(bytes)?;
            staging.as_file().sync_all()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                staging
                    .as_file()
                    .set_permissions(fs::Permissions::from_mode(0o700))?;
            }
            match staging.persist_noclobber(&path) {
                Ok(_) => {}
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        ensure!(
            format!("{:x}", Sha256::digest(fs::read(&path)?)) == env!("PI_DESKTOP_DURABLE_ID"),
            "Cached durable runner checksum mismatch"
        );
        crate::executable::validate(&path, std::env::consts::OS, std::env::consts::ARCH)?;
        Ok(path)
    }
    #[cfg(not(feature = "bundled-durable"))]
    anyhow::bail!(
        "This helper has no bundled durable runner. Build with bundled-durable or set PI_DESKTOP_DURABLE_RUNNER on the SSH host. Stock Pi was not started."
    )
}

pub fn models() -> Result<()> {
    let status = std::process::Command::new(program()?)
        .arg("--list-models")
        .stdin(std::process::Stdio::null())
        .status()
        .context("Could not read the computer's model catalog")?;
    ensure!(status.success(), "Model catalog discovery failed");
    Ok(())
}

pub fn commands() -> Result<()> {
    let status = std::process::Command::new(program()?)
        .arg("--list-commands")
        .stdin(std::process::Stdio::null())
        .status()
        .context("Could not read the computer's command catalog")?;
    ensure!(status.success(), "Command catalog discovery failed");
    Ok(())
}

pub fn launch(target: &SshTarget, cwd: PathBuf) -> Result<Launch> {
    ensure!(
        cfg!(unix),
        "The experimental durable backend currently requires a Unix SSH host"
    );
    // Validate availability before publishing the daemon endpoint, and never fall back to Pi.
    program()?;
    Ok(Launch {
        program: std::env::current_exe()?.into(),
        args: vec![
            "durable-worker".into(),
            serde_json::to_string(target)?.into(),
        ],
        cwd,
        env: vec![(
            "NODE_USE_SYSTEM_CA".into(),
            std::env::var_os("NODE_USE_SYSTEM_CA").unwrap_or_else(|| "1".into()),
        )],
        request_timeout: Duration::from_secs(30),
        extension: None,
    })
}

/// Separate owner so daemon SIGKILL cannot release the storage lock while its runner is still writing.
#[cfg(unix)]
pub fn worker(target: SshTarget) -> Result<()> {
    use std::os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    };
    target.validate()?;
    let directory = storage_dir(&target)?;
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(directory.join("owner.lock"))?;
    // Do not wait indefinitely for an orphan writer. A later explicit reconnect may try again.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(error.into()),
        }
    }
    let cwd = PathBuf::from(&target.cwd).canonicalize()?;
    let identity = directory.join("identity.json");
    let expected = json!({"version":1,"key":target.key,"cwd":cwd});
    if identity.exists() {
        ensure!(
            serde_json::from_slice::<Value>(&fs::read(&identity)?)? == expected,
            "Durable storage identity mismatch; refusing to move or migrate this session"
        );
    } else {
        use std::io::Write;
        let mut staging = tempfile::NamedTempFile::new_in(&directory)?;
        serde_json::to_writer(&mut staging, &expected)?;
        staging.flush()?;
        staging.as_file().sync_all()?;
        staging.persist(&identity)?;
    }
    // No other threads exist in this fresh internal helper. Inherit the same flock open-file description.
    let fd = lock.as_raw_fd();
    // SAFETY: fcntl changes only this valid descriptor's close-on-exec flag, before spawning any threads.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        ensure!(
            flags != -1 && libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) != -1,
            "Could not inherit durable storage lock"
        );
    }
    let status = Command::new(program()?)
        .args([
            "--state".into(),
            directory.into_os_string(),
            "--cwd".into(),
            cwd.into_os_string(),
            "--key".into(),
            target.key.into(),
        ])
        .env("PI_DESKTOP_DURABLE_OWNED", "1")
        .env("PI_DESKTOP_DURABLE_LOCK_FD", fd.to_string())
        // Stay in the RPC owner's process group, so shutdown also stops shell descendants.
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("Could not launch the durable runner")?;
    ensure!(status.success(), "Durable runner exited: {status}");
    Ok(())
}
#[cfg(not(unix))]
pub fn worker(_: SshTarget) -> Result<()> {
    anyhow::bail!("The experimental durable backend currently requires a Unix SSH host")
}

/// Build a disposable desktop projection from ONE committed durable view. Never use it for execution/recovery.
pub fn project(record: &Value, previous: &Session, target: &SshTarget) -> Result<Session> {
    ensure!(
        record["key"] == target.key,
        "Durable snapshot identity mismatch"
    );
    let view = &record["data"];
    let entries = view["entries"]
        .as_array()
        .context("Missing durable transcript")?;
    let docs = &view["docs"];
    ensure!(docs.is_object(), "Missing durable documents");
    let mut model = Session::new(previous.cwd.clone());
    model.available_models = previous.available_models.clone();
    model.models_loaded = previous.models_loaded;
    model.auth_providers = previous.auth_providers.clone();
    model.thinking_levels = previous.thinking_levels.clone();
    model.commands = previous.commands.clone();
    model.commands_loaded = previous.commands_loaded;
    model.settings = previous.settings.clone();
    model.backend = BackendInfo::Found(record["backend"].clone());
    model.state.session_id = Some(target.key.clone());
    model.state.active_tools = previous.state.active_tools.clone();
    // No SQLite path may be mistaken for a stock Pi JSONL file by session management.
    model.state.session_file = None;
    model.state.session_name = docs["app.desktop"]["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    model.state.auto_compaction_enabled = docs["app.desktop"]["autoCompaction"]
        .as_bool()
        .unwrap_or(true);
    model.state.thinking_level = docs["pi.agent"]["thinkingLevel"]
        .as_str()
        .unwrap_or("off")
        .into();
    if let (Some(provider), Some(id)) = (
        docs["pi.agent"]["model"]["provider"].as_str(),
        docs["pi.agent"]["model"]["modelId"].as_str(),
    ) {
        model.state.model = Some(
            previous
                .available_models
                .iter()
                .find(|m| m.provider == provider && m.id == id)
                .cloned()
                .unwrap_or_else(|| pi_core::protocol::Model {
                    provider: provider.into(),
                    id: id.into(),
                    ..Default::default()
                }),
        );
    }
    let messages: Vec<Value> = entries
        .iter()
        .filter_map(|entry| entry["model"].as_array())
        .flatten()
        .filter(|message| message["role"] != "system")
        .cloned()
        .collect();
    model.apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":messages}}))?;
    let live = &docs["pi.live"];
    if live["run"].is_object() {
        model.apply(&json!({"type":"agent_start"}))?;
        model.state.is_streaming = true;
    }
    if live["generation"]["retry"].is_object() {
        model.run = RunState::Retrying;
    }
    if live["compactions"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["blocking"] == true))
    {
        model.run = RunState::Compacting;
        model.state.is_compacting = true;
    }
    if live["generation"]["message"].is_object() {
        model.apply(&json!({"type":"message_start","message":live["generation"]["message"]}))?;
    }
    if let Some(slots) = live["tools"].as_array() {
        for slot in slots {
            if slot["status"] == "done" {
                continue;
            }
            let id = slot["callId"].as_str().context("Invalid durable tool ID")?;
            if !model.tools.iter().any(|tool| tool.id == id) {
                model.apply(&json!({"type":"tool_execution_start","toolCallId":id,"toolName":slot["name"],"args":{}}))?;
            }
            model.apply(&json!({"type":"tool_execution_update","toolCallId":id,"toolName":slot["name"],"partialResult":{"content":[{"type":"text","text":slot["output"].as_str().unwrap_or("")}],"details":slot["details"]}}))?;
        }
    }
    // Subagents carry on after their call returned: how each is doing now.
    if let Some(calls) = docs["app.subagent-calls"]["calls"].as_object() {
        for tool in &mut model.tools {
            if let Some(details) = calls.get(&tool.id).filter(|details| details.is_object()) {
                tool.details = details.clone();
            }
        }
    }
    if let Some(items) = docs["pi.inbox"]["items"].as_array() {
        for item in items {
            let content = pi_core::session::content_text(&item["content"]);
            match item["mode"].as_str() {
                Some("steer") => model.steering.push(content),
                Some("followUp") => model.follow_up.push(content),
                _ => {}
            }
        }
        // IDs follow the same display order, without rebuilding any queued
        // payloads (which may include image data the wire projection omits).
        for mode in ["steer", "followUp"] {
            model.queued_submissions.extend(
                items
                    .iter()
                    .filter(|item| item["mode"] == mode)
                    .filter_map(|item| {
                        item["id"]
                            .as_u64()
                            .map(|id| id.to_string())
                            .or_else(|| item["id"].as_str().map(str::to_owned))
                    }),
            );
        }
    }
    for bucket in ["models", "tools"] {
        if let Some(usages) = docs["pi.usage"][bucket].as_object() {
            for usage in usages.values() {
                model.stats.tokens.input += usage["input"].as_u64().unwrap_or(0);
                model.stats.tokens.output += usage["output"].as_u64().unwrap_or(0);
                model.stats.tokens.cache_read += usage["cacheRead"].as_u64().unwrap_or(0);
                model.stats.tokens.cache_write += usage["cacheWrite"].as_u64().unwrap_or(0);
                *model.stats.cost.get_or_insert(0.) +=
                    usage["cost"]["total"].as_f64().unwrap_or(0.);
            }
        }
    }
    model.stats.total_messages = model.messages.len() as u64;
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn committed_projection_preserves_partial_tools_queue_and_remote_identity() {
        let mut target = SshTarget::new("dev".into(), "/remote/work".into()).unwrap();
        target.backend = pi_core::ssh::RemoteBackend::Durable;
        let previous = Session::new(target.identity());
        let record = json!({"key":target.key,"backend":{"backend":"pi-durable"},"data":{
            "entries":[
                {"model":[{"role":"system","content":"not a transcript row"}]},
                {"model":[{"role":"user","content":"work"}]},
                {"model":[{"role":"assistant","content":[{"type":"toolCall","id":"t","name":"bash","arguments":{"command":"remote"}}]}]}
            ],
            "docs":{
                "app.desktop":{"name":"Checkpointed","autoCompaction":true},
                "pi.agent":{"model":{"provider":"faux","modelId":"faux-1"}},
                "pi.live":{"run":{"inputs":[1]},"generation":{"message":{"role":"assistant","content":[{"type":"text","text":"partial"}]}},"tools":[{"callId":"t","name":"bash","status":"running","output":"remote bytes"}]},
                "pi.inbox":{"items":[{"mode":"steer","content":"direction"},{"mode":"followUp","content":"next task"}]},
                "pi.usage":{"models":{"faux/faux-1":{"input":10,"output":3,"cost":{"total":0.5}}}}
            }
        }});
        let model = project(&record, &previous, &target).unwrap();
        assert_eq!(model.cwd, target.identity());
        assert_eq!(model.state.session_file, None);
        assert_eq!(model.messages.len(), 3);
        assert_eq!(model.streaming_message_index(), Some(2));
        assert!(model.busy());
        assert_eq!(model.tools[0].args["command"], "remote");
        assert_eq!(model.tools[0].output, "remote bytes");
        assert!(!model.tools[0].finished);
        assert_eq!(model.steering, ["direction"]);
        assert_eq!(model.follow_up, ["next task"]);
        assert_eq!(model.stats.tokens.input, 10);
        let next = project(&record, &model, &target).unwrap();
        assert_eq!(
            next.messages.len(),
            model.messages.len(),
            "Snapshots replace, never append"
        );
        assert!(project(&json!({"key":"wrong","data":{}}), &previous, &target).is_err());
        assert!(
            project(
                &json!({"key":target.key,"data":{"entries":[]}}),
                &previous,
                &target
            )
            .is_err()
        );
    }
}
