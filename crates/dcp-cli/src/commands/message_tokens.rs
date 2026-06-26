//! `message-tokens` subcommand — per-message token breakdown for a session.

use clap::Parser;

#[cfg(feature = "scripts")]
use crate::commands::db::OpencodeDb;

/// Show per-message token counts and size bars for a session.
///
/// Requires `--session`. Output is a formatted table with:
///
/// - Message index and role
/// - Token count (via Char4Tokenizer: chars/4, or feature-gated tiktoken/claude)
/// - Size bar (proportional to longest message in the session)
/// - Preview of the first text part (truncated to 40 chars)
///
/// The top 5 largest messages are highlighted separately at the end.
#[derive(Parser, Debug)]
pub struct Args {
    /// Session ID (required).
    #[arg(long = "session", short = 's', required = true)]
    pub session: Option<String>,

    /// Output as JSON instead of the formatted table.
    #[arg(long = "json")]
    pub json: bool,

    /// Disable ANSI color output.
    #[arg(long = "no-color")]
    pub no_color: bool,
}

/// Size bar constants.
const BAR_WIDTH: usize = 20;
const FULL_CHAR: char = '█';
const EMPTY_CHAR: char = '░';

// ---------------------------------------------------------------------------
// Tokenizer abstraction
// ---------------------------------------------------------------------------

/// Count tokens using char/4 approximation (always available).
fn count_tokens_fallback(text: &str) -> usize {
    text.chars().count() / 4
}

/// Count tokens for a message's parts using Char4 approximation.
/// Iterates over all text/reasoning parts and counts chars/4.
fn count_message_tokens(parts: &[crate::commands::db::PartData]) -> usize {
    let mut total = 0;
    for part in parts {
        if let Some(ref text) = part.text {
            total += count_tokens_fallback(text);
        }
        // Also count snapshot content for step-start parts
        if let Some(ref snap) = part.snapshot {
            total += count_tokens_fallback(snap);
        }
    }
    total
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize, Clone)]
pub struct MessageTokenEntry {
    pub index: usize,
    pub message_id: String,
    pub role: String,
    pub token_count: usize,
    pub size_bar: String,
    pub preview: String,
}

#[derive(Debug, serde::Serialize)]
pub struct MessageTokensOutput {
    pub session_id: String,
    pub total_messages: usize,
    pub total_tokens: usize,
    pub entries: Vec<MessageTokenEntry>,
    pub top_5: Vec<MessageTokenEntry>,
}

// ---------------------------------------------------------------------------
// Color helpers (ANSI escapes)
// ---------------------------------------------------------------------------

fn role_color(role: &str) -> &'static str {
    match role {
        "user" => "\x1b[34m",      // blue
        "assistant" => "\x1b[32m", // green
        "system" => "\x1b[35m",    // magenta
        _ => "\x1b[0m",            // reset
    }
}

fn color_reset() -> &'static str {
    "\x1b[0m"
}

// ---------------------------------------------------------------------------
// Run function
// ---------------------------------------------------------------------------

/// Run the message-tokens subcommand.
#[cfg(feature = "scripts")]
pub fn run(args: &Args) -> anyhow::Result<()> {
    let session_id = args
        .session
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("--session is required"))?;

    let db = OpencodeDb::open(None)?;

    let messages = db.get_session_messages(session_id)?;

    if messages.is_empty() {
        if args.json {
            let output = MessageTokensOutput {
                session_id: session_id.clone(),
                total_messages: 0,
                total_tokens: 0,
                entries: vec![],
                top_5: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("Session {} has no messages.", session_id);
        }
        return Ok(());
    }

    // Count tokens for each message
    let mut entries: Vec<MessageTokenEntry> = messages
        .iter()
        .enumerate()
        .map(|(i, msg)| {
            let token_count = count_message_tokens(&msg.parts);
            let preview = extract_preview(&msg.parts);

            MessageTokenEntry {
                index: i + 1,
                message_id: msg.message_id.clone(),
                role: msg.role.clone(),
                token_count,
                size_bar: String::new(),
                preview,
            }
        })
        .collect();

    let max_tokens = entries.iter().map(|e| e.token_count).max().unwrap_or(1);

    for entry in &mut entries {
        entry.size_bar = build_size_bar(entry.token_count, max_tokens);
    }

    let mut top_5: Vec<_> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.token_count, i, e.clone()))
        .collect();
    top_5.sort_by_key(|b| std::cmp::Reverse(b.0));
    let top_5: Vec<_> = top_5.into_iter().take(5).map(|(_, _, e)| e).collect();

    if args.json {
        let output = MessageTokensOutput {
            session_id: session_id.clone(),
            total_messages: entries.len(),
            total_tokens: entries.iter().map(|e| e.token_count).sum::<usize>() as i64 as usize,
            entries: entries.clone(),
            top_5: top_5.clone(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_table(&entries, &top_5, session_id, !args.no_color);
    }

    Ok(())
}

/// Run the message-tokens subcommand (stub when `scripts` feature is disabled).
#[cfg(not(feature = "scripts"))]
pub fn run(_args: &Args) -> anyhow::Result<()> {
    anyhow::bail!("message-tokens requires the `scripts` feature (run with --features scripts)")
}

fn build_size_bar(value: usize, max: usize) -> String {
    if max == 0 {
        return EMPTY_CHAR.to_string().repeat(BAR_WIDTH);
    }
    let filled = (value as f64 / max as f64 * BAR_WIDTH as f64).round() as usize;
    let filled = filled.min(BAR_WIDTH);
    let empty = BAR_WIDTH - filled;
    FULL_CHAR.to_string().repeat(filled) + &EMPTY_CHAR.to_string().repeat(empty)
}

fn extract_preview(parts: &[crate::commands::db::PartData]) -> String {
    for part in parts {
        if part.part_type == "text" {
            if let Some(ref text) = part.text {
                let trimmed = text.trim();
                if trimmed.len() > 40 {
                    return format!("{}...", &trimmed[..37]);
                }
                return trimmed.to_string();
            }
        }
    }
    "(no text)".to_string()
}

fn print_table(
    entries: &[MessageTokenEntry],
    top_5: &[MessageTokenEntry],
    session_id: &str,
    use_color: bool,
) {
    let rreset = if use_color { color_reset() } else { "" };

    println!("=== Message Tokens: {} ===", session_id);
    println!();

    let total_tokens: usize = entries.iter().map(|e| e.token_count).sum();

    // Column header
    let role_hdr = if use_color {
        "\x1b[1mRole\x1b[0m"
    } else {
        "Role"
    };
    let preview_hdr = if use_color {
        "\x1b[1mPreview\x1b[0m"
    } else {
        "Preview"
    };
    println!(
        "  {:>4} | {:>8} | {:<20} | {:<12} | {}",
        "Msg#", "Tokens", "Size Bar", role_hdr, preview_hdr
    );
    println!(
        "  {:─<4}───{:─<8}───{:─<20}───{:─<12}───{:─<20}",
        "", "", "", "", ""
    );

    for entry in entries {
        let role_str = if use_color {
            format!("{}{}{}", role_color(&entry.role), entry.role, rreset)
        } else {
            entry.role.clone()
        };

        println!(
            "  {:>4} | {:>8} | {:<20} | {:<12} | {}",
            entry.index,
            entry.token_count,
            entry.size_bar,
            role_str,
            truncate(&entry.preview, 40)
        );
    }

    println!();
    println!(
        "  Total messages: {} | Total tokens: {}",
        entries.len(),
        total_tokens
    );

    // Top 5 highlights
    if !top_5.is_empty() {
        println!();
        println!("  Top 5 by token count:");
        for entry in top_5 {
            let role_str = if use_color {
                format!("{}{}{}", role_color(&entry.role), entry.role, rreset)
            } else {
                entry.role.clone()
            };
            println!(
                "    msg {:>3} → {:>6} tokens | {} | \"{}\"",
                entry.index,
                entry.token_count,
                role_str,
                truncate(&entry.preview, 50)
            );
        }
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_tokens_args_parsing_required() {
        // Note: clap requires --session but in tests we bypass that
        let args = Args::parse_from(["message-tokens", "--session", "sess-abc"]);
        assert_eq!(args.session, Some("sess-abc".to_string()));
        assert!(!args.json);
        assert!(!args.no_color);
    }

    #[test]
    fn message_tokens_args_json() {
        let args = Args::parse_from(["message-tokens", "--session", "sess-abc", "--json"]);
        assert!(args.json);
    }

    #[test]
    fn message_tokens_args_no_color() {
        let args = Args::parse_from(["message-tokens", "--session", "sess-abc", "--no-color"]);
        assert!(args.no_color);
    }

    #[test]
    fn message_tokens_args_all_flags() {
        let args = Args::parse_from([
            "message-tokens",
            "--session",
            "sess-xyz",
            "--json",
            "--no-color",
        ]);
        assert_eq!(args.session, Some("sess-xyz".to_string()));
        assert!(args.json);
        assert!(args.no_color);
    }

    #[test]
    fn size_bar_full() {
        let bar = build_size_bar(100, 100);
        assert!(bar.ends_with(&FULL_CHAR.to_string().repeat(20)));
    }

    #[test]
    fn size_bar_empty() {
        let bar = build_size_bar(0, 100);
        assert!(bar.chars().all(|c| c == EMPTY_CHAR));
    }

    #[test]
    fn size_bar_half() {
        let bar = build_size_bar(50, 100);
        let filled = bar.chars().filter(|&c| c == FULL_CHAR).count();
        let empty = bar.chars().filter(|&c| c == EMPTY_CHAR).count();
        assert_eq!(filled + empty, BAR_WIDTH);
        // Should be approximately half
        assert!((8..=12).contains(&filled));
    }

    #[test]
    fn size_bar_proportional() {
        // 25% -> 5 filled chars
        let bar = build_size_bar(25, 100);
        let filled = bar.chars().filter(|&c| c == FULL_CHAR).count();
        assert_eq!(filled, 5);
    }

    #[test]
    fn extract_preview_text() {
        let parts = vec![
            crate::commands::db::PartData {
                part_type: "text".to_string(),
                text: Some("Hello world this is a test".to_string()),
                snapshot: None,
                extra: serde_json::json!({}),
            },
            crate::commands::db::PartData {
                part_type: "tool_call".to_string(),
                text: None,
                snapshot: None,
                extra: serde_json::json!({}),
            },
        ];
        let preview = extract_preview(&parts);
        assert_eq!(preview, "Hello world this is a test");
    }

    #[test]
    fn extract_preview_truncation() {
        let long_text = "a".repeat(100);
        let parts = vec![crate::commands::db::PartData {
            part_type: "text".to_string(),
            text: Some(long_text),
            snapshot: None,
            extra: serde_json::json!({}),
        }];
        let preview = extract_preview(&parts);
        assert!(preview.ends_with("..."));
        assert!(preview.len() <= 43); // 40 + "..."
    }

    #[test]
    fn extract_preview_no_text() {
        let parts = vec![
            crate::commands::db::PartData {
                part_type: "tool_call".to_string(),
                text: None,
                snapshot: None,
                extra: serde_json::json!({}),
            },
            crate::commands::db::PartData {
                part_type: "reasoning".to_string(),
                text: Some("thinking...".to_string()),
                snapshot: None,
                extra: serde_json::json!({}),
            },
        ];
        let preview = extract_preview(&parts);
        assert_eq!(preview, "(no text)");
    }

    #[test]
    fn truncate_short() {
        assert_eq!(truncate("short", 10), "short");
    }

    #[test]
    fn truncate_long() {
        let result = truncate("this is a very long string", 10);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 13); // 10 + "..."
    }
}
