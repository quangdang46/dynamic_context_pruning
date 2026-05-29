//! `get-message` subcommand — retrieve full message payloads by ID.

use clap::Parser;

#[cfg(feature = "scripts")]
use crate::commands::db::OpencodeDb;

/// Retrieve one or more message payloads by ID.
///
/// If `--session` is provided, performs a direct lookup in that session.
/// Otherwise, scans up to `--scan-sessions` recent sessions (default 200)
///
/// # Output
///
/// - Single message found → JSON object
/// - Multiple messages found → JSON array
/// - Not found → `{"error": "message_not_found"}`
#[derive(Parser, Debug)]
pub struct Args {
    /// One or more message IDs to retrieve.
    #[arg(required = true)]
    pub message_ids: Vec<String>,

    /// Session ID for direct lookup. If omitted, scans recent sessions.
    #[arg(long = "session", short = 's')]
    pub session: Option<String>,

    /// Maximum number of sessions to scan when no `--session` is given.
    #[arg(long = "scan-sessions", default_value = "200")]
    pub scan_sessions: usize,
}

/// Run the get-message subcommand.
#[cfg(feature = "scripts")]
pub fn run(args: &Args) -> anyhow::Result<()> {
    let db = OpencodeDb::open(None)?;

    if let Some(session_id) = &args.session {
        // Direct lookup in the specified session
        let mut found = Vec::new();
        for msg_id in &args.message_ids {
            if let Some(msg) = db.get_session_message(session_id, msg_id)? {
                found.push(msg);
            }
        }

        if found.is_empty() {
            // No messages found at all — print error object
            println!("{{\"error\": \"message_not_found\"}}");
        } else if found.len() == 1 {
            // Single message — print object
            println!("{}", serde_json::to_string_pretty(&found[0])?);
        } else {
            // Multiple — print array
            println!("{}", serde_json::to_string_pretty(&found)?);
        }
    } else {
        // Scan recent sessions
        let sessions = db.list_sessions(None, args.scan_sessions)?;

        let mut found = Vec::new();

        for session in sessions {
            for msg_id in &args.message_ids {
                if let Some(msg) = db.get_session_message(&session.id, msg_id)? {
                    found.push(msg);
                }
            }
            // If we found all requested messages, stop scanning
            if found.len() >= args.message_ids.len() {
                break;
            }
        }

        if found.is_empty() {
            println!("{{\"error\": \"message_not_found\"}}");
        } else if found.len() == 1 {
            println!("{}", serde_json::to_string_pretty(&found[0])?);
        } else {
            println!("{}", serde_json::to_string_pretty(&found)?);
        }
    }

    Ok(())
}

/// Run the get-message subcommand (stub when `scripts` feature is disabled).
#[cfg(not(feature = "scripts"))]
pub fn run(_args: &Args) -> anyhow::Result<()> {
    anyhow::bail!("get-message requires the `scripts` feature (run with --features scripts)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_message_args_parsing_single() {
        let args = Args::parse_from(["get-message", "msg-abc123"]);
        assert_eq!(args.message_ids, vec!["msg-abc123"]);
        assert!(args.session.is_none());
        assert_eq!(args.scan_sessions, 200);
    }

    #[test]
    fn get_message_args_parsing_multiple() {
        let args = Args::parse_from(["get-message", "msg-1", "msg-2", "msg-3"]);
        assert_eq!(args.message_ids.len(), 3);
    }

    #[test]
    fn get_message_args_parsing_with_session() {
        let args = Args::parse_from(["get-message", "--session", "sess-xyz", "msg-abc"]);
        assert_eq!(args.message_ids, vec!["msg-abc"]);
        assert_eq!(args.session, Some("sess-xyz".to_string()));
    }

    #[test]
    fn get_message_args_scan_sessions_default() {
        let args = Args::parse_from(["get-message", "msg-1"]);
        assert_eq!(args.scan_sessions, 200);
    }

    #[test]
    fn get_message_args_scan_sessions_explicit() {
        let args = Args::parse_from(["get-message", "--scan-sessions", "50", "msg-1"]);
        assert_eq!(args.scan_sessions, 50);
    }
}
