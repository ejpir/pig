use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::protocol::{
    Command, ImageContent, SavedSession, SessionState, SessionStats, StreamingBehavior,
};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum RunState {
    #[default]
    Idle,
    Running,
    Compacting,
    Retrying,
}

impl RunState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Running => "Running",
            Self::Compacting => "Compacting",
            Self::Retrying => "Retrying",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub args: Value,
    pub output: String,
    pub diff: Option<String>,
    pub finished: bool,
    pub is_error: bool,
    /// Image blocks in its result, such as a screenshot it read. A durable
    /// session sends each with empty `data` and an `imageId` to fetch it by.
    /// Snapshots from helpers older than this field leave it out.
    #[serde(default)]
    pub images: Vec<Value>,
    /// A `subagent` call's progress and results ([`crate::subagent`]). Other
    /// tools' details stay out: they can be large, and nothing reads them.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl Tool {
    pub fn target(&self) -> String {
        self.args["path"]
            .as_str()
            .or(self.args["command"].as_str())
            .or(self.args["pattern"].as_str())
            .unwrap_or("")
            .to_owned()
    }
}

/// A bounded live preview; the SDK's final message remains the history authority.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellExecution {
    pub id: String,
    pub command: String,
    pub exclude_from_context: bool,
    pub output: String,
    pub finished: bool,
    pub result: Option<Value>,
}

/// What `get_backend_info` found out about the program running this session.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub enum BackendInfo {
    /// Not asked, or no answer yet.
    #[default]
    Unknown,
    /// The desktop extension's answer: `backend`, `version`, `piVersion`,
    /// `protocolVersion`, `nodeVersion`, `bunVersion` (pi's release binary), `commands`,
    /// `features`.
    Found(Value),
    /// No answer: a program without the desktop extension.
    Unsupported,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    pub cwd: PathBuf,
    pub state: SessionState,
    pub stats: SessionStats,
    pub run: RunState,
    pub shell: Option<ShellExecution>,
    pub messages: Vec<Value>,
    #[serde(skip)]
    pub history: Option<std::sync::Arc<crate::history::History>>,
    pub settings: Option<Value>,
    pub backend: BackendInfo,
    /// Retry events observed during this connection; not fabricated from saved history.
    pub retries: Vec<Value>,
    pub tools: Vec<Tool>,
    pub steering: Vec<String>,
    pub follow_up: Vec<String>,
    /// Stable durable submission IDs, ordered like steering then follow_up.
    #[serde(default)]
    pub queued_submissions: Vec<String>,
    pub saved: Vec<SavedSession>,
    pub available_models: Vec<crate::protocol::Model>,
    pub models_loaded: bool,
    pub auth_providers: Option<Vec<crate::protocol::AuthProvider>>,
    pub packages: Option<Vec<crate::protocol::Package>>,
    pub project_trust: Option<crate::protocol::ProjectTrust>,
    pub thinking_levels: Vec<String>,
    pub commands: Vec<crate::protocol::SlashCommand>,
    pub commands_loaded: bool,
    pub extension_status: BTreeMap<String, String>,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// Latest TPS extension notification, preserved verbatim for the status bar.
    pub turn_metrics: Option<String>,
    /// Saved-list preview while history is loading; never persisted as a rename.
    pub preview_title: Option<String>,
    streaming_message: Option<usize>,
}

impl Session {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            ..Self::default()
        }
    }

    pub fn title(&self) -> &str {
        let first_message = self
            .messages
            .iter()
            .filter(|message| message["role"] == "user")
            .find_map(|message| {
                let content = &message["content"];
                content
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| {
                        content
                            .as_array()?
                            .iter()
                            .filter(|block| block["type"] == "text")
                            .filter_map(|block| block["text"].as_str())
                            .find(|value| !value.trim().is_empty())
                    })
            });
        crate::protocol::session_title(
            self.state.session_name.as_deref(),
            first_message.or(self.preview_title.as_deref()),
        )
    }

    /// The row affected by the next indexed streaming delta, if a reply is open.
    pub fn streaming_message_index(&self) -> Option<usize> {
        self.streaming_message
    }

    pub fn busy(&self) -> bool {
        self.run != RunState::Idle || self.shell_running()
    }

    pub fn shell_running(&self) -> bool {
        self.state.is_bash_running == Some(true)
            || self.shell.as_ref().is_some_and(|shell| !shell.finished)
    }
    pub fn run_label(&self) -> &'static str {
        if self.run == RunState::Idle && self.shell_running() {
            "Shell running"
        } else {
            self.run.label()
        }
    }
    pub fn prompt(&self, message: String, images: Vec<ImageContent>, follow_up: bool) -> Command {
        Command::Prompt {
            message,
            images,
            streaming_behavior: Some(if follow_up {
                StreamingBehavior::FollowUp
            } else {
                StreamingBehavior::Steer
            }),
        }
    }

    /// State is driven by session events, never by the prompt acknowledgement.
    pub fn apply(&mut self, record: &Value) -> Result<()> {
        match record["type"].as_str().unwrap_or("") {
            "remote_snapshot" => {
                let mut snapshot: Self = serde_json::from_value(record["data"].clone())
                    .context("invalid remote snapshot")?;
                anyhow::ensure!(
                    snapshot.streaming_message.is_none_or(|index| {
                        snapshot
                            .messages
                            .get(index)
                            .is_some_and(|m| m["role"] == "assistant")
                    }),
                    "invalid remote streaming message index"
                );
                // Remote filesystem paths are never local project identities.
                snapshot.cwd = self.cwd.clone();
                snapshot.saved.clear();
                *self = snapshot;
            }
            "response" => self.response(record)?,
            "bash_execution_update" => {
                if let Some(shell) = &mut self.shell
                    && record["id"].as_str() == Some(&shell.id)
                    && !shell.finished
                {
                    shell
                        .output
                        .push_str(record["delta"].as_str().unwrap_or(""));
                    // Retain a UTF-8-safe tail while the SDK owns the full output.
                    if shell.output.len() > 64 * 1024 {
                        let mut start = shell.output.len() - 64 * 1024;
                        while !shell.output.is_char_boundary(start) {
                            start += 1;
                        }
                        shell.output.drain(..start);
                    }
                }
            }
            "agent_start" => {
                self.run = RunState::Running;
                self.error = None;
                self.turn_metrics = None;
            }
            "agent_settled" => {
                self.run = RunState::Idle;
                self.streaming_message = None;
            }
            "compaction_start" => self.run = RunState::Compacting,
            "compaction_end" => {
                if record["result"].is_object() {
                    // Pre-compaction counts are no longer valid; wait for RPC stats.
                    self.stats.context_usage = None;
                }
                if let Some(error) = record["errorMessage"].as_str() {
                    self.error = Some(error.to_owned());
                }
            }
            "auto_retry_start" | "summarization_retry_scheduled" => {
                self.run = RunState::Retrying;
                self.record_retry(record);
            }
            "auto_retry_end" => {
                self.record_retry(record);
                if let Some(error) = record["finalError"].as_str() {
                    self.error = Some(error.to_owned());
                }
            }
            "message_start" if record["message"]["role"] == "assistant" => {
                self.streaming_message = Some(self.messages.len());
                self.messages.push(record["message"].clone());
            }
            "message_update" => self.update_message(&record["assistantMessageEvent"]),
            "message_end" => self.finish_message(record["message"].clone(), true),
            // An extension's message added as a run settles; shaped as `get_messages` returns it.
            "entry_appended" if record["entry"]["type"] == "custom_message" => {
                let entry = &record["entry"];
                // As pi builds it for `get_messages`: the entry's time in ms.
                let timestamp = entry["timestamp"]
                    .as_str()
                    .and_then(crate::clock::parse_timestamp);
                self.messages.push(json!({
                    "role": "custom",
                    "customType": entry["customType"],
                    "content": entry["content"],
                    "display": entry["display"],
                    "details": entry["details"],
                    "timestamp": timestamp,
                }));
            }
            "tool_execution_start" => {
                self.upsert_tool(
                    text(record, "toolCallId"),
                    text(record, "toolName"),
                    record["args"].clone(),
                );
            }
            "tool_execution_update" | "tool_execution_end" => {
                let finished = record["type"] == "tool_execution_end";
                let id = text(record, "toolCallId");
                if !self.tools.iter().any(|tool| tool.id == id) {
                    self.upsert_tool(id.clone(), text(record, "toolName"), record["args"].clone());
                }
                self.tool_result(
                    &id,
                    if finished {
                        &record["result"]
                    } else {
                        &record["partialResult"]
                    },
                    finished,
                    record["isError"].as_bool().unwrap_or(false),
                );
            }
            "queue_update" => {
                self.steering = serde_json::from_value(record["steering"].clone())?;
                self.follow_up = serde_json::from_value(record["followUp"].clone())?;
            }
            "session_info_changed" => {
                self.state.session_name = record["name"].as_str().map(str::to_owned)
            }
            "thinking_level_changed" => self.state.thinking_level = text(record, "level"),
            "extension_error" => {
                self.error = Some(format!(
                    "{}: {}",
                    text(record, "extensionPath"),
                    text(record, "error")
                ))
            }
            "extension_ui_request" => match record["method"].as_str() {
                Some("setStatus") => {
                    let key = text(record, "statusKey");
                    if let Some(status) = record["statusText"].as_str() {
                        self.extension_status.insert(key, status.to_owned());
                    } else {
                        self.extension_status.remove(&key);
                    }
                }
                Some("notify") => {
                    let message = text(record, "message");
                    if matches!(record["notifyType"].as_str(), None | Some("info"))
                        && is_turn_metrics(&message)
                    {
                        self.turn_metrics = Some(message);
                    } else {
                        self.notice = Some(message);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn record_retry(&mut self, record: &Value) {
        if self.retries.len() >= 100 {
            self.retries.remove(0);
        }
        self.retries.push(record.clone());
    }

    /// A shell completion snapshot may append without destroying retained row identities.
    pub fn shell_snapshot_appends(&self, data: &Value) -> bool {
        if !self.shell.as_ref().is_some_and(|shell| shell.finished) {
            return false;
        }
        let Some(messages) = data["messages"].as_array() else {
            return false;
        };
        let visible: Vec<_> = messages
            .iter()
            .filter(|message| message["role"] != "toolResult")
            .collect();
        visible.len() >= self.messages.len()
            && visible
                .iter()
                .zip(&self.messages)
                .all(|(next, old)| *next == old)
            && messages
                .iter()
                .filter(|message| message["role"] == "toolResult")
                .all(|message| {
                    self.tools
                        .iter()
                        .find(|tool| message["toolCallId"].as_str() == Some(&tool.id))
                        .is_none_or(|tool| {
                            tool.output == content_text(&message["content"])
                                && tool.diff.as_deref() == message["details"]["diff"].as_str()
                                && tool.is_error == message["isError"].as_bool().unwrap_or(false)
                        })
                })
    }
    fn response(&mut self, record: &Value) -> Result<()> {
        if record["command"] == "bash"
            && let Some(shell) = &mut self.shell
            && record["id"].as_str() == Some(&shell.id)
        {
            shell.finished = true;
            if record["success"] == true {
                shell.output = text(&record["data"], "output");
                shell.result = Some(record["data"].clone());
            } else {
                self.shell = None;
            }
        }
        // Optional metadata: a failure keeps "not reported" and is not a session error.
        if record["command"] == "get_active_tools" && record["success"] != true {
            return Ok(());
        }
        // Optional: a program without it is not an error.
        if record["command"] == "get_backend_info" {
            self.backend = if record["success"] == true {
                BackendInfo::Found(record["data"].clone())
            } else {
                BackendInfo::Unsupported
            };
            return Ok(());
        }
        // The desktop's own records, which it keeps and handles itself.
        if matches!(
            record["command"].as_str(),
            Some("get_custom_entries" | "append_custom_entry")
        ) {
            return Ok(());
        }
        if record["success"] == false {
            self.error = Some(text(record, "error"));
            return Ok(());
        }
        let data = &record["data"];
        match record["command"].as_str().unwrap_or("") {
            "get_state" => {
                // pi's state has no tools; `get_active_tools` reports them.
                let active_tools = self.state.active_tools.take();
                self.state =
                    serde_json::from_value(data.clone()).context("invalid get_state response")?;
                self.state.active_tools = active_tools;
                if self.state.is_streaming {
                    self.run = RunState::Running;
                }
                if self.state.is_compacting {
                    self.run = RunState::Compacting;
                }
            }
            "get_active_tools" => {
                self.state.active_tools = Some(
                    serde_json::from_value(data["activeTools"].clone())
                        .context("invalid active tools")?,
                )
            }
            "get_entries" => {
                self.history = Some(std::sync::Arc::new(crate::history::History::parse(data)?))
            }
            "get_settings" => self.settings = Some(data.clone()),
            "get_session_stats" => {
                self.stats =
                    serde_json::from_value(data.clone()).context("invalid session stats")?
            }
            "list_sessions" => {
                self.saved = serde_json::from_value(data["sessions"].clone())
                    .context("invalid session list")?
            }
            "get_messages" if data["remoteSnapshot"] == true => {}
            "get_messages" => {
                let append = self.shell_snapshot_appends(data);
                let previous = if append { self.messages.len() } else { 0 };
                let messages: Vec<Value> = serde_json::from_value(data["messages"].clone())?;
                if self.shell.as_ref().is_some_and(|shell| shell.finished) {
                    self.shell = None;
                }
                if !append {
                    self.messages.clear();
                    self.tools.clear();
                    self.streaming_message = None;
                }
                let mut visible = 0;
                for message in messages {
                    if message["role"] == "toolResult" {
                        self.finish_message(message, false);
                    } else {
                        if visible >= previous {
                            self.finish_message(message, false);
                        }
                        visible += 1;
                    }
                }
            }
            "get_available_models" => {
                self.available_models = serde_json::from_value(data["models"].clone())?;
                self.models_loaded = true;
            }
            "get_auth_providers" => {
                self.auth_providers = Some(serde_json::from_value(data["providers"].clone())?)
            }
            "list_packages" => {
                self.packages = Some(serde_json::from_value(data["packages"].clone())?)
            }
            "get_project_trust" => self.project_trust = Some(serde_json::from_value(data.clone())?),
            "get_available_thinking_levels" => {
                self.thinking_levels = serde_json::from_value(data["levels"].clone())?
            }
            "get_commands" => {
                let mut commands: Vec<crate::protocol::SlashCommand> =
                    serde_json::from_value(data["commands"].clone())?;
                commands.retain(|command| !crate::extension::is_own(command));
                self.commands = commands;
                self.commands_loaded = true;
            }
            "set_model" => self.state.model = serde_json::from_value(data.clone())?,
            "cycle_model" if data.is_object() => {
                self.state.model = serde_json::from_value(data["model"].clone())?;
                self.state.thinking_level = text(data, "thinkingLevel");
            }
            "cycle_thinking_level" if data.is_object() => {
                self.state.thinking_level = text(data, "level")
            }
            _ => {}
        }
        Ok(())
    }

    fn upsert_tool(&mut self, id: String, name: String, args: Value) {
        if let Some(tool) = self.tools.iter_mut().find(|tool| tool.id == id) {
            if args.is_object() {
                tool.args = args;
            }
        } else {
            self.tools.push(Tool {
                id,
                name,
                args,
                output: String::new(),
                diff: None,
                finished: false,
                is_error: false,
                images: Vec::new(),
                details: Value::Null,
            });
        }
    }

    fn tool_result(&mut self, id: &str, result: &Value, finished: bool, is_error: bool) {
        if let Some(tool) = self.tools.iter_mut().find(|tool| tool.id == id) {
            tool.output = content_text(&result["content"]);
            tool.images = result["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|block| block["type"] == "image")
                .cloned()
                .collect();
            tool.diff = result["details"]["diff"].as_str().map(str::to_owned);
            if tool.name == crate::subagent::TOOL && !result["details"].is_null() {
                tool.details = result["details"].clone();
            }
            tool.finished = finished;
            tool.is_error = is_error;
        }
    }

    /// `live` is false while replaying history.
    fn finish_message(&mut self, message: Value, live: bool) {
        if message["role"] == "toolResult" {
            self.tool_result(
                &text(&message, "toolCallId"),
                &message,
                true,
                message["isError"].as_bool().unwrap_or(false),
            );
            return;
        }
        if let Some(blocks) = message["content"].as_array() {
            for block in blocks {
                if block["type"] == "toolCall" {
                    self.upsert_tool(
                        text(block, "id"),
                        text(block, "name"),
                        block["arguments"].clone(),
                    );
                }
            }
        }
        // Failures stay with their message in the transcript. Only a run failing now
        // raises the session error; a stop the user asked for (`aborted`) is not one.
        if live
            && message["stopReason"] != "aborted"
            && let Some(error) = message["errorMessage"].as_str()
        {
            self.error = Some(error.to_owned());
        }
        if message["role"] == "assistant"
            && let Some(index) = self.streaming_message.take()
            && let Some(partial) = self.messages.get_mut(index)
        {
            *partial = message;
            return;
        }
        self.messages.push(message);
    }

    fn update_message(&mut self, event: &Value) {
        let Some(index) = self.streaming_message else {
            return;
        };
        let Some(message) = self.messages.get_mut(index) else {
            return;
        };
        let Some(block_index) = event["contentIndex"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
        else {
            return;
        };
        // An untrusted index must not turn one wire record into an unbounded allocation.
        if block_index > 4096 {
            return;
        }
        let Some(blocks) = message["content"].as_array_mut() else {
            return;
        };
        while blocks.len() <= block_index {
            blocks.push(json!({}));
        }
        let block = &mut blocks[block_index];
        let kind = event["type"].as_str().unwrap_or("");
        match kind {
            "text_start" | "thinking_start" => {
                *block = if kind == "text_start" {
                    json!({"type":"text","text":""})
                } else {
                    json!({"type":"thinking","thinking":""})
                };
            }
            "text_delta" | "thinking_delta" | "text_end" | "thinking_end" => {
                let field = if kind.starts_with("text") {
                    "text"
                } else {
                    "thinking"
                };
                let value = if kind.ends_with("delta") {
                    format!("{}{}", text(block, field), text(event, "delta"))
                } else {
                    text(event, "content")
                };
                *block = json!({"type":field, field:value});
            }
            "toolcall_start" => {
                *block = json!({"type":"toolCall","id":event["id"],"name":event["toolName"],"arguments":{}})
            }
            "toolcall_end" => *block = event["toolCall"].clone(),
            _ => {}
        }
    }

    /// These are observed successful edit/write calls, not a fabricated git diff.
    pub fn changed_files(&self) -> Vec<String> {
        let mut paths: Vec<String> = self
            .tools
            .iter()
            .filter(|tool| {
                tool.finished && !tool.is_error && matches!(tool.name.as_str(), "edit" | "write")
            })
            .filter_map(|tool| tool.args["path"].as_str().map(str::to_owned))
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }
}

/// Recognize the info-only notification emitted by the TPS extension without
/// turning arbitrary prose or warnings into quiet status text. No usage is inferred.
fn is_turn_metrics(message: &str) -> bool {
    let Some((rate, rest)) = message
        .strip_prefix("TPS ")
        .and_then(|message| message.split_once(" tok/s. out "))
    else {
        return false;
    };
    rate.parse::<f64>()
        .is_ok_and(|rate| rate.is_finite() && rate >= 0.)
        && rest.contains(", in ")
        && rest.contains(", cache r/w ")
        && rest.contains(", total ")
        && rest.ends_with('s')
        && !message.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
}

pub fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").to_owned()
}

pub fn content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_from_older_helpers_still_load() {
        let tool: Tool = serde_json::from_value(serde_json::json!({
            "id": "t", "name": "read", "args": {}, "output": "", "diff": null,
            "finished": true, "is_error": false
        }))
        .unwrap();
        assert!(tool.images.is_empty());
    }
}
