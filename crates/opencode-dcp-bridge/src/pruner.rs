use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::message;
use dcp_core::commands::CommandOutcome;

/// Convert camelCase JSON keys to snake_case for Rust deserialization.
fn camel_to_snake(key: &str) -> String {
    let mut result = String::with_capacity(key.len());
    for (i, ch) in key.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

/// Recursively convert all object keys in a JSON value from camelCase to snake_case.
fn keys_to_snake(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                let snake = camel_to_snake(&key);
                if let Some(mut v) = map.remove(&key) {
                    keys_to_snake(&mut v);
                    map.insert(snake, v);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                keys_to_snake(item);
            }
        }
        _ => {}
    }
}

fn parse_compress_args(json_str: &str) -> Result<dcp_core::CompressArgs> {
    let mut val: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| Error::from_reason(format!("Args parse: {e}")))?;
    // Convert camelCase keys from TS/OpenCode to snake_case for Rust
    keys_to_snake(&mut val);

    let mode = val.get("mode").and_then(|v| v.as_str()).unwrap_or("range");
    let topic = val
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    match mode {
        "message" => {
            let content: Vec<dcp_core::MessageEntry> = serde_json::from_value(
                val.get("content")
                    .cloned()
                    .unwrap_or(serde_json::Value::Array(vec![])),
            )
            .map_err(|e| Error::from_reason(format!("Content parse: {e}")))?;
            Ok(dcp_core::CompressArgs::Message { topic, content })
        }
        _ => {
            let content: Vec<dcp_core::RangeEntry> = serde_json::from_value(
                val.get("content")
                    .cloned()
                    .unwrap_or(serde_json::Value::Array(vec![])),
            )
            .map_err(|e| Error::from_reason(format!("Content parse: {e}")))?;
            Ok(dcp_core::CompressArgs::Range { topic, content })
        }
    }
}

/// Parse a JSON array of string arguments.
fn parse_json_args(json_str: &str) -> Result<Vec<String>> {
    let val: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| Error::from_reason(format!("Args JSON parse: {e}")))?;
    let arr = val
        .as_array()
        .ok_or_else(|| Error::from_reason("args must be a JSON array of strings".to_string()))?;
    let mut args = Vec::with_capacity(arr.len());
    for v in arr {
        args.push(
            v.as_str()
                .ok_or_else(|| Error::from_reason("each arg must be a string".to_string()))?
                .to_string(),
        );
    }
    Ok(args)
}

/// Format a CommandOutcome into user-facing text and status.
fn format_command_outcome(outcome: &CommandOutcome) -> (String, &'static str) {
    match outcome {
        CommandOutcome::Context {
            current_turn,
            active_blocks,
            total_blocks,
            pending_tokens,
            frontier,
            cache_stability_mode,
        } => {
            let frontier_str = frontier.as_deref().unwrap_or("none");
            (
                format!(
                    "**DCP Context**\n- Turn: {}\n- Active blocks: {}/{}\n- Pending tokens: {}\n- Frontier: {}\n- Cache stability: {}",
                    current_turn,
                    active_blocks,
                    total_blocks,
                    pending_tokens,
                    frontier_str,
                    cache_stability_mode
                ),
                "ok",
            )
        }
        CommandOutcome::Stats(stats) => (
            serde_json::to_string_pretty(stats).unwrap_or_default(),
            "ok",
        ),
        CommandOutcome::Sweep { applied_ids } => (
            format!("Sweep applied {applied_ids} pending prune entries."),
            "ok",
        ),
        CommandOutcome::Manual { enabled } => (
            format!(
                "Manual mode {}.",
                if *enabled { "enabled" } else { "disabled" }
            ),
            "ok",
        ),
        CommandOutcome::Compress(result) => (
            format!(
                "Compression complete: {} blocks created.",
                result.blocks.len()
            ),
            "ok",
        ),
        CommandOutcome::Decompress { block_id } => {
            (format!("Decompressed block b{}.", block_id.0), "ok")
        }
        CommandOutcome::Recompress { block_id } => {
            (format!("Recompressed block b{}.", block_id.0), "ok")
        }
        CommandOutcome::Unknown { command } => (
            format!(
                "Unknown command: {command}. Try context, stats, sweep, manual, compress, decompress, recompress."
            ),
            "error",
        ),
        CommandOutcome::Error { message } => (format!("Error: {message}"), "error"),
        _ => ("Command processed.".to_string(), "ok"),
    }
}

#[napi]
pub struct DcpPruner {
    inner: std::sync::Mutex<dcp_core::ContextPruner>,
    /// Last known OpenCode session id (for persistence / TUI).
    session_id: std::sync::Mutex<Option<String>>,
}

#[napi]
impl DcpPruner {
    #[napi(constructor)]
    pub fn new(config_json: String) -> Result<Self> {
        let config: dcp_config::Config = serde_json::from_str(&config_json)
            .map_err(|e| Error::from_reason(format!("Config parse: {e}")))?;
        let pruner = dcp_core::ContextPruner::new(config)
            .map_err(|e| Error::from_reason(format!("Pruner init: {e}")))?;
        let _ = pruner.save();
        Ok(Self {
            inner: std::sync::Mutex::new(pruner),
            session_id: std::sync::Mutex::new(None),
        })
    }

    /// Transform OpenCode messages before sending to the LLM.
    ///
    /// Preserves the original OpenCode envelope (sessionID, agent, part
    /// ids, tool metadata) and only applies DCP semantic changes.
    #[napi]
    pub fn transform_messages(&self, messages_json: String) -> Result<String> {
        if let Some(sid) = message::extract_session_id(&messages_json) {
            self.bind_session(&sid);
        }

        let dcp_messages =
            message::opencode_to_dcp(&messages_json).map_err(|e| Error::from_reason(e))?;

        let mut pruner = self
            .inner
            .lock()
            .map_err(|_| Error::from_reason("mutex poisoned".to_string()))?;

        let transformed = pruner
            .transform_messages(dcp_messages)
            .map_err(|e| Error::from_reason(format!("Transform: {e}")))?;

        message::merge_dcp_into_opencode(&messages_json, &transformed)
            .map_err(|e| Error::from_reason(e))
    }

    /// Append DCP system prompt addendum.
    #[napi]
    pub fn transform_system(&self, system: String) -> String {
        if let Ok(pruner) = self.inner.lock() {
            let mut s = system;
            pruner.transform_system(&mut s);
            return s;
        }
        system
    }

    /// Handle compress tool call from the LLM.
    ///
    /// `messages_json` should be the current OpenCode session messages
    /// (not `"[]"`). When empty, compression still records the request
    /// against whatever state the pruner already holds.
    #[napi]
    pub fn handle_compress(&self, args_json: String, messages_json: String) -> Result<String> {
        if let Some(sid) = message::extract_session_id(&messages_json) {
            self.bind_session(&sid);
        }

        let args = parse_compress_args(&args_json)?;
        let messages = if messages_json.trim().is_empty() || messages_json.trim() == "[]" {
            Vec::new()
        } else {
            message::opencode_to_dcp(&messages_json).map_err(|e| Error::from_reason(e))?
        };
        let mut pruner = self
            .inner
            .lock()
            .map_err(|_| Error::from_reason("mutex poisoned".to_string()))?;
        let result = pruner
            .handle_compress(args, &messages)
            .map_err(|e| Error::from_reason(format!("Compress: {e}")))?;
        serde_json::to_string(&result).map_err(|e| Error::from_reason(format!("Serialize: {e}")))
    }

    /// Restore a compressed block to its original messages.
    #[napi]
    pub fn decompress(&self, block_id: u32) -> Result<String> {
        let mut pruner = self
            .inner
            .lock()
            .map_err(|_| Error::from_reason("mutex poisoned".to_string()))?;
        let result = pruner
            .decompress(dcp_types::BlockId(block_id))
            .map_err(|e| Error::from_reason(format!("Decompress: {e}")))?;
        serde_json::to_string(&result).map_err(|e| Error::from_reason(format!("Serialize: {e}")))
    }

    /// Re-activate a user-decompressed block for future compression.
    #[napi]
    pub fn recompress(&self, block_id: u32) -> Result<String> {
        let mut pruner = self
            .inner
            .lock()
            .map_err(|_| Error::from_reason("mutex poisoned".to_string()))?;
        let result = pruner
            .recompress(dcp_types::BlockId(block_id))
            .map_err(|e| Error::from_reason(format!("Recompress: {e}")))?;
        serde_json::to_string(&result).map_err(|e| Error::from_reason(format!("Serialize: {e}")))
    }

    #[napi]
    pub fn has_pending_work(&self) -> bool {
        self.inner
            .lock()
            .map(|p| p.has_pending_work())
            .unwrap_or(false)
    }

    #[napi]
    pub fn stats(&self) -> String {
        if let Ok(pruner) = self.inner.lock() {
            serde_json::to_string(&pruner.stats()).unwrap_or_default()
        } else {
            String::new()
        }
    }

    /// Return a JSON snapshot useful for the TUI panel.
    #[napi]
    pub fn context_snapshot(&self) -> String {
        let Ok(pruner) = self.inner.lock() else {
            return "{}".into();
        };
        let state = pruner.state();
        let stats = pruner.stats();
        let session = self
            .session_id
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default();
        serde_json::json!({
            "sessionId": session,
            "currentTurn": state.current_turn,
            "manualMode": state.manual_mode.enabled,
            "isSubAgent": state.is_subagent,
            "activeBlocks": state.prune.messages.active_block_ids.len(),
            "totalBlocks": state.prune.messages.blocks_by_id.len(),
            "pendingTools": state.prune.tools.len(),
            "pendingTokens": state.pending_prune.as_ref().map(|p| p.cumulative_tokens).unwrap_or(0),
            "toolIdCount": state.tool_id_list.len(),
            "stats": stats,
        })
        .to_string()
    }

    #[napi]
    pub fn set_session_id(&self, session_id: String) {
        self.bind_session(&session_id);
    }

    /// Whether the master switch is enabled.
    #[napi]
    pub fn is_enabled(&self) -> bool {
        self.inner
            .lock()
            .map(|p| p.config().enabled)
            .unwrap_or(true)
    }

    /// Return resolved config JSON (camelCase).
    #[napi]
    pub fn config_json(&self) -> String {
        self.inner
            .lock()
            .ok()
            .and_then(|p| serde_json::to_string(p.config()).ok())
            .unwrap_or_else(|| "{}".into())
    }

    // ──────────────────────────────────────────────────────────────
    // Slash-command handling
    // ──────────────────────────────────────────────────────────────

    /// Handle a /dcp slash command.
    /// cmd: the subcommand name (e.g. "context", "stats", "decompress")
    /// args_json: JSON array of string arguments (e.g. '["b1"]')
    /// messages_json: current messages as JSON array (for compress commands)
    /// Returns JSON: {"text": "...", "status": "ok|error"}
    #[napi]
    pub fn handle_command(
        &self,
        cmd: String,
        args_json: String,
        messages_json: String,
    ) -> Result<String> {
        if let Some(sid) = message::extract_session_id(&messages_json) {
            self.bind_session(&sid);
        }

        let args = parse_json_args(&args_json)?;
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let messages = if messages_json.trim().is_empty() || messages_json.trim() == "[]" {
            Vec::new()
        } else {
            message::opencode_to_dcp(&messages_json).map_err(|e| Error::from_reason(e))?
        };

        let mut pruner = self
            .inner
            .lock()
            .map_err(|_| Error::from_reason("mutex poisoned".to_string()))?;

        // Sync state from messages when provided so context/stats are fresh.
        if !messages.is_empty() {
            let _ = pruner.transform_messages(messages.clone());
        }

        let outcome = pruner.handle_command(&cmd, &args_refs, &messages);
        let (text, status) = format_command_outcome(&outcome);
        let result = serde_json::json!({ "text": text, "status": status });
        Ok(result.to_string())
    }

    /// Notify the pruner of a lifecycle event (session switches, etc).
    #[napi]
    pub fn notify_event(&self, event_json: String) -> Result<()> {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&event_json) else {
            return Ok(());
        };
        let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        // Common OpenCode event shapes:
        // { type: "session.updated", properties: { sessionID } }
        // { type: "message.updated", properties: { info: { sessionID } } }
        let sid = val
            .pointer("/properties/sessionID")
            .or_else(|| val.pointer("/properties/info/sessionID"))
            .or_else(|| val.pointer("/sessionID"))
            .and_then(|v| v.as_str());

        if let Some(sid) = sid {
            self.bind_session(sid);
        }

        if event_type == "plugin.dispose" {
            if let Ok(pruner) = self.inner.lock() {
                let _ = pruner.save();
            }
        }

        Ok(())
    }

    fn bind_session(&self, session_id: &str) {
        if session_id.is_empty() || session_id == "__dispose__" {
            return;
        }
        if let Ok(mut guard) = self.session_id.lock() {
            if guard.as_deref() != Some(session_id) {
                *guard = Some(session_id.to_string());
                if let Ok(mut pruner) = self.inner.lock() {
                    pruner.set_session_id(session_id);
                }
            }
        }
    }
}

impl Drop for DcpPruner {
    fn drop(&mut self) {
        if let Ok(pruner) = self.inner.lock() {
            let _ = pruner.save();
        }
    }
}
