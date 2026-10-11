//! Privacy-safe, bounded summaries of session transport records.
//!
//! This module never retains the source JSON. It only emits values from closed
//! vocabularies, booleans, counts and byte sizes. Transcript text, tool I/O,
//! raw paths, identifiers and arbitrary scalar values are deliberately absent.

use serde_json::{Map, Value, json};
use std::collections::VecDeque;

pub(crate) const TRANSPORT_DIAGNOSTIC_MAX_ENTRIES: usize = 128;
pub(crate) const TRANSPORT_DIAGNOSTIC_MAX_BYTES: usize = 48 * 1024;
const TRANSPORT_DIAGNOSTIC_PREVIEW_BYTES: usize = 1024;
const SUMMARY_ENTRY_BUDGET: usize = 256;
const STRINGIFIED_JSON_BYTE_BUDGET: usize = 128 * 1024;
const MAX_STRINGIFIED_JSON_BYTES: usize = 64 * 1024;
const MAX_DECODE_LAYERS: usize = 3;
const MAX_NESTED_SUMMARY_DEPTH: usize = 3;
const MAX_PATH_SEGMENTS: usize = 12;
const MAX_PATH_BYTES: usize = 512;
const OMITTED_CATEGORIES: &[&str] = &[
    "text",
    "tool_io",
    "image_data",
    "errors",
    "raw_paths",
    "credentials",
    "headers",
    "ids",
    "free_scalars",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportDirection {
    Outbound,
    Inbound,
}

/// A deliberately small, display-safe description of one session-pipe record.
/// The original JSON and raw correlation identifiers are never retained here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TransportDiagnostic {
    pub(crate) at_millis: u64,
    pub(crate) direction: TransportDirection,
    pub(crate) record_type: String,
    pub(crate) command: Option<String>,
    pub(crate) has_id: bool,
    pub(crate) byte_size: usize,
    pub(crate) preview: Option<String>,
}

impl TransportDiagnostic {
    pub(super) fn from_record(direction: TransportDirection, record: &Value) -> Self {
        let record_type = known_record_kind(record.get("type").and_then(Value::as_str)).to_owned();
        let command = record
            .get("command")
            .and_then(Value::as_str)
            .and_then(known_command)
            .map(str::to_owned)
            .or_else(|| {
                (direction == TransportDirection::Outbound)
                    .then(|| known_command(&record_type).map(str::to_owned))
                    .flatten()
            });
        let byte_size = json_byte_size(record);
        Self {
            at_millis: super::milliseconds_now(),
            direction,
            record_type,
            command,
            has_id: record.get("id").is_some_and(|value| !value.is_null()),
            byte_size,
            preview: safe_record_preview(record, byte_size),
        }
    }

    fn retained_bytes(&self) -> usize {
        self.record_type.len()
            + self.command.as_ref().map_or(0, String::len)
            + self.preview.as_ref().map_or(0, String::len)
    }
}

#[derive(Default)]
pub(super) struct TransportDiagnostics {
    pub(super) entries: VecDeque<TransportDiagnostic>,
    pub(super) retained_bytes: usize,
}

impl TransportDiagnostics {
    pub(super) fn record(&mut self, direction: TransportDirection, record: &Value) {
        self.push(TransportDiagnostic::from_record(direction, record));
    }

    pub(super) fn push(&mut self, entry: TransportDiagnostic) {
        self.retained_bytes = self.retained_bytes.saturating_add(entry.retained_bytes());
        self.entries.push_back(entry);
        while self.entries.len() > TRANSPORT_DIAGNOSTIC_MAX_ENTRIES
            || self.retained_bytes > TRANSPORT_DIAGNOSTIC_MAX_BYTES
        {
            let Some(removed) = self.entries.pop_front() else {
                break;
            };
            self.retained_bytes = self.retained_bytes.saturating_sub(removed.retained_bytes());
        }
    }
}

#[derive(Default)]
struct ByteCounter(usize);

impl std::io::Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Compact JSON bytes plus the newline used by the SSH JSON-lines pipe.
fn json_byte_size(record: &Value) -> usize {
    let mut counter = ByteCounter::default();
    if serde_json::to_writer(&mut counter, record).is_err() {
        return 0;
    }
    counter.0.saturating_add(1)
}

#[derive(Debug)]
struct SanitizationBudget {
    remaining_entries: usize,
    remaining_json_bytes: usize,
    dropped_entries: usize,
    encoded_json_omitted: bool,
}

impl Default for SanitizationBudget {
    fn default() -> Self {
        Self {
            remaining_entries: SUMMARY_ENTRY_BUDGET,
            remaining_json_bytes: STRINGIFIED_JSON_BYTE_BUDGET,
            dropped_entries: 0,
            encoded_json_omitted: false,
        }
    }
}

impl SanitizationBudget {
    fn inspect(&mut self, entries: usize) -> usize {
        let inspected = entries.min(self.remaining_entries);
        self.remaining_entries -= inspected;
        self.dropped_entries = self
            .dropped_entries
            .saturating_add(entries.saturating_sub(inspected));
        inspected
    }
}

enum DecodedStorage<'a> {
    Borrowed(&'a Value),
    Parsed(Value),
}

struct DecodedValue<'a> {
    storage: DecodedStorage<'a>,
    encoded_bytes: Option<usize>,
    layers: usize,
}

impl DecodedValue<'_> {
    fn value(&self) -> &Value {
        match &self.storage {
            DecodedStorage::Borrowed(value) => value,
            DecodedStorage::Parsed(value) => value,
        }
    }

    fn annotate(&self, summary: &mut Map<String, Value>) {
        if let Some(bytes) = self.encoded_bytes {
            summary.insert("encoding".into(), Value::String("json_string".into()));
            summary.insert("encoded_bytes".into(), json!(bytes));
            if self.layers > 1 {
                summary.insert("encoding_layers".into(), json!(self.layers));
            }
        }
    }
}

fn decode_nested<'a>(value: &'a Value, budget: &mut SanitizationBudget) -> DecodedValue<'a> {
    let Some(text) = value.as_str() else {
        return DecodedValue {
            storage: DecodedStorage::Borrowed(value),
            encoded_bytes: None,
            layers: 0,
        };
    };
    let encoded_bytes = text.len();
    if let Some((parsed, layers)) = parse_stringified(text, budget, MAX_DECODE_LAYERS) {
        DecodedValue {
            storage: DecodedStorage::Parsed(parsed),
            encoded_bytes: Some(encoded_bytes),
            layers,
        }
    } else {
        DecodedValue {
            storage: DecodedStorage::Borrowed(value),
            encoded_bytes: None,
            layers: 0,
        }
    }
}

fn parse_stringified(
    text: &str,
    budget: &mut SanitizationBudget,
    layers_left: usize,
) -> Option<(Value, usize)> {
    if layers_left == 0 {
        return None;
    }
    let first = text.as_bytes().first().copied()?;
    let possible_json = matches!(first, b'{' | b'[' | b'"' | b' ' | b'\n' | b'\r' | b'\t');
    if !possible_json {
        return None;
    }
    if text.len() > MAX_STRINGIFIED_JSON_BYTES || text.len() > budget.remaining_json_bytes {
        budget.encoded_json_omitted = true;
        return None;
    }
    budget.remaining_json_bytes -= text.len();
    let trimmed = text.trim();
    if !matches!(
        trimmed.as_bytes().first(),
        Some(b'{') | Some(b'[') | Some(b'"')
    ) {
        return None;
    }
    let parsed: Value = serde_json::from_str(trimmed).ok()?;
    if let Some(inner) = parsed.as_str()
        && let Some((nested, layers)) = parse_stringified(inner, budget, layers_left - 1)
    {
        return Some((nested, layers + 1));
    }
    matches!(parsed, Value::Object(_) | Value::Array(_)).then_some((parsed, 1))
}

fn known_record_kind(kind: Option<&str>) -> &'static str {
    match kind.unwrap_or("") {
        "remote_attach" => "remote_attach",
        "remote_hello" => "remote_hello",
        "remote_snapshot" => "remote_snapshot",
        "remote_error" => "remote_error",
        "remote_session_deleted" => "remote_session_deleted",
        "response" => "response",
        "state_update" => "state_update",
        "stats_update" => "stats_update",
        "event" => "event",
        "remote_event" => "remote_event",
        "prompt" => "prompt",
        "cancel_submission" => "cancel_submission",
        "get_state" => "get_state",
        "get_backend_info" => "get_backend_info",
        "get_available_models" => "get_available_models",
        "get_available_thinking_levels" => "get_available_thinking_levels",
        "get_commands" => "get_commands",
        "compact" => "compact",
        "set_model" => "set_model",
        "set_thinking_level" => "set_thinking_level",
        "clear_queue" => "clear_queue",
        "abort" => "abort",
        "get_session_stats" => "get_session_stats",
        "get_subagent" => "get_subagent",
        "stop_subagent" => "stop_subagent",
        "get_image" => "get_image",
        "extension_ui_response" => "extension_ui_response",
        "remote_delete_session" => "remote_delete_session",
        "remote_shutdown" => "remote_shutdown",
        "agent_start" => "agent_start",
        "agent_settled" => "agent_settled",
        "agent_end" => "agent_end",
        "compaction_start" => "compaction_start",
        "compaction_end" => "compaction_end",
        "auto_retry_start" => "auto_retry_start",
        "auto_retry_end" => "auto_retry_end",
        "summarization_retry_scheduled" => "summarization_retry_scheduled",
        "message_start" => "message_start",
        "message_update" => "message_update",
        "message_end" => "message_end",
        "entry_appended" => "entry_appended",
        "tool_execution_start" => "tool_execution_start",
        "tool_execution_update" => "tool_execution_update",
        "tool_execution_end" => "tool_execution_end",
        "queue_update" => "queue_update",
        "session_info_changed" => "session_info_changed",
        "thinking_level_changed" => "thinking_level_changed",
        "extension_error" => "extension_error",
        "extension_ui_request" => "extension_ui_request",
        "extension_ui_cancel" => "extension_ui_cancel",
        "bash_execution_update" => "bash_execution_update",
        _ => "unknown",
    }
}

fn known_command(command: &str) -> Option<&'static str> {
    Some(match command {
        "prompt" => "prompt",
        "cancel_submission" => "cancel_submission",
        "get_state" => "get_state",
        "get_backend_info" => "get_backend_info",
        "get_available_models" => "get_available_models",
        "get_available_thinking_levels" => "get_available_thinking_levels",
        "get_commands" => "get_commands",
        "compact" => "compact",
        "set_model" => "set_model",
        "set_thinking_level" => "set_thinking_level",
        "clear_queue" => "clear_queue",
        "abort" => "abort",
        "get_session_stats" => "get_session_stats",
        "get_subagent" => "get_subagent",
        "stop_subagent" => "stop_subagent",
        "get_image" => "get_image",
        "extension_ui_response" => "extension_ui_response",
        "remote_delete_session" => "remote_delete_session",
        "remote_shutdown" => "remote_shutdown",
        "bash" => "bash",
        "get_active_tools" => "get_active_tools",
        "get_entries" => "get_entries",
        "get_settings" => "get_settings",
        "list_sessions" => "list_sessions",
        "get_messages" => "get_messages",
        "get_auth_providers" => "get_auth_providers",
        "list_packages" => "list_packages",
        "get_project_trust" => "get_project_trust",
        "cycle_model" => "cycle_model",
        "cycle_thinking_level" => "cycle_thinking_level",
        "get_custom_entries" => "get_custom_entries",
        "append_custom_entry" => "append_custom_entry",
        _ => return None,
    })
}

fn known_event_kind(kind: Option<&str>) -> &'static str {
    match kind.unwrap_or("") {
        "message" => "message",
        "block" => "block",
        "text_start" => "text_start",
        "text_delta" => "text_delta",
        "text_end" => "text_end",
        "thinking_start" => "thinking_start",
        "thinking_delta" => "thinking_delta",
        "thinking_end" => "thinking_end",
        "toolcall_start" => "toolcall_start",
        "toolcall_delta" => "toolcall_delta",
        "toolcall_end" => "toolcall_end",
        _ => "unknown",
    }
}

fn known_version(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(number)) => number.as_u64().filter(|value| matches!(value, 1 | 2)),
        Some(Value::String(value)) if matches!(value.as_str(), "1" | "2") => value.parse().ok(),
        _ => None,
    }
}

fn object_field<'a>(value: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| value.get(*name))
}

fn decoded_array_len(value: Option<&Value>, budget: &mut SanitizationBudget) -> usize {
    let Some(value) = value else {
        return 0;
    };
    let decoded = decode_nested(value, budget);
    decoded.value().as_array().map_or(0, Vec::len)
}

fn value_shape(value: &Value, budget: &mut SanitizationBudget) -> Value {
    let decoded = decode_nested(value, budget);
    let value = decoded.value();
    let mut summary = Map::new();
    match value {
        Value::Null => summary.insert("shape".into(), Value::String("null".into())),
        Value::Bool(_) | Value::Number(_) => {
            summary.insert("shape".into(), Value::String("scalar".into()))
        }
        Value::String(text) => {
            summary.insert("shape".into(), Value::String("string".into()));
            summary.insert("bytes".into(), json!(text.len()))
        }
        Value::Array(values) => {
            summary.insert("shape".into(), Value::String("array".into()));
            summary.insert("items".into(), json!(values.len()))
        }
        Value::Object(values) => {
            summary.insert("shape".into(), Value::String("object".into()));
            summary.insert("fields".into(), json!(values.len()))
        }
    };
    decoded.annotate(&mut summary);
    Value::Object(summary)
}

fn known_role(value: Option<&str>) -> &'static str {
    match value.unwrap_or("") {
        "user" => "user",
        "assistant" => "assistant",
        "toolResult" | "tool_result" => "tool_result",
        "system" => "system",
        "custom" => "custom",
        _ => "other",
    }
}

fn known_block_kind(value: Option<&str>) -> &'static str {
    match value.unwrap_or("") {
        "text" => "text",
        "thinking" | "redactedThinking" | "redacted_thinking" => "thinking",
        "toolCall" | "tool_call" => "tool_call",
        "image" => "image",
        _ => "other",
    }
}

fn increment(map: &mut Map<String, Value>, key: &'static str) {
    let next = map
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .saturating_add(1);
    map.insert(key.into(), json!(next));
}

#[derive(Default)]
struct MessageCounts {
    total: usize,
    roles: Map<String, Value>,
    observed_blocks: usize,
    block_kinds: Map<String, Value>,
    text_bytes: usize,
    image_encoded_bytes: usize,
}

impl MessageCounts {
    fn value(self) -> Value {
        let mut summary = Map::new();
        summary.insert("count".into(), json!(self.total));
        if !self.roles.is_empty() {
            summary.insert("roles".into(), Value::Object(self.roles));
        }
        if self.observed_blocks > 0 {
            summary.insert("observed_blocks".into(), json!(self.observed_blocks));
        }
        if !self.block_kinds.is_empty() {
            summary.insert("block_kinds".into(), Value::Object(self.block_kinds));
        }
        if self.text_bytes > 0 {
            summary.insert("text_bytes".into(), json!(self.text_bytes));
        }
        if self.image_encoded_bytes > 0 {
            summary.insert(
                "image_encoded_bytes".into(),
                json!(self.image_encoded_bytes),
            );
        }
        Value::Object(summary)
    }
}

fn inspect_block(value: &Value, counts: &mut MessageCounts, budget: &mut SanitizationBudget) {
    let decoded = decode_nested(value, budget);
    let value = decoded.value();
    if let Some(text) = value.as_str() {
        counts.observed_blocks = counts.observed_blocks.saturating_add(1);
        increment(&mut counts.block_kinds, "text");
        counts.text_bytes = counts.text_bytes.saturating_add(text.len());
        return;
    }
    let Some(object) = value.as_object() else {
        counts.observed_blocks = counts.observed_blocks.saturating_add(1);
        increment(&mut counts.block_kinds, "other");
        return;
    };
    let kind = known_block_kind(object.get("type").and_then(Value::as_str));
    counts.observed_blocks = counts.observed_blocks.saturating_add(1);
    increment(&mut counts.block_kinds, kind);
    match kind {
        "text" => {
            counts.text_bytes = counts.text_bytes.saturating_add(
                object
                    .get("text")
                    .and_then(Value::as_str)
                    .map_or(0, str::len),
            );
        }
        "thinking" => {
            counts.text_bytes = counts.text_bytes.saturating_add(
                object
                    .get("thinking")
                    .or_else(|| object.get("text"))
                    .and_then(Value::as_str)
                    .map_or(0, str::len),
            );
        }
        "image" => {
            counts.image_encoded_bytes = counts.image_encoded_bytes.saturating_add(
                object
                    .get("data")
                    .and_then(Value::as_str)
                    .map_or(0, str::len),
            );
        }
        _ => {}
    }
}

fn inspect_content(value: &Value, counts: &mut MessageCounts, budget: &mut SanitizationBudget) {
    let decoded = decode_nested(value, budget);
    match decoded.value() {
        Value::String(text) => {
            counts.observed_blocks = counts.observed_blocks.saturating_add(1);
            increment(&mut counts.block_kinds, "text");
            counts.text_bytes = counts.text_bytes.saturating_add(text.len());
        }
        Value::Array(blocks) => {
            let inspected = budget.inspect(blocks.len());
            for block in blocks.iter().take(inspected) {
                inspect_block(block, counts, budget);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => {}
    }
}

fn inspect_message(value: &Value, counts: &mut MessageCounts, budget: &mut SanitizationBudget) {
    let decoded = decode_nested(value, budget);
    let Some(object) = decoded.value().as_object() else {
        increment(&mut counts.roles, "other");
        return;
    };
    increment(
        &mut counts.roles,
        known_role(object.get("role").and_then(Value::as_str)),
    );
    if let Some(content) = object.get("content") {
        inspect_content(content, counts, budget);
    }
}

fn summarize_messages(value: &Value, budget: &mut SanitizationBudget) -> Value {
    let decoded = decode_nested(value, budget);
    let mut counts = MessageCounts::default();
    match decoded.value() {
        Value::Array(messages) => {
            counts.total = messages.len();
            let inspected = budget.inspect(messages.len());
            for message in messages.iter().take(inspected) {
                inspect_message(message, &mut counts, budget);
            }
        }
        Value::Object(_) => {
            counts.total = 1;
            inspect_message(decoded.value(), &mut counts, budget);
        }
        _ => return value_shape(decoded.value(), budget),
    }
    let mut summary = counts.value();
    if let Value::Object(summary) = &mut summary {
        decoded.annotate(summary);
    }
    summary
}

fn known_tool_kind(value: Option<&str>) -> &'static str {
    match value.unwrap_or("") {
        "read" => "read",
        "write" => "write",
        "edit" => "edit",
        "bash" => "bash",
        "grep" => "grep",
        "find" => "find",
        "ls" => "list",
        "subagent" => "subagent",
        _ => "other",
    }
}

fn summarize_tools(value: &Value, budget: &mut SanitizationBudget) -> Value {
    let decoded = decode_nested(value, budget);
    let Some(tools) = decoded.value().as_array() else {
        return value_shape(decoded.value(), budget);
    };
    let mut states = Map::new();
    let mut kinds = Map::new();
    let mut images = 0usize;
    let mut output_bytes = 0usize;
    let mut argument_fields = 0usize;
    let mut with_diff = 0usize;
    let inspected = budget.inspect(tools.len());
    for tool in tools.iter().take(inspected) {
        let tool = decode_nested(tool, budget);
        let Some(tool) = tool.value().as_object() else {
            increment(&mut kinds, "other");
            continue;
        };
        increment(
            &mut kinds,
            known_tool_kind(tool.get("name").and_then(Value::as_str)),
        );
        if tool.get("is_error").or_else(|| tool.get("isError")) == Some(&Value::Bool(true)) {
            increment(&mut states, "error");
        } else if tool.get("finished") == Some(&Value::Bool(true)) {
            increment(&mut states, "finished");
        } else {
            increment(&mut states, "running");
        }
        images = images.saturating_add(decoded_array_len(tool.get("images"), budget));
        output_bytes = output_bytes.saturating_add(
            tool.get("output")
                .and_then(Value::as_str)
                .map_or(0, str::len),
        );
        argument_fields = argument_fields.saturating_add(
            tool.get("args")
                .or_else(|| tool.get("arguments"))
                .and_then(Value::as_object)
                .map_or(0, Map::len),
        );
        with_diff = with_diff.saturating_add(usize::from(
            tool.get("diff").is_some_and(|value| !value.is_null()),
        ));
    }
    let mut summary = Map::new();
    summary.insert("count".into(), json!(tools.len()));
    if !states.is_empty() {
        summary.insert("states".into(), Value::Object(states));
    }
    if !kinds.is_empty() {
        summary.insert("kinds".into(), Value::Object(kinds));
    }
    if images > 0 {
        summary.insert("images".into(), json!(images));
    }
    if output_bytes > 0 {
        summary.insert("output_bytes".into(), json!(output_bytes));
    }
    if argument_fields > 0 {
        summary.insert("argument_fields".into(), json!(argument_fields));
    }
    if with_diff > 0 {
        summary.insert("with_diff".into(), json!(with_diff));
    }
    decoded.annotate(&mut summary);
    Value::Object(summary)
}

fn known_run(value: Option<&str>) -> &'static str {
    match value.unwrap_or("") {
        "Idle" | "idle" => "idle",
        "Running" | "running" => "running",
        "Compacting" | "compacting" => "compacting",
        "Retrying" | "retrying" => "retrying",
        _ => "unknown",
    }
}

fn summarize_state(value: &Value, budget: &mut SanitizationBudget) -> Value {
    let decoded = decode_nested(value, budget);
    let Some(state) = decoded.value().as_object() else {
        return value_shape(decoded.value(), budget);
    };
    let compacting = object_field(decoded.value(), &["is_compacting", "isCompacting"])
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let streaming = object_field(decoded.value(), &["is_streaming", "isStreaming"])
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let bash = object_field(decoded.value(), &["is_bash_running", "isBashRunning"])
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let phase = if compacting {
        "compacting"
    } else if streaming {
        "streaming"
    } else if bash {
        "bash_running"
    } else {
        "idle"
    };
    let mut summary = Map::new();
    summary.insert("phase".into(), Value::String(phase.into()));
    let active_tools = decoded_array_len(
        object_field(decoded.value(), &["active_tools", "activeTools"]),
        budget,
    );
    if active_tools > 0 || state.contains_key("active_tools") || state.contains_key("activeTools") {
        summary.insert("active_tools".into(), json!(active_tools));
    }
    decoded.annotate(&mut summary);
    Value::Object(summary)
}

fn summarize_stats(value: &Value, budget: &mut SanitizationBudget) -> Value {
    let decoded = decode_nested(value, budget);
    let Some(stats) = decoded.value().as_object() else {
        return value_shape(decoded.value(), budget);
    };
    let mut summary = Map::new();
    if let Some(total) =
        object_field(decoded.value(), &["total_messages", "totalMessages"]).and_then(Value::as_u64)
    {
        summary.insert("total_messages".into(), json!(total));
    }
    if let Some(tokens) = stats.get("tokens") {
        let tokens = decode_nested(tokens, budget);
        let mut token_summary = Map::new();
        for (shown, names) in [
            ("input", ["input", "input"]),
            ("output", ["output", "output"]),
            ("cache_read", ["cache_read", "cacheRead"]),
            ("cache_write", ["cache_write", "cacheWrite"]),
        ] {
            if let Some(value) = object_field(tokens.value(), &names).and_then(Value::as_u64) {
                token_summary.insert(shown.into(), json!(value));
            }
        }
        if !token_summary.is_empty() {
            tokens.annotate(&mut token_summary);
            summary.insert("tokens".into(), Value::Object(token_summary));
        }
    }
    if let Some(context) = object_field(decoded.value(), &["context_usage", "contextUsage"])
        && !context.is_null()
    {
        let context = decode_nested(context, budget);
        let mut context_summary = Map::new();
        if let Some(tokens) = context.value().get("tokens").and_then(Value::as_u64) {
            context_summary.insert("tokens".into(), json!(tokens));
        }
        if let Some(window) = object_field(context.value(), &["context_window", "contextWindow"])
            .and_then(Value::as_u64)
        {
            context_summary.insert("window".into(), json!(window));
        }
        context.annotate(&mut context_summary);
        summary.insert("context".into(), Value::Object(context_summary));
    }
    decoded.annotate(&mut summary);
    Value::Object(summary)
}

fn looks_like_session(value: &Value) -> bool {
    [
        "run",
        "state",
        "messages",
        "tools",
        "queued_submissions",
        "queuedSubmissions",
        "steering",
        "follow_up",
        "followUp",
    ]
    .iter()
    .any(|key| value.get(*key).is_some())
}

fn summarize_snapshot(value: &Value, budget: &mut SanitizationBudget, depth: usize) -> Value {
    let decoded = decode_nested(value, budget);
    let value = decoded.value();
    if depth < MAX_NESTED_SUMMARY_DEPTH && !looks_like_session(value) {
        for key in ["snapshot", "session", "data", "payload"] {
            if let Some(nested) = value.get(key) {
                let mut summary = summarize_snapshot(nested, budget, depth + 1);
                if let Value::Object(summary) = &mut summary {
                    decoded.annotate(summary);
                }
                return summary;
            }
        }
    }
    let Some(object) = value.as_object() else {
        return value_shape(value, budget);
    };
    let mut summary = Map::new();
    if object.contains_key("run") {
        summary.insert(
            "run".into(),
            Value::String(known_run(object.get("run").and_then(Value::as_str)).into()),
        );
    }
    if let Some(state) = object.get("state") {
        summary.insert("state".into(), summarize_state(state, budget));
    }
    if let Some(messages) = object.get("messages") {
        summary.insert("messages".into(), summarize_messages(messages, budget));
    } else {
        summary.insert("messages".into(), json!({"count":0}));
    }
    if let Some(tools) = object.get("tools") {
        summary.insert("tools".into(), summarize_tools(tools, budget));
    } else {
        summary.insert("tools".into(), json!({"count":0}));
    }
    let mut queues = Map::new();
    queues.insert(
        "steering".into(),
        json!(decoded_array_len(object.get("steering"), budget)),
    );
    queues.insert(
        "follow_up".into(),
        json!(decoded_array_len(
            object_field(value, &["follow_up", "followUp"]),
            budget
        )),
    );
    queues.insert(
        "queued_submissions".into(),
        json!(decoded_array_len(
            object_field(value, &["queued_submissions", "queuedSubmissions"]),
            budget
        )),
    );
    summary.insert("queues".into(), Value::Object(queues));
    decoded.annotate(&mut summary);
    Value::Object(summary)
}

fn known_path_field(segment: &str) -> &'static str {
    match segment {
        "messages" => "messages",
        "message" => "message",
        "content" => "content",
        "arguments" | "args" => "arguments",
        "assistantMessageEvent" | "assistant_message_event" => "event",
        "data" => "data",
        "result" => "result",
        "partialResult" | "partial_result" => "partial_result",
        "details" => "details",
        "text" => "text",
        "thinking" => "thinking",
        "delta" => "delta",
        "block" => "block",
        "state" => "state",
        "tools" => "tools",
        "entries" => "entries",
        "queuedSubmissions" | "queued_submissions" => "queued_submissions",
        "steering" => "steering",
        "followUp" | "follow_up" => "follow_up",
        _ => "field",
    }
}

fn path_segment(value: &Value) -> &'static str {
    match value {
        Value::Number(_) => "[]",
        Value::String(value)
            if value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            "[]"
        }
        Value::String(value) if value.len() <= 64 => known_path_field(value),
        Value::String(_) => "field",
        _ => "value",
    }
}

fn bounded_prefix(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    &text[..end]
}

fn path_shape(value: &Value, budget: &mut SanitizationBudget) -> Option<String> {
    let decoded = decode_nested(value, budget);
    match decoded.value() {
        Value::Array(segments) => {
            let inspected = budget.inspect(segments.len().min(MAX_PATH_SEGMENTS));
            let mut shape = segments
                .iter()
                .take(inspected)
                .map(path_segment)
                .collect::<Vec<_>>();
            if segments.len() > MAX_PATH_SEGMENTS {
                budget.dropped_entries = budget
                    .dropped_entries
                    .saturating_add(segments.len() - MAX_PATH_SEGMENTS);
                shape.push("more");
            }
            (!shape.is_empty()).then(|| shape.join("/"))
        }
        Value::String(path) => {
            let path = bounded_prefix(path, MAX_PATH_BYTES);
            let separator = if path.contains('/') { '/' } else { '.' };
            let mut shape = Vec::new();
            for segment in path
                .split(separator)
                .filter(|segment| !segment.is_empty())
                .take(MAX_PATH_SEGMENTS + 1)
            {
                if shape.len() == MAX_PATH_SEGMENTS {
                    shape.push("more");
                    break;
                }
                shape.push(if segment.parse::<usize>().is_ok() {
                    "[]"
                } else {
                    known_path_field(segment)
                });
            }
            (!shape.is_empty()).then(|| shape.join("/"))
        }
        _ => None,
    }
}

fn known_method(value: Option<&str>) -> &'static str {
    match value.unwrap_or("") {
        "setStatus" => "set_status",
        "notify" => "notify",
        "confirm" => "confirm",
        "select" => "select",
        "input" => "input",
        _ => "unknown",
    }
}

fn known_notify_type(value: Option<&str>) -> &'static str {
    match value.unwrap_or("") {
        "info" => "info",
        "warning" => "warning",
        "error" => "error",
        _ => "unknown",
    }
}

fn known_thinking_level(value: Option<&str>) -> &'static str {
    match value.unwrap_or("") {
        "off" => "off",
        "minimal" => "minimal",
        "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "xhigh" => "xhigh",
        _ => "unknown",
    }
}

fn summarize_message_event(value: &Value, budget: &mut SanitizationBudget) -> Value {
    let decoded = decode_nested(value, budget);
    let event = decoded.value();
    let mut summary = Map::new();
    summary.insert(
        "event_kind".into(),
        Value::String(known_event_kind(event.get("type").and_then(Value::as_str)).into()),
    );
    if let Some(index) = event.get("contentIndex").and_then(Value::as_u64) {
        summary.insert("content_index".into(), json!(index.min(4097)));
        if index > 4096 {
            summary.insert("content_index_limited".into(), Value::Bool(true));
        }
    }
    if let Some(delta) = event.get("delta").and_then(Value::as_str) {
        summary.insert("delta_bytes".into(), json!(delta.len()));
    }
    if let Some(content) = event.get("content").and_then(Value::as_str) {
        summary.insert("content_bytes".into(), json!(content.len()));
    }
    if let Some(path) = event.get("path").and_then(|path| path_shape(path, budget)) {
        summary.insert("path_shape".into(), Value::String(path));
    }
    if let Some(message) = event.get("message") {
        summary.insert("message".into(), summarize_messages(message, budget));
    }
    if let Some(block) = event.get("block").or_else(|| event.get("toolCall")) {
        let mut counts = MessageCounts::default();
        inspect_block(block, &mut counts, budget);
        summary.insert("block".into(), counts.value());
    }
    decoded.annotate(&mut summary);
    Value::Object(summary)
}

fn summarize_tool_event(kind: &str, record: &Value, budget: &mut SanitizationBudget) -> Value {
    let mut summary = Map::new();
    summary.insert(
        "phase".into(),
        Value::String(
            match kind {
                "tool_execution_start" => "start",
                "tool_execution_update" => "update",
                "tool_execution_end" => "end",
                _ => "unknown",
            }
            .into(),
        ),
    );
    summary.insert(
        "tool_kind".into(),
        Value::String(known_tool_kind(record.get("toolName").and_then(Value::as_str)).into()),
    );
    summary.insert(
        "has_tool_id".into(),
        Value::Bool(
            record
                .get("toolCallId")
                .is_some_and(|value| !value.is_null()),
        ),
    );
    if let Some(is_error) = record.get("isError").and_then(Value::as_bool) {
        summary.insert("is_error".into(), Value::Bool(is_error));
    }
    if let Some(args) = record.get("args") {
        summary.insert("arguments".into(), value_shape(args, budget));
    }
    if let Some(result) = record.get("partialResult").or_else(|| record.get("result")) {
        let decoded = decode_nested(result, budget);
        let mut result_summary = Map::new();
        if let Some(content) = decoded.value().get("content") {
            let mut counts = MessageCounts::default();
            inspect_content(content, &mut counts, budget);
            result_summary.insert("content".into(), counts.value());
        } else {
            result_summary.insert("value".into(), value_shape(decoded.value(), budget));
        }
        decoded.annotate(&mut result_summary);
        summary.insert("result".into(), Value::Object(result_summary));
    }
    Value::Object(summary)
}

fn summarize_queue(record: &Value, budget: &mut SanitizationBudget) -> Value {
    json!({
        "steering": decoded_array_len(record.get("steering"), budget),
        "follow_up": decoded_array_len(object_field(record, &["followUp", "follow_up"]), budget),
        "queued_submissions": decoded_array_len(object_field(record, &["queuedSubmissions", "queued_submissions"]), budget),
    })
}

fn summarize_response(record: &Value, budget: &mut SanitizationBudget) -> Value {
    let command = record
        .get("command")
        .and_then(Value::as_str)
        .and_then(known_command)
        .unwrap_or("unknown");
    let mut summary = Map::new();
    if let Some(success) = record.get("success").and_then(Value::as_bool) {
        summary.insert("success".into(), Value::Bool(success));
    }
    if record.get("error").is_some_and(|value| !value.is_null()) {
        summary.insert("has_error".into(), Value::Bool(true));
    }
    let Some(data) = record.get("data") else {
        return Value::Object(summary);
    };
    let decoded = decode_nested(data, budget);
    let data = decoded.value();
    let details = match command {
        "get_messages" => object_field(data, &["messages"])
            .map(|messages| summarize_messages(messages, budget))
            .unwrap_or_else(|| value_shape(data, budget)),
        "get_state" => summarize_state(data, budget),
        "get_subagent" => summarize_snapshot(data, budget, 0),
        "get_available_models" => json!({
            "models": decoded_array_len(data.get("models"), budget),
        }),
        "get_available_thinking_levels" => json!({
            "levels": decoded_array_len(data.get("levels"), budget),
        }),
        "get_commands" => json!({
            "commands": decoded_array_len(data.get("commands"), budget),
        }),
        "get_active_tools" => json!({
            "active_tools": decoded_array_len(object_field(data, &["activeTools", "active_tools"]), budget),
        }),
        "list_sessions" => json!({
            "sessions": decoded_array_len(data.get("sessions"), budget),
        }),
        "get_auth_providers" => json!({
            "providers": decoded_array_len(data.get("providers"), budget),
        }),
        "list_packages" => json!({
            "packages": decoded_array_len(data.get("packages"), budget),
        }),
        "get_image" => {
            let image = data.get("image").unwrap_or(data);
            json!({
                "images": usize::from(image.is_object()),
                "encoded_bytes": image.get("data").and_then(Value::as_str).map_or(0, str::len),
            })
        }
        "prompt" => json!({
            "has_submission_id": data.get("submissionId").or_else(|| data.get("submission_id")).is_some_and(|value| !value.is_null()),
            "disposition": match data.get("disposition").and_then(Value::as_str) {
                Some("queued") => "queued",
                Some("started") => "started",
                _ => "unknown",
            },
        }),
        "compact" => json!({
            "estimated_tokens_after": data.get("estimatedTokensAfter").and_then(Value::as_u64),
        }),
        _ => value_shape(data, budget),
    };
    summary.insert("data".into(), details);
    decoded.annotate(&mut summary);
    Value::Object(summary)
}

fn summarize_nested_event(value: &Value, budget: &mut SanitizationBudget, depth: usize) -> Value {
    let decoded = decode_nested(value, budget);
    let value = decoded.value();
    if depth < MAX_NESTED_SUMMARY_DEPTH {
        for key in ["event", "record", "data", "payload"] {
            if value.get("type").is_none()
                && let Some(nested) = value.get(key)
            {
                let mut summary = summarize_nested_event(nested, budget, depth + 1);
                if let Value::Object(summary) = &mut summary {
                    decoded.annotate(summary);
                }
                return summary;
            }
        }
    }
    let kind = known_record_kind(value.get("type").and_then(Value::as_str));
    let mut summary = Map::new();
    summary.insert("record_kind".into(), Value::String(kind.into()));
    if kind != "unknown"
        && let Some(details) = summarize_record_details(kind, value, budget, depth + 1)
    {
        summary.insert("summary".into(), details);
    } else {
        summary.insert("value".into(), value_shape(value, budget));
    }
    decoded.annotate(&mut summary);
    Value::Object(summary)
}

fn summarize_record_details(
    kind: &str,
    record: &Value,
    budget: &mut SanitizationBudget,
    depth: usize,
) -> Option<Value> {
    Some(match kind {
        "remote_snapshot" => summarize_snapshot(record.get("data")?, budget, 0),
        "response" => summarize_response(record, budget),
        "state_update" => summarize_state(record.get("state")?, budget),
        "stats_update" => summarize_stats(record.get("stats")?, budget),
        "message_start" | "message_end" => summarize_messages(record.get("message")?, budget),
        "message_update" => summarize_message_event(record.get("assistantMessageEvent")?, budget),
        "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
            summarize_tool_event(kind, record, budget)
        }
        "queue_update" => summarize_queue(record, budget),
        "bash_execution_update" => json!({
            "delta_bytes": record.get("delta").and_then(Value::as_str).map_or(0, str::len),
            "has_id": record.get("id").is_some_and(|value| !value.is_null()),
        }),
        "compaction_end" => json!({
            "has_result": record.get("result").is_some_and(Value::is_object),
            "has_error": record.get("errorMessage").is_some_and(|value| !value.is_null()),
        }),
        "auto_retry_start" | "auto_retry_end" | "summarization_retry_scheduled" => json!({
            "attempt": record.get("attempt").and_then(Value::as_u64),
            "has_error": record.get("finalError").or_else(|| record.get("error")).is_some_and(|value| !value.is_null()),
        }),
        "entry_appended" => {
            let entry = record.get("entry")?;
            json!({
                "entry_kind": match entry.get("type").and_then(Value::as_str) {
                    Some("custom_message") => "custom_message",
                    Some("message") => "message",
                    _ => "unknown",
                },
                "content": entry.get("content").map(|value| value_shape(value, budget)),
            })
        }
        "extension_ui_request" => json!({
            "method": known_method(record.get("method").and_then(Value::as_str)),
            "notify_type": known_notify_type(record.get("notifyType").and_then(Value::as_str)),
            "options": decoded_array_len(record.get("options"), budget),
            "has_id": record.get("id").is_some_and(|value| !value.is_null()),
        }),
        "thinking_level_changed" | "set_thinking_level" => json!({
            "level": known_thinking_level(record.get("level").and_then(Value::as_str)),
        }),
        "session_info_changed" => json!({
            "has_name": record.get("name").is_some_and(|value| !value.is_null()),
        }),
        "extension_error" | "remote_error" => json!({"has_error":true}),
        "event" | "remote_event" if depth < MAX_NESTED_SUMMARY_DEPTH => {
            let nested = record
                .get("event")
                .or_else(|| record.get("data"))
                .or_else(|| record.get("record"))?;
            summarize_nested_event(nested, budget, depth + 1)
        }
        "prompt" => json!({
            "message_bytes": record.get("message").and_then(Value::as_str).map_or(0, str::len),
            "images": decoded_array_len(record.get("images"), budget),
        }),
        "compact" => json!({
            "has_custom_instructions": record.get("customInstructions").is_some_and(|value| !value.is_null()),
        }),
        "remote_attach" => json!({
            "target": record.get("target").map(|value| value_shape(value, budget)),
        }),
        _ => {
            let mut generic = Map::new();
            if let Some(path) = record.get("path").and_then(|path| path_shape(path, budget)) {
                generic.insert("path_shape".into(), Value::String(path));
            }
            if let Some(data) = record.get("data") {
                generic.insert("data".into(), value_shape(data, budget));
            }
            if generic.is_empty() {
                return None;
            }
            Value::Object(generic)
        }
    })
}

/// Keep only protocol structure from a strict allowlist. The result is always
/// valid JSON; if the fixed-size display budget is unexpectedly exceeded, a
/// compact marker replaces details rather than truncating into invalid JSON.
fn safe_record_preview(record: &Value, wire_bytes: usize) -> Option<String> {
    let object = record.as_object()?;
    let kind = known_record_kind(object.get("type").and_then(Value::as_str));
    let mut budget = SanitizationBudget::default();
    let mut safe = Map::new();
    safe.insert("kind".into(), Value::String(kind.into()));
    safe.insert("wire_bytes".into(), json!(wire_bytes));
    if let Some(version) = known_version(object.get("version")) {
        safe.insert("version".into(), json!(version));
    }
    if let Some(command) = object
        .get("command")
        .and_then(Value::as_str)
        .and_then(known_command)
    {
        safe.insert("command".into(), Value::String(command.into()));
    }
    if let Some(success) = object.get("success").and_then(Value::as_bool) {
        safe.insert("success".into(), Value::Bool(success));
    }
    if object.get("id").is_some_and(|value| !value.is_null()) {
        safe.insert("has_id".into(), Value::Bool(true));
    }
    if let Some(summary) = summarize_record_details(kind, record, &mut budget, 0) {
        safe.insert("summary".into(), summary);
    }
    safe.insert("omitted".into(), json!(OMITTED_CATEGORIES));
    if budget.dropped_entries > 0 {
        safe.insert("dropped_entries".into(), json!(budget.dropped_entries));
    }
    if budget.encoded_json_omitted {
        safe.insert("summary_limited".into(), Value::Bool(true));
    }
    let preview = Value::Object(safe).to_string();
    if preview.len() <= TRANSPORT_DIAGNOSTIC_PREVIEW_BYTES {
        return Some(preview);
    }
    Some(
        json!({
            "kind": kind,
            "wire_bytes": wire_bytes,
            "summary_limited": true,
            "dropped_entries": budget.dropped_entries,
            "omitted": OMITTED_CATEGORIES,
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(record: &Value) -> (TransportDiagnostic, Value) {
        let diagnostic = TransportDiagnostic::from_record(TransportDirection::Inbound, record);
        let preview: Value = serde_json::from_str(diagnostic.preview.as_deref().unwrap()).unwrap();
        (diagnostic, preview)
    }

    #[test]
    fn stringified_snapshot_keeps_structure_and_never_content_or_identifiers() {
        let secret = "private transcript / credential / path";
        let snapshot = json!({
            "cwd": format!("/home/{secret}"),
            "run": "Running",
            "state": {"sessionId":secret,"sessionName":secret,"isStreaming":true,"activeTools":[{"name":secret}]},
            "messages": [
                {"role":"user","content":secret},
                {"role":"assistant","content":[
                    {"type":"text","text":secret},
                    {"type":"thinking","thinking":secret},
                    {"type":"toolCall","id":secret,"name":"read","arguments":{"path":secret}},
                    {"type":"image","data":"base64-secret","imageId":secret}
                ]},
                {"role":"toolResult","toolCallId":secret,"content":[{"type":"text","text":secret}],"isError":true},
                {"role":"custom","content":secret}
            ],
            "tools": [
                {"id":secret,"name":"read","args":{"path":secret},"output":secret,"diff":secret,"finished":true,"is_error":false,"images":[]},
                {"id":secret,"name":secret,"args":{"command":secret},"output":secret,"finished":false,"is_error":true,"images":[{"data":"base64-secret"}]}
            ],
            "steering":[secret],
            "follow_up":[secret],
            "queued_submissions":[secret,secret],
            "error":secret,
            "Authorization":secret
        });
        let record = json!({
            "type":"remote_snapshot",
            "version":2,
            "key":secret,
            "data":serde_json::to_string(&snapshot).unwrap()
        });
        let (diagnostic, preview) = preview(&record);

        assert_eq!(diagnostic.record_type, "remote_snapshot");
        assert_eq!(preview["kind"], "remote_snapshot");
        assert_eq!(preview["version"], 2);
        assert_eq!(preview["wire_bytes"], diagnostic.byte_size);
        assert_eq!(preview["summary"]["run"], "running");
        assert_eq!(preview["summary"]["state"]["phase"], "streaming");
        assert_eq!(preview["summary"]["state"]["active_tools"], 1);
        assert_eq!(preview["summary"]["messages"]["count"], 4);
        assert_eq!(preview["summary"]["messages"]["roles"]["user"], 1);
        assert_eq!(preview["summary"]["messages"]["roles"]["assistant"], 1);
        assert_eq!(preview["summary"]["messages"]["roles"]["tool_result"], 1);
        assert_eq!(preview["summary"]["messages"]["block_kinds"]["text"], 4);
        assert_eq!(preview["summary"]["messages"]["block_kinds"]["thinking"], 1);
        assert_eq!(
            preview["summary"]["messages"]["block_kinds"]["tool_call"],
            1
        );
        assert_eq!(preview["summary"]["messages"]["block_kinds"]["image"], 1);
        assert_eq!(preview["summary"]["tools"]["count"], 2);
        assert_eq!(preview["summary"]["tools"]["states"]["finished"], 1);
        assert_eq!(preview["summary"]["tools"]["states"]["error"], 1);
        assert_eq!(preview["summary"]["queues"]["queued_submissions"], 2);
        assert_eq!(preview["summary"]["encoding"], "json_string");
        assert!(
            preview["omitted"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "ids")
        );

        let rendered = diagnostic.preview.unwrap();
        for forbidden in [secret, "base64-secret", "sessionId", "Authorization"] {
            assert!(
                !rendered.contains(forbidden),
                "retained {forbidden:?}: {rendered}"
            );
        }
    }

    #[test]
    fn incremental_events_report_closed_kinds_path_shapes_counts_and_sizes() {
        let secret = "arbitrary-secret-field";
        let event = json!({
            "type":"toolcall_delta",
            "contentIndex":7,
            "path":["messages",3,"content",1,secret],
            "delta":"do not retain this tool argument"
        });
        let record = json!({
            "type":"message_update",
            "id":"raw-request-id",
            "assistantMessageEvent":serde_json::to_string(&event).unwrap(),
            "headers":{"Authorization":secret},
            "error":secret
        });
        let (diagnostic, preview) = preview(&record);

        assert!(diagnostic.has_id);
        assert_eq!(preview["summary"]["event_kind"], "toolcall_delta");
        assert_eq!(preview["summary"]["content_index"], 7);
        assert_eq!(preview["summary"]["delta_bytes"], 32);
        assert_eq!(
            preview["summary"]["path_shape"],
            "messages/[]/content/[]/field"
        );
        assert_eq!(preview["summary"]["encoding"], "json_string");
        let rendered = diagnostic.preview.unwrap();
        for forbidden in [secret, "raw-request-id", "do not retain", "Authorization"] {
            assert!(
                !rendered.contains(forbidden),
                "retained {forbidden:?}: {rendered}"
            );
        }
    }

    #[test]
    fn current_incremental_state_and_stats_updates_are_useful_without_identity() {
        let secret = "private-session-identifier";
        let state = json!({
            "type":"state_update",
            "state":serde_json::to_string(&json!({
                "sessionId":secret,
                "sessionName":secret,
                "isStreaming":true,
                "activeTools":[{"name":secret}]
            })).unwrap()
        });
        let (state_diagnostic, state_preview) = preview(&state);
        assert_eq!(state_diagnostic.record_type, "state_update");
        assert_eq!(state_preview["summary"]["phase"], "streaming");
        assert_eq!(state_preview["summary"]["active_tools"], 1);
        assert_eq!(state_preview["summary"]["encoding"], "json_string");
        assert!(!state_diagnostic.preview.unwrap().contains(secret));

        let stats = json!({
            "type":"stats_update",
            "stats":{"tokens":{"input":12,"output":3,"cacheRead":4,"cacheWrite":1},"contextUsage":{"tokens":500,"contextWindow":1000},"totalMessages":9}
        });
        let (_, stats_preview) = preview(&stats);
        assert_eq!(stats_preview["summary"]["tokens"]["input"], 12);
        assert_eq!(stats_preview["summary"]["tokens"]["cache_read"], 4);
        assert_eq!(stats_preview["summary"]["context"]["window"], 1000);
        assert_eq!(stats_preview["summary"]["total_messages"], 9);
    }

    #[test]
    fn unknown_scalars_and_ids_collapse_to_closed_vocabulary() {
        let secret = "ghp_arbitrary_secret_identifier";
        let record = json!({
            "type":secret,
            "command":secret,
            "id":secret,
            "path":format!("/private/{secret}/project/file.txt"),
            "data":{"unknown":secret,"token":secret,"nested":[secret]},
            "success":true
        });
        let (diagnostic, preview) = preview(&record);

        assert_eq!(diagnostic.record_type, "unknown");
        assert_eq!(diagnostic.command, None);
        assert!(diagnostic.has_id);
        assert_eq!(preview["kind"], "unknown");
        assert_eq!(preview["has_id"], true);
        assert_eq!(preview["success"], true);
        assert!(preview.get("command").is_none());
        assert!(!diagnostic.preview.unwrap().contains(secret));
    }

    #[test]
    fn structural_work_is_bounded_and_marks_dropped_entries() {
        let messages = (0..1_000)
            .map(|index| {
                json!({
                    "role":if index % 2 == 0 {"user"} else {"assistant"},
                    "content":[{"type":"text","text":"secret"}]
                })
            })
            .collect::<Vec<_>>();
        let record = json!({"type":"remote_snapshot","data":{"messages":messages,"tools":[]}});
        let (diagnostic, preview) = preview(&record);

        assert!(preview["dropped_entries"].as_u64().unwrap() > 0);
        assert_eq!(preview["summary"]["messages"]["count"], 1_000);
        assert!(!diagnostic.preview.unwrap().contains("secret"));
    }

    #[test]
    fn oversized_stringified_json_is_not_parsed_or_truncated_into_invalid_json() {
        let secret = "credential-must-not-survive";
        let record = json!({
            "type":"remote_snapshot",
            "data":format!("{{\"messages\":[{{\"role\":\"user\",\"content\":\"{}\"}}]}}", secret.repeat(4_096))
        });
        let (diagnostic, preview) = preview(&record);

        assert_eq!(preview["summary_limited"], true);
        assert_eq!(preview["summary"]["shape"], "string");
        assert!(diagnostic.preview.as_ref().unwrap().len() <= TRANSPORT_DIAGNOSTIC_PREVIEW_BYTES);
        assert!(!diagnostic.preview.unwrap().contains(secret));
    }
}
