//! Notification builders — port of lib/ui/notification.ts.
//!
//! Provides: send_unified_notification, send_compress_notification,
//! build_compress_visual_output, CompressionNotificationEntry, PruneReason.

use dcp_config::{Config, NotificationLevel};
use dcp_state::SessionState;
use dcp_compress::NotificationEntry;

/// Error type for notification operations.
#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    /// Notifications are disabled.
    #[error("notification disabled")]
    Disabled,
    /// Invalid configuration.
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Entry for compression notification containing message reference and tokens saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressionNotificationEntry {
    /// The message reference identifier.
    pub message_ref: String,
    /// Number of tokens saved by compression.
    pub tokens_saved: u64,
}

/// Sends a unified notification based on configuration.
///
/// This function builds a notification message based on the config's notification
/// level:
/// - `Off`: Returns `Ok(false)` without sending anything.
/// - `Minimal`: Returns `Ok(true)` with a short message.
/// - `Detailed`: Returns `Ok(true)` with full stats including the reason.
///
/// The `reason` parameter describes why the notification was triggered.
pub fn send_unified_notification(
    config: &Config,
    state: &SessionState,
    reason: &str,
) -> Result<bool, NotificationError> {
    let level = config.notification.level;

    match level {
        NotificationLevel::Off => Ok(false),
        NotificationLevel::Minimal => {
            let compress_runs = state.stats.compress_runs;
            let msg = format!("DCP: {compress_runs} compress run(s)");
            println!("{msg}");
            Ok(true)
        }
        NotificationLevel::Detailed => {
            let compress_runs = state.stats.compress_runs;
            let total_saved = state.stats.total_prune_tokens;
            let current_turn = state.current_turn;
            let msg = format!(
                "DCP [{reason}] | turn {current_turn} | {compress_runs} compress run(s) | ~{}K tokens saved",
                total_saved / 1000
            );
            println!("{msg}");
            Ok(true)
        }
    }
}

/// Sends a compression notification for the given entries.
///
/// This is called when compression events occur (e.g. after a compress tool run).
/// It builds a message based on the compression entries and optional batch topic.
pub fn send_compress_notification(
    entries: &[CompressionNotificationEntry],
    batch_topic: Option<&str>,
) -> Result<bool, NotificationError> {
    if entries.is_empty() {
        return Ok(false);
    }

    let total_saved: u64 = entries.iter().map(|e| e.tokens_saved).sum();

    let topic_str = match batch_topic {
        Some(topic) => format!(" [{topic}]"),
        None => String::new(),
    };

    let msg = if entries.len() == 1 {
        let entry = &entries[0];
        format!(
            "DCP: compressed {}{} (saved {} tokens)",
            entry.message_ref, topic_str, entry.tokens_saved
        )
    } else {
        format!(
            "DCP: compressed {}{} entries (saved {} tokens total)",
            entries.len(),
            topic_str,
            total_saved
        )
    };

    println!("{msg}");
    Ok(true)
}

/// Build a human-readable visual output for compress tool results.
/// This replaces raw JSON in MCP tool responses.
pub fn build_compress_visual_output(
    state: &SessionState,
    blocks: &[NotificationEntry],
    session_message_ids: &[String],
) -> String {
    use crate::format::{format_progress_bar, format_stats_header, format_token_count};
    use std::collections::HashMap;

    if blocks.is_empty() {
        return "No blocks compressed.".to_string();
    }

    let mut lines = Vec::new();

    // Header — total removed + total summary across all blocks
    let total_removed: u64 = blocks.iter().map(|b| b.compressed_tokens).sum();
    let total_summary: u64 = blocks.iter().map(|b| b.summary_tokens).sum();
    lines.push(format_stats_header(total_removed, total_summary));

    // Progress bar
    let newly_compressed: Vec<String> = blocks
        .iter()
        .flat_map(|b| {
            state
                .prune
                .messages
                .blocks_by_id
                .get(&dcp_types::BlockId::new(b.block_id))
                .map(|blk| blk.direct_message_ids.clone())
                .unwrap_or_default()
        })
        .collect();

    // Build pruned messages map — messages that have been replaced by a block
    let active_pruned: HashMap<String, u64> = state
        .prune
        .messages
        .by_message_id
        .iter()
        .filter(|(_, entry)| !entry.replacement_text.is_empty())
        .map(|(id, entry)| (id.clone(), entry.tokens_saved))
        .collect();

    let bar = format_progress_bar(
        session_message_ids,
        &active_pruned,
        &newly_compressed,
        50,
    );
    lines.push(String::new());
    lines.push(bar);

    // Per-block details
    for entry in blocks {
        lines.push(format!(
            "✂ Compression #{} -{} removed, +{} summary",
            entry.run_id,
            format_token_count(entry.compressed_tokens, true),
            format_token_count(entry.summary_tokens, true),
        ));
        lines.push(format!("→ Topic: {}", entry.topic));

        let msg_count = entry.direct_message_count;
        let tool_count = entry.direct_tool_count;
        if tool_count > 0 {
            lines.push(format!(
                "→ Items: {} messages and {} tools compressed",
                msg_count, tool_count
            ));
        } else {
            lines.push(format!("→ Items: {} messages compressed", msg_count));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_state::create_session_state;

    fn make_config(level: NotificationLevel) -> Config {
        let mut config = Config::default();
        config.notification.level = level;
        config
    }

    fn make_state() -> SessionState {
        let mut state = create_session_state();
        state.stats.compress_runs = 5;
        state.stats.total_prune_tokens = 1500;
        state.current_turn = 3;
        state
    }

    #[test]
    fn test_send_unified_notification_off() {
        let config = make_config(NotificationLevel::Off);
        let state = make_state();
        let result = send_unified_notification(&config, &state, "dedup");
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_send_unified_notification_minimal() {
        let config = make_config(NotificationLevel::Minimal);
        let state = make_state();
        let result = send_unified_notification(&config, &state, "dedup");
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_send_unified_notification_detailed() {
        let config = make_config(NotificationLevel::Detailed);
        let state = make_state();
        let result = send_unified_notification(&config, &state, "dedup");
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_send_compress_notification_single_entry() {
        let entries = vec![CompressionNotificationEntry {
            message_ref: "m42".to_string(),
            tokens_saved: 500,
        }];
        let result = send_compress_notification(&entries, None);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_send_compress_notification_multiple_entries() {
        let entries = vec![
            CompressionNotificationEntry {
                message_ref: "m42".to_string(),
                tokens_saved: 500,
            },
            CompressionNotificationEntry {
                message_ref: "m43".to_string(),
                tokens_saved: 300,
            },
        ];
        let result = send_compress_notification(&entries, Some("batch-1"));
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_send_compress_notification_empty() {
        let entries: Vec<CompressionNotificationEntry> = vec![];
        let result = send_compress_notification(&entries, None);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }
}
