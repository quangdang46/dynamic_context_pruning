//! Message query functions — port of lib/messages/query.ts.
//!
//! Provides: get_last_user_message, message_has_compress,
//! is_ignored_user_message, is_protected_user_message.

use dcp_config::Config;
use dcp_types::{Message, Role};

/// Returns the last user message before `start_index` (exclusive), or `None`
/// if no user message is found.
///
/// If `start_index` is `None`, searches the entire message list.
#[must_use]
pub fn get_last_user_message(messages: &[Message], start_index: Option<usize>) -> Option<&Message> {
    let bound = start_index.unwrap_or(messages.len());
    messages[..bound]
        .iter()
        .rfind(|m| m.role == Role::User && !m.ignored)
}

/// Returns `true` if the message contains at least one tool call with
/// name `"compress"`.
#[must_use]
pub fn message_has_compress(message: &Message) -> bool {
    use dcp_types::Part;
    message
        .parts
        .iter()
        .any(|p| matches!(p, Part::ToolCall { tool, .. } if tool == "compress"))
}

/// Returns `true` if this user message should be ignored from
/// priority tracking and ref allocation.
///
/// Currently returns `true` when `message.ignored` is set.
#[must_use]
pub fn is_ignored_user_message(message: &Message) -> bool {
    message.ignored && message.role == Role::User
}

/// Returns `true` if this user message is protected from compression.
///
/// Protected user messages are skipped in priority map building.
/// Currently returns `true` when `config.compress.protect_user_messages`
/// is enabled and the message role is `User`.
#[must_use]
pub fn is_protected_user_message(config: &Config, message: &Message) -> bool {
    config.compress.protect_user_messages && message.role == Role::User
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::Part;

    fn user_msg(id: &str, text: &str) -> Message {
        Message::user_text(id, 0, text)
    }

    fn assistant_msg(id: &str, text: &str) -> Message {
        Message::assistant_text(id, 0, text)
    }

    // ─────────────────────────────────────────────────────────────────
    // get_last_user_message tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn get_last_user_message_finds_user() {
        let msgs = vec![
            user_msg("u1", "hello"),
            assistant_msg("a1", "hi"),
            user_msg("u2", "there"),
        ];
        let result = get_last_user_message(&msgs, None);
        assert_eq!(result.map(|m| m.id.as_str()), Some("u2"));
    }

    #[test]
    fn get_last_user_message_respects_start_index() {
        let msgs = vec![
            user_msg("u1", "hello"),
            assistant_msg("a1", "hi"),
            user_msg("u2", "there"),
            user_msg("u3", "final"),
        ];
        // Look before index 3 (should find u2, not u3)
        let result = get_last_user_message(&msgs, Some(3));
        assert_eq!(result.map(|m| m.id.as_str()), Some("u2"));
    }

    #[test]
    fn get_last_user_message_skips_ignored() {
        let mut m = user_msg("u1", "hello");
        m.ignored = true;
        let msgs = vec![m, user_msg("u2", "world")];
        let result = get_last_user_message(&msgs, None);
        assert_eq!(result.map(|m| m.id.as_str()), Some("u2"));
    }

    #[test]
    fn get_last_user_message_none_when_no_user() {
        let msgs = vec![assistant_msg("a1", "hi")];
        let result = get_last_user_message(&msgs, None);
        assert!(result.is_none());
    }

    // ─────────────────────────────────────────────────────────────────
    // message_has_compress tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn message_has_compress_true_when_tool_present() {
        let m = Message::new(
            "a1",
            Role::Assistant,
            vec![Part::tool_call("c1", "compress", serde_json::json!({}))],
            0,
        );
        assert!(message_has_compress(&m));
    }

    #[test]
    fn message_has_compress_false_for_other_tool() {
        let m = Message::new(
            "a1",
            Role::Assistant,
            vec![Part::tool_call("c1", "read_file", serde_json::json!({}))],
            0,
        );
        assert!(!message_has_compress(&m));
    }

    #[test]
    fn message_has_compress_false_for_text_only() {
        let m = Message::assistant_text("a1", 0, "hello");
        assert!(!message_has_compress(&m));
    }

    // ─────────────────────────────────────────────────────────────────
    // is_ignored_user_message tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn is_ignored_user_message_true_when_ignored() {
        let mut m = user_msg("u1", "hello");
        m.ignored = true;
        assert!(is_ignored_user_message(&m));
    }

    #[test]
    fn is_ignored_user_message_false_when_not_ignored() {
        let m = user_msg("u1", "hello");
        assert!(!is_ignored_user_message(&m));
    }

    #[test]
    fn is_ignored_user_message_false_for_assistant() {
        let mut m = assistant_msg("a1", "hi");
        m.ignored = true;
        assert!(!is_ignored_user_message(&m));
    }

    // ─────────────────────────────────────────────────────────────────
    // is_protected_user_message tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn is_protected_user_message_true_when_protect_enabled() {
        let mut cfg = Config::default();
        cfg.compress.protect_user_messages = true;
        let m = user_msg("u1", "hello");
        assert!(is_protected_user_message(&cfg, &m));
    }

    #[test]
    fn is_protected_user_message_false_when_protect_disabled() {
        let mut cfg = Config::default();
        cfg.compress.protect_user_messages = false;
        let m = user_msg("u1", "hello");
        assert!(!is_protected_user_message(&cfg, &m));
    }

    #[test]
    fn is_protected_user_message_false_for_assistant() {
        let mut cfg = Config::default();
        cfg.compress.protect_user_messages = true;
        let m = assistant_msg("a1", "hi");
        assert!(!is_protected_user_message(&cfg, &m));
    }
}
