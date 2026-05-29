//! `timeline` subcommand — show compression events over time.

use clap::Parser;
use dcp_storage::{default_storage_dir, FileStateStore};
use dcp_traits::{PersistedState, StatePersistence};
use std::collections::BTreeMap;

/// Show compression events over time.
#[derive(Parser, Debug)]
pub struct Args {
    /// Session ID to display timeline for.
    #[arg(long = "session-id", short = 's')]
    pub session_id: String,
}

/// Run the timeline subcommand.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let store = FileStateStore::new(default_storage_dir());

    let persisted = store
        .load(&args.session_id)
        .map_err(|e| anyhow::anyhow!("failed to load session '{}': {}", args.session_id, e))?
        .ok_or_else(|| anyhow::anyhow!("session '{}' not found", args.session_id))?;

    let PersistedState::V1(v1) = persisted;

    // Parse blocks from prune JSON
    let blocks: BTreeMap<String, CompressionBlockEntry> = v1
        .prune
        .as_object()
        .and_then(|map| map.get("messages"))
        .and_then(|m| m.as_object())
        .and_then(|msg| msg.get("blocks_by_id"))
        .and_then(|b| b.as_object())
        .map(|blocks_map| {
            blocks_map
                .iter()
                .filter_map(|(id, value)| {
                    let block: CompressionBlockEntry = serde_json::from_value(value.clone()).ok()?;
                    Some((id.clone(), block))
                })
                .collect()
        })
        .unwrap_or_default();

    println!("=== Compression Timeline: {} ===", args.session_id);
    println!();
    println!(
        "  {:<20} | {:<6} | {:<6} | {:>12} | Topic",
        "Timestamp", "Block", "Mode", "Tokens Saved"
    );
    println!(
        "  {:<20}---{:-<6}---{:-<6}---{:-<12}---",
        "", "", "", ""
    );

    if blocks.is_empty() {
        println!("  (no compression events)");
    } else {
        for (id, block) in &blocks {
            let timestamp = if block.created_at > 0 {
                format_timestamp(block.created_at)
            } else {
                "N/A".to_string()
            };
            let mode_str = block.mode_as_str();
            let tokens_saved = block.compressed_tokens.saturating_sub(block.summary_tokens);
            let topic = block.topic.as_deref().unwrap_or("-");
            let topic_short = if topic.len() > 12 {
                format!("{}...", &topic[..9])
            } else {
                topic.to_string()
            };

            println!(
                "  {:<20} | b{:<5} | {:<6} | {:>12} | {}",
                timestamp,
                id.strip_prefix("b").unwrap_or(id),
                mode_str,
                tokens_saved,
                topic_short
            );
        }
    }

    println!();
    println!("  Total blocks: {}", blocks.len());

    Ok(())
}

/// Minimal structure to deserialize compression blocks from JSON.
#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct CompressionBlockEntry {
    #[serde(rename = "block_id")]
    block_id: BlockIdJson,
    #[serde(rename = "run_id")]
    run_id: RunIdJson,
    #[serde(rename = "mode")]
    mode: serde_json::Value,
    #[serde(rename = "topic")]
    topic: Option<String>,
    #[serde(rename = "summary")]
    summary: String,
    #[serde(rename = "compressed_tokens")]
    compressed_tokens: u64,
    #[serde(rename = "summary_tokens")]
    summary_tokens: u64,
    #[serde(rename = "created_at")]
    created_at: i64,
    #[serde(rename = "anchor_message_id")]
    anchor_message_id: String,
    #[serde(rename = "active")]
    active: bool,
}

#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct BlockIdJson {
    #[serde(rename = "value")]
    value: Option<u32>,
}

#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct RunIdJson {
    #[serde(rename = "value")]
    value: Option<u32>,
}

impl CompressionBlockEntry {
    fn mode_as_str(&self) -> &str {
        match &self.mode {
            serde_json::Value::String(s) => match s.as_str() {
                "Range" => "Range",
                "range" => "Range",
                "Message" => "Msg",
                "message" => "Msg",
                _ => "Unknown",
            },
            serde_json::Value::Object(map) => match map.get("type").and_then(|v| v.as_str()) {
                Some("Range") | Some("range") => "Range",
                Some("Message") | Some("message") => "Msg",
                _ => "Unknown",
            },
            _ => "Unknown",
        }
    }
}

fn format_timestamp(ms: i64) -> String {
    // Simple timestamp formatting without external time crate
    let secs = ms / 1000;
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;

    if days > 0 {
        format!("{}d {}h ago", days, hours % 24)
    } else if hours > 0 {
        format!("{}h {}m ago", hours, mins % 60)
    } else if mins > 0 {
        format!("{}m {}s ago", mins, secs % 60)
    } else {
        format!("{}s ago", secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_args_parsing() {
        let args = Args::parse_from(["timeline", "--session-id", "test-session"]);
        assert_eq!(args.session_id, "test-session");
    }

    #[test]
    fn timeline_args_parsing_short() {
        let args = Args::parse_from(["timeline", "-s", "another-session"]);
        assert_eq!(args.session_id, "another-session");
    }
}