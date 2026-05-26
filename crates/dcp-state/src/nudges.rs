//! Turn-nudge anchor collection per SPEC.md §8.2.
//!
//! A *turn nudge* fires for every `(user_message, assistant_message)` pair
//! where the assistant message is a turn-end (text without an open tool
//! call). The nudge anchors to the **assistant** message of the pair.
//!
//! [`collect_turn_nudge_anchors`] returns the set of assistant message
//! raw ids that are eligible anchors, derived deterministically from the
//! message stream. The caller (`dcp-nudges`) then intersects this with
//! `state.nudges.turn_nudged_pairs` to figure out which anchors still
//! need a nudge.
//!
//! Eligibility rules (mirrors SPEC.md §3.2 / §8.2):
//!
//! 1. The assistant message must be a turn-end:
//!    - has at least one `Text` part, AND
//!    - has no `ToolCall` part whose `call_id` is unmatched in the
//!      remainder of the stream.
//! 2. There must be a preceding `user`-role message in the stream with at
//!    least one `Text` part. The most recent such message is treated as
//!    the user half of the pair.

use std::collections::HashSet;

use dcp_types::{Message, Part, Role};

/// Return the set of assistant message raw ids that are eligible turn-end
/// anchors.
///
/// The set is computed in a single forward pass and is order-independent
/// in the sense that two equivalent message lists produce identical sets.
///
/// # Example
///
/// ```rust
/// use dcp_state::nudges::collect_turn_nudge_anchors;
/// use dcp_types::{Message, Part, Role};
///
/// let messages = vec![
///     Message::user_text("u1", 0, "hi"),
///     Message::assistant_text("a1", 0, "hello"),
///     Message::user_text("u2", 0, "follow up"),
///     Message::assistant_text("a2", 0, "ack"),
/// ];
/// let anchors = collect_turn_nudge_anchors(&messages);
/// assert!(anchors.contains("a1"));
/// assert!(anchors.contains("a2"));
/// assert_eq!(anchors.len(), 2);
/// ```
pub fn collect_turn_nudge_anchors(messages: &[Message]) -> HashSet<String> {
    let mut anchors = HashSet::new();
    let mut last_user_text: Option<&Message> = None;

    for (idx, msg) in messages.iter().enumerate() {
        match msg.role {
            Role::User if !msg.ignored && has_text(msg) => {
                last_user_text = Some(msg);
            }
            Role::Assistant
                if !msg.ignored
                    && last_user_text.is_some()
                    && has_text(msg)
                    && !has_open_tool_call(msg, &messages[idx + 1..]) =>
            {
                anchors.insert(msg.id.clone());
            }
            // Other roles (System, future variants) and the fall-through
            // user/assistant cases don't seed nor anchor a turn nudge.
            _ => {}
        }
    }

    anchors
}

/// True when `msg` has at least one [`Part::Text`].
pub(crate) fn has_text(msg: &Message) -> bool {
    msg.parts.iter().any(|p| matches!(p, Part::Text(_)))
}

/// True when `msg` contains a `ToolCall` whose `call_id` does not appear as
/// a `ToolResult` in `tail` (the messages following `msg`).
pub(crate) fn has_open_tool_call(msg: &Message, tail: &[Message]) -> bool {
    let mut open_ids: Vec<&str> = Vec::new();
    for part in &msg.parts {
        if let Part::ToolCall { call_id, .. } = part {
            open_ids.push(call_id.as_str());
        }
    }
    if open_ids.is_empty() {
        return false;
    }
    for later in tail {
        for part in &later.parts {
            if let Part::ToolResult { call_id, .. } = part {
                open_ids.retain(|id| id != call_id);
                if open_ids.is_empty() {
                    return false;
                }
            }
        }
    }
    !open_ids.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{Message, Part, Role, ToolStatus};
    use serde_json::json;

    #[test]
    fn collects_simple_turn_pair() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let anchors = collect_turn_nudge_anchors(&messages);
        assert!(anchors.contains("a1"));
        assert_eq!(anchors.len(), 1);
    }

    #[test]
    fn collects_multiple_turn_pairs() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "ack 1"),
            Message::user_text("u2", 0, "more"),
            Message::assistant_text("a2", 0, "ack 2"),
        ];
        let anchors = collect_turn_nudge_anchors(&messages);
        assert_eq!(anchors.len(), 2);
        assert!(anchors.contains("a1"));
        assert!(anchors.contains("a2"));
    }

    #[test]
    fn assistant_with_open_tool_call_is_not_anchor() {
        let messages = vec![
            Message::user_text("u1", 0, "do it"),
            Message::new(
                "a1",
                Role::Assistant,
                vec![
                    Part::text("working"),
                    Part::tool_call("c1", "read", json!({"path": "x"})),
                ],
                0,
            ),
            // No matching ToolResult — the call stays open.
        ];
        let anchors = collect_turn_nudge_anchors(&messages);
        assert!(anchors.is_empty());
    }

    #[test]
    fn assistant_with_paired_tool_call_is_anchor() {
        let messages = vec![
            Message::user_text("u1", 0, "do it"),
            Message::new(
                "a1",
                Role::Assistant,
                vec![
                    Part::text("doing"),
                    Part::tool_call("c1", "read", json!({"path": "x"})),
                ],
                0,
            ),
            Message::new(
                "u2",
                Role::User,
                vec![Part::tool_result(
                    "c1",
                    ToolStatus::Completed,
                    Some("ok".into()),
                    None,
                )],
                0,
            ),
            Message::assistant_text("a2", 0, "done"),
        ];
        let anchors = collect_turn_nudge_anchors(&messages);
        // a1 has paired tool_call AND text → anchor.
        // a2 also has text → anchor (its preceding user-text turn is u1).
        // u2 carries only tool_result, so the most recent user-text remains u1.
        assert!(anchors.contains("a1"));
        assert!(anchors.contains("a2"));
    }

    #[test]
    fn assistant_without_text_is_not_anchor() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            // Only reasoning — no text part.
            Message::new("a1", Role::Assistant, vec![Part::reasoning("thinking")], 0),
        ];
        let anchors = collect_turn_nudge_anchors(&messages);
        assert!(anchors.is_empty());
    }

    #[test]
    fn assistant_without_preceding_user_is_not_anchor() {
        let messages = vec![Message::assistant_text("a1", 0, "leading")];
        let anchors = collect_turn_nudge_anchors(&messages);
        assert!(anchors.is_empty());
    }

    #[test]
    fn user_without_text_does_not_seed_pair() {
        // user message with only a tool_result is not a "user text" message
        let messages = vec![
            Message::new(
                "u1",
                Role::User,
                vec![Part::tool_result(
                    "orphan",
                    ToolStatus::Completed,
                    None,
                    None,
                )],
                0,
            ),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let anchors = collect_turn_nudge_anchors(&messages);
        assert!(anchors.is_empty());
    }

    #[test]
    fn system_messages_do_not_break_pairing() {
        let messages = vec![
            Message::system_text("s1", 0, "sys"),
            Message::user_text("u1", 0, "hi"),
            Message::system_text("s2", 0, "more sys"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let anchors = collect_turn_nudge_anchors(&messages);
        assert!(anchors.contains("a1"));
    }

    #[test]
    fn empty_input_yields_empty_set() {
        assert!(collect_turn_nudge_anchors(&[]).is_empty());
    }
}
