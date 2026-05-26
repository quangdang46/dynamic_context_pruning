//! Hallucination stripping.
//!
//! Removes any `<dcp-…>…</dcp-…>` and `<dcp-…/>` XML markers from
//! LLM output (both text parts and tool-result outputs). These markers
//! are internal library protocol elements; if the model re-emits them
//! verbatim, they must be cleaned before the message is shown to the
//! user.

use regex::Regex;

use crate::Message;

lazy_static::lazy_static! {
    /// Matches paired DCP XML tags: `<dcp-…>…</dcp-…>` (non-greedy, dot-all).
    static ref DCP_PAIRED_REGEX: Regex =
        Regex::new(r"<dcp[^>]*>[\s\S]*?</dcp[^>]*>").unwrap();
    /// Matches unpaired / empty DCP XML tags: `<dcp-…/>` or `</dcp-…>`.
    static ref DCP_UNPAIRED_REGEX: Regex =
        Regex::new(r"</?dcp[^>]*>").unwrap();
}

/// Remove all DCP XML markers from `text`.
pub fn strip_from_string(text: &str) -> String {
    let text = DCP_PAIRED_REGEX.replace_all(text, "");
    DCP_UNPAIRED_REGEX.replace_all(&text, "").into_owned()
}

/// Strip DCP XML markers from every [`crate::Part::Text`] and
/// [`crate::Part::ToolResult`] in `messages` (mutates in-place).
pub fn strip_messages(messages: &mut [Message]) {
    for msg in messages.iter_mut() {
        for part in msg.parts.iter_mut() {
            match part {
                crate::Part::Text(t) => {
                    *t = strip_from_string(t);
                }
                crate::Part::ToolResult { output: Some(s), error, .. } => {
                    *s = strip_from_string(s);
                    if let Some(e) = error {
                        *e = strip_from_string(e);
                    }
                }
                // Reasoning, ToolCall, Image, ToolResult(None) — no-op.
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_paired_tags() {
        let text = "Hello <dcp-message-id>m0001</dcp-message-id> world";
        let out = strip_from_string(text);
        assert_eq!(out, "Hello  world");
    }

    #[test]
    fn strip_unpaired_open_tag() {
        let text = "See <dcp-block id=\"b1\"> for details";
        let out = strip_from_string(text);
        assert!(!out.contains("<dcp-block"));
    }

    #[test]
    fn strip_unpaired_close_tag() {
        let text = "See </dcp-subagent-result>";
        let out = strip_from_string(text);
        assert!(!out.contains("</dcp-subagent-result>"));
    }

    #[test]
    fn strip_subagent_result_block() {
        let text = "Result:\n<dcp-subagent-result>\nsynthesis\n</dcp-subagent-result>";
        let out = strip_from_string(text);
        assert!(!out.contains("<dcp-subagent-result>"));
        assert!(out.contains("Result:"));
    }

    #[test]
    fn strip_tool_result_error_field() {
        use crate::{Message, Part, Role, ToolStatus};
        let mut messages = vec![Message::new(
            "t1",
            Role::User,
            vec![Part::tool_result(
                "c1",
                ToolStatus::Completed,
                None,
                Some("Oops  something went wrong".into()),
            )],
            0,
        )];
        strip_messages(&mut messages);
        match &messages[0].parts[0] {
            Part::ToolResult { error: Some(e), .. } => {
                assert!(!e.contains("<dcp"));
                assert_eq!(e, "Oops  something went wrong");
            }
            _ => panic!("expected tool_result with error"),
        }
    }

    #[test]
    fn strip_tool_result_both_output_and_error() {
        use crate::{Message, Part, Role, ToolStatus};
        let mut messages = vec![Message::new(
            "t1",
            Role::User,
            vec![Part::tool_result(
                "c1",
                ToolStatus::Completed,
                Some("Output:  here".into()),
                Some("Error:  failed".into()),
            )],
            0,
        )];
        strip_messages(&mut messages);
        match &messages[0].parts[0] {
            Part::ToolResult { output: Some(o), error: Some(e), .. } => {
                assert!(!o.contains("<dcp"));
                assert!(!e.contains("<dcp"));
                assert_eq!(o, "Output:  here");
                assert_eq!(e, "Error:  failed");
            }
            _ => panic!("expected tool_result with both"),
        }
    }

    #[test]
    fn strip_preserves_normal_text() {
        let text = "Hello world <foo>bar</foo>";
        let out = strip_from_string(text);
        assert_eq!(out, "Hello world <foo>bar</foo>");
    }

    #[test]
    fn strip_messages_in_place() {
        use crate::{Message, Part};
        let mut messages = vec![
            Message::user_text("u1", 0, "normal text"),
            Message::assistant_text("a1", 0, "text with <dcp-message-id>m0001</dcp-message-id>"),
        ];
        strip_messages(&mut messages);
        match &messages[0].parts[0] {
            Part::Text(t) => assert_eq!(t, "normal text"),
            _ => panic!("expected text"),
        }
        match &messages[1].parts[0] {
            Part::Text(t) => assert!(!t.contains("<dcp-")),
            _ => panic!("expected text"),
        }
    }
}
