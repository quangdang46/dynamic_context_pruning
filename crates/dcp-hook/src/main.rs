//! `dcp-hook` — Unified hook binary for Claude Code and Codex CLI.
//!
//! Supports both Claude Code's hook protocol and Codex CLI's hook protocol.
//! Detects the host protocol automatically from the input structure.
//!
//! # Protocol Detection
//!
//! Both Claude Code and Codex CLI use `hook_event_name` field.
//! Detection is based on field presence and naming conventions:
//! - Claude Code: `source`, `model` fields in SessionStart
//! - Codex CLI: `turn_id`, `tool_use_id` fields in tool events
//!
//! # Important Note on Message Pruning
//!
//! DCP's core functionality is context pruning (message transformation).
//! For SessionStart hooks, messages may be included depending on the host version.
//! For PreToolUse/PostToolUse hooks, messages are NOT included in the hook input -
//! they exist in the session transcript file (transcript_path), which is not a
//! stable interface for hooks.

use std::io::{self, Read, Write as IoWrite};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use dcp_config::Config;
use dcp_core::ContextPruner;
use dcp_types::{Message, Part, Role, ToolStatus};

// ============================================================================
// Logging
// ============================================================================

fn get_log_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| String::from("."));
    PathBuf::from(home).join(".dcp").join("hooks.log")
}

fn log_to_file(msg: &str) {
    let log_path = get_log_path();
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let timestamp = chrono_lite_now();
        let _ = writeln!(file, "[{}] {}", timestamp, msg);
    }
}

fn chrono_lite_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let millis = dur.subsec_millis();
    format!("{}.{:03}", secs, millis)
}

fn log_hook_event(input: &HookInput, json_str: &str) {
    let tool = input.tool_name.as_deref().unwrap_or("-");
    let tool_input = input
        .tool_input
        .as_ref()
        .map(|v| {
            if let Some(cmd) = v.get("command").and_then(|c| c.as_str()) {
                format!(" cmd={}", cmd.chars().take(60).collect::<String>())
            } else {
                String::new()
            }
        })
        .unwrap_or_default();

    log_to_file(&format!(
        "hook {} | session={} | tool={}{} | bytes={} | exit=0",
        input.hook_event_name,
        input.session_id.as_deref().unwrap_or("-"),
        tool,
        tool_input,
        json_str.len()
    ));
}

// ============================================================================
// Protocol Detection
// ============================================================================

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
enum HostProtocol {
    ClaudeCode,
    CodexCli,
    Unknown,
}

/// Detect which protocol the input follows based on field names.
fn detect_protocol(value: &JsonValue) -> HostProtocol {
    // Codex CLI tool events have turn_id and tool_use_id
    if value.get("turn_id").is_some() || value.get("tool_use_id").is_some() {
        return HostProtocol::CodexCli;
    }
    // Claude Code has source and model fields in SessionStart
    if value.get("source").is_some() || value.get("model").is_some() {
        return HostProtocol::ClaudeCode;
    }
    // Check for hook_event_name which both use
    if value.get("hook_event_name").is_some() {
        return HostProtocol::ClaudeCode;
    }
    HostProtocol::Unknown
}

// ============================================================================
// Unified Hook Protocol Types
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookInput {
    #[serde(rename = "hook_event_name")]
    pub hook_event_name: String,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub cwd: Option<String>,
    pub permission_mode: Option<String>,
    pub effort: Option<HookEffort>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub source: Option<String>,
    pub model: Option<String>,
    pub turn_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub tool_input: Option<JsonValue>,
    pub messages: Option<Vec<JsonValue>>,
    pub context: Option<HookContext>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookEffort {
    pub level: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HookContext {
    pub cwd: Option<String>,
    pub env: Option<std::collections::HashMap<String, String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookOutput {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub hook_type: Option<String>,
    pub session_id: Option<String>,
    pub tool_name: Option<String>,
    pub messages: Option<Vec<JsonValue>>,
    pub warning: Option<String>,
    pub skipped: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<HookSpecificOutput>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    #[serde(rename = "permissionDecision", skip_serializing_if = "Option::is_none")]
    pub permission_decision: Option<String>,
    #[serde(
        rename = "permissionDecisionReason",
        skip_serializing_if = "Option::is_none"
    )]
    pub permission_decision_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
}

impl HookOutput {
    pub fn skipped(reason: &str) -> Self {
        Self {
            hook_type: None,
            session_id: None,
            tool_name: None,
            messages: None,
            warning: Some(reason.to_string()),
            skipped: Some(true),
            hook_specific_output: None,
        }
    }

    pub fn success(hook_event_name: &str, messages: Vec<JsonValue>) -> Self {
        Self {
            hook_type: Some(hook_event_name.to_string()),
            session_id: None,
            tool_name: None,
            messages: Some(messages),
            warning: None,
            skipped: Some(false),
            hook_specific_output: None,
        }
    }

    pub fn allow(hook_event_name: &str) -> Self {
        // For PreToolUse, only output hookSpecificOutput - no extra null fields
        if hook_event_name == "PreToolUse" {
            return Self {
                hook_type: None,
                session_id: None,
                tool_name: None,
                messages: None,
                warning: None,
                skipped: None,
                hook_specific_output: Some(HookSpecificOutput {
                    hook_event_name: hook_event_name.to_string(),
                    permission_decision: Some("allow".to_string()),
                    permission_decision_reason: None,
                    additional_context: None,
                }),
            };
        }
        Self {
            hook_type: None,
            session_id: None,
            tool_name: None,
            messages: None,
            warning: None,
            skipped: None,
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: hook_event_name.to_string(),
                permission_decision: Some("allow".to_string()),
                permission_decision_reason: None,
                additional_context: None,
            }),
        }
    }
}

// ============================================================================
// Message Conversion
// ============================================================================

fn parse_message_from_json(obj: &serde_json::Map<String, JsonValue>) -> Option<Message> {
    let id = obj.get("id")?.as_str()?.to_string();
    let role = match obj.get("role")?.as_str()? {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "system" => Role::System,
        _ => return None,
    };
    let time = obj
        .get("timestamp")
        .or_else(|| obj.get("time"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let parts: Vec<Part> = obj
        .get("content")
        .or_else(|| obj.get("parts"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let p_obj = p.as_object()?;
                    let content = p_obj
                        .get("content")
                        .or_else(|| p_obj.get("text"))
                        .or(Some(p));
                    if let Some(text) = content.and_then(|c| c.as_str()) {
                        return Some(Part::Text(text.to_string()));
                    }
                    None
                })
                .collect()
        })
        .unwrap_or_default();

    if parts.is_empty() {
        if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
            return Some(Message::new(
                id,
                role,
                vec![Part::Text(text.to_string())],
                time,
            ));
        }
        if let Some(text) = obj.get("content").and_then(|c| c.as_str()) {
            return Some(Message::new(
                id,
                role,
                vec![Part::Text(text.to_string())],
                time,
            ));
        }
    }

    Some(Message::new(id, role, parts, time))
}

fn message_to_json(msg: &Message) -> JsonValue {
    let role_str = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        _ => "unknown",
    };

    let parts: Vec<JsonValue> = msg
        .parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => serde_json::json!({"type": "text", "content": t}),
            Part::Reasoning(r) => serde_json::json!({"type": "reasoning", "content": r}),
            Part::ToolCall { call_id, tool, input } => {
                serde_json::json!({"type": "tool_call", "id": call_id, "name": tool, "input": input})
            }
            Part::ToolResult { call_id, status, output, error } => {
                let mut obj = serde_json::json!({
                    "type": "tool_result",
                    "tool_call_id": call_id,
                    "status": match status {
                        ToolStatus::Pending => "pending",
                        ToolStatus::Running => "running",
                        ToolStatus::Completed => "completed",
                        ToolStatus::Error => "error",
                        _ => "unknown",
                    }
                });
                if let Some(o) = output {
                    obj["content"] = JsonValue::String(o.clone());
                }
                if let Some(e) = error {
                    obj["error"] = JsonValue::String(e.clone());
                }
                obj
            }
            Part::Image { media_type, data } => {
                serde_json::json!({"type": "image", "media_type": media_type, "data": data})
            }
            _ => serde_json::json!({"type": "unknown"}),
        })
        .collect();

    serde_json::json!({"id": msg.id, "role": role_str, "content": parts, "timestamp": msg.time})
}

// ============================================================================
// Core Transform Logic
// ============================================================================

fn transform_messages(
    pruner: &mut ContextPruner,
    messages: Vec<Message>,
) -> Result<Vec<Message>, String> {
    pruner
        .transform_messages(messages)
        .map_err(|e| e.to_string())
}

fn run_transform(input: &HookInput) -> HookOutput {
    let config = match Config::load_default() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("[dcp-hook] Config load failed: {}", e);
            return HookOutput::skipped(&format!("config error: {}", e));
        }
    };

    let mut pruner = match ContextPruner::new(config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[dcp-hook] ContextPruner init failed: {}", e);
            return HookOutput::skipped(&format!("pruner init error: {}", e));
        }
    };

    if let Some(ref session_id) = input.session_id {
        pruner.set_session_id(session_id);
    }

    let messages = match &input.messages {
        Some(msgs) => msgs.clone(),
        None => {
            if input.tool_name.is_some() {
                return HookOutput::allow(&input.hook_event_name);
            }
            return HookOutput::skipped("no messages in input");
        }
    };

    if messages.is_empty() {
        return HookOutput::skipped("no messages to transform");
    }

    let mut dcp_messages: Vec<Message> = Vec::new();
    for msg_json in &messages {
        if let Some(obj) = msg_json.as_object() {
            if let Some(msg) = parse_message_from_json(obj) {
                dcp_messages.push(msg);
            }
        }
    }

    if dcp_messages.is_empty() {
        return HookOutput::skipped("no valid messages to transform");
    }

    match transform_messages(&mut pruner, dcp_messages) {
        Ok(transformed) => {
            let result_json: Vec<JsonValue> = transformed.iter().map(message_to_json).collect();
            HookOutput::success(&input.hook_event_name, result_json)
        }
        Err(e) => {
            eprintln!("[dcp-hook] Transform error: {}", e);
            HookOutput::skipped(&format!("transform error: {}", e))
        }
    }
}

// ============================================================================
// CLI Options
// ============================================================================

#[derive(Clone, Debug, Default)]
pub struct CliOptions {
    pub on_compact: bool,
    pub debug: bool,
    pub pre_tool_use: bool,
    pub dry_run: bool,
}

impl CliOptions {
    pub fn from_args() -> Self {
        let mut opts = CliOptions::default();
        for arg in std::env::args().skip(1) {
            match arg.as_str() {
                "--on-compact" => opts.on_compact = true,
                "--debug" | "-d" => opts.debug = true,
                "--pre-tool-use" => opts.pre_tool_use = true,
                "--dry-run" => opts.dry_run = true,
                _ => {}
            }
        }
        opts
    }
}

// ============================================================================
// Main Entry Point
// ============================================================================

fn main() -> anyhow::Result<()> {
    let opts = CliOptions::from_args();
    let debug = opts.debug || std::env::var("DCP_DEBUG").is_ok();

    log_to_file(&format!("START debug={} opts={:?}", debug, opts));

    let mut input_buffer = String::new();
    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();

    if let Err(e) = stdin_lock.read_to_string(&mut input_buffer) {
        log_to_file(&format!("ERROR: Failed to read stdin: {}", e));
        std::process::exit(1);
    }

    if input_buffer.trim().is_empty() {
        log_to_file("EXIT: empty stdin");
        std::process::exit(0);
    }

    log_to_file(&format!("READ {} bytes", input_buffer.len()));

    let json_value: JsonValue = match serde_json::from_str(&input_buffer) {
        Ok(v) => v,
        Err(e) => {
            log_to_file(&format!("ERROR: Failed to parse JSON: {}", e));
            println!("{{}}");
            std::process::exit(0);
        }
    };

    let protocol = detect_protocol(&json_value);
    log_to_file(&format!("PROTOCOL: {:?}", protocol));

    let input: HookInput = match serde_json::from_str(&input_buffer) {
        Ok(inp) => inp,
        Err(e) => {
            log_to_file(&format!("ERROR: Failed to parse input: {}", e));
            println!("{{}}");
            std::process::exit(0);
        }
    };

    let msg_count = input.messages.as_ref().map(|m| m.len()).unwrap_or(0);
    log_to_file(&format!(
        "EVENT: {} tool={:?} messages={}",
        input.hook_event_name, input.tool_name, msg_count
    ));

    if opts.dry_run {
        log_to_file("DRYRUN: echoing input");
        println!("{}", serde_json::to_string(&input).unwrap_or_default());
        std::process::exit(0);
    }

    // PreToolUse without messages - allow without transform
    // Codex docs: "Exit 0 with no output is treated as success and Codex continues"
    if input.hook_event_name == "PreToolUse" && input.messages.is_none() {
        log_hook_event(&input, &input_buffer);
        std::process::exit(0);
    }

    let output = run_transform(&input);
    let output_json = serde_json::to_string(&output).unwrap_or_default();
    log_hook_event(&input, &output_json);
    println!("{}", output_json);

    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_protocol_codex() {
        let json = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "session_id": "abc123",
            "tool_name": "Bash",
            "turn_id": "turn1"
        });
        assert_eq!(detect_protocol(&json), HostProtocol::CodexCli);
    }

    #[test]
    fn test_detect_protocol_claude_code() {
        let json = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "abc123",
            "source": "startup",
            "model": "claude-sonnet-4-6"
        });
        assert_eq!(detect_protocol(&json), HostProtocol::ClaudeCode);
    }

    #[test]
    fn test_hook_output_allow() {
        let output = HookOutput::allow("PreToolUse");
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("allow"));
        assert!(json.contains("PreToolUse"));
    }

    #[test]
    fn test_hook_output_skipped() {
        let output = HookOutput::skipped("test reason");
        assert!(output.warning.is_some());
        assert!(output.skipped.unwrap_or(false));
    }
}
