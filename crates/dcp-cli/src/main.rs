//! `dcp-cli` — Interactive REPL and one-shot CLI for dynamic context pruning.

use std::io::{self, BufRead, Write};

use anyhow::Context as _;
use dcp_config::Config;
use dcp_core::ContextPruner;
use dcp_types::{Message, Part, Role};
use serde_json::Value as JsonValue;

// ============================================================================
// CLI configuration
// ============================================================================

/// Global CLI state managed across REPL sessions.
struct CliState {
    messages: Vec<Message>,
    pruner: ContextPruner,
    session_id: Option<String>,
    debug: bool,
}

impl CliState {
    fn new() -> anyhow::Result<Self> {
        let config = Config::load_default().unwrap_or_else(|_| Config::default());
        let pruner = ContextPruner::builder()
            .config(config.clone())
            .build()?;
        Ok(Self {
            messages: Vec::new(),
            pruner,
            session_id: None,
            debug: false,
        })
    }

    /// Load messages from a JSON file.
    fn load_messages(&mut self, path: &str) -> anyhow::Result<usize> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("failed to read file: {path}"))?;
        let json: JsonValue =
            serde_json::from_str(&content).with_context(|| format!("failed to parse JSON from: {path}"))?;
        let arr = json.as_array().cloned().unwrap_or_default();
        self.messages.clear();
        let mut count = 0;
        for (i, v) in arr.iter().enumerate() {
            if let Some(obj) = v.as_object() {
                if let Some(msg) = parse_msg_json(obj) {
                    self.messages.push(msg);
                    count += 1;
                } else {
                    eprintln!("  warning: skipped message at index {}", i);
                }
            }
        }
        Ok(count)
    }

    /// Save current session state to disk.
    fn save_session(&self) -> anyhow::Result<()> {
        if self.session_id.is_some() {
            self.pruner.save().context("save failed")?;
            println!("  session saved");
        } else {
            println!("  no active session; state is in-memory only");
            println!("  (use 'session <id>' to name this session first)");
        }
        Ok(())
    }



    /// Run transform on current messages.
    fn transform(&mut self) -> anyhow::Result<usize> {
        if self.messages.is_empty() {
            return Ok(0);
        }
        let before = self.messages.len();
        match self.pruner.transform_messages(std::mem::take(&mut self.messages)) {
            Ok(pruned) => {
                let after = pruned.len();
                self.messages = pruned;
                if self.debug {
                    println!("  transform: {before} -> {after} messages");
                }
                Ok(before.saturating_sub(after))
            }
            Err(e) => anyhow::bail!("transform error: {}", e),
        }
    }

    /// Show current statistics.
    fn show_stats(&self) {
        let stats = self.pruner.stats();
        println!("  === DCP Stats ===");
        println!("  total_prune_tokens:         {}", stats.total_prune_tokens);
        println!("  dedup_pruned:              {}", stats.dedup_pruned);
        println!("  purge_errors_pruned:       {}", stats.purge_errors_pruned);
        println!("  stale_file_reads_pruned:  {}", stats.stale_file_reads_pruned);
        println!("  compress_runs:            {}", stats.compress_runs);
        println!("  compress_blocks_committed: {}", stats.compress_blocks_committed);
        println!("  compress_oversized:       {}", stats.compress_oversized);
        println!("  compress_useful:          {}", stats.compress_useful);
        println!("  compactions_observed:      {}", stats.compactions_observed);
        println!("  cache_bust_events:         {}", stats.cache_bust_events);
        println!("  orphan_tool_results:       {}", stats.orphan_tool_results);
        println!("  dropped_invalid:           {}", stats.dropped_invalid);
        println!("  storage_save_failed:       {}", stats.storage_save_failed);
        println!("  persisted_corruption:      {}", stats.persisted_corruption);
    }

    /// Show internal pruner state for debugging.
    fn show_debug_state(&self) {
        let state = self.pruner.state();
        println!("  === DCP Internal State ===");
        println!("  session_id:                  {:?}", state.session_id);
        println!("  is_subagent:                 {}", state.is_subagent);
        println!("  manual_mode.enabled:         {}", state.manual_mode.enabled);
        println!("  compress_permission:          {:?}", state.compress_permission);
        println!("  current_turn:                {}", state.current_turn);
        println!("  last_compaction:             {}", state.last_compaction);
        println!(
            "  last_message_was_asst_text:   {}",
            state.last_message_was_assistant_text
        );
        println!("  model_context_limit:         {:?}", state.model_context_limit);
        println!("  system_prompt_tokens:        {:?}", state.system_prompt_tokens);
        println!("  message_refs:                {} allocated", state.message_ids.by_raw_id.len());
        println!("  tool_parameters:             {} tracked", state.tool_parameters.len());
        println!("  tool_id_list:               {} items", state.tool_id_list.len());
        println!("  active blocks:               {}", state.prune.messages.active_block_ids.len());
        println!("  total blocks:                {}", state.prune.messages.blocks_by_id.len());
        println!("  pending prune tools:          {}", state.prune.tools.len());
    }

    /// Reset pruner state.
    fn reset_pruner(&mut self) {
        self.pruner.reset();
        self.messages.clear();
        if let Some(ref sid) = self.session_id {
            self.pruner.set_session_id(sid.clone());
        }
        println!("  pruner reset (state cleared)");
    }
}

// ============================================================================
// Message parsing helpers
// ============================================================================

fn parse_msg_json(obj: &serde_json::Map<String, JsonValue>) -> Option<Message> {
    let id = obj.get("id")?.as_str()?.to_string();
    let role = match obj.get("role")?.as_str()? {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => return None,
    };
    let time = obj.get("time").and_then(|v| v.as_i64()).unwrap_or(0);
    let parts: Vec<Part> = obj
        .get("parts")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let p_obj = p.as_object()?;
                    let t = p_obj.get("type")?.as_str()?;
                    match t {
                        "text" => Some(Part::Text(p_obj.get("text")?.as_str()?.to_string())),
                        "reasoning" => Some(Part::Reasoning(
                            p_obj
                                .get("text")
                                .or_else(|| p_obj.get("reasoning"))?
                                .as_str()?
                                .to_string(),
                        )),
                        "tool_call" | "tool" => {
                            let call_id = p_obj.get("call_id")?.as_str()?.to_string();
                            let tool = p_obj.get("tool")?.as_str()?.to_string();
                            let input = p_obj
                                .get("input")
                                .or_else(|| p_obj.get("state"))
                                .cloned()
                                .unwrap_or(JsonValue::Null);
                            Some(Part::ToolCall { call_id, tool, input })
                        }
                        "tool_result" => {
                            let call_id = p_obj.get("call_id")?.as_str()?.to_string();
                            let status = p_obj
                                .get("status")
                                .or_else(|| p_obj.get("state").and_then(|s| s.get("status")))
                                .and_then(|v| v.as_str())
                                .map(|s| match s {
                                    "completed" => dcp_types::ToolStatus::Completed,
                                    "error" => dcp_types::ToolStatus::Error,
                                    _ => dcp_types::ToolStatus::Pending,
                                })
                                .unwrap_or(dcp_types::ToolStatus::Pending);
                            let output = p_obj
                                .get("output")
                                .or_else(|| p_obj.get("state").and_then(|s| s.get("output")))
                                .and_then(|v| v.as_str())
                                .map(String::from);
                            let error = p_obj
                                .get("error")
                                .or_else(|| p_obj.get("state").and_then(|s| s.get("error")))
                                .and_then(|v| v.as_str())
                                .map(String::from);
                            Some(Part::ToolResult {
                                call_id,
                                status,
                                output,
                                error,
                            })
                        }
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Message::new(id, role, parts, time))
}

fn print_msg(msg: &Message, idx: usize) {
    let role_str = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        _ => "unknown",
    };
    print!("[{}] {} ({} parts): ", idx, role_str, msg.parts.len());
    for (i, p) in msg.parts.iter().enumerate() {
        if i > 0 {
            print!(" | ");
        }
        match p {
            Part::Text(t) => print!("text: {:.40}", t.chars().take(40).collect::<String>()),
            Part::Reasoning(r) => print!("reasoning: {:.40}", r.chars().take(40).collect::<String>()),
            Part::ToolCall { call_id, tool, .. } => {
                print!(
                    "tool_call({}): {}",
                    call_id.chars().take(8).collect::<String>(),
                    tool
                )
            }
            Part::ToolResult { call_id, status, .. } => print!(
                "tool_result({}): {:?}",
                call_id.chars().take(8).collect::<String>(),
                status
            ),
            _ => print!("..."),
        }
    }
    println!();
}

// ============================================================================
// Interactive REPL mode
// ============================================================================

fn run_repl() -> anyhow::Result<()> {
    let mut state = CliState::new()?;
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    println!("=== dcp-cli interactive REPL ===");
    println!("Commands:");
    println!("  load <json_file>   Load messages from JSON file");
    println!("  list               List current messages");
    println!("  push <json>        Add a message (JSON on command line)");
    println!("  transform          Run transform on current messages");
    println!("  save               Save current session to disk");
    println!("  load <session_id>  Load a persisted session");
    println!("  stats              Show DCP statistics");
    println!("  debug              Show internal pruner state");
    println!("  reset              Reset pruner state");
    println!("  session <id>      Set session ID");
    println!("  clear              Clear all messages");
    println!("  exit / quit        Exit");
    println!();

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

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "quit" | "exit" => break,

            "list" => {
                if state.messages.is_empty() {
                    println!("  (no messages)");
                } else {
                    for (i, msg) in state.messages.iter().enumerate() {
                        print_msg(msg, i);
                    }
                }
            }

            "stats" => {
                state.show_stats();
            }

            "debug" => {
                state.show_debug_state();
            }

            "clear" => {
                state.messages.clear();
                println!("  cleared");
            }

            "reset" => {
                state.reset_pruner();
            }

            "session" => {
                if parts.len() < 2 {
                    println!("  usage: session <id>");
                    continue;
                }
                let sid = parts[1];
                state.session_id = Some(sid.to_string());
                state.pruner.set_session_id(sid);
                println!("  session id set: {}", sid);
            }

            "load" => {
                if parts.len() < 2 {
                    println!("  usage: load <json_file>");
                    continue;
                }
                match state.load_messages(parts[1]) {
                    Ok(count) => println!("  loaded {} messages", count),
                    Err(e) => println!("  error: {}", e),
                }
            }

            "push" => {
                if parts.len() < 2 {
                    println!("  usage: push <json>");
                    continue;
                }
                let json: JsonValue = match serde_json::from_str(parts[1]) {
                    Ok(v) => v,
                    Err(e) => {
                        println!("  JSON parse error: {}", e);
                        continue;
                    }
                };
                if let Some(obj) = json.as_object() {
                    if let Some(msg) = parse_msg_json(obj) {
                        state.messages.push(msg);
                        println!("  added message");
                    } else {
                        println!("  error: could not parse message (check id/role/parts)");
                    }
                } else {
                    println!("  error: expected JSON object");
                }
            }

            "transform" => {
                match state.transform() {
                    Ok(diff) => {
                        if diff > 0 {
                            println!("  pruned {} messages", diff);
                        } else {
                            println!("  no changes");
                        }
                    }
                    Err(e) => {
                        println!("  error: {}", e);
                    }
                }
            }

            "save" => {
                if let Err(e) = state.save_session() {
                    println!("  error: {}", e);
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

// ============================================================================
// Main entry point
// ============================================================================

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "-i" || a == "--interactive") {
        run_repl()
    } else {
        // One-shot mode: first arg is input file
        let input_path = args.first().map(|s| s.as_str()).unwrap_or("-");
        let transform_flag = !args.contains(&"-n".to_string()) && !args.contains(&"--no-transform".to_string());

        // Load config
        let config = Config::load_default().unwrap_or_else(|_| Config::default());

        let mut pruner = ContextPruner::builder()
            .config(config)
            .build()?;

        // Load messages
        let content =
            std::fs::read_to_string(input_path).with_context(|| format!("failed to read: {input_path}"))?;
        let json: JsonValue =
            serde_json::from_str(&content).with_context(|| format!("failed to parse: {input_path}"))?;
        let arr = json.as_array().cloned().unwrap_or_default();
        let messages: Vec<Message> = arr
            .iter()
            .filter_map(|v| v.as_object().and_then(parse_msg_json))
            .collect();
        let original_count = messages.len();

        println!("loaded {} messages from {}", original_count, input_path);

        let result = if transform_flag {
            let transformed = pruner.transform_messages(messages)?;
            println!("transformed: {} -> {} messages", original_count, transformed.len());
            transformed
        } else {
            messages
        };

        // Show stats
        let stats = pruner.stats();
        println!();
        println!("=== Statistics ===");
        println!("total_prune_tokens:     {}", stats.total_prune_tokens);
        println!("dedup_pruned:          {}", stats.dedup_pruned);
        println!("purge_errors_pruned:    {}", stats.purge_errors_pruned);
        println!("stale_file_reads_pruned: {}", stats.stale_file_reads_pruned);
        println!("compress_runs:         {}", stats.compress_runs);
        println!("messages_in:           {}", original_count);
        println!("messages_out:          {}", result.len());

        // Output messages as JSON to stdout
        let output: Vec<JsonValue> = result
            .iter()
            .map(|msg| {
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
                        Part::Text(t) => serde_json::json!({ "type": "text", "text": t }),
                        Part::Reasoning(r) => {
                            serde_json::json!({ "type": "reasoning", "reasoning": r })
                        }
                        Part::ToolCall {
                            call_id,
                            tool,
                            input,
                        } => serde_json::json!({
                            "type": "tool_call",
                            "call_id": call_id,
                            "tool": tool,
                            "input": input,
                        }),
                        Part::ToolResult {
                            call_id,
                            status,
                            output,
                            error,
                        } => {
                            let mut obj = serde_json::json!({
                                "type": "tool_result",
                                "call_id": call_id,
                                "status": match status {
                                    dcp_types::ToolStatus::Pending => "pending",
                                    dcp_types::ToolStatus::Running => "running",
                                    dcp_types::ToolStatus::Completed => "completed",
                                    dcp_types::ToolStatus::Error => "error",
                                    _ => "unknown",
                                },
                            });
                            if let Some(o) = output {
                                obj["output"] = JsonValue::String(o.to_string());
                            }
                            if let Some(e) = error {
                                obj["error"] = JsonValue::String(e.to_string());
                            }
                            obj
                        }
                        Part::Image {
                            media_type,
                            data,
                        } => serde_json::json!({
                            "type": "image",
                            "media_type": media_type,
                            "data": data,
                        }),
                        _ => serde_json::json!({ "type": "unknown" }),
                    })
                    .collect();
                serde_json::json!({
                    "id": msg.id,
                    "role": role_str,
                    "parts": parts,
                    "time": msg.time,
                })
            })
            .collect();
        let json = serde_json::to_string_pretty(&output)?;
        println!();
        println!("=== Output ===");
        println!("{}", json);

        Ok(())
    }
}
