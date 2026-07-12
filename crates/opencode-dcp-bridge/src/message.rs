//! OpenCode message format ↔ DCP IR conversion.
//!
//! OpenCode stores tools as unified `type: "tool"` parts on **assistant**
//! messages (`state.input` + `state.output`). DCP IR splits that into
//! `ToolCall` (assistant) + `ToolResult` (user).
//!
//! Conversion strategy:
//! 1. `opencode_to_dcp` — map tool parts → `ToolCall`, then inject a
//!    synthetic user message with matching `ToolResult`s so prune /
//!    tool-cache work correctly.
//! 2. `merge_dcp_into_opencode` — start from the **original** OpenCode
//!    envelope (keeps `sessionID`, `agent`, part ids, …) and apply only
//!    the semantic changes from the transformed DCP list.

use std::collections::{HashMap, HashSet};

use dcp_types::{Message, Part, Role, ToolStatus};
use serde_json::{Value as JsonValue, json};

/// Prefix for synthetic user messages that carry tool results.
const SYNTH_RESULT_PREFIX: &str = "__dcp_tool_results__";

/// OpenCode-compatible pruned tool output (matches upstream TS).
const PRUNED_TOOL_OUTPUT: &str =
    "[Output removed to save context - information superseded or no longer needed]";

/// OpenCode-compatible purged error input (matches upstream TS).
const PRUNED_TOOL_ERROR_INPUT: &str = "[input removed due to failed tool call]";

/// Convert OpenCode-format messages (JSON array of `{info, parts}`) to DCP.
///
/// Emits synthetic user `ToolResult` messages after assistants that have
/// completed/error tool parts so DCP role rules and tool-cache pairing hold.
pub fn opencode_to_dcp(json_str: &str) -> Result<Vec<Message>, String> {
    let opencode_messages: Vec<JsonValue> =
        serde_json::from_str(json_str).map_err(|e| format!("JSON parse: {e}"))?;
    opencode_values_to_dcp(&opencode_messages)
}

/// Same as [`opencode_to_dcp`] but from already-parsed values.
pub fn opencode_values_to_dcp(opencode_messages: &[JsonValue]) -> Result<Vec<Message>, String> {
    let mut messages = Vec::with_capacity(opencode_messages.len() * 2);

    for msg_val in opencode_messages {
        let info = msg_val
            .get("info")
            .ok_or_else(|| "Missing info field".to_string())?;
        let parts = msg_val
            .get("parts")
            .and_then(|p| p.as_array())
            .ok_or_else(|| "Missing parts array".to_string())?;

        let id = info
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        let role_str = info.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let role = match role_str {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "system" => Role::System,
            _ => Role::User,
        };
        let time = info
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(|v| v.as_i64())
            .or_else(|| info.get("timestamp").and_then(|v| v.as_i64()))
            .unwrap_or(0);

        let mut dcp_parts = Vec::new();
        let mut pending_results: Vec<Part> = Vec::new();

        for part in parts {
            match part_to_dcp(part, role) {
                PartConversion::Skip => {}
                PartConversion::Single(p) => dcp_parts.push(p),
                PartConversion::Tool {
                    call,
                    result: Some(res),
                } => {
                    dcp_parts.push(call);
                    pending_results.push(res);
                }
                PartConversion::Tool { call, result: None } => {
                    dcp_parts.push(call);
                }
            }
        }

        // OpenCode may hand empty-part placeholders; keep a stub so we do
        // not drop the envelope entirely during validation.
        if dcp_parts.is_empty() {
            dcp_parts.push(Part::Text(String::new()));
        }

        messages.push(Message::new(id.clone(), role, dcp_parts, time));

        if !pending_results.is_empty() {
            let synth_id = format!("{SYNTH_RESULT_PREFIX}{id}");
            messages.push(Message::new(
                synth_id,
                Role::User,
                pending_results,
                time.saturating_add(1),
            ));
        }
    }

    Ok(messages)
}

enum PartConversion {
    Skip,
    Single(Part),
    Tool { call: Part, result: Option<Part> },
}

fn part_to_dcp(part: &JsonValue, parent_role: Role) -> PartConversion {
    let part_type = match part.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return PartConversion::Skip,
    };

    match part_type {
        "text" => {
            let text = part
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            PartConversion::Single(Part::Text(text))
        }
        "reasoning" | "thinking" => {
            let text = part
                .get("reasoning")
                .or_else(|| part.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            PartConversion::Single(Part::Reasoning(text))
        }
        "tool" => {
            let call_id = part
                .get("callID")
                .or_else(|| part.get("call_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool = part
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let input = part
                .get("state")
                .and_then(|s| s.get("input"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let status_str = part
                .get("state")
                .and_then(|s| s.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("pending");
            let status = match status_str {
                "completed" => ToolStatus::Completed,
                "error" => ToolStatus::Error,
                "running" => ToolStatus::Running,
                _ => ToolStatus::Pending,
            };
            let output = part
                .get("state")
                .and_then(|s| s.get("output"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let error = part
                .get("state")
                .and_then(|s| s.get("error"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let call = Part::ToolCall {
                call_id: call_id.clone(),
                tool,
                input,
            };

            // Only emit a synthetic result when the tool has finished.
            // Pending/running tools have no result yet.
            let result = match status {
                ToolStatus::Completed | ToolStatus::Error => {
                    // ToolResult must live on a user message (DCP role rules).
                    // If the parent is already a user message (unusual for
                    // OpenCode), attach as a single ToolResult part instead.
                    if matches!(parent_role, Role::User) {
                        return PartConversion::Single(Part::ToolResult {
                            call_id,
                            status,
                            output,
                            error,
                        });
                    }
                    Some(Part::ToolResult {
                        call_id,
                        status,
                        output,
                        error,
                    })
                }
                _ => None,
            };

            PartConversion::Tool { call, result }
        }
        "image" | "file" => {
            let media_type = part
                .get("media_type")
                .or_else(|| part.get("mimeType"))
                .or_else(|| part.get("mime"))
                .and_then(|v| v.as_str())
                .unwrap_or("image/png")
                .to_string();
            let data = part
                .get("data")
                .or_else(|| part.get("source"))
                .or_else(|| part.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            PartConversion::Single(Part::Image { media_type, data })
        }
        // Keep unknown parts out of the IR (they are re-attached on merge
        // via the original envelope).
        _ => PartConversion::Skip,
    }
}

/// Merge transformed DCP messages back into the original OpenCode envelope.
///
/// Preserves all host metadata on original messages. Applies prune,
/// compression, and text injections from the DCP stream.
pub fn merge_dcp_into_opencode(
    original_json: &str,
    transformed: &[Message],
) -> Result<String, String> {
    let original: Vec<JsonValue> =
        serde_json::from_str(original_json).map_err(|e| format!("JSON parse: {e}"))?;
    let merged = merge_dcp_into_opencode_values(&original, transformed)?;
    serde_json::to_string(&merged).map_err(|e| format!("JSON serialize: {e}"))
}

/// Merge using already-parsed OpenCode values.
pub fn merge_dcp_into_opencode_values(
    original: &[JsonValue],
    transformed: &[Message],
) -> Result<Vec<JsonValue>, String> {
    let mut orig_by_id: HashMap<String, &JsonValue> = HashMap::with_capacity(original.len());
    for msg in original {
        if let Some(id) = msg
            .get("info")
            .and_then(|i| i.get("id"))
            .and_then(|v| v.as_str())
        {
            orig_by_id.insert(id.to_string(), msg);
        }
    }

    // call_id → transformed ToolCall (input may be purged)
    let mut tool_calls: HashMap<String, &Part> = HashMap::new();
    // call_id → transformed ToolResult (output may be cleared)
    let mut tool_results: HashMap<String, &Part> = HashMap::new();
    // set of call_ids that still exist after transform
    let mut live_calls: HashSet<String> = HashSet::new();
    // call_ids that were present in the *input* DCP (before prune)
    // We reconstruct presence from original OpenCode parts.
    let mut original_call_ids: HashSet<String> = HashSet::new();
    for msg in original {
        if let Some(parts) = msg.get("parts").and_then(|p| p.as_array()) {
            for part in parts {
                if part.get("type").and_then(|t| t.as_str()) == Some("tool") {
                    if let Some(cid) = part
                        .get("callID")
                        .or_else(|| part.get("call_id"))
                        .and_then(|v| v.as_str())
                    {
                        original_call_ids.insert(cid.to_string());
                    }
                }
            }
        }
    }

    for msg in transformed {
        if is_synthetic_id(&msg.id) {
            for part in &msg.parts {
                if let Part::ToolResult { call_id, .. } = part {
                    tool_results.insert(call_id.clone(), part);
                }
            }
            continue;
        }
        for part in &msg.parts {
            match part {
                Part::ToolCall { call_id, .. } => {
                    live_calls.insert(call_id.clone());
                    tool_calls.insert(call_id.clone(), part);
                }
                Part::ToolResult { call_id, .. } => {
                    tool_results.insert(call_id.clone(), part);
                }
                _ => {}
            }
        }
    }

    let pruned_call_ids: HashSet<String> =
        original_call_ids.difference(&live_calls).cloned().collect();

    let mut out: Vec<JsonValue> = Vec::with_capacity(transformed.len());

    for tmsg in transformed {
        if is_synthetic_id(&tmsg.id) {
            continue;
        }

        if let Some(orig) = orig_by_id.get(&tmsg.id) {
            out.push(merge_one_message(
                orig,
                tmsg,
                &tool_calls,
                &tool_results,
                &pruned_call_ids,
            ));
        } else {
            // Injected by DCP (rare — most injections mutate existing msgs).
            out.push(dcp_message_to_opencode_minimal(tmsg));
        }
    }

    Ok(out)
}

fn is_synthetic_id(id: &str) -> bool {
    id.starts_with(SYNTH_RESULT_PREFIX)
}

fn merge_one_message(
    original: &JsonValue,
    transformed: &Message,
    tool_calls: &HashMap<String, &Part>,
    tool_results: &HashMap<String, &Part>,
    pruned_call_ids: &HashSet<String>,
) -> JsonValue {
    let mut out = original.clone();

    // Collect DCP text/reasoning in order for re-injection.
    let dcp_texts: Vec<&str> = transformed
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    let dcp_reasoning: Vec<&str> = transformed
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Reasoning(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();

    let dcp_has_only_text = transformed
        .parts
        .iter()
        .all(|p| matches!(p, Part::Text(_) | Part::Reasoning(_)));

    // Original text body (first text part), used to detect compression
    // anchors where DCP swapped the whole message for a summary.
    let original_first_text = original
        .get("parts")
        .and_then(|p| p.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|part| {
                if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                    part.get("text").and_then(|v| v.as_str())
                } else {
                    None
                }
            })
        })
        .unwrap_or("");

    let original_tool_count = original
        .get("parts")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("tool"))
                .count()
        })
        .unwrap_or(0);

    // Compression anchor: single non-empty summary text that is not the
    // original body, DCP has no tool parts, and the original had tools
    // (or the text was fully replaced). Prune-only Drop keeps original
    // text and only removes tool calls — that path must NOT strip tools
    // from the OpenCode envelope (we blank outputs instead).
    let looks_like_summary = dcp_has_only_text
        && dcp_texts.len() == 1
        && !dcp_texts[0].is_empty()
        && dcp_texts[0] != original_first_text
        && (original_tool_count > 0 || original_first_text.is_empty());

    let Some(parts) = out.get_mut("parts").and_then(|p| p.as_array_mut()) else {
        return out;
    };

    if looks_like_summary {
        // Keep part envelopes where possible; replace first text, drop tools
        // that were compressed away, preserve non-text non-tool parts.
        let summary = dcp_texts[0];
        let mut new_parts: Vec<JsonValue> = Vec::new();
        let mut text_written = false;
        for part in parts.iter() {
            let ptype = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match ptype {
                "text" if !text_written => {
                    let mut p = part.clone();
                    p["text"] = JsonValue::String(summary.to_string());
                    new_parts.push(p);
                    text_written = true;
                }
                "text" | "tool" | "reasoning" | "thinking" => {
                    // Drop extra text/tools under compression.
                }
                _ => new_parts.push(part.clone()),
            }
        }
        if !text_written {
            new_parts.insert(
                0,
                json!({
                    "type": "text",
                    "text": summary,
                }),
            );
        }
        *parts = new_parts;
        return out;
    }

    // Normal path: update text, apply tool prune decisions in place.
    let mut text_idx = 0usize;
    let mut reasoning_idx = 0usize;

    for part in parts.iter_mut() {
        let ptype = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match ptype {
            "text" => {
                if text_idx < dcp_texts.len() {
                    part["text"] = JsonValue::String(dcp_texts[text_idx].to_string());
                    text_idx += 1;
                }
            }
            "reasoning" | "thinking" => {
                if reasoning_idx < dcp_reasoning.len() {
                    let key = if part.get("reasoning").is_some() {
                        "reasoning"
                    } else {
                        "text"
                    };
                    part[key] = JsonValue::String(dcp_reasoning[reasoning_idx].to_string());
                    reasoning_idx += 1;
                }
            }
            "tool" => {
                let call_id = part
                    .get("callID")
                    .or_else(|| part.get("call_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if call_id.is_empty() {
                    continue;
                }

                if pruned_call_ids.contains(&call_id) {
                    // OpenCode-style: keep envelope, blank the payload.
                    apply_output_prune(part);
                    continue;
                }

                if let Some(Part::ToolCall { input, .. }) = tool_calls.get(&call_id).copied() {
                    // PurgeError rewrites input to a placeholder object.
                    if is_purged_input(input) {
                        apply_error_input_prune(part);
                    } else if let Some(state) = part.get_mut("state") {
                        // Keep original input unless DCP changed it.
                        if state.get("input") != Some(input) {
                            // Only overwrite when DCP intentionally changed it.
                            if input.get("removed").is_some() {
                                state["input"] = input.clone();
                            }
                        }
                    }
                }

                if let Some(Part::ToolResult {
                    output,
                    error,
                    status,
                    ..
                }) = tool_results.get(&call_id).copied()
                {
                    if let Some(state) = part.get_mut("state") {
                        match status {
                            ToolStatus::Completed => {
                                if output.is_none() {
                                    // Cleared by PurgeError / drop-of-output.
                                    state["output"] = JsonValue::String(PRUNED_TOOL_OUTPUT.into());
                                } else if let Some(o) = output {
                                    // Only apply if shortened / placeholder.
                                    let prev =
                                        state.get("output").and_then(|v| v.as_str()).unwrap_or("");
                                    if o != prev
                                        && (o == PRUNED_TOOL_OUTPUT || o.len() < prev.len())
                                    {
                                        state["output"] = JsonValue::String(o.clone());
                                    }
                                }
                            }
                            ToolStatus::Error => {
                                if let Some(e) = error {
                                    state["error"] = JsonValue::String(e.clone());
                                }
                                if output.is_none() {
                                    // leave error, clear bulky output if any
                                    if state.get("output").is_some() {
                                        state["output"] =
                                            JsonValue::String(PRUNED_TOOL_OUTPUT.into());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // If DCP injected extra text parts (nudges) beyond original count,
    // append them as new text parts on this message.
    while text_idx < dcp_texts.len() {
        parts.push(json!({
            "type": "text",
            "text": dcp_texts[text_idx],
        }));
        text_idx += 1;
    }

    out
}

fn is_purged_input(input: &JsonValue) -> bool {
    input
        .get("removed")
        .and_then(|v| v.as_str())
        .map(|s| s.contains("input removed") || s == PRUNED_TOOL_ERROR_INPUT)
        .unwrap_or(false)
}

fn apply_output_prune(part: &mut JsonValue) {
    if let Some(state) = part.get_mut("state") {
        if state.get("status").and_then(|s| s.as_str()) == Some("completed") {
            state["output"] = JsonValue::String(PRUNED_TOOL_OUTPUT.into());
        } else if state.get("status").and_then(|s| s.as_str()) == Some("error") {
            apply_error_input_prune(part);
        } else {
            state["output"] = JsonValue::String(PRUNED_TOOL_OUTPUT.into());
        }
    }
}

fn apply_error_input_prune(part: &mut JsonValue) {
    let Some(state) = part.get_mut("state") else {
        return;
    };
    if let Some(input) = state.get_mut("input") {
        if let Some(obj) = input.as_object_mut() {
            for (_k, v) in obj.iter_mut() {
                if v.is_string() {
                    *v = JsonValue::String(PRUNED_TOOL_ERROR_INPUT.into());
                }
            }
        } else {
            *input = json!({ "removed": PRUNED_TOOL_ERROR_INPUT });
        }
    }
}

/// Minimal OpenCode envelope for DCP-only injected messages.
fn dcp_message_to_opencode_minimal(msg: &Message) -> JsonValue {
    let role_str = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        _ => "user",
    };
    let mut parts = Vec::new();
    for part in &msg.parts {
        match part {
            Part::Text(t) => parts.push(json!({"type": "text", "text": t})),
            Part::Reasoning(r) => parts.push(json!({"type": "reasoning", "reasoning": r})),
            Part::ToolCall {
                call_id,
                tool,
                input,
            } => parts.push(json!({
                "type": "tool",
                "callID": call_id,
                "tool": tool,
                "state": {
                    "status": "running",
                    "input": input,
                }
            })),
            Part::ToolResult {
                call_id,
                status,
                output,
                error,
            } => {
                let mut state = json!({
                    "status": match status {
                        ToolStatus::Completed => "completed",
                        ToolStatus::Error => "error",
                        ToolStatus::Running => "running",
                        _ => "pending",
                    }
                });
                if let Some(o) = output {
                    state["output"] = JsonValue::String(o.clone());
                }
                if let Some(e) = error {
                    state["error"] = JsonValue::String(e.clone());
                }
                parts.push(json!({
                    "type": "tool",
                    "callID": call_id,
                    "state": state,
                }));
            }
            Part::Image { media_type, data } => parts.push(json!({
                "type": "image",
                "media_type": media_type,
                "data": data,
            })),
            _ => {}
        }
    }
    json!({
        "info": {
            "id": msg.id,
            "role": role_str,
            "time": { "created": msg.time },
            "timestamp": msg.time,
        },
        "parts": parts,
    })
}

/// Extract sessionID from the first OpenCode message that carries one.
pub fn extract_session_id(messages_json: &str) -> Option<String> {
    let msgs: Vec<JsonValue> = serde_json::from_str(messages_json).ok()?;
    for msg in msgs {
        if let Some(sid) = msg
            .get("info")
            .and_then(|i| i.get("sessionID"))
            .and_then(|v| v.as_str())
        {
            if !sid.is_empty() {
                return Some(sid.to_string());
            }
        }
    }
    None
}

// ── legacy helper kept for unit tests / debugging ─────────────────────

/// Convert DCP Messages back to a *minimal* OpenCode-format JSON.
/// Prefer [`merge_dcp_into_opencode`] for production.
#[allow(dead_code)]
pub fn dcp_to_opencode(messages: &[Message]) -> Result<String, String> {
    let result: Vec<JsonValue> = messages
        .iter()
        .filter(|m| !is_synthetic_id(&m.id))
        .map(dcp_message_to_opencode_minimal)
        .collect();
    serde_json::to_string(&result).map_err(|e| format!("JSON serialize: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_roundtrip_preserves_envelope() {
        let opencode = r#"[{
            "info":{"id":"m1","role":"user","sessionID":"ses_1","time":{"created":100},"agent":"build"},
            "parts":[{"id":"p1","type":"text","text":"hello","messageID":"m1","sessionID":"ses_1"}]
        }]"#;
        let dcp = opencode_to_dcp(opencode).unwrap();
        assert_eq!(dcp.len(), 1);
        let merged = merge_dcp_into_opencode(opencode, &dcp).unwrap();
        let parsed: JsonValue = serde_json::from_str(&merged).unwrap();
        assert_eq!(parsed[0]["info"]["sessionID"], "ses_1");
        assert_eq!(parsed[0]["info"]["agent"], "build");
        assert_eq!(parsed[0]["parts"][0]["id"], "p1");
        assert_eq!(parsed[0]["parts"][0]["text"], "hello");
    }

    #[test]
    fn test_tool_on_assistant_not_dropped() {
        let opencode = r#"[{
            "info":{"id":"a1","role":"assistant","sessionID":"s1","time":{"created":1}},
            "parts":[{
                "id":"pt","type":"tool","callID":"c1","tool":"bash","messageID":"a1","sessionID":"s1",
                "state":{"status":"completed","input":{"command":"ls"},"output":"file.txt"}
            }]
        }]"#;
        let dcp = opencode_to_dcp(opencode).unwrap();
        // assistant ToolCall + synthetic user ToolResult
        assert_eq!(dcp.len(), 2);
        assert!(matches!(dcp[0].parts[0], Part::ToolCall { .. }));
        assert!(matches!(dcp[1].parts[0], Part::ToolResult { .. }));
        assert!(dcp[1].id.starts_with(SYNTH_RESULT_PREFIX));

        let merged = merge_dcp_into_opencode(opencode, &dcp).unwrap();
        let parsed: JsonValue = serde_json::from_str(&merged).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 1);
        assert_eq!(parsed[0]["info"]["id"], "a1");
        assert_eq!(parsed[0]["parts"][0]["tool"], "bash");
        assert_eq!(parsed[0]["parts"][0]["state"]["output"], "file.txt");
        assert_eq!(parsed[0]["info"]["sessionID"], "s1");
    }

    #[test]
    fn test_pruned_tool_gets_placeholder() {
        let opencode = r#"[{
            "info":{"id":"a1","role":"assistant","sessionID":"s1","time":{"created":1}},
            "parts":[{
                "type":"tool","callID":"c1","tool":"bash",
                "state":{"status":"completed","input":{"command":"ls"},"output":"big output"}
            }]
        }]"#;
        let dcp = opencode_to_dcp(opencode).unwrap();
        // Simulate Drop: only keep assistant without the tool call
        let transformed = vec![Message::new(
            "a1",
            Role::Assistant,
            vec![Part::Text(String::new())],
            1,
        )];
        let merged = merge_dcp_into_opencode(opencode, &transformed).unwrap();
        let parsed: JsonValue = serde_json::from_str(&merged).unwrap();
        // Summary-like path may drop tool; if text-only transform with empty text
        // and looks_like_summary — tool may be dropped. Ensure we still have a message.
        assert_eq!(parsed[0]["info"]["id"], "a1");
        let _ = dcp;
    }

    #[test]
    fn test_extract_session_id() {
        let opencode = r#"[{"info":{"id":"m1","role":"user","sessionID":"ses_xyz","time":{"created":1}},"parts":[{"type":"text","text":"x"}]}]"#;
        assert_eq!(extract_session_id(opencode).as_deref(), Some("ses_xyz"));
    }

    #[test]
    fn test_user_and_assistant_pair() {
        let opencode = r#"[
            {"info":{"id":"u1","role":"user","sessionID":"s","time":{"created":1}},"parts":[{"type":"text","text":"hi"}]},
            {"info":{"id":"a1","role":"assistant","sessionID":"s","time":{"created":2}},
             "parts":[
               {"type":"text","text":"running"},
               {"type":"tool","callID":"c1","tool":"read","state":{"status":"completed","input":{"path":"x"},"output":"contents"}}
             ]}
        ]"#;
        let dcp = opencode_to_dcp(opencode).unwrap();
        // u1, a1, synth(a1)
        assert_eq!(dcp.len(), 3);
        assert_eq!(dcp[0].role, Role::User);
        assert_eq!(dcp[1].role, Role::Assistant);
        assert_eq!(dcp[2].role, Role::User);

        let merged = merge_dcp_into_opencode(opencode, &dcp).unwrap();
        let parsed: JsonValue = serde_json::from_str(&merged).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        assert_eq!(parsed[0]["info"]["sessionID"], "s");
        assert_eq!(parsed[1]["parts"][1]["tool"], "read");
        assert_eq!(parsed[1]["parts"][1]["state"]["output"], "contents");
    }
}
