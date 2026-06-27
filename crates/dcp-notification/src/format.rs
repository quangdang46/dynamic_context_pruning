//! Formatting functions for notifications — port of lib/ui/utils.ts.
//!
//! Provides: format_stats_header, format_token_count, format_progress_bar,
//! truncate, shorten_path, format_pruned_items_list, format_pruning_result_for_tool.

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
/// Returns `"✂ DCP | -X.XK removed, +X.XK summary"` showing total tokens
/// removed (compressed) and total summary tokens added.
pub fn format_stats_header(total_tokens_removed: u64, total_summary_tokens: u64) -> String {
    format!(
        "✂ DCP | -{} removed, +{} summary",
        format_token_count(total_tokens_removed, true),
        format_token_count(total_summary_tokens, true)
    )
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

/// Truncate a string to max_len, adding "..." if truncated.
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len < 3 {
        s.chars().take(max_len).collect()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

/// Extract a human-readable key from tool parameters for display.
/// Port of TS `extractParameterKey()` from ADDING.md §5.1.
pub fn extract_parameter_key(tool: &str, parameters: &serde_json::Value) -> String {
    if !parameters.is_object() {
        return String::new();
    }

    // read — show filePath with optional line range
    if tool == "read" {
        if let Some(fp) = parameters.get("filePath").and_then(|v| v.as_str()) {
            let offset = parameters.get("offset").and_then(|v| v.as_u64());
            let limit = parameters.get("limit").and_then(|v| v.as_u64());
            return match (offset, limit) {
                (Some(o), Some(l)) => format!("{fp} (lines {}-{})", o, o + l),
                (Some(o), None) => format!("{fp} (lines {}+)", o),
                (None, Some(l)) => format!("{fp} (lines 0-{l})"),
                _ => fp.to_string(),
            };
        }
    }

    // write, edit, multiedit — filePath
    if matches!(tool, "write" | "edit" | "multiedit") {
        if let Some(fp) = parameters.get("filePath").and_then(|v| v.as_str()) {
            return fp.to_string();
        }
    }

    // apply_patch — parse embedded paths from patchText
    if tool == "apply_patch" {
        if let Some(patch) = parameters.get("patchText").and_then(|v| v.as_str()) {
            // Parse "--- a/path" and "+++ b/path" lines for unified diff format
            let mut paths = std::collections::HashSet::new();
            for line in patch.lines() {
                // Claude Code apply_patch format:
                // "*** Add File: path", "*** Update File: path", "*** Delete File: path"
                if let Some(rest) = line.strip_prefix("*** Add File: ").or_else(|| {
                    line.strip_prefix("*** Update File: ")
                        .or_else(|| line.strip_prefix("*** Delete File: "))
                }) {
                    let path = rest.trim();
                    if !path.is_empty() {
                        paths.insert(path.to_string());
                    }
                }
            }
            let paths: Vec<_> = paths.into_iter().collect();
            return match paths.len() {
                0 => "patch".to_string(),
                1 => paths[0].clone(),
                2 => format!("{}, {}", paths[0], paths[1]),
                n => format!("{n} files: {}, {}...", paths[0], paths[1]),
            };
        }
    }

    // list — path
    if tool == "list" {
        return parameters
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("(current directory)")
            .to_string();
    }

    // glob — pattern
    if tool == "glob" {
        if let Some(pattern) = parameters.get("pattern").and_then(|v| v.as_str()) {
            let path_info = parameters
                .get("path")
                .and_then(|v| v.as_str())
                .map(|p| format!(" in {p}"))
                .unwrap_or_default();
            return format!("\"{pattern}\"{path_info}");
        }
        return "(unknown pattern)".to_string();
    }

    // grep — pattern
    if tool == "grep" {
        if let Some(pattern) = parameters.get("pattern").and_then(|v| v.as_str()) {
            let path_info = parameters
                .get("path")
                .and_then(|v| v.as_str())
                .map(|p| format!(" in {p}"))
                .unwrap_or_default();
            return format!("\"{pattern}\"{path_info}");
        }
        return "(unknown pattern)".to_string();
    }

    // bash — description or command
    if tool == "bash" {
        if let Some(desc) = parameters.get("description").and_then(|v| v.as_str()) {
            return desc.to_string();
        }
        if let Some(cmd) = parameters.get("command").and_then(|v| v.as_str()) {
            return if cmd.chars().count() > 50 {
                let cmd_preview: String = cmd.chars().take(47).collect();
                format!("{}...", cmd_preview)
            } else {
                cmd.to_string()
            };
        }
    }

    // webfetch — url
    if tool == "webfetch" {
        return parameters
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
    }

    // websearch / codesearch — query
    if matches!(tool, "websearch" | "codesearch") {
        return parameters
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| format!("\"{q}\""))
            .unwrap_or_default();
    }

    // Fallback: truncate JSON at 50 chars
    let param_str = serde_json::to_string(parameters).unwrap_or_default();
    if param_str == "{}" || param_str == "[]" || param_str == "null" {
        return String::new();
    }
    truncate(&param_str, 50)
}

/// Format a list of pruned tool IDs with their parameter summaries.
/// Port of TS `formatPrunedItemsList()` from ADDING.md §5.2.
pub fn format_pruned_items_list(
    pruned_tool_ids: &[String],
    tool_metadata: &std::collections::HashMap<String, (String, serde_json::Value)>,
    working_directory: Option<&str>,
) -> Vec<String> {
    let mut lines = Vec::new();

    for id in pruned_tool_ids {
        if let Some((tool, params)) = tool_metadata.get(id) {
            let param_key = extract_parameter_key(tool, params);
            if !param_key.is_empty() {
                let display = truncate(&shorten_path(&param_key, working_directory), 60);
                lines.push(format!("→ {tool}: {display}"));
            } else {
                lines.push(format!("→ {tool}"));
            }
        }
    }

    let known_count = pruned_tool_ids
        .iter()
        .filter(|id| tool_metadata.contains_key(*id))
        .count();
    let unknown_count = pruned_tool_ids.len() - known_count;

    if unknown_count > 0 {
        let plural = if unknown_count > 1 { "s" } else { "" };
        lines.push(format!(
            "→ ({unknown_count} tool{plural} with unknown metadata)"
        ));
    }

    lines
}

/// Format a complete pruning result for display in MCP tool output.
/// Port of TS `formatPruningResultForTool()` from ADDING.md §5.2.
pub fn format_pruning_result_for_tool(
    pruned_ids: &[String],
    tool_metadata: &std::collections::HashMap<String, (String, serde_json::Value)>,
    working_directory: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "Context pruning complete. Pruned {} tool outputs.",
        pruned_ids.len()
    ));
    lines.push(String::new());

    if !pruned_ids.is_empty() {
        lines.push(format!("Semantically pruned ({}):", pruned_ids.len()));
        lines.extend(format_pruned_items_list(
            pruned_ids,
            tool_metadata,
            working_directory,
        ));
    }

    lines.join("\n").trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_stats_header() {
        assert_eq!(format_stats_header(0, 0), "✂ DCP | -0 removed, +0 summary");
        assert_eq!(
            format_stats_header(1500, 800),
            "✂ DCP | -1.5K removed, +800 summary"
        );
        assert_eq!(
            format_stats_header(2500, 100),
            "✂ DCP | -2.5K removed, +100 summary"
        );
        assert_eq!(
            format_stats_header(100_000, 5000),
            "✂ DCP | -100K removed, +5K summary"
        );
        assert_eq!(
            format_stats_header(322_200, 4100),
            "✂ DCP | -322.2K removed, +4.1K summary"
        );
        assert_eq!(
            format_stats_header(18_900, 4100),
            "✂ DCP | -18.9K removed, +4.1K summary"
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

    #[test]
    fn test_truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long_string_truncated() {
        assert_eq!(truncate("hello world", 8), "hello...");
    }

    #[test]
    fn test_truncate_very_short_max() {
        assert_eq!(truncate("hello", 2), "he");
    }

    #[test]
    fn test_extract_parameter_key_read_with_filepath() {
        let params = serde_json::json!({ "filePath": "/src/lib.rs" });
        assert_eq!(extract_parameter_key("read", &params), "/src/lib.rs");
    }

    #[test]
    fn test_extract_parameter_key_read_with_offset_limit() {
        let params = serde_json::json!({
            "filePath": "/src/lib.rs",
            "offset": 10,
            "limit": 5
        });
        assert_eq!(
            extract_parameter_key("read", &params),
            "/src/lib.rs (lines 10-15)"
        );
    }

    #[test]
    fn test_extract_parameter_key_read_with_offset_only() {
        let params = serde_json::json!({
            "filePath": "/src/lib.rs",
            "offset": 20
        });
        assert_eq!(
            extract_parameter_key("read", &params),
            "/src/lib.rs (lines 20+)"
        );
    }

    #[test]
    fn test_extract_parameter_key_read_with_limit_only() {
        let params = serde_json::json!({
            "filePath": "/src/lib.rs",
            "limit": 10
        });
        assert_eq!(
            extract_parameter_key("read", &params),
            "/src/lib.rs (lines 0-10)"
        );
    }

    #[test]
    fn test_extract_parameter_key_write() {
        let params = serde_json::json!({ "filePath": "/src/main.rs" });
        assert_eq!(extract_parameter_key("write", &params), "/src/main.rs");
    }

    #[test]
    fn test_extract_parameter_key_edit() {
        let params = serde_json::json!({ "filePath": "/src/main.rs" });
        assert_eq!(extract_parameter_key("edit", &params), "/src/main.rs");
    }

    #[test]
    fn test_extract_parameter_key_multiedit() {
        let params = serde_json::json!({ "filePath": "/src/main.rs" });
        assert_eq!(extract_parameter_key("multiedit", &params), "/src/main.rs");
    }

    #[test]
    fn test_extract_parameter_key_apply_patch() {
        let params = serde_json::json!({
            "patchText": "*** Update File: src/lib.rs\n@@ -1,3 +1,3 @@\n old\n old\n-old\n+new"
        });
        assert_eq!(extract_parameter_key("apply_patch", &params), "src/lib.rs");
    }

    #[test]
    fn test_extract_parameter_key_apply_patch_multiple() {
        let params = serde_json::json!({
            "patchText": "*** Update File: src/a.rs\n*** Update File: src/b.rs"
        });
        let result = extract_parameter_key("apply_patch", &params);
        assert!(
            result.contains("src/a.rs") && result.contains("src/b.rs"),
            "expected both files, got: {result}"
        );
    }

    #[test]
    fn test_extract_parameter_key_apply_patch_many() {
        let params = serde_json::json!({
            "patchText": "*** Update File: src/a.rs\n*** Update File: src/b.rs\n*** Add File: src/c.rs"
        });
        let result = extract_parameter_key("apply_patch", &params);
        assert!(
            result.starts_with("3 files:"),
            "expected '3 files:' prefix, got: {result}"
        );
        assert!(
            result.ends_with("..."),
            "expected trailing '...', got: {result}"
        );
    }

    #[test]
    fn test_extract_parameter_key_list() {
        let params = serde_json::json!({ "path": "/src" });
        assert_eq!(extract_parameter_key("list", &params), "/src");
    }

    #[test]
    fn test_extract_parameter_key_list_default() {
        let params = serde_json::json!({});
        assert_eq!(
            extract_parameter_key("list", &params),
            "(current directory)"
        );
    }

    #[test]
    fn test_extract_parameter_key_glob() {
        let params = serde_json::json!({ "pattern": "**/*.rs", "path": "/src" });
        assert_eq!(
            extract_parameter_key("glob", &params),
            "\"**/*.rs\" in /src"
        );
    }

    #[test]
    fn test_extract_parameter_key_glob_no_path() {
        let params = serde_json::json!({ "pattern": "**/*.rs" });
        assert_eq!(extract_parameter_key("glob", &params), "\"**/*.rs\"");
    }

    #[test]
    fn test_extract_parameter_key_glob_missing() {
        let params = serde_json::json!({});
        assert_eq!(extract_parameter_key("glob", &params), "(unknown pattern)");
    }

    #[test]
    fn test_extract_parameter_key_grep() {
        let params = serde_json::json!({ "pattern": "fn main", "path": "/src" });
        assert_eq!(
            extract_parameter_key("grep", &params),
            "\"fn main\" in /src"
        );
    }

    #[test]
    fn test_extract_parameter_key_grep_no_path() {
        let params = serde_json::json!({ "pattern": "fn main" });
        assert_eq!(extract_parameter_key("grep", &params), "\"fn main\"");
    }

    #[test]
    fn test_extract_parameter_key_bash_description() {
        let params = serde_json::json!({ "description": "Run tests", "command": "cargo test" });
        assert_eq!(extract_parameter_key("bash", &params), "Run tests");
    }

    #[test]
    fn test_extract_parameter_key_bash_command_only() {
        let params = serde_json::json!({ "command": "cargo test --lib" });
        assert_eq!(extract_parameter_key("bash", &params), "cargo test --lib");
    }

    #[test]
    fn test_extract_parameter_key_bash_command_truncated() {
        let params = serde_json::json!({ "command": "cargo test --lib -- --test-threads=4 --nocapture 2>&1" });
        let result = extract_parameter_key("bash", &params);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 53); // 50 + "..."
    }

    #[test]
    fn test_extract_parameter_key_webfetch() {
        let params = serde_json::json!({ "url": "https://example.com/api" });
        assert_eq!(
            extract_parameter_key("webfetch", &params),
            "https://example.com/api"
        );
    }

    #[test]
    fn test_extract_parameter_key_websearch() {
        let params = serde_json::json!({ "query": "rust programming" });
        assert_eq!(
            extract_parameter_key("websearch", &params),
            "\"rust programming\""
        );
    }

    #[test]
    fn test_extract_parameter_key_codesearch() {
        let params = serde_json::json!({ "query": "async fn" });
        assert_eq!(extract_parameter_key("codesearch", &params), "\"async fn\"");
    }

    #[test]
    fn test_extract_parameter_key_unknown_tool_fallback() {
        let params = serde_json::json!({ "foo": "bar", "baz": 42 });
        let result = extract_parameter_key("unknown_tool", &params);
        assert!(!result.is_empty());
        assert!(result.len() <= 53); // truncated
    }

    #[test]
    fn test_extract_parameter_key_empty_object() {
        let params = serde_json::json!({});
        assert_eq!(extract_parameter_key("read", &params), "");
    }

    #[test]
    fn test_extract_parameter_key_non_object() {
        let params = serde_json::json!("not an object");
        assert_eq!(extract_parameter_key("read", &params), "");
    }

    #[test]
    fn test_format_pruned_items_list_empty() {
        let tool_metadata: HashMap<String, (String, serde_json::Value)> = HashMap::new();
        let result = format_pruned_items_list(&[], &tool_metadata, None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_pruned_items_list_single_item() {
        let mut tool_metadata = HashMap::new();
        tool_metadata.insert(
            "c1".to_string(),
            (
                "read".to_string(),
                serde_json::json!({ "filePath": "/src/lib.rs" }),
            ),
        );
        let result = format_pruned_items_list(&["c1".to_string()], &tool_metadata, None);
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("read"));
        assert!(result[0].contains("/src/lib.rs"));
    }

    #[test]
    fn test_format_pruned_items_list_unknown_tools() {
        let tool_metadata: HashMap<String, (String, serde_json::Value)> = HashMap::new();
        let result =
            format_pruned_items_list(&["c1".to_string(), "c2".to_string()], &tool_metadata, None);
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("unknown metadata"));
    }

    #[test]
    fn test_format_pruning_result_for_tool_empty() {
        let tool_metadata: HashMap<String, (String, serde_json::Value)> = HashMap::new();
        let msg = format_pruning_result_for_tool(&[], &tool_metadata, None);
        assert!(msg.contains("Context pruning complete"));
        assert!(msg.contains("0"));
    }

    #[test]
    fn test_format_pruning_result_for_tool_with_items() {
        let mut tool_metadata = HashMap::new();
        tool_metadata.insert(
            "c1".to_string(),
            (
                "read".to_string(),
                serde_json::json!({ "filePath": "/src/lib.rs" }),
            ),
        );
        let msg = format_pruning_result_for_tool(&["c1".to_string()], &tool_metadata, None);
        assert!(msg.contains("Context pruning complete"));
        assert!(msg.contains("Semantically pruned"));
        assert!(msg.contains("read"));
    }
}
