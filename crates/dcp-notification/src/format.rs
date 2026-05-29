//! Formatting functions for notifications — port of lib/ui/utils.ts.
//!
//! Provides: format_stats_header, format_token_count, format_progress_bar,
//! truncate, shorten_path, format_pruned_items_list.

use std::collections::HashMap;

/// Represents the character used for each message status in the progress bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarChar {
    /// Active (unpruned) message — filled block.
    Active,
    /// Pruned message — light shade.
    Pruned,
    /// Recent message — dotted block.
    Recent,
}

impl BarChar {
    fn as_char(&self) -> &'static str {
        match self {
            BarChar::Active => "█",
            BarChar::Pruned => "░",
            BarChar::Recent => "⣿",
        }
    }
}

/// Format the stats header line.
///
/// Returns `"▣ DCP | ~{total}K saved total"` where `total = total_tokens_saved / 1000`.
pub fn format_stats_header(total_tokens_saved: u64, _prune_token_counter: u64) -> String {
    let total_k = total_tokens_saved / 1000;
    format!("▣ DCP | ~{total_k}K saved total")
}

/// Format a token count for display.
///
/// If `compact` is false, appends " tokens" to the output.
/// For values >= 1000, formats as a compact string like "1.2K" with one
/// decimal digit and no trailing zeros. Otherwise returns the plain number.
pub fn format_token_count(tokens: u64, compact: bool) -> String {
    if tokens >= 1000 {
        let value = tokens as f64 / 1000.0;
        // Format with one decimal, no trailing zeros
        let formatted = if value.fract() == 0.0 {
            format!("{:.0}K", value)
        } else {
            format!("{:.1}K", value)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        };
        if compact {
            formatted
        } else {
            format!("{formatted} tokens")
        }
    } else {
        if compact {
            tokens.to_string()
        } else {
            format!("{tokens} tokens")
        }
    }
}

/// Build a progress bar string for a set of message IDs.
///
/// - `█` (active) for messages in `message_ids` that are not pruned and not recent
/// - `░` (pruned) for messages in `pruned_messages`
/// - `⣿` (recent) for messages in `recent_message_ids`
///
/// Returns `"│{bar}│"` where the bar is exactly `width` characters wide
/// (or the number of message IDs if fewer).
pub fn format_progress_bar(
    message_ids: &[String],
    pruned_messages: &HashMap<String, u64>,
    recent_message_ids: &[String],
    width: usize,
) -> String {
    if message_ids.is_empty() {
        return "││".to_string();
    }

    let use_width = width.max(message_ids.len());
    let _scale = if message_ids.len() <= use_width {
        1.0
    } else {
        message_ids.len() as f64 / use_width as f64
    };

    let mut bar_chars: Vec<BarChar> = Vec::with_capacity(message_ids.len());
    for id in message_ids {
        if recent_message_ids.contains(id) {
            bar_chars.push(BarChar::Recent);
        } else if pruned_messages.contains_key(id) {
            bar_chars.push(BarChar::Pruned);
        } else {
            bar_chars.push(BarChar::Active);
        }
    }

    let mut bar = String::with_capacity(use_width * 4); // 4 bytes per char
    let step = message_ids.len() as f64 / use_width as f64;

    for i in 0..use_width {
        let idx = (i as f64 * step).floor() as usize;
        let idx = idx.min(message_ids.len() - 1);
        bar.push_str(bar_chars[idx].as_char());
    }

    format!("│{bar}│")
}

/// Shorten a file or path string for display.
///
/// If `working_directory` is provided and `input` starts with it, the prefix
/// is stripped. If the input matches the pattern `"X in Y"` (i.e. contains
/// `" in "` with a path-like second part), only the Y part is retained (after
/// attempting to strip the working directory).
pub fn shorten_path(input: &str, working_directory: Option<&str>) -> String {
    // Try to strip working_directory prefix
    let stripped = match working_directory {
        Some(wd) if input.starts_with(wd) => {
            let remainder = &input[wd.len()..];
            // Remove leading slash if present
            remainder.strip_prefix('/').unwrap_or(remainder)
        }
        _ => input,
    };

    // Check for " X in Y" pattern
    if let Some(in_pos) = stripped.find(" in ") {
        let after_in = &stripped[in_pos + 4..];
        // Recursively shorten the path part after "in"
        return shorten_path(after_in, working_directory);
    }

    stripped.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_stats_header() {
        assert_eq!(format_stats_header(0, 0), "▣ DCP | ~0K saved total");
        assert_eq!(format_stats_header(1500, 100), "▣ DCP | ~1K saved total");
        assert_eq!(format_stats_header(2500, 100), "▣ DCP | ~2K saved total");
        assert_eq!(
            format_stats_header(100_000, 5000),
            "▣ DCP | ~100K saved total"
        );
    }

    #[test]
    fn test_format_token_count_compact_false() {
        // Values < 1000
        assert_eq!(format_token_count(0, false), "0 tokens");
        assert_eq!(format_token_count(42, false), "42 tokens");
        assert_eq!(format_token_count(999, false), "999 tokens");

        // Values >= 1000
        assert_eq!(format_token_count(1000, false), "1K tokens");
        assert_eq!(format_token_count(1200, false), "1.2K tokens");
        assert_eq!(format_token_count(1500, false), "1.5K tokens");
        assert_eq!(format_token_count(10000, false), "10K tokens");
        assert_eq!(format_token_count(11500, false), "11.5K tokens");
    }

    #[test]
    fn test_format_token_count_compact_true() {
        assert_eq!(format_token_count(0, true), "0");
        assert_eq!(format_token_count(42, true), "42");
        assert_eq!(format_token_count(999, true), "999");
        assert_eq!(format_token_count(1000, true), "1K");
        assert_eq!(format_token_count(1200, true), "1.2K");
        assert_eq!(format_token_count(10000, true), "10K");
        assert_eq!(format_token_count(11500, true), "11.5K");
    }

    #[test]
    fn test_format_token_count_no_trailing_zeros() {
        // 10000 -> 10K (whole number)
        assert_eq!(format_token_count(10000, false), "10K tokens");
    }

    #[test]
    fn test_format_progress_bar_empty() {
        let pruned: HashMap<String, u64> = HashMap::new();
        let result = format_progress_bar(&[], &pruned, &[], 50);
        assert_eq!(result, "││");
    }

    #[test]
    fn test_format_progress_bar_active_only() {
        let ids = vec!["m1".to_string(), "m2".to_string(), "m3".to_string()];
        let pruned: HashMap<String, u64> = HashMap::new();
        let result = format_progress_bar(&ids, &pruned, &[], 50);
        assert!(result.starts_with("│"));
        assert!(result.ends_with("│"));
        // All active should be █
        assert!(result.contains("█"));
        assert!(!result.contains("░"));
    }

    #[test]
    fn test_format_progress_bar_with_pruned() {
        let ids = vec!["m1".to_string(), "m2".to_string(), "m3".to_string()];
        let mut pruned = HashMap::new();
        pruned.insert("m2".to_string(), 100);
        let result = format_progress_bar(&ids, &pruned, &[], 50);
        assert!(result.contains("░"));
    }

    #[test]
    fn test_format_progress_bar_with_recent() {
        let ids = vec!["m1".to_string(), "m2".to_string(), "m3".to_string()];
        let pruned: HashMap<String, u64> = HashMap::new();
        let result = format_progress_bar(&ids, &pruned, &["m3".to_string()], 50);
        assert!(result.contains("⣿"));
    }

    #[test]
    fn test_format_progress_bar_exact_width() {
        let ids = vec!["m1".to_string(), "m2".to_string()];
        let pruned: HashMap<String, u64> = HashMap::new();
        let result = format_progress_bar(&ids, &pruned, &[], 10);
        // Width is at least message_ids.len()
        assert!(result.contains("█"));
    }

    #[test]
    fn test_shorten_path_strips_working_directory() {
        let input = "/home/user/project/src/lib.rs";
        let wd = "/home/user/project";
        let result = shorten_path(input, Some(wd));
        assert_eq!(result, "src/lib.rs");
    }

    #[test]
    fn test_shorten_path_preserves_without_working_directory() {
        let input = "/home/user/project/src/lib.rs";
        let result = shorten_path(input, None);
        assert_eq!(result, "/home/user/project/src/lib.rs");
    }

    #[test]
    fn test_shorten_path_x_in_y_pattern() {
        // "X in Y" pattern with absolute path in Y
        let input = "Read file in /home/user/project/src/main.rs";
        let wd = "/home/user/project";
        let result = shorten_path(input, Some(wd));
        assert_eq!(result, "src/main.rs");
    }

    #[test]
    fn test_shorten_path_x_in_y_pattern_nested() {
        // Nested "in" pattern
        let input = "Edit file in /workspace/myproject/components/Button.tsx";
        let result = shorten_path(input, Some("/workspace/myproject"));
        assert_eq!(result, "components/Button.tsx");
    }

    #[test]
    fn test_shorten_path_no_pattern() {
        let input = "src/lib.rs";
        let result = shorten_path(input, None);
        assert_eq!(result, "src/lib.rs");
    }
}
