use std::io::BufRead;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// pi-desktop-backend's versions and commands; plain `pi --mode rpc` does not know it.
    GetBackendInfo,
    GetState,
    GetMessages,
    GetEntries,
    /// pi-desktop-backend: every `custom` entry of one type, from all branches.
    GetCustomEntries {
        #[serde(rename = "customType")]
        custom_type: String,
    },
    /// pi-desktop-backend: a data-only entry pi keeps out of the model's context.
    /// Only types starting with `pi-desktop-`; only while pi is idle.
    AppendCustomEntry {
        #[serde(rename = "customType")]
        custom_type: String,
        data: Value,
    },
    GetSettings,
    NavigateTree {
        #[serde(rename = "targetId")]
        target_id: String,
        summarize: bool,
    },
    SetLabel {
        #[serde(rename = "targetId")]
        target_id: String,
        label: String,
    },
    Fork {
        #[serde(rename = "entryId")]
        entry_id: String,
        /// pi-desktop-backend (`fork_cwd`): a new session file whose header
        /// names this folder; the running process keeps its session.
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    SetAutoCompaction {
        enabled: bool,
    },
    GetSessionStats,
    ListSessions {
        scope: String,
    },
    Prompt {
        message: String,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageContent>,
        #[serde(rename = "streamingBehavior", skip_serializing_if = "Option::is_none")]
        streaming_behavior: Option<StreamingBehavior>,
    },
    ClearQueue,
    Abort,
    Bash {
        command: String,
        #[serde(rename = "excludeFromContext")]
        exclude_from_context: bool,
    },
    AbortBash,
    CycleModel,
    CycleThinkingLevel,
    GetAvailableModels,
    GetAvailableThinkingLevels,
    GetCommands,
    GetAuthProviders,
    GetProjectTrust,
    ListPackages,
    SetProjectTrust {
        choice: String,
    },
    InstallPackage {
        source: String,
        local: bool,
    },
    RemovePackage {
        source: String,
        local: bool,
    },
    UpdatePackages {
        source: String,
    },
    SetScopedModels {
        patterns: Option<Vec<String>>,
        persist: bool,
    },
    SetModelThinkingLevel {
        provider: String,
        #[serde(rename = "modelId")]
        model_id: String,
        level: String,
    },
    SetModel {
        provider: String,
        #[serde(rename = "modelId")]
        model_id: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        persist: bool,
    },
    SetThinkingLevel {
        level: String,
    },
    Compact {
        #[serde(rename = "customInstructions", skip_serializing_if = "Option::is_none")]
        custom_instructions: Option<String>,
    },
    SetSessionName {
        name: String,
        /// Another saved session's file; `None` renames this process's session.
        #[serde(rename = "sessionPath", skip_serializing_if = "Option::is_none")]
        session_path: Option<String>,
    },
    DeleteSession {
        #[serde(rename = "sessionPath")]
        session_path: String,
    },
    ExportHtml {
        #[serde(rename = "outputPath", skip_serializing_if = "Option::is_none")]
        output_path: Option<String>,
    },
    Share,
    Clone,
    Reload,
}

/// An image sent with a prompt; pi resizes it for the model if its settings say so.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageContent {
    /// Always `image`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Base64.
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

impl ImageContent {
    pub fn new(data: String, mime_type: impl Into<String>) -> Self {
        Self {
            kind: "image".into(),
            data,
            mime_type: mime_type.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StreamingBehavior {
    Steer,
    FollowUp,
}

impl Command {
    pub fn record(&self, id: &str) -> Result<Value> {
        let mut record = serde_json::to_value(self)?;
        record["id"] = id.into();
        Ok(record)
    }

    /// pi answers these only after the work completes (a compaction can take minutes),
    /// so a request deadline would report a running command as failed.
    pub fn replies_when_finished(&self) -> bool {
        matches!(
            self,
            Self::Bash { .. }
                | Self::Compact { .. }
                | Self::NavigateTree { .. }
                | Self::Fork { .. }
                | Self::Reload
                | Self::InstallPackage { .. }
                | Self::RemovePackage { .. }
                | Self::UpdatePackages { .. }
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::GetState => "get_state",
            Self::GetMessages => "get_messages",
            Self::GetEntries => "get_entries",
            Self::GetSettings => "get_settings",
            Self::NavigateTree { .. } => "navigate_tree",
            Self::SetLabel { .. } => "set_label",
            Self::Fork { .. } => "fork",
            Self::SetAutoCompaction { .. } => "set_auto_compaction",
            Self::GetSessionStats => "get_session_stats",
            Self::ListSessions { .. } => "list_sessions",
            Self::Prompt { .. } => "prompt",
            Self::ClearQueue => "clear_queue",
            Self::Abort => "abort",
            Self::Bash { .. } => "bash",
            Self::AbortBash => "abort_bash",
            Self::CycleModel => "cycle_model",
            Self::CycleThinkingLevel => "cycle_thinking_level",
            Self::GetAvailableModels => "get_available_models",
            Self::GetAvailableThinkingLevels => "get_available_thinking_levels",
            Self::GetCommands => "get_commands",
            Self::GetBackendInfo => "get_backend_info",
            Self::GetCustomEntries { .. } => "get_custom_entries",
            Self::AppendCustomEntry { .. } => "append_custom_entry",
            Self::GetAuthProviders => "get_auth_providers",
            Self::GetProjectTrust => "get_project_trust",
            Self::ListPackages => "list_packages",
            Self::SetProjectTrust { .. } => "set_project_trust",
            Self::InstallPackage { .. } => "install_package",
            Self::RemovePackage { .. } => "remove_package",
            Self::UpdatePackages { .. } => "update_packages",
            Self::SetScopedModels { .. } => "set_scoped_models",
            Self::SetModelThinkingLevel { .. } => "set_model_thinking_level",
            Self::SetModel { .. } => "set_model",
            Self::SetThinkingLevel { .. } => "set_thinking_level",
            Self::Compact { .. } => "compact",
            Self::SetSessionName { .. } => "set_session_name",
            Self::DeleteSession { .. } => "delete_session",
            Self::ExportHtml { .. } => "export_html",
            Self::Share => "share",
            Self::Clone => "clone",
            Self::Reload => "reload",
        }
    }
}

/// Only LF frames RPC. In particular, U+2028 and U+2029 are ordinary string data.
pub fn read_record(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut bytes = Vec::new();
    loop {
        let buffer = reader.fill_buf().context("reading RPC stdout")?;
        if buffer.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            bail!("RPC stdout ended in an unterminated record");
        }
        let count = buffer.iter().position(|byte| *byte == b'\n').map(|i| i + 1);
        let count = count.unwrap_or(buffer.len());
        if bytes.len() + count > MAX_RECORD_BYTES {
            bail!("RPC record exceeds {MAX_RECORD_BYTES} bytes");
        }
        bytes.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if bytes.last() == Some(&b'\n') {
            break;
        }
    }
    let value: Value = serde_json::from_slice(&bytes).context("invalid RPC JSON")?;
    if !value.is_object() || value["type"].as_str().is_none() {
        bail!("RPC record must be an object with a string type");
    }
    Ok(Some(value))
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub session_id: Option<String>,
    pub session_name: Option<String>,
    pub session_file: Option<String>,
    pub model: Option<Model>,
    #[serde(default)]
    pub thinking_level: String,
    #[serde(default)]
    pub is_streaming: bool,
    #[serde(default)]
    pub is_compacting: bool,
    pub is_bash_running: Option<bool>,
    #[serde(default)]
    pub auto_compaction_enabled: bool,
    /// Current SDK loadout, not tools inferred from conversation history.
    /// Missing means this backend does not report it; `[]` means none active.
    pub active_tools: Option<Vec<ActiveTool>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveTool {
    pub name: String,
    pub description: Option<String>,
    pub source_info: Option<SourceInfo>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub name: Option<String>,
    pub reasoning: Option<bool>,
    pub max_tokens: Option<u64>,
    pub cost: Option<ModelCost>,
    pub provider: String,
    #[serde(default)]
    pub context_window: u64,
    /// `text`, and `image` when the model accepts images.
    #[serde(default)]
    pub input: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCost {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProvider {
    pub id: String,
    pub name: String,
    pub auth_type: String,
    pub status: Option<AuthStatus>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct AuthStatus {
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub source: String,
    pub scope: String,
    #[serde(default)]
    pub filtered: bool,
    pub installed_path: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectTrust {
    pub cwd: String,
    pub trusted: bool,
    pub has_project_resources: bool,
    pub saved_decision: Option<TrustDecision>,
    /// Allow-listed effective project settings, supplied by the owning SDK.
    pub project_settings: Option<Value>,
    pub loaded_extensions: Option<Vec<LoadedExtension>>,
    pub user_settings: Option<Value>,
    pub context_files: Option<Vec<String>>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedExtension {
    pub path: String,
    pub source_info: Option<SourceInfo>,
    pub status: String,
    pub error: Option<String>,
    #[serde(default)]
    pub commands: Vec<String>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct TrustDecision {
    pub path: String,
    pub decision: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
    /// `extension`, `prompt`, or `skill`.
    pub source: String,
    pub source_info: Option<SourceInfo>,
}

/// Where a command's resource was loaded from.
#[derive(Clone, Debug, Deserialize)]
pub struct SourceInfo {
    pub path: String,
    /// Package spec for packaged resources, such as `npm:@acme/git-guard@1.4.0`.
    pub source: String,
    /// `user`, `project`, or `temporary`.
    pub scope: String,
    /// `package` or `top-level`.
    pub origin: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    #[serde(default)]
    pub tokens: Tokens,
    pub cost: Option<f64>,
    pub context_usage: Option<ContextUsage>,
    #[serde(default)]
    pub total_messages: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub tokens: Option<u64>,
    pub context_window: u64,
    pub percent: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedSession {
    pub id: String,
    pub path: String,
    pub cwd: String,
    pub name: Option<String>,
    #[serde(default)]
    pub first_message: String,
    /// RFC 3339 time of the latest user or assistant message.
    pub modified: Option<String>,
    /// RFC 3339 time from the session header.
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub message_count: Option<u64>,
    /// The session this one was forked from.
    #[serde(default)]
    pub parent_session_path: Option<String>,
}

/// Saved rows and open threads use the same single-line, Unicode-safe fallback.
pub fn session_title<'a>(name: Option<&'a str>, preview: Option<&'a str>) -> &'a str {
    let title = name
        .filter(|value| !value.trim().is_empty())
        .or_else(|| preview.filter(|value| !value.trim().is_empty()))
        .unwrap_or("New session")
        .trim();
    let title = title.lines().next().unwrap_or("New session");
    &title[..title
        .char_indices()
        .nth(80)
        .map_or(title.len(), |(index, _)| index)]
}

impl SavedSession {
    pub fn title(&self) -> &str {
        session_title(self.name.as_deref(), Some(&self.first_message))
    }
}
