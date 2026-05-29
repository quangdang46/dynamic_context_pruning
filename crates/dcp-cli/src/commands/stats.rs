//! `stats` subcommand — show session statistics.

use clap::Parser;
use dcp_storage::{FileStateStore, default_storage_dir};
use dcp_traits::{PersistedState, StatePersistence};

/// Show session statistics: total tokens saved, message count, compression ratio.
#[derive(Parser, Debug)]
pub struct Args {
    /// Session ID to display stats for.
    #[arg(long = "session-id", short = 's')]
    pub session_id: String,
}

/// Run the stats subcommand.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let store = FileStateStore::new(default_storage_dir());

    let persisted = store
        .load(&args.session_id)
        .map_err(|e| anyhow::anyhow!("failed to load session '{}': {}", args.session_id, e))?
        .ok_or_else(|| anyhow::anyhow!("session '{}' not found", args.session_id))?;

    let PersistedState::V1(v1) = persisted;

    // Parse stats from JSON
    let stats: dcp_types::Stats = serde_json::from_value(v1.stats.clone())
        .map_err(|e| anyhow::anyhow!("failed to parse stats: {}", e))?;

    // Parse block count from prune JSON
    let blocks_count = v1
        .prune
        .as_object()
        .and_then(|map| map.get("messages"))
        .and_then(|m| m.as_object())
        .and_then(|msg| msg.get("blocks_by_id"))
        .and_then(|b| b.as_object())
        .map(|b| b.len())
        .unwrap_or(0);

    // Calculate total messages from message_id_map
    let message_count = v1
        .message_id_map
        .as_object()
        .and_then(|m| m.get("by_raw_id"))
        .and_then(|b| b.as_object())
        .map(|b| b.len())
        .unwrap_or(0);

    // Calculate compression ratio
    let total_original = stats.total_prune_tokens;
    let compression_ratio = if total_original > 0 {
        let compressed = stats.compress_blocks_committed as u64 * stats.compress_useful as u64;
        if compressed > 0 {
            total_original as f64 / compressed as f64
        } else {
            0.0
        }
    } else {
        0.0
    };

    println!("=== Session Statistics: {} ===", args.session_id);
    println!("  Session ID:               {}", v1.session_id);
    println!("  Last updated:              {}", v1.last_updated);
    println!("  Current turn:             {}", v1.current_turn);
    println!();
    println!("  --- Token Counts ---");
    println!("  Total tokens pruned:       {}", stats.total_prune_tokens);
    println!("  Active blocks:             {}", blocks_count);
    println!();
    println!("  --- Compression ---");
    println!("  Compression runs:         {}", stats.compress_runs);
    println!(
        "  Blocks committed:         {}",
        stats.compress_blocks_committed
    );
    println!("  Compression useful:        {}", stats.compress_useful);
    println!("  Compression oversized:     {}", stats.compress_oversized);
    println!("  Compression ratio (est):  {:.2}x", compression_ratio);
    println!();
    println!("  --- Pruning ---");
    println!("  Deduplicated:              {}", stats.dedup_pruned);
    println!("  Purge errors:              {}", stats.purge_errors_pruned);
    println!(
        "  Stale file reads:         {}",
        stats.stale_file_reads_pruned
    );
    println!();
    println!("  --- System ---");
    println!("  Message count:            {}", message_count);
    println!("  Compactions observed:     {}", stats.compactions_observed);
    println!("  Cache bust events:        {}", stats.cache_bust_events);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_args_parsing() {
        let args = Args::parse_from(["stats", "--session-id", "test-session"]);
        assert_eq!(args.session_id, "test-session");
    }

    #[test]
    fn stats_args_parsing_short() {
        let args = Args::parse_from(["stats", "-s", "another-session"]);
        assert_eq!(args.session_id, "another-session");
    }
}
