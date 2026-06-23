use dcp_types::{Message, Part, Role, ToolStatus};

/// Convert OpenCode-format messages (JSON array of {info, parts}) to DCP Messages.
pub fn opencode_to_dcp(json_str: &str) -> Result<Vec<Message>, String> {
    let opencode_messages: Vec<serde_json::Value> = serde_json::from_str(json_str)
        .map_err(|e| format!("JSON parse: {}", e))?;

    let mut messages = Vec::new();
    for msg_val in &opencode_messages {
        let info = msg_val.get("info").ok_or("Missing info field")?;
        let parts = msg_val
            .get("parts")
            .and_then(|p| p.as_array())
            .ok_or("Missing parts array")?;

        let id = info
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        let role_str = info
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("user");
        let role = match role_str {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "system" => Role::System,
            _ => Role::User,
        };
        let time = info
            .get("timestamp")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let mut dcp_parts = Vec::new();
        for part in parts {
            if let Some(pt) = part_to_dcp(part) {
                dcp_parts.push(pt);
            }
        }

        messages.push(Message::new(id, role, dcp_parts, time));
    }

    Ok(messages)
}

fn part_to_dcp(part: &serde_json::Value) -> Option<Part> {
    let part_type = part.get("type")?.as_str()?;
    match part_type {
        "text" => {
            let text = part.get("text")?.as_str()?;
            Some(Part::Text(text.to_string()))
        }
        "reasoning" | "thinking" => {
            let text = part
                .get("reasoning")
                .or_else(|| part.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(Part::Reasoning(text.to_string()))
        }
        "tool" => {
            let call_id = part
                .get("callID")
                .or_else(|| part.get("call_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
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
            let status = part
                .get("state")
                .and_then(|s| s.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("pending");
            let ts = match status {
                "completed" => ToolStatus::Completed,
                "error" => ToolStatus::Error,
                "running" => ToolStatus::Running,
                _ => ToolStatus::Pending,
            };
            Some(Part::ToolResult {
                call_id,
                status: ts,
                output,
                error,
            })
        }
        "image" => {
            let media_type = part
                .get("media_type")
                .or_else(|| part.get("mimeType"))
                .and_then(|v| v.as_str())
                .unwrap_or("image/png")
                .to_string();
            let data = part
                .get("data")
                .or_else(|| part.get("source"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(Part::Image { media_type, data })
        }
        _ => None,
    }
}

/// Convert DCP Messages back to OpenCode-format JSON.
pub fn dcp_to_opencode(messages: &[Message]) -> Result<String, String> {
    let mut result = Vec::new();
    for msg in messages {
        let role_str = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
            _ => "unknown",
        };

        let mut parts_json = Vec::new();
        for part in &msg.parts {
            parts_json.push(part_to_json(part));
        }

        result.push(serde_json::json!({
            "info": {
                "id": msg.id,
                "role": role_str,
                "timestamp": msg.time
            },
            "parts": parts_json
        }));
    }

    serde_json::to_string(&result)
        .map_err(|e| format!("JSON serialize: {}", e))
}

fn part_to_json(part: &Part) -> serde_json::Value {
    match part {
        Part::Text(t) => serde_json::json!({"type": "text", "text": t}),
        Part::Reasoning(r) => serde_json::json!({"type": "reasoning", "reasoning": r}),
        Part::ToolCall {
            call_id,
            tool,
            input,
        } => serde_json::json!({
            "type": "tool",
            "callID": call_id,
            "tool": tool,
            "state": {
                "status": "running",
                "input": input
            }
        }),
        Part::ToolResult {
            call_id,
            status,
            output,
            error,
        } => {
            let mut obj = serde_json::json!({
                "type": "tool",
                "callID": call_id,
                "state": {
                    "status": match status {
                        ToolStatus::Completed => "completed",
                        ToolStatus::Error => "error",
                        _ => "pending",
                    }
                }
            });
            if let Some(o) = output {
                obj["state"]["output"] = serde_json::Value::String(o.clone());
            }
            if let Some(e) = error {
                obj["state"]["error"] = serde_json::Value::String(e.clone());
            }
            obj
        }
        Part::Image {
            media_type,
            data,
        } => serde_json::json!({
            "type": "image",
            "media_type": media_type,
            "data": data
        }),
        _ => serde_json::json!({"type": "unknown"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_roundtrip() {
        let opencode = r#"[{"info":{"id":"m1","role":"user","timestamp":100},"parts":[{"type":"text","text":"hello"}]}]"#;
        let dcp_msgs = opencode_to_dcp(opencode).unwrap();
        assert_eq!(dcp_msgs.len(), 1);
        assert_eq!(dcp_msgs[0].id, "m1");
        assert!(matches!(dcp_msgs[0].parts[0], Part::Text(_)));

        let back = dcp_to_opencode(&dcp_msgs).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&back).unwrap();
        assert_eq!(parsed[0]["info"]["id"], "m1");
        assert_eq!(parsed[0]["parts"][0]["type"], "text");
        assert_eq!(parsed[0]["parts"][0]["text"], "hello");
    }

    #[test]
    fn test_tool_roundtrip() {
        let opencode = r#"[{"info":{"id":"m2","role":"assistant","timestamp":200},"parts":[{"type":"tool","callID":"call1","tool":"bash","state":{"status":"completed","output":"done","input":{}}}]}]"#;
        let dcp_msgs = opencode_to_dcp(opencode).unwrap();
        assert!(matches!(dcp_msgs[0].parts[0], Part::ToolResult { .. }));

        let back = dcp_to_opencode(&dcp_msgs).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&back).unwrap();
        assert_eq!(parsed[0]["parts"][0]["state"]["status"], "completed");
        assert_eq!(parsed[0]["parts"][0]["state"]["output"], "done");
    }

    #[test]
    fn test_multiple_messages() {
        let opencode = r#"[
            {"info":{"id":"m1","role":"user","timestamp":0},"parts":[{"type":"text","text":"hi"}]},
            {"info":{"id":"m2","role":"assistant","timestamp":1},"parts":[{"type":"text","text":"hello"}]}
        ]"#;
        let dcp_msgs = opencode_to_dcp(opencode).unwrap();
        assert_eq!(dcp_msgs.len(), 2);
        assert_eq!(dcp_msgs[0].role, Role::User);
        assert_eq!(dcp_msgs[1].role, Role::Assistant);
    }
}
