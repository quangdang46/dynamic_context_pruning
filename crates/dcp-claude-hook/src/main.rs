//! `dcp-claude-hook` — Claude Code hook binary for dynamic_context_pruning.
//!
//! This binary implements Claude Code's hook protocol for PreToolUse and
//! SessionStart hooks. It receives JSON from stdin, transforms messages via
//! the DCP ContextPruner, and outputs the transformed JSON to stdout.
//!
//! # Claude Code Hook Registration
//!
//! Add to `~/.claude/settings.json`:
//! ```json
//! {
//!   "hooks": {
//!     "PreToolUse": [{
//!       "matcher": "*",
//!       "hooks": [{
//!         "type": "command",
//!         "command": "$HOME/.cargo/bin/dcp-claude-hook"
//!       }]
//!     }],
//!     "SessionStart": [{
//!       "matcher": "compact",
//!       "hooks": [{
//!         "type": "command",
//!         "command": "$HOME/.cargo/bin/dcp-claude-hook --on-compact"
//!       }]
//!     }]
//!   }
//! }
//! ```
//!
//! # Hook Protocol
//!
//! Claude Code sends JSON via stdin with this structure:
//! ```json
//! {
//!   "type": "SessionStart" | "PreToolUse",
//!   "sessionId": "...",
//!   "messages": [...],
//!   "toolName": "..." (for PreToolUse)
//! }
//! ```
//!
//! The hook outputs the same structure with transformed messages:
//! ```json
//! {
//!   "type": "...",
//!   "sessionId": "...",
//!   "messages": [...transformed...],
//!   "toolName": "..." (preserved from input)
//! }
//! ```

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use dcp_config::Config;
use dcp_core::ContextPruner;
use dcp_types::{Message, Part, Role, ToolStatus};

// ============================================================================
// Claude Code Hook Protocol Types
// ============================================================================

/// Claude Code hook input received via stdin.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookInput {
    /// The hook type: "SessionStart" or "PreToolUse".
    #[serde(rename = "type")]
    pub hook_type: String,
    /// The session identifier.
    pub session_id: Option<String>,
    /// The conversation messages.
    pub messages: Vec<JsonValue>,
    /// The tool name (for PreToolUse hooks).
    pub tool_name: Option<String>,
    /// Additional context from Claude Code.
    pub context: Option<HookContext>,
}

/// Additional context from Claude Code.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HookContext {
    /// The current working directory.
    pub cwd: Option<String>,
    /// Environment variables (subset).
    pub env: Option<std::collections::HashMap<String, String>>,
}

/// Hook output sent to stdout.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookOutput {
    /// The hook type (echoed from input).
    #[serde(rename = "type")]
    pub hook_type: String,
    /// The session identifier.
    pub session_id: Option<String>,
    /// The transformed messages.
    pub messages: Vec<JsonValue>,
    /// The tool name (preserved from input).
    pub tool_name: Option<String>,
    /// Optional warning message.
    pub warning: Option<String>,
    /// Whether the transform was skipped (e.g., errors, empty input).
    pub skipped: Option<bool>,
}

impl HookOutput {
    /// Create an output that indicates the transform was skipped.
    pub fn skipped(input: &HookInput, reason: &str) -> Self {
        Self {
            hook_type: input.hook_type.clone(),
            session_id: input.session_id.clone(),
            messages: input.messages.clone(),
            tool_name: input.tool_name.clone(),
            warning: Some(reason.to_string()),
            skipped: Some(true),
        }
    }

    /// Create a successful output with transformed messages.
    pub fn success(input: &HookInput, messages: Vec<JsonValue>) -> Self {
        Self {
            hook_type: input.hook_type.clone(),
            session_id: input.session_id.clone(),
            messages,
            tool_name: input.tool_name.clone(),
            warning: None,
            skipped: Some(false),
        }
    }
}

// ============================================================================
// Message Conversion (Claude Code JSON <-> DCP Types)
// ============================================================================

/// Convert a Claude Code JSON message to a DCP Message.
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

    // Handle different message formats
    if parts.is_empty() {
        // Try to extract content from various formats
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

/// Convert a DCP Message back to Claude Code JSON format.
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
            Part::Text(t) => serde_json::json!({
                "type": "text",
                "content": t
            }),
            Part::Reasoning(r) => serde_json::json!({
                "type": "reasoning",
                "content": r
            }),
            Part::ToolCall {
                call_id,
                tool,
                input,
            } => serde_json::json!({
                "type": "tool_call",
                "id": call_id,
                "name": tool,
                "input": input
            }),
            Part::ToolResult {
                call_id,
                status,
                output,
                error,
            } => {
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
            Part::Image { media_type, data } => serde_json::json!({
                "type": "image",
                "media_type": media_type,
                "data": data
            }),
            _ => serde_json::json!({"type": "unknown"}),
        })
        .collect();

    serde_json::json!({
        "id": msg.id,
        "role": role_str,
        "content": parts,
        "timestamp": msg.time
    })
}

// ============================================================================
// Core Transform Logic
// ============================================================================

/// Transform messages using the DCP ContextPruner.
fn transform_messages(
    pruner: &mut ContextPruner,
    messages: Vec<Message>,
) -> Result<Vec<Message>, String> {
    pruner
        .transform_messages(messages)
        .map_err(|e| e.to_string())
}

/// Run the transformation on Claude Code hook input.
fn run_transform(input: &HookInput) -> HookOutput {
    // Load configuration
    let config = match Config::load_default() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("[dcp-claude-hook] Config load failed: {}", e);
            return HookOutput::skipped(input, &format!("config error: {}", e));
        }
    };

    // Initialize the ContextPruner
    let mut pruner = match ContextPruner::new(config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[dcp-claude-hook] ContextPruner init failed: {}", e);
            return HookOutput::skipped(input, &format!("pruner init error: {}", e));
        }
    };

    // Set session ID if provided
    if let Some(ref session_id) = input.session_id {
        pruner.set_session_id(session_id);
    }

    // Convert Claude Code JSON messages to DCP Message types
    let mut dcp_messages: Vec<Message> = Vec::new();
    for msg_json in &input.messages {
        if let Some(obj) = msg_json.as_object() {
            if let Some(msg) = parse_message_from_json(obj) {
                dcp_messages.push(msg);
            }
        }
    }

    if dcp_messages.is_empty() {
        return HookOutput::skipped(input, "no valid messages to transform");
    }

    // Transform the messages
    match transform_messages(&mut pruner, dcp_messages) {
        Ok(transformed) => {
            // Convert back to JSON
            let result_json: Vec<JsonValue> = transformed.iter().map(message_to_json).collect();

            HookOutput::success(input, result_json)
        }
        Err(e) => {
            eprintln!("[dcp-claude-hook] Transform error: {}", e);
            HookOutput::skipped(input, &format!("transform error: {}", e))
        }
    }
}

// ============================================================================
// CLI Options
// ============================================================================

#[derive(Clone, Debug, Default)]
pub struct CliOptions {
    /// Run in compact mode (for SessionStart hook).
    pub on_compact: bool,
    /// Show debug output.
    pub debug: bool,
    /// PreToolUse mode.
    pub pre_tool_use: bool,
    /// Echo the input back without transformation (for testing).
    pub dry_run: bool,
}

impl CliOptions {
    /// Parse command-line arguments.
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

    // Debug output
    let debug = opts.debug || std::env::var("DCP_DEBUG").is_ok();
    if debug {
        eprintln!("[dcp-claude-hook] Starting (debug={})", debug);
        eprintln!(
            "[dcp-claude-hook] on_compact={}, pre_tool_use={}, dry_run={}",
            opts.on_compact, opts.pre_tool_use, opts.dry_run
        );
    }

    // Read JSON from stdin
    let mut input_buffer = String::new();
    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();

    if let Err(e) = stdin_lock.read_to_string(&mut input_buffer) {
        eprintln!("[dcp-claude-hook] ERROR: Failed to read stdin: {}", e);
        std::process::exit(1);
    }

    // Handle empty input (e.g., when called without pipe)
    if input_buffer.trim().is_empty() {
        if debug {
            eprintln!("[dcp-claude-hook] Empty stdin, exiting gracefully");
        }
        std::process::exit(0);
    }

    if debug {
        eprintln!(
            "[dcp-claude-hook] Read {} bytes from stdin",
            input_buffer.len()
        );
    }

    // Parse the input JSON
    let input: HookInput = match serde_json::from_str(&input_buffer) {
        Ok(inp) => inp,
        Err(e) => {
            // If we can't parse, try to handle gracefully
            if debug {
                eprintln!("[dcp-claude-hook] Parse error: {}", e);
            }
            // Try parsing as a simple messages array
            match serde_json::from_str::<Vec<JsonValue>>(&input_buffer) {
                Ok(msgs) => HookInput {
                    hook_type: "SessionStart".to_string(),
                    session_id: None,
                    messages: msgs,
                    tool_name: None,
                    context: None,
                },
                Err(_) => {
                    eprintln!("[dcp-claude-hook] ERROR: Failed to parse input JSON: {}", e);
                    // Output empty result to avoid breaking Claude Code
                    println!("{{\"messages\":[]}}");
                    std::process::exit(0);
                }
            }
        }
    };

    if debug {
        eprintln!(
            "[dcp-claude-hook] Hook type: {}, messages: {}",
            input.hook_type,
            input.messages.len()
        );
    }

    // Dry run mode - echo back
    if opts.dry_run {
        if debug {
            eprintln!("[dcp-claude-hook] Dry run mode - echoing input");
        }
        println!("{}", serde_json::to_string(&input).unwrap_or_default());
        std::process::exit(0);
    }

    // For PreToolUse hooks, only process if not in compact mode
    if input.hook_type == "PreToolUse" && opts.on_compact {
        if debug {
            eprintln!("[dcp-claude-hook] Skipping PreToolUse in compact mode");
        }
        // In compact mode, skip PreToolUse processing
        println!("{}", serde_json::to_string(&input).unwrap_or_default());
        std::process::exit(0);
    }

    // For SessionStart hooks with --on-compact, only process compact events
    if opts.on_compact && input.hook_type != "SessionStart" {
        if debug {
            eprintln!("[dcp-claude-hook] Skipping non-SessionStart in compact mode");
        }
        println!("{}", serde_json::to_string(&input).unwrap_or_default());
        std::process::exit(0);
    }

    // Run the transformation
    let output = run_transform(&input);

    // Write output to stdout
    let output_json = serde_json::to_string(&output).unwrap_or_else(|e| {
        eprintln!("[dcp-claude-hook] ERROR: Failed to serialize output: {}", e);
        // Fall back to echoing input
        serde_json::to_string(&input).unwrap_or_default()
    });

    if debug {
        eprintln!(
            "[dcp-claude-hook] Writing {} bytes to stdout",
            output_json.len()
        );
    }

    // Use stdout write directly for hook protocol
    let mut stdout = io::stdout();
    stdout.write_all(output_json.as_bytes())?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;

    if debug {
        eprintln!("[dcp-claude-hook] Done");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_from_json_user() {
        let json = serde_json::json!({
            "id": "msg1",
            "role": "user",
            "content": [
                {"type": "text", "content": "hello"}
            ],
            "timestamp": 1234567890
        });
        let obj = json.as_object().unwrap();
        let msg = parse_message_from_json(obj).unwrap();
        assert_eq!(msg.id, "msg1");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.parts.len(), 1);
    }

    #[test]
    fn test_message_to_json_roundtrip() {
        let msg = Message::user_text("u1", 0, "hello");
        let json = message_to_json(&msg);
        assert!(json.is_object());
        let obj = json.as_object().unwrap();
        assert_eq!(obj.get("id").unwrap().as_str().unwrap(), "u1");
        assert_eq!(obj.get("role").unwrap().as_str().unwrap(), "user");
    }

    #[test]
    fn test_hook_output_success() {
        let input = HookInput {
            hook_type: "SessionStart".to_string(),
            session_id: Some("sess123".to_string()),
            messages: vec![],
            tool_name: None,
            context: None,
        };
        let output = HookOutput::success(&input, vec![serde_json::json!({"id": "m1"})]);
        assert_eq!(output.hook_type, "SessionStart");
        assert_eq!(output.session_id, Some("sess123".to_string()));
        assert_eq!(output.messages.len(), 1);
        assert!(!output.skipped.unwrap_or(false));
    }

    #[test]
    fn test_hook_output_skipped() {
        let input = HookInput {
            hook_type: "PreToolUse".to_string(),
            session_id: None,
            messages: vec![],
            tool_name: Some("read".to_string()),
            context: None,
        };
        let output = HookOutput::skipped(&input, "test reason");
        assert_eq!(output.hook_type, "PreToolUse");
        assert!(output.warning.is_some());
        assert!(output.skipped.unwrap_or(false));
    }

    #[test]
    fn test_cli_options() {
        let args = ["dcp-claude-hook", "--on-compact", "--debug"];
        // SAFETY: This is only used in test context and restored immediately
        unsafe {
            std::env::set_var("DCP_ARGS", args.join(" "));
        }
        // Note: In actual use, args come from std::env::args()
    }
}
