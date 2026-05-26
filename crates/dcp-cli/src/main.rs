//! `dcp-cli` — Interactive REPL for testing dynamic context pruning.

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use dcp_config::Config;
use dcp_core::ContextPruner;
use dcp_types::Message;
use serde_json::Value as JsonValue;

fn parse_msg_json(obj: &serde_json::Map<String, JsonValue>) -> Option<Message> {
    let id = obj.get("id")?.as_str()?.to_string();
    let role = match obj.get("role")?.as_str()? {
        "user" => dcp_types::Role::User,
        "assistant" => dcp_types::Role::Assistant,
        _ => return None,
    };
    let time = obj.get("time").and_then(|v| v.as_i64()).unwrap_or(0);
    let parts: Vec<dcp_types::Part> = obj
        .get("parts")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().filter_map(|p| {
                let p_obj = p.as_object()?;
                let t = p_obj.get("type")?.as_str()?;
                match t {
                    "text" => Some(dcp_types::Part::Text(p_obj.get("text")?.as_str()?.to_string())),
                    "reasoning" => Some(dcp_types::Part::Reasoning(
                        p_obj.get("text").or_else(|| p_obj.get("reasoning"))?.as_str()?.to_string(),
                    )),
                    "tool_call" | "tool" => {
                        let call_id = p_obj.get("call_id")?.as_str()?.to_string();
                        let tool = p_obj.get("tool")?.as_str()?.to_string();
                        let input = p_obj.get("input").or_else(|| p_obj.get("state")).cloned().unwrap_or(JsonValue::Null);
                        Some(dcp_types::Part::ToolCall { call_id, tool, input })
                    }
                    "tool_result" => {
                        let call_id = p_obj.get("call_id")?.as_str()?.to_string();
                        let status = p_obj.get("status").or_else(|| p_obj.get("state").and_then(|s| s.get("status")))
                            .and_then(|v| v.as_str()).map(|s| match s {
                                "completed" => dcp_types::ToolStatus::Completed,
                                "error" => dcp_types::ToolStatus::Error,
                                _ => dcp_types::ToolStatus::Pending,
                            }).unwrap_or(dcp_types::ToolStatus::Pending);
                        let output = p_obj.get("output").or_else(|| p_obj.get("state").and_then(|s| s.get("output")))
                            .and_then(|v| v.as_str()).map(String::from);
                        let error = p_obj.get("error").or_else(|| p_obj.get("state").and_then(|s| s.get("error")))
                            .and_then(|v| v.as_str()).map(String::from);
                        Some(dcp_types::Part::ToolResult { call_id, status, output, error })
                    }
                    _ => None,
                }
            }).collect()
        })
        .unwrap_or_default();
    Some(dcp_types::Message::new(id, role, parts, time))
}

fn lower_message(msg: &Message) -> JsonValue {
    serde_json::json!({
        "id": msg.id,
        "role": match msg.role {
            dcp_types::Role::User => "user",
            dcp_types::Role::Assistant => "assistant",
            _ => "",
        },
        "parts": msg.parts.iter().map(|p| match p {
            dcp_types::Part::Text(t) => serde_json::json!({ "type": "text", "text": t }),
            dcp_types::Part::Reasoning(r) => serde_json::json!({ "type": "reasoning", "reasoning": r }),
            dcp_types::Part::ToolCall { call_id, tool, input } => serde_json::json!({
                "type": "tool_call", "call_id": call_id, "tool": tool, "input": input,
            }),
            dcp_types::Part::ToolResult { call_id, status, output, error } => {
                let mut obj = serde_json::json!({
                    "type": "tool_result",
                    "call_id": call_id,
                    "status": match status {
                        dcp_types::ToolStatus::Pending => "pending",
                        dcp_types::ToolStatus::Completed => "completed",
                        dcp_types::ToolStatus::Error => "error",
                        _ => "unknown",
                    },
                });
                if let Some(o) = output { obj["output"] = JsonValue::String(o.to_string()); }
                if let Some(e) = error { obj["error"] = JsonValue::String(e.to_string()); }
                obj
            }
            _ => serde_json::json!({}),
        }).collect::<Vec<_>>(),
        "time": msg.time,
    })
}

fn print_msg(msg: &Message, idx: usize) {
    let role_str = match msg.role {
        dcp_types::Role::User => "user",
        dcp_types::Role::Assistant => "assistant",
        _ => "unknown",
    };
    print!("[{}] {} ({} parts): ", idx, role_str, msg.parts.len());
    for (i, p) in msg.parts.iter().enumerate() {
        if i > 0 { print!(" | "); }
        match p {
            dcp_types::Part::Text(t) => print!("text: {:.40}", t.chars().take(40).collect::<String>()),
            dcp_types::Part::Reasoning(r) => print!("reasoning: {:.40}", r.chars().take(40).collect::<String>()),
            dcp_types::Part::ToolCall { call_id, tool, .. } => print!("tool_call({}): {}", call_id.chars().take(8).collect::<String>(), tool),
            dcp_types::Part::ToolResult { call_id, status, .. } => print!("tool_result({}): {:?}", call_id, status),
            _ => print!("..."),
        }
    }
    println!();
}

fn main() -> anyhow::Result<()> {
    let config = Config::load_default().unwrap_or_else(|_| Config::default());
    let config = Arc::new(config);
    let mut pruner = ContextPruner::new((*config).clone())?;

    println!("=== dcp-cli interactive REPL ===");
    println!("Commands:");
    println!("  load <json_file>   Load messages from JSON file");
    println!("  list              List current messages");
    println!("  transform         Run transform on current messages");
    println!("  stats             Show DCP config");
    println!("  push <json>       Add a message (JSON on command line)");
    println!("  clear             Clear all messages");
    println!("  quit              Exit");
    println!();

    let mut messages: Vec<Message> = Vec::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    loop {
        print!("dcp> ");
        stdout.flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).is_err() {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        match parts[0] {
            "quit" | "exit" => break,
            "list" => {
                if messages.is_empty() {
                    println!("  (no messages)");
                } else {
                    for (i, msg) in messages.iter().enumerate() {
                        print_msg(msg, i);
                    }
                }
            }
            "stats" => {
                println!("  enabled: {}", config.enabled);
                println!("  debug: {}", config.debug);
                println!("  cache_stability_mode: {:?}", config.cache_stability_mode);
            }
            "clear" => {
                messages.clear();
                println!("  cleared");
            }
            "load" => {
                if parts.len() < 2 {
                    println!("  usage: load <json_file>");
                    continue;
                }
                let path = parts[1];
                let content = std::fs::read_to_string(path)?;
                let json: JsonValue = serde_json::from_str(&content)?;
                let arr = json.as_array().map(|a| a.clone()).unwrap_or_else(Vec::new);
                messages.clear();
                for (i, v) in arr.iter().enumerate() {
                    if let Some(obj) = v.as_object() {
                        if let Some(msg) = parse_msg_json(obj) {
                            messages.push(msg);
                        } else {
                            println!("  warning: skipped message at index {}", i);
                        }
                    }
                }
                println!("  loaded {} messages", messages.len());
            }
            "push" => {
                if parts.len() < 2 {
                    println!("  usage: push <json>");
                    continue;
                }
                let json: JsonValue = serde_json::from_str(parts[1])?;
                if let Some(obj) = json.as_object() {
                    if let Some(msg) = parse_msg_json(obj) {
                        messages.push(msg);
                        println!("  added message");
                    } else {
                        println!("  error: could not parse message");
                    }
                } else {
                    println!("  error: expected JSON object");
                }
            }
            "transform" => {
                if messages.is_empty() {
                    println!("  (no messages to transform)");
                    continue;
                }
                match pruner.transform_messages(messages.clone()) {
                    Ok(pruned) => {
                        let before = messages.len();
                        messages = pruned;
                        println!("  transformed: {} -> {} messages", before, messages.len());
                    }
                    Err(e) => {
                        println!("  transform error: {:?}", e);
                    }
                }
            }
            _ => {
                println!("  unknown command: {}", parts[0]);
            }
        }
    }

    println!("\nGoodbye!");
    Ok(())
}
