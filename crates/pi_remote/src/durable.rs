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
    // Current context is measured by the runner's get_session_stats response,
    // not derivable from the committed lifetime usage ledger. Keep that latest
    // measurement when a subsequent durable snapshot rebuilds the projection.
    model.stats.context_usage = previous.stats.context_usage.clone();
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

fn apply_record(model: &mut Session, output: &mut Vec<Value>, record: Value) -> Result<()> {
    model.apply(&record)?;
    output.push(record);
    Ok(())
}

pub(crate) fn state_update(model: &Session) -> Result<Value> {
    Ok(json!({"type":"state_update","state":serde_json::to_value(&model.state)?}))
}

pub(crate) fn stats_update(model: &Session) -> Result<Value> {
    Ok(json!({"type":"stats_update","stats":serde_json::to_value(&model.stats)?}))
}

fn trim_start_utf16(text: &mut String, units: usize) {
    let mut consumed = 0;
    let mut boundary = text.len();
    for (index, character) in text.char_indices() {
        if consumed >= units {
            boundary = index;
            break;
        }
        consumed += character.len_utf16();
        if consumed >= units {
            boundary = index + character.len_utf8();
            break;
        }
    }
    text.drain(..boundary);
}

fn resumes_later(events: &[Value], index: usize) -> bool {
    events[index + 1..]
        .iter()
        .any(|event| matches!(event["type"].as_str(), Some("run_start" | "turn_start")))
}

fn apply_usage(model: &mut Session, usage: &Value) {
    model.stats.tokens = Default::default();
    model.stats.cost = None;
    for bucket in ["models", "tools"] {
        if let Some(usages) = usage[bucket].as_object() {
            for value in usages.values() {
                model.stats.tokens.input += value["input"].as_u64().unwrap_or(0);
                model.stats.tokens.output += value["output"].as_u64().unwrap_or(0);
                model.stats.tokens.cache_read += value["cacheRead"].as_u64().unwrap_or(0);
                model.stats.tokens.cache_write += value["cacheWrite"].as_u64().unwrap_or(0);
                *model.stats.cost.get_or_insert(0.) +=
                    value["cost"]["total"].as_f64().unwrap_or(0.);
            }
        }
    }
    model.stats.total_messages = model.messages.len() as u64;
}

fn queue_record(event: &Value) -> Value {
    let inbox = event["inbox"].as_array().cloned().unwrap_or_default();
    let mut steering = Vec::new();
    let mut follow_up = Vec::new();
    let mut steering_ids = Vec::new();
    let mut follow_up_ids = Vec::new();
    for item in inbox {
        let Some(mode) = item["mode"].as_str() else {
            continue;
        };
        let content = pi_core::session::content_text(&item["content"]);
        let id = item["id"]
            .as_u64()
            .map(|id| id.to_string())
            .or_else(|| item["id"].as_str().map(str::to_owned));
        match mode {
            "steer" => {
                steering.push(content);
                if let Some(id) = id {
                    steering_ids.push(id);
                }
            }
            "followUp" => {
                follow_up.push(content);
                if let Some(id) = id {
                    follow_up_ids.push(id);
                }
            }
            _ => {}
        }
    }
    steering_ids.extend(follow_up_ids);
    json!({
        "type":"queue_update",
        "steering":steering,
        "followUp":follow_up,
        "queuedSubmissions":steering_ids,
    })
}

/// Translate one committed durable event batch to the incremental coding-agent
/// records every app already understands. The returned records were applied to
/// a temporary projection, so the daemon can apply and broadcast them in order.
pub fn events(record: &Value, previous: &Session, target: &SshTarget) -> Result<Vec<Value>> {
    ensure!(
        record["key"] == target.key,
        "Durable event identity mismatch"
    );
    let events = record["events"]
        .as_array()
        .context("Missing durable events")?;
    let mut model = previous.clone();
    let mut output = Vec::new();
    for (index, event) in events.iter().enumerate() {
        match event["type"].as_str().unwrap_or("") {
            "run_start" => apply_record(&mut model, &mut output, json!({"type":"agent_start"}))?,
            "run_end" if !resumes_later(events, index) => {
                apply_record(&mut model, &mut output, json!({"type":"agent_settled"}))?
            }
            // A blocking compaction ends before its resumed generation's turn
            // starts. Re-enter Running there; ordinary tool turns already are.
            "turn_start" if model.run != RunState::Running => {
                apply_record(&mut model, &mut output, json!({"type":"agent_start"}))?
            }
            "message_start" if event["message"]["role"] != "system" => {
                apply_record(
                    &mut model,
                    &mut output,
                    json!({"type":"message_start","message":event["message"]}),
                )?;
            }
            "message_update" => {
                for change in event["changes"].as_array().into_iter().flatten() {
                    apply_record(
                        &mut model,
                        &mut output,
                        json!({"type":"message_update","assistantMessageEvent":change}),
                    )?;
                }
            }
            "message_end" => {
                if let Some(message) = event["entry"]["model"]
                    .as_array()
                    .and_then(|messages| messages.first())
                    .filter(|message| message["role"] != "system")
                {
                    apply_record(
                        &mut model,
                        &mut output,
                        json!({"type":"message_end","message":message}),
                    )?;
                }
            }
            "tool_execution_start" => apply_record(
                &mut model,
                &mut output,
                json!({
                    "type":"tool_execution_start",
                    "toolCallId":event["toolCallId"],
                    "toolName":event["toolName"],
                    "args":event["args"],
                }),
            )?,
            "tool_execution_update" => {
                let id = event["toolCallId"].as_str().unwrap_or("");
                let mut text = model
                    .tools
                    .iter()
                    .find(|tool| tool.id == id)
                    .map(|tool| tool.output.clone())
                    .unwrap_or_default();
                if let Some(set) = event["output"]["set"].as_str() {
                    text = set.to_owned();
                } else if event["output"].is_object() {
                    let trim = event["output"]["trimStart"]
                        .as_u64()
                        .and_then(|trim| usize::try_from(trim).ok())
                        .unwrap_or(0);
                    trim_start_utf16(&mut text, trim);
                    text.push_str(event["output"]["append"].as_str().unwrap_or(""));
                }
                let details = event
                    .as_object()
                    .and_then(|event| event.get("details"))
                    .cloned()
                    .unwrap_or_else(|| {
                        model
                            .tools
                            .iter()
                            .find(|tool| tool.id == id)
                            .map(|tool| tool.details.clone())
                            .unwrap_or(Value::Null)
                    });
                apply_record(
                    &mut model,
                    &mut output,
                    json!({
                        "type":"tool_execution_update",
                        "toolCallId":event["toolCallId"],
                        "toolName":event["toolName"],
                        "partialResult":{"content":[{"type":"text","text":text}],"details":details},
                    }),
                )?;
            }
            "tool_execution_end" => {
                let id = event["toolCallId"].as_str().unwrap_or("");
                let existing = model.tools.iter().find(|tool| tool.id == id);
                let result = event["entry"]["model"]
                    .as_array()
                    .and_then(|messages| messages.first())
                    .filter(|message| message["role"] == "toolResult")
                    .cloned()
                    .unwrap_or_else(|| {
                        json!({
                            "content":[{"type":"text","text":existing.map(|tool| tool.output.as_str()).unwrap_or("")}],
                            "details":existing.map(|tool| tool.details.clone()).unwrap_or(Value::Null),
                        })
                    });
                let is_error = result["isError"]
                    .as_bool()
                    .unwrap_or(event["entry"].is_null());
                apply_record(
                    &mut model,
                    &mut output,
                    json!({
                        "type":"tool_execution_end",
                        "toolCallId":event["toolCallId"],
                        "toolName":event["toolName"],
                        "result":result,
                        "isError":is_error,
                    }),
                )?;
            }
            "inbox_update" => {
                let update = queue_record(event);
                apply_record(&mut model, &mut output, update)?;
            }
            "auto_retry_start" => apply_record(
                &mut model,
                &mut output,
                json!({
                    "type":"auto_retry_start",
                    "attempt":event["attempt"],
                    "at":event["at"],
                    "errorMessage":event["errorMessage"],
                }),
            )?,
            "auto_retry_end" => apply_record(
                &mut model,
                &mut output,
                json!({"type":"auto_retry_end","attempt":event["attempt"]}),
            )?,
            "agent_changed" => {
                let agent = &event["agent"];
                let mut state = model.state.clone();
                state.thinking_level = agent["thinkingLevel"].as_str().unwrap_or("off").into();
                state.model = match (
                    agent["model"]["provider"].as_str(),
                    agent["model"]["modelId"].as_str(),
                ) {
                    (Some(provider), Some(id)) => Some(
                        model
                            .available_models
                            .iter()
                            .find(|candidate| candidate.provider == provider && candidate.id == id)
                            .cloned()
                            .unwrap_or_else(|| pi_core::protocol::Model {
                                provider: provider.into(),
                                id: id.into(),
                                ..Default::default()
                            }),
                    ),
                    _ => None,
                };
                let mut updated = model.clone();
                updated.state = state;
                let update = state_update(&updated)?;
                apply_record(&mut model, &mut output, update)?;
            }
            "usage_changed" => {
                apply_usage(&mut model, &event["usage"]);
                output.push(stats_update(&model)?);
            }
            "compaction_start" if event["blocking"] == true => {
                apply_record(&mut model, &mut output, json!({"type":"compaction_start"}))?
            }
            "compaction_end" if model.run == RunState::Compacting => {
                apply_record(&mut model, &mut output, json!({"type":"compaction_end"}))?;
                if !resumes_later(events, index) {
                    apply_record(&mut model, &mut output, json!({"type":"agent_settled"}))?;
                }
            }
            "task_failed" => apply_record(
                &mut model,
                &mut output,
                json!({
                    "type":"extension_error",
                    "extensionPath":event["kind"],
                    "error":event["message"],
                }),
            )?,
            _ => {}
        }
    }
    Ok(output)
}

pub fn metadata(record: &Value, previous: &Session, target: &SshTarget) -> Result<Vec<Value>> {
    ensure!(
        record["key"] == target.key,
        "Durable metadata identity mismatch"
    );
    let mut state = previous.state.clone();
    state.session_name = record["data"]["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    state.auto_compaction_enabled = record["data"]["autoCompaction"].as_bool().unwrap_or(true);
    let mut updated = previous.clone();
    updated.state = state;
    state_update(&updated).map(|record| vec![record])
}

pub fn subagents(record: &Value, previous: &Session, target: &SshTarget) -> Result<Vec<Value>> {
    ensure!(
        record["key"] == target.key,
        "Durable subagent identity mismatch"
    );
    let Some(calls) = record["data"]["calls"].as_object() else {
        return Ok(Vec::new());
    };
    Ok(previous
        .tools
        .iter()
        .filter_map(|tool| calls.get(&tool.id).map(|details| (tool, details)))
        .map(|(tool, details)| {
            json!({
                "type":"tool_execution_update",
                "toolCallId":tool.id,
                "toolName":tool.name,
                "partialResult":{
                    "content":[{"type":"text","text":tool.output}],
                    "details":details,
                },
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn committed_projection_preserves_partial_tools_queue_and_remote_identity() {
        let mut target = SshTarget::new("dev".into(), "/remote/work".into()).unwrap();
        target.backend = pi_core::ssh::RemoteBackend::Durable;
        let mut previous = Session::new(target.identity());
        previous.stats.context_usage = Some(pi_core::protocol::ContextUsage {
            tokens: Some(60_000),
            context_window: 200_000,
            percent: Some(30.),
        });
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
        assert_eq!(
            model.stats.context_usage.as_ref().unwrap().tokens,
            Some(60_000)
        );
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

    fn durable_target() -> SshTarget {
        let mut target = SshTarget::new("dev".into(), "/remote/work".into()).unwrap();
        target.backend = pi_core::ssh::RemoteBackend::Durable;
        target
    }

    #[test]
    fn unsolicited_agent_usage_and_metadata_changes_are_events() {
        let target = durable_target();
        let mut previous = Session::new(target.identity());
        previous.state.session_id = Some(target.key.clone());
        previous.stats.context_usage = Some(pi_core::protocol::ContextUsage {
            tokens: Some(500),
            context_window: 1000,
            percent: Some(50.),
        });
        let records = events(
            &json!({"key":target.key,"events":[
                {"type":"agent_changed","agent":{"thinkingLevel":"high","model":{"provider":"faux","modelId":"faux-1"}}},
                {"type":"usage_changed","usage":{"models":{"faux/faux-1":{"input":12,"output":3,"cacheRead":4,"cacheWrite":1,"cost":{"total":0.25}}}}}
            ]}),
            &previous,
            &target,
        )
        .unwrap();
        assert_eq!(records[0]["type"], "state_update");
        assert_eq!(records[1]["type"], "stats_update");
        assert!(records.iter().all(|record| record["type"] != "response"));

        let mut phone = previous;
        for record in records {
            phone.apply(&record).unwrap();
        }
        let metadata = metadata(
            &json!({"key":target.key,"data":{"name":"Committed","autoCompaction":false}}),
            &phone,
            &target,
        )
        .unwrap();
        assert_eq!(metadata[0]["type"], "state_update");
        assert!(metadata[0].get("id").is_none());
        phone.apply(&metadata[0]).unwrap();
        assert_eq!(phone.state.session_name.as_deref(), Some("Committed"));
        assert_eq!(phone.state.thinking_level, "high");
        assert!(!phone.state.auto_compaction_enabled);
        assert_eq!(phone.stats.tokens.input, 12);
        assert_eq!(
            phone.stats.context_usage.as_ref().unwrap().tokens,
            Some(500)
        );
    }

    #[test]
    fn resumed_compaction_batch_never_projects_a_settled_gap() {
        let target = durable_target();
        let mut previous = Session::new(target.identity());
        previous.apply(&json!({"type":"agent_start"})).unwrap();
        let records = events(
            &json!({"key":target.key,"events":[
                {"type":"compaction_start","blocking":true},
                {"type":"compaction_end"},
                {"type":"run_end"},
                {"type":"run_start"},
                {"type":"turn_start"}
            ]}),
            &previous,
            &target,
        )
        .unwrap();
        assert!(
            records
                .iter()
                .all(|record| record["type"] != "agent_settled")
        );
        let mut phone = previous;
        for record in records {
            phone.apply(&record).unwrap();
            assert_ne!(phone.run, RunState::Idle);
        }
        assert_eq!(phone.run, RunState::Running);
    }

    #[test]
    fn tool_updates_trim_utf16_units_and_distinguish_missing_from_null_details() {
        let target = durable_target();
        let mut previous = Session::new(target.identity());
        previous.apply(&json!({"type":"tool_execution_start","toolCallId":"s","toolName":"subagent","args":{}})).unwrap();
        previous.apply(&json!({"type":"tool_execution_update","toolCallId":"s","toolName":"subagent","partialResult":{
            "content":[{"type":"text","text":"éab"}],
            "details":{"results":[{"status":"running"}]}
        }})).unwrap();
        let records = events(
            &json!({"key":target.key,"events":[
                {"type":"tool_execution_update","toolCallId":"s","toolName":"subagent","output":{"trimStart":2,"append":"!"}},
                {"type":"tool_execution_update","toolCallId":"s","toolName":"subagent","details":null}
            ]}),
            &previous,
            &target,
        )
        .unwrap();
        assert_eq!(records[0]["partialResult"]["content"][0]["text"], "b!");
        assert!(records[0]["partialResult"]["details"].is_object());
        assert!(records[1]["partialResult"]["details"].is_null());

        let mut phone = previous;
        for record in records {
            phone.apply(&record).unwrap();
        }
        assert_eq!(phone.tools[0].output, "b!");
        assert!(phone.tools[0].details.is_null());

        let mut split_surrogate = "😀x".to_owned();
        trim_start_utf16(&mut split_surrogate, 1);
        assert_eq!(split_surrogate, "x");
    }
}
