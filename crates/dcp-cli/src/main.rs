//! `dcp-cli` — Command-line interface for dynamic context pruning.

use std::io::{self, Read};

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use dcp_config::Config;
use dcp_core::ContextPruner;
use dcp_core::commands::CommandOutcome;
use dcp_types::{BlockId, Message, Part, Role};
use serde_json::Value as JsonValue;

mod commands;

/// Dynamic Context Pruning CLI
#[derive(Parser)]
#[command(name = "dcp")]
#[command(version, about = "Dynamic context pruning for LLM coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show token usage breakdown and pruning stats
    Context,
    /// Show session statistics (total tokens saved, message count, compression ratio)
    Stats {
        /// Session ID to display stats for
        #[arg(long = "session-id", short = 's')]
        session_id: String,
    },
    /// Show compression events over time
    Timeline {
        /// Session ID to display timeline for
        #[arg(long = "session-id", short = 's')]
        session_id: String,
    },
    /// Find sessions by ID pattern or date range
    FindSession {
        /// Glob pattern to match session IDs against
        #[arg(long = "pattern", short = 'p')]
        pattern: Option<String>,
        /// Find sessions after this date (RFC3339 or YYYY-MM-DD)
        #[arg(long = "after", short = 'a')]
        after: Option<String>,
        /// Find sessions before this date (RFC3339 or YYYY-MM-DD)
        #[arg(long = "before", short = 'b')]
        before: Option<String>,
    },
    /// Get full message payload(s) by ID
    GetMessage {
        /// One or more message IDs to retrieve
        #[arg(required = true)]
        message_ids: Vec<String>,
        /// Session ID for direct lookup
        #[arg(long = "session", short = 's')]
        session: Option<String>,
        /// Maximum sessions to scan when no --session given
        #[arg(long = "scan-sessions", default_value = "200")]
        scan_sessions: usize,
    },
    /// Token usage statistics across sessions
    TokenStats {
        /// Number of recent sessions to aggregate
        #[arg(long = "sessions", short = 'n', default_value = "10")]
        sessions: usize,
        /// Focus on a single session
        #[arg(long = "session", short = 's')]
        session: Option<String>,
        /// Output as JSON
        #[arg(long = "json")]
        json: bool,
    },
    /// Per-message token breakdown for a session
    MessageTokens {
        /// Session ID (required)
        #[arg(long = "session", short = 's')]
        session: Option<String>,
        /// Output as JSON
        #[arg(long = "json")]
        json: bool,
        /// Disable ANSI color output
        #[arg(long = "no-color")]
        no_color: bool,
    },
    /// Flush pending prune tools
    Sweep {
        /// Number of pending prune tools to flush (default: all)
        #[arg(default_value = "0")]
        count: u32,
    },
    /// Run compress tool (one-shot, reads messages from stdin or file)
    Compress {
        /// File to read messages from (use - for stdin, omit for stdin)
        file: Option<String>,
    },
    /// Restore a compressed block by ID
    Decompress {
        /// Block ID to decompress (e.g., b1, 1)
        block_id: String,
    },
    /// Re-activate a user-decompressed block
    Recompress {
        /// Block ID to recompress (e.g., b1, 1)
        block_id: String,
    },
    /// Toggle manual mode (omit to show current status)
    Manual {
        /// on/off (omit to show current status)
        enabled: Option<String>,
    },
}

/// Build a ContextPruner instance with default config.
fn build_pruner() -> anyhow::Result<ContextPruner> {
    let config = Config::load_default().unwrap_or_else(|_| Config::default());
    ContextPruner::builder()
        .config(config)
        .build()
        .context("failed to build pruner")
}

/// Read messages from stdin or a file.
fn read_messages(path: Option<&str>) -> anyhow::Result<Vec<Message>> {
    let content = if let Some(p) = path {
        if p == "-" {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read stdin")?;
            buf
        } else {
            std::fs::read_to_string(p).with_context(|| format!("failed to read file: {}", p))?
        }
    } else {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .context("failed to read stdin")?;
        buf
    };
    let json: JsonValue = serde_json::from_str(&content).with_context(|| "failed to parse JSON")?;
    // Accept both a JSON array of messages and a single message object.
    let arr = match json.as_array() {
        Some(a) => a.clone(),
        None => json
            .as_object()
            .map(|_| vec![json.clone()])
            .unwrap_or_default(),
    };
    let mut messages = Vec::new();
    for (i, v) in arr.iter().enumerate() {
        if let Some(obj) = v.as_object() {
            if let Some(msg) = parse_msg_json(obj) {
                messages.push(msg);
            } else {
                eprintln!("warning: skipped message at index {}", i);
            }
        }
    }
    Ok(messages)
}

/// Parse a JSON object into a Message.
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
                            Some(Part::ToolCall {
                                call_id,
                                tool,
                                input,
                            })
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

/// Parse block ID from string (handles b1, 1, etc.).
fn parse_block_id(raw: &str) -> Result<BlockId, String> {
    let body = raw.strip_prefix('b').unwrap_or(raw);
    body.parse::<u32>()
        .map(BlockId::new)
        .map_err(|_| format!("invalid block id: {raw:?}"))
}

/// Handle the context command.
fn handle_context(pruner: &mut ContextPruner) -> anyhow::Result<()> {
    match pruner.handle_command("context", &[], &[]) {
        CommandOutcome::Context {
            current_turn,
            active_blocks,
            total_blocks,
            pending_tokens,
            frontier,
            cache_stability_mode,
        } => {
            println!("=== DCP Context ===");
            println!("  Current turn:               {}", current_turn);
            println!("  Active blocks:               {}", active_blocks);
            println!("  Total blocks:               {}", total_blocks);
            println!("  Pending tokens:             {}", pending_tokens);
            println!("  Frontier:                   {:?}", frontier);
            println!("  Cache stability mode:       {}", cache_stability_mode);
            Ok(())
        }
        other => {
            eprintln!("unexpected outcome: {:?}", other);
            anyhow::bail!("context command failed");
        }
    }
}

/// Handle the stats command.
#[allow(dead_code)]
fn handle_stats(pruner: &ContextPruner) -> anyhow::Result<()> {
    let stats = pruner.stats();
    println!("=== DCP Statistics ===");
    println!(
        "  total_prune_tokens:          {}",
        stats.total_prune_tokens
    );
    println!("  dedup_pruned:                {}", stats.dedup_pruned);
    println!(
        "  purge_errors_pruned:         {}",
        stats.purge_errors_pruned
    );
    println!(
        "  stale_file_reads_pruned:     {}",
        stats.stale_file_reads_pruned
    );
    println!("  compress_runs:               {}", stats.compress_runs);
    println!(
        "  compress_blocks_committed:   {}",
        stats.compress_blocks_committed
    );
    println!(
        "  compress_oversized:          {}",
        stats.compress_oversized
    );
    println!("  compress_useful:             {}", stats.compress_useful);
    println!(
        "  compactions_observed:        {}",
        stats.compactions_observed
    );
    println!("  cache_bust_events:           {}", stats.cache_bust_events);
    println!(
        "  orphan_tool_results:        {}",
        stats.orphan_tool_results
    );
    println!("  dropped_invalid:             {}", stats.dropped_invalid);
    println!(
        "  invalid_status_transitions:  {}",
        stats.invalid_status_transitions
    );
    println!(
        "  normalize_depth_clamped:     {}",
        stats.normalize_depth_clamped
    );
    println!(
        "  path_null_byte_stripped:     {}",
        stats.path_null_byte_stripped
    );
    println!(
        "  storage_save_failed:         {}",
        stats.storage_save_failed
    );
    println!(
        "  persisted_corruption:        {}",
        stats.persisted_corruption
    );
    Ok(())
}

/// Handle the sweep command.
fn handle_sweep(pruner: &mut ContextPruner, _count: u32) -> anyhow::Result<()> {
    match pruner.handle_command("sweep", &[], &[]) {
        CommandOutcome::Sweep { applied_ids } => {
            println!("Flushed {} pending prune tools", applied_ids);
            Ok(())
        }
        other => {
            eprintln!("unexpected outcome: {:?}", other);
            anyhow::bail!("sweep command failed");
        }
    }
}

/// Handle the compress command.
fn handle_compress(pruner: &mut ContextPruner, file: Option<&str>) -> anyhow::Result<()> {
    let messages = read_messages(file)?;
    if messages.is_empty() {
        eprintln!("no messages provided");
        anyhow::bail!("compress requires messages");
    }
    // Transform messages through the pruner first so message IDs are
    // registered in session state, allowing the resolver to find them.
    let transformed = pruner.transform_messages(messages)?;
    if transformed.is_empty() {
        eprintln!("all messages were pruned, nothing to compress");
        return Ok(());
    }
    let first = &transformed[0];
    let last = &transformed[transformed.len() - 1];
    use dcp_compress::{CompressArgs, RangeEntry};
    let cargs = CompressArgs::Range {
        topic: "manual compress".to_string(),
        content: vec![RangeEntry {
            start_id: first.id.clone(),
            end_id: last.id.clone(),
            summary: "Manual compression from CLI".to_string(),
        }],
    };
    match pruner.handle_compress(cargs, &transformed) {
        Ok(result) => {
            println!("=== Compress Result ===");
            println!("  Messages compressed:  {}", result.compressed_messages);
            println!("  Blocks committed:    {}", result.blocks.len());
            for (i, block) in result.blocks.iter().enumerate() {
                println!(
                    "    Block {}: b{} ({} tokens)",
                    i + 1,
                    block.block_id,
                    block.summary_tokens
                );
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("compress error: {}", e);
            anyhow::bail!("compress failed");
        }
    }
}

/// Handle the decompress command.
fn handle_decompress(pruner: &mut ContextPruner, block_id: &str) -> anyhow::Result<()> {
    let id = parse_block_id(block_id).map_err(|e| anyhow::anyhow!("{}", e))?;
    match pruner.decompress(id) {
        Ok(result) => {
            println!(
                "Decompressed block b{} (anchor: {})",
                result.block_id.value(),
                result.anchor_message_id
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("decompress failed: {}", e);
            anyhow::bail!("decompress failed");
        }
    }
}

/// Handle the recompress command.
fn handle_recompress(pruner: &mut ContextPruner, block_id: &str) -> anyhow::Result<()> {
    let id = parse_block_id(block_id).map_err(|e| anyhow::anyhow!("{}", e))?;
    match pruner.recompress(id) {
        Ok(result) => {
            println!("Recompressed block b{}", result.block_id.value());
            Ok(())
        }
        Err(e) => {
            eprintln!("recompress failed: {}", e);
            anyhow::bail!("recompress failed");
        }
    }
}

fn handle_manual(pruner: &mut ContextPruner, enabled: &Option<String>) -> anyhow::Result<()> {
    match enabled {
        Some(val) => {
            let enable = match val.as_str() {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                other => {
                    eprintln!("invalid value: {other}, expected on/off");
                    anyhow::bail!("invalid manual mode value");
                }
            };
            pruner.set_manual_mode(enable);
            if enable {
                println!("Manual mode enabled");
            } else {
                println!("Manual mode disabled");
            }
        }
        None => {
            let status = if pruner.state().manual_mode.enabled {
                "on"
            } else {
                "off"
            };
            println!("Manual mode is currently {status}");
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Context => {
            let mut pruner = build_pruner()?;
            handle_context(&mut pruner)?;
        }
        Commands::Stats { session_id } => {
            commands::stats::run(&commands::stats::Args { session_id })?;
        }
        Commands::Timeline { session_id } => {
            commands::timeline::run(&commands::timeline::Args { session_id })?;
        }
        Commands::FindSession {
            pattern,
            after,
            before,
        } => {
            commands::find_session::run(&commands::find_session::Args {
                pattern,
                after,
                before,
            })?;
        }
        Commands::GetMessage {
            message_ids,
            session,
            scan_sessions,
        } => {
            #[cfg(feature = "scripts")]
            {
                let args = commands::get_message::Args {
                    message_ids,
                    session,
                    scan_sessions,
                };
                commands::get_message::run(&args)?;
            }
            #[cfg(not(feature = "scripts"))]
            {
                let _ = (message_ids, session, scan_sessions);
                anyhow::bail!(
                    "get-message requires the `scripts` feature (rebuild with --features scripts)"
                );
            }
        }
        Commands::TokenStats {
            sessions,
            session,
            json,
        } => {
            #[cfg(feature = "scripts")]
            {
                let args = commands::token_stats::Args {
                    sessions,
                    session,
                    json,
                };
                commands::token_stats::run(&args)?;
            }
            #[cfg(not(feature = "scripts"))]
            {
                let _ = (sessions, session, json);
                anyhow::bail!(
                    "token-stats requires the `scripts` feature (rebuild with --features scripts)"
                );
            }
        }
        Commands::MessageTokens {
            session,
            json,
            no_color,
        } => {
            #[cfg(feature = "scripts")]
            {
                let args = commands::message_tokens::Args {
                    session,
                    json,
                    no_color,
                };
                commands::message_tokens::run(&args)?;
            }
            #[cfg(not(feature = "scripts"))]
            {
                let _ = (session, json, no_color);
                anyhow::bail!(
                    "message-tokens requires the `scripts` feature (rebuild with --features scripts)"
                );
            }
        }
        Commands::Sweep { count } => {
            let mut pruner = build_pruner()?;
            handle_sweep(&mut pruner, count)?;
        }
        Commands::Compress { file } => {
            let mut pruner = build_pruner()?;
            handle_compress(&mut pruner, file.as_deref())?;
        }
        Commands::Decompress { block_id } => {
            let mut pruner = build_pruner()?;
            handle_decompress(&mut pruner, &block_id)?;
        }
        Commands::Recompress { block_id } => {
            let mut pruner = build_pruner()?;
            handle_recompress(&mut pruner, &block_id)?;
        }
        Commands::Manual { ref enabled } => {
            let mut pruner = build_pruner()?;
            handle_manual(&mut pruner, enabled)?;
        }
    }
    Ok(())
}
