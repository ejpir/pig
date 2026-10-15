//! Shared, disposable durable protocol projection. Execution/recovery remains in the upstream SDK.
use crate::session::{BackendInfo, RunState, Session};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

/// Build a disposable desktop projection from ONE committed durable view. Never use it for execution/recovery.
pub fn project(record: &Value, previous: &Session, key: &str) -> Result<Session> {
    ensure!(record["key"] == key, "Durable snapshot identity mismatch");
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
    model.stats.compaction = previous.stats.compaction.clone();
    model.stats.context_head_id = previous.stats.context_head_id;
    model.backend = BackendInfo::Found(record["backend"].clone());
    model.state.session_id = Some(key.to_owned());
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
    model.state.swarm_enabled = docs["app.desktop"]["swarmEnabled"].as_bool();
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
                .unwrap_or_else(|| crate::protocol::Model {
                    provider: provider.into(),
                    id: id.into(),
                    ..Default::default()
                }),
        );
    }
    model.state.context_head_id = Some(
        entries
            .iter()
            .filter(|entry| entry["head"].is_number())
            .filter_map(|entry| entry["id"].as_u64())
            .max()
            .unwrap_or(0),
    );
    let messages: Vec<Value> = entries
        .iter()
        .filter_map(|entry| entry["model"].as_array())
        .flatten()
        .filter(|message| message["role"] != "system")
        .cloned()
        .collect();
    model.apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":messages}}))?;
    if model
        .messages
        .iter()
        .filter(|message| message["role"] == "compactionSummary")
        .count()
        > previous
            .messages
            .iter()
            .filter(|message| message["role"] == "compactionSummary")
            .count()
    {
        model.stats.context_usage = None;
        model.stats.context_head_id = model.state.context_head_id;
    }
    let live = &docs["pi.live"];
    if live["run"].is_object() {
        model.apply(&json!({"type":"agent_start"}))?;
        model.state.is_streaming = true;
    }
    if live["generation"]["retry"].is_object() {
        model.run = RunState::Retrying;
    }
    model.state.compactions = serde_json::from_value(
        live["compactions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into(),
    )
    .context("Invalid durable compaction activity")?;
    if model.state.compactions.iter().any(|item| item.blocking) {
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
            let content = crate::session::content_text(&item["content"]);
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

pub fn state_update(model: &Session) -> Result<Value> {
    Ok(json!({"type":"state_update","state":serde_json::to_value(&model.state)?}))
}

/// A bounds-captured context read can finish after a newer summary was projected.
/// Preserve correlation, but answer with the current projection instead of old usage.
pub fn discard_stale_stats(record: &mut Value, model: &Session) -> Result<()> {
    if record["type"] == "response"
        && record["command"] == "get_session_stats"
        && record["success"] == true
        && record["data"]["contextHeadId"]
            .as_u64()
            .zip(model.state.context_head_id)
            .is_some_and(|(read, current)| read < current)
    {
        record["data"] = serde_json::to_value(&model.stats)?;
    }
    Ok(())
}

pub fn stats_update(model: &Session) -> Result<Value> {
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
        let content = crate::session::content_text(&item["content"]);
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
pub fn events(record: &Value, previous: &Session, key: &str) -> Result<Vec<Value>> {
    ensure!(record["key"] == key, "Durable event identity mismatch");
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
                    if message["role"] == "compactionSummary" {
                        // Kept assistants still carry usage for the removed prefix.
                        // Do not display that measurement while waiting for fresh stats.
                        if let Some(id) = event["entry"]["id"].as_u64() {
                            model.state.context_head_id = Some(id);
                            output.push(state_update(&model)?);
                        }
                        model.stats.context_usage = None;
                        model.stats.context_head_id = model.state.context_head_id;
                        output.push(stats_update(&model)?);
                    }
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
                            .unwrap_or_else(|| crate::protocol::Model {
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
            "compaction_start" => {
                if let Some(task_id) = event["taskId"].as_u64() {
                    model
                        .state
                        .compactions
                        .retain(|item| item.task_id != task_id);
                    model
                        .state
                        .compactions
                        .push(crate::protocol::CompactionActivity {
                            task_id,
                            blocking: event["blocking"] == true,
                        });
                }
                if event["blocking"] == true {
                    apply_record(&mut model, &mut output, json!({"type":"compaction_start"}))?;
                }
                let update = state_update(&model)?;
                apply_record(&mut model, &mut output, update)?;
            }
            "compaction_end" => {
                let task_id = event["taskId"].as_u64();
                let blocking = task_id.map_or(model.run == RunState::Compacting, |id| {
                    model
                        .state
                        .compactions
                        .iter()
                        .any(|item| item.task_id == id && item.blocking)
                });
                model
                    .state
                    .compactions
                    .retain(|item| Some(item.task_id) != task_id);
                let update = state_update(&model)?;
                apply_record(&mut model, &mut output, update)?;
                if blocking && !model.state.compactions.iter().any(|item| item.blocking) {
                    apply_record(&mut model, &mut output, json!({"type":"compaction_end"}))?;
                    if !resumes_later(events, index) {
                        apply_record(&mut model, &mut output, json!({"type":"agent_settled"}))?;
                    }
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

pub fn metadata(record: &Value, previous: &Session, key: &str) -> Result<Vec<Value>> {
    ensure!(record["key"] == key, "Durable metadata identity mismatch");
    let mut state = previous.state.clone();
    state.session_name = record["data"]["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    state.auto_compaction_enabled = record["data"]["autoCompaction"].as_bool().unwrap_or(true);
    state.swarm_enabled = record["data"]["swarmEnabled"].as_bool();
    let mut updated = previous.clone();
    updated.state = state;
    state_update(&updated).map(|record| vec![record])
}

pub fn subagents(record: &Value, previous: &Session, key: &str) -> Result<Vec<Value>> {
    ensure!(record["key"] == key, "Durable subagent identity mismatch");
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
    use crate::ssh::SshTarget;
    #[test]
    fn committed_projection_preserves_partial_tools_queue_and_remote_identity() {
        let mut target = SshTarget::new("dev".into(), "/remote/work".into()).unwrap();
        target.backend = crate::ssh::RemoteBackend::Durable;
        let mut previous = Session::new(target.identity());
        previous.stats.context_usage = Some(crate::protocol::ContextUsage {
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
        let model = project(&record, &previous, &target.key).unwrap();
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
        let next = project(&record, &model, &target.key).unwrap();
        assert_eq!(
            next.messages.len(),
            model.messages.len(),
            "Snapshots replace, never append"
        );
        assert!(project(&json!({"key":"wrong","data":{}}), &previous, &target.key).is_err());
        assert!(
            project(
                &json!({"key":target.key,"data":{"entries":[]}}),
                &previous,
                &target.key
            )
            .is_err()
        );
    }

    fn durable_target() -> SshTarget {
        let mut target = SshTarget::new("dev".into(), "/remote/work".into()).unwrap();
        target.backend = crate::ssh::RemoteBackend::Durable;
        target
    }

    fn swarm_call() -> Value {
        json!({"version":1,"mode":"parallel","background":true,"results":[
            {"index":0,"agent":"scout","task":"Find clients","status":"running","conversationId":"3","peerId":"peer-1"},
            {"index":1,"agent":"scout","task":"Check the protocol","status":"running","conversationId":"4","peerId":"peer-2"}
        ],"swarm":{"version":1,"id":"swarm-call","closed":false,"members":[
            {"id":"peer-1","agent":"scout","task":"Find clients"},
            {"id":"peer-2","agent":"scout","task":"Check the protocol"}
        ],"messages":[
            {"id":"m-1","sequence":1,"from":"peer-1","to":"peer-2","kind":"question","text":"Keep snapshots?","status":"queued","createdAt":1000}
        ]}})
    }

    #[test]
    fn swarm_projection_survives_snapshots_and_incremental_receipts_without_new_rpcs() {
        use crate::subagent::{Handoff, SwarmMessageStatus};
        let target = durable_target();
        let previous = Session::new(target.identity());
        let details = swarm_call();
        let snapshot = json!({"key":target.key,"backend":{"backend":"pi-durable","features":["swarm_messaging"]},"data":{
            "entries":[{"model":[{"role":"assistant","content":[{"type":"toolCall","id":"swarm-call","name":"subagent","arguments":{"swarm":true,"tasks":[
                {"agent":"scout","task":"Find clients"},{"agent":"scout","task":"Check the protocol"}
            ]}}]}]}],
            "docs":{"app.subagent-calls":{"calls":{"swarm-call":details}}}
        }});
        let mut projected = project(&snapshot, &previous, &target.key).unwrap();
        let handoff = Handoff::of(&projected.tools[0]).unwrap();
        assert_eq!(handoff.subagents[1].peer_id.as_deref(), Some("peer-2"));
        let swarm = handoff.swarm.unwrap();
        assert_eq!(swarm.messages[0].status, SwarmMessageStatus::Queued);
        assert_eq!(swarm.member("peer-2").unwrap().agent, "scout");
        let mut next = swarm_call();
        next["swarm"]["messages"][0]["status"] = json!("received");
        next["swarm"]["messages"][0]["receivedAt"] = json!(2000);
        let event = json!({"key":target.key,"data":{"calls":{"swarm-call":next}}});
        for record in subagents(&event, &projected, &target.key).unwrap() {
            projected.apply(&record).unwrap();
        }
        let received = Handoff::of(&projected.tools[0]).unwrap().swarm.unwrap();
        assert_eq!(received.messages[0].status, SwarmMessageStatus::Received);
        assert_eq!(received.messages[0].received_at, Some(2000));
        assert_eq!(projected.tools.len(), 1);
        assert!(
            subagents(
                &json!({"key":"other-session","data":{"calls":{"swarm-call":swarm_call()}}}),
                &projected,
                &target.key
            )
            .is_err()
        );
    }

    #[test]
    fn unsolicited_agent_usage_and_metadata_changes_are_events() {
        let target = durable_target();
        let mut previous = Session::new(target.identity());
        previous.state.session_id = Some(target.key.clone());
        previous.stats.context_usage = Some(crate::protocol::ContextUsage {
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
            &target.key,
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
            &json!({"key":target.key,"data":{"name":"Committed","autoCompaction":false,"swarmEnabled":false}}),
            &phone,
            &target.key,
        )
        .unwrap();
        assert_eq!(metadata[0]["type"], "state_update");
        assert!(metadata[0].get("id").is_none());
        phone.apply(&metadata[0]).unwrap();
        assert_eq!(phone.state.session_name.as_deref(), Some("Committed"));
        assert_eq!(phone.state.thinking_level, "high");
        assert!(!phone.state.auto_compaction_enabled);
        assert_eq!(phone.state.swarm_enabled, Some(false));
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
            &target.key,
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
    fn background_compaction_is_visible_without_blocking_generation() {
        let target = durable_target();
        let mut phone = Session::new(target.identity());
        phone.apply(&json!({"type":"agent_start"})).unwrap();
        let records = events(
            &json!({"key":target.key,"events":[
                {"type":"compaction_start","taskId":42,"blocking":false}
            ]}),
            &phone,
            &target.key,
        )
        .unwrap();
        for record in records {
            phone.apply(&record).unwrap();
        }
        assert_eq!(phone.run, RunState::Running);
        assert!(!phone.state.is_compacting);
        assert_eq!(phone.state.compactions[0].task_id, 42);
        assert!(!phone.state.compactions[0].blocking);

        // Ending background work must not settle an overlapping blocking task.
        for record in events(
            &json!({"key":target.key,"events":[
                {"type":"compaction_start","taskId":43,"blocking":true},
                {"type":"compaction_end","taskId":42}
            ]}),
            &phone,
            &target.key,
        )
        .unwrap()
        {
            phone.apply(&record).unwrap();
        }
        assert_eq!(phone.run, RunState::Compacting);
        assert!(phone.state.is_compacting);
        assert_eq!(phone.state.compactions.len(), 1);
        assert_eq!(phone.state.compactions[0].task_id, 43);
        for record in events(
            &json!({"key":target.key,"events":[
                {"type":"compaction_end","taskId":43}, {"type":"turn_start"}
            ]}),
            &phone,
            &target.key,
        )
        .unwrap()
        {
            phone.apply(&record).unwrap();
        }
        assert_eq!(phone.run, RunState::Running);
        assert!(phone.state.compactions.is_empty());
    }

    #[test]
    fn idle_background_compaction_survives_reconnect_and_clears_on_end() {
        let target = durable_target();
        let previous = Session::new(target.identity());
        let mut phone = project(
            &json!({"key":target.key,"data":{
                "entries":[], "docs":{"pi.live":{"compactions":[{"taskId":42,"blocking":false}]}}
            }}),
            &previous,
            &target.key,
        )
        .unwrap();
        assert!(!phone.busy());
        assert_eq!(phone.state.compactions.len(), 1);
        for record in events(
            &json!({"key":target.key,"events":[
                {"type":"compaction_end","taskId":42}
            ]}),
            &phone,
            &target.key,
        )
        .unwrap()
        {
            phone.apply(&record).unwrap();
        }
        assert!(!phone.busy());
        assert!(phone.state.compactions.is_empty());
    }

    #[test]
    fn old_stats_cannot_restore_usage_after_a_summary_or_reconnect() {
        let target = durable_target();
        let mut previous = Session::new(target.identity());
        previous.state.context_head_id = Some(0);
        previous
            .apply(&json!({"type":"stats_update","stats":{
                "contextUsage":{"tokens":237000,"contextWindow":272000,"percent":87.1},
                "contextHeadId":0
            }}))
            .unwrap();
        let mut phone = project(&json!({"key":target.key,"data":{
            "entries":[{"id":40,"head":4,"model":[{"role":"compactionSummary","summary":"short"}]}],
            "docs":{}
        }}), &previous, &target.key).unwrap();
        assert_eq!(phone.state.context_head_id, Some(40));
        assert_eq!(phone.stats.context_head_id, Some(40));
        assert!(phone.stats.context_usage.is_none());
        let mut old = json!({"type":"response","id":"pending-read","command":"get_session_stats","success":true,
            "data":{"contextHeadId":0,"contextUsage":{"tokens":237000,"contextWindow":272000,"percent":87.1}}
        });
        discard_stale_stats(&mut old, &phone).unwrap();
        assert_eq!(old["id"], "pending-read");
        assert_eq!(old["data"]["contextHeadId"], 40);
        phone.apply(&old).unwrap();
        assert!(phone.stats.context_usage.is_none());

        let mut fresh = json!({"type":"response","id":"fresh-read","command":"get_session_stats","success":true,
            "data":{"contextHeadId":40,"contextUsage":{"tokens":46000,"contextWindow":272000,"percent":16.9}}
        });
        discard_stale_stats(&mut fresh, &phone).unwrap();
        phone.apply(&fresh).unwrap();
        assert_eq!(
            phone.stats.context_usage.as_ref().unwrap().tokens,
            Some(46000)
        );
        old["data"]["contextHeadId"] = json!(0);
        discard_stale_stats(&mut old, &phone).unwrap();
        phone.apply(&old).unwrap(); // A second old read must preserve the newer measurement too.
        assert_eq!(
            phone.stats.context_usage.as_ref().unwrap().tokens,
            Some(46000)
        );
    }

    #[test]
    fn unknown_duplicate_and_overlapping_blocking_ends_do_not_settle_other_work() {
        let target = durable_target();
        let mut phone = Session::new(target.identity());
        for record in events(
            &json!({"key":target.key,"events":[
                {"type":"compaction_start","taskId":42,"blocking":true},
                {"type":"compaction_start","taskId":43,"blocking":true},
                {"type":"compaction_end","taskId":42},
                {"type":"compaction_end","taskId":42},
                {"type":"compaction_end","taskId":99}
            ]}),
            &phone,
            &target.key,
        )
        .unwrap()
        {
            phone.apply(&record).unwrap();
        }
        assert_eq!(phone.run, RunState::Compacting);
        assert!(phone.state.is_compacting);
        assert_eq!(phone.state.compactions.len(), 1);
        assert_eq!(phone.state.compactions[0].task_id, 43);
    }

    #[test]
    fn placed_summary_clears_stale_usage_before_the_refresh_arrives() {
        let target = durable_target();
        let mut phone = Session::new(target.identity());
        phone.apply(&json!({"type":"agent_start"})).unwrap();
        phone
            .apply(&json!({"type":"message_start","message":{
                "role":"assistant","content":[{"type":"text","text":"in flight"}]
            }}))
            .unwrap();
        phone
            .apply(&json!({"type":"stats_update","stats":{
                "contextUsage":{"tokens":237000,"contextWindow":272000,"percent":87.1},
                "compaction":{"enabled":true,"reserveTokens":16384,"backgroundTokens":32768}
            }}))
            .unwrap();
        for record in events(&json!({"key":target.key,"events":[
            {"type":"message_end","entry":{"model":[{"role":"compactionSummary","summary":"short"}]}}
        ]}), &phone, &target.key).unwrap() { phone.apply(&record).unwrap(); }
        assert!(phone.stats.context_usage.is_none());
        assert_eq!(
            phone.stats.compaction.as_ref().unwrap().background_tokens,
            32768
        );
        assert_eq!(phone.streaming_message_index(), Some(0));
        assert_eq!(phone.messages[0]["content"][0]["text"], "in flight");
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
            &target.key,
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
