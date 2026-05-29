//! Stale provider metadata removal — port of lib/messages/reasoning-strip.ts.
//!
//! Provides: strip_stale_provider_metadata.

use dcp_types::{Message, Part, Role};
use regex::Regex;

/// Strip stale provider-specific metadata from a message.
///
/// For assistant messages, this removes XML tags like `<thinking>`, `</thinking>`,
/// `<reflection>`, `</reflection>` from text parts, along with their content.
/// For non-assistant messages, returns the message unchanged (cloned).
///
/// This is an immutable transformation — the original message is not modified.
pub fn strip_stale_provider_metadata(message: &Message) -> Message {
    if message.role != Role::Assistant {
        return message.clone();
    }

    let mut msg = message.clone();

    // Regex patterns for provider-specific XML tags and their content.
    // These are case-sensitive and match nested content.
    let thinking_re = Regex::new(r"(?s)<thinking>.*?</thinking>").expect("invalid thinking regex");
    let reflection_re =
        Regex::new(r"(?s)<reflection>.*?</reflection>").expect("invalid reflection regex");

    for part in &mut msg.parts {
        if let Part::Text(text) = part {
            // Strip thinking blocks first, then reflection blocks.
            *text = thinking_re.replace_all(text, "").to_string();
            *text = reflection_re.replace_all(text, "").to_string();
        }
    }

    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::Part;

    // ---- strip_stale_provider_metadata ----

    #[test]
    fn test_strip_stale_provider_metadata_non_assistant_passthrough() {
        // User messages should be returned unchanged (cloned).
        let user_msg = Message::user_text("u1", 0, "hello world");
        let result = strip_stale_provider_metadata(&user_msg);
        assert_eq!(result.role, Role::User);
        assert_eq!(result.id, "u1");
        // Verify it's a clone, not the same reference.
        assert!(!std::ptr::eq(&user_msg, &result));
    }

    #[test]
    fn test_strip_stale_provider_metadata_assistant_no_tags() {
        // Assistant message without tags should pass through unchanged.
        let msg = Message::assistant_text("a1", 0, "ordinary response");
        let result = strip_stale_provider_metadata(&msg);
        assert_eq!(result.parts.len(), 1);
        let text = match &result.parts[0] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(text, "ordinary response");
    }

    #[test]
    fn test_strip_stale_provider_metadata_strips_thinking_tags() {
        let msg =
            Message::assistant_text("a1", 0, "hello <thinking>inner thought</thinking> world");
        let result = strip_stale_provider_metadata(&msg);
        let text = match &result.parts[0] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(text, "hello  world");
    }

    #[test]
    fn test_strip_stale_provider_metadata_strips_reflection_tags() {
        let msg =
            Message::assistant_text("a1", 0, "answer <reflection>self-review</reflection> done");
        let result = strip_stale_provider_metadata(&msg);
        let text = match &result.parts[0] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(text, "answer  done");
    }

    #[test]
    fn test_strip_stale_provider_metadata_strips_both_tags() {
        let msg = Message::assistant_text(
            "a1",
            0,
            "<thinking>first</thinking> text <reflection>second</reflection> end",
        );
        let result = strip_stale_provider_metadata(&msg);
        let text = match &result.parts[0] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(text, " text  end");
    }

    #[test]
    fn test_strip_stale_provider_metadata_multiline_content() {
        let msg =
            Message::assistant_text("a1", 0, "start\n<thinking>line1\nline2\n</thinking>\nend");
        let result = strip_stale_provider_metadata(&msg);
        let text = match &result.parts[0] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(text, "start\n\nend");
    }

    #[test]
    fn test_strip_stale_provider_metadata_multiple_thinking_blocks() {
        let msg = Message::assistant_text("a1", 0, "<thinking>a</thinking>b<thinking>c</thinking>");
        let result = strip_stale_provider_metadata(&msg);
        let text = match &result.parts[0] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(text, "b");
    }

    #[test]
    fn test_strip_stale_provider_metadata_multiple_reflection_blocks() {
        let msg = Message::assistant_text(
            "a1",
            0,
            "<reflection>x</reflection>y<reflection>z</reflection>",
        );
        let result = strip_stale_provider_metadata(&msg);
        let text = match &result.parts[0] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(text, "y");
    }

    #[test]
    fn test_strip_stale_provider_metadata_preserves_non_tag_text() {
        let msg = Message::assistant_text("a1", 0, "hello <something>not a tag</something> world");
        let result = strip_stale_provider_metadata(&msg);
        let text = match &result.parts[0] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(text, "hello <something>not a tag</something> world");
    }

    #[test]
    fn test_strip_stale_provider_metadata_empty_thinking() {
        let msg = Message::assistant_text("a1", 0, "before <thinking></thinking> after");
        let result = strip_stale_provider_metadata(&msg);
        let text = match &result.parts[0] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(text, "before  after");
    }

    #[test]
    fn test_strip_stale_provider_metadata_assistant_system_role() {
        // System messages should pass through unchanged.
        let msg = Message::system_text("s1", 0, "you are a helpful assistant");
        let result = strip_stale_provider_metadata(&msg);
        assert_eq!(result.role, Role::System);
        assert_eq!(result.id, "s1");
    }

    #[test]
    fn test_strip_stale_provider_metadata_preserves_other_parts() {
        // Text parts that don't have tags should be preserved.
        let msg = Message::new(
            "a1",
            Role::Assistant,
            vec![
                Part::text("clean text"),
                Part::reasoning("some reasoning"),
                Part::text("<thinking>ignored</thinking>"),
            ],
            0,
        );
        let result = strip_stale_provider_metadata(&msg);
        assert_eq!(result.parts.len(), 3);
        let clean = match &result.parts[0] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(clean, "clean text");
        // The thinking in the third part should be stripped.
        let third = match &result.parts[2] {
            Part::Text(t) => t,
            _ => panic!("expected text part"),
        };
        assert_eq!(third, "");
    }
}
