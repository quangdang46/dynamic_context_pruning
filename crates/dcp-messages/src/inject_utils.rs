//! Nudge injection utilities — port of lib/messages/inject/utils.ts.
//!
//! Provides: apply_anchored_nudges, is_context_over_limits, anchor management.

use dcp_types::{Message, Part, SessionState};

/// Formats a nudge message with optional mrefs list.
///
/// Format: "{nudge}\n\nAffected messages: {mrefs joined by ', '}" if mrefs non-empty,
/// otherwise just "{nudge}"
#[must_use]
pub fn format_nudge_text(nudge: &str, mrefs: &[String]) -> String {
    if mrefs.is_empty() {
        nudge.to_string()
    } else {
        format!("{}\n\nAffected messages: {}", nudge, mrefs.join(", "))
    }
}

/// Finds messages matching anchor refs, appends nudge_text to their content.
///
/// For each anchor_ref, looks up the message ID in `state.message_ids.by_raw_id`,
/// finds the message in the messages slice by ID, and appends nudge_text to the
/// last Text part (or adds a new Text part if none exists).
pub fn apply_anchored_nudges(
    messages: &mut [Message],
    state: &SessionState,
    nudge_text: &str,
    anchor_refs: &[String],
) {
    for anchor_ref in anchor_refs {
        // Look up the message ID from the raw id
        if let Some(msg_id) = state.message_ids.by_raw_id.get(anchor_ref) {
            // Find the message in the slice by ID
            if let Some(msg) = messages.iter_mut().find(|m| m.id == *msg_id) {
                // Find the last Text part and append, or add new Text part
                let has_text = msg.parts.iter().any(|p| p.is_text());
                if has_text {
                    // Find index of last Text part
                    for i in (0..msg.parts.len()).rev() {
                        if msg.parts[i].is_text() {
                            if let Part::Text(ref mut text) = msg.parts[i] {
                                *text = format!("{}\n\n{}", *text, nudge_text);
                            }
                            break;
                        }
                    }
                } else {
                    // No Text parts, add a new Text part
                    msg.parts.push(Part::Text(nudge_text.to_string()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{Message, MessageIdState, Part, Role, SessionState};
    use std::collections::HashMap;

    fn make_state(by_raw_id: HashMap<String, String>) -> SessionState {
        SessionState {
            session_id: None,
            is_subagent: false,
            manual_mode: dcp_types::ManualMode::default(),
            compress_permission: dcp_types::CompressPermission::default(),
            pending_manual_trigger: None,
            prune: dcp_types::Prune::default(),
            nudges: dcp_types::Nudges::default(),
            stats: dcp_types::Stats::default(),
            compression_timing: dcp_types::CompressionTimingState::default(),
            tool_parameters: HashMap::new(),
            subagent_result_cache: HashMap::new(),
            tool_id_list: Vec::new(),
            message_ids: MessageIdState {
                by_raw_id,
                by_ref: HashMap::new(),
                next_ref: 0,
            },
            last_compaction: 0,
            current_turn: 0,
            model_context_limit: None,
            system_prompt_tokens: None,
            last_message_was_assistant_text: false,
            pending_prune: None,
            last_apply_turn: None,
            force_apply_requested: false,
        }
    }

    fn msg(id: &str, role: Role, parts: Vec<Part>) -> Message {
        Message {
            id: id.to_string(),
            role,
            parts,
            time: 0,
            ignored: false,
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // format_nudge_text tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn format_nudge_text_empty_mrefs() {
        let result = format_nudge_text("Please review", &[]);
        assert_eq!(result, "Please review");
    }

    #[test]
    fn format_nudge_text_single_mref() {
        let result = format_nudge_text("Please review", &["m0001".into()]);
        assert_eq!(result, "Please review\n\nAffected messages: m0001");
    }

    #[test]
    fn format_nudge_text_multiple_mrefs() {
        let result = format_nudge_text(
            "Please review",
            &["m0001".into(), "m0002".into(), "m0003".into()],
        );
        assert_eq!(
            result,
            "Please review\n\nAffected messages: m0001, m0002, m0003"
        );
    }

    #[test]
    fn format_nudge_text_empty_nudge_with_mrefs() {
        let result = format_nudge_text("", &["m0001".into()]);
        assert_eq!(result, "\n\nAffected messages: m0001");
    }

    #[test]
    fn format_nudge_text_nudge_with_newlines() {
        let result = format_nudge_text("Line1\nLine2", &["m0001".into(), "m0002".into()]);
        assert_eq!(result, "Line1\nLine2\n\nAffected messages: m0001, m0002");
    }

    // ─────────────────────────────────────────────────────────────────
    // apply_anchored_nudges tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn apply_anchored_nudges_empty_anchor_refs() {
        let mut messages = vec![msg("m0001", Role::User, vec![Part::Text("hello".into())])];
        let state = make_state(HashMap::new());
        apply_anchored_nudges(&mut messages, &state, "nudge text", &[]);
        assert_eq!(messages[0].parts.len(), 1);
        assert!(matches!(&messages[0].parts[0], Part::Text(t) if t == "hello"));
    }

    #[test]
    fn apply_anchored_nudges_single_anchor_ref() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let mut by_raw = HashMap::new();
        by_raw.insert("anchor1".into(), "msg1".into());
        let state = make_state(by_raw);

        apply_anchored_nudges(&mut messages, &state, "nudge text", &["anchor1".into()]);

        assert_eq!(messages[0].parts.len(), 1);
        assert!(matches!(&messages[0].parts[0], Part::Text(t) if t == "hello\n\nnudge text"));
    }

    #[test]
    fn apply_anchored_nudges_multiple_anchor_refs_same_message() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let mut by_raw = HashMap::new();
        by_raw.insert("anchor1".into(), "msg1".into());
        by_raw.insert("anchor2".into(), "msg1".into());
        let state = make_state(by_raw);

        apply_anchored_nudges(
            &mut messages,
            &state,
            "nudge",
            &["anchor1".into(), "anchor2".into()],
        );

        // Both refs point to same message, nudge appended twice
        assert_eq!(messages[0].parts.len(), 1);
        assert!(matches!(&messages[0].parts[0], Part::Text(t) if t == "hello\n\nnudge\n\nnudge"));
    }

    #[test]
    fn apply_anchored_nudges_different_messages() {
        let mut messages = vec![
            msg("msg1", Role::User, vec![Part::Text("hello".into())]),
            msg("msg2", Role::Assistant, vec![Part::Text("world".into())]),
        ];
        let mut by_raw = HashMap::new();
        by_raw.insert("a1".into(), "msg1".into());
        by_raw.insert("a2".into(), "msg2".into());
        let state = make_state(by_raw);

        apply_anchored_nudges(&mut messages, &state, "nudge", &["a1".into(), "a2".into()]);

        assert!(matches!(&messages[0].parts[0], Part::Text(t) if t == "hello\n\nnudge"));
        assert!(matches!(&messages[1].parts[0], Part::Text(t) if t == "world\n\nnudge"));
    }

    #[test]
    fn apply_anchored_nudges_unknown_ref() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let state = make_state(HashMap::new());

        // anchor_ref not in by_raw_id
        apply_anchored_nudges(&mut messages, &state, "nudge", &["unknown".into()]);

        assert_eq!(messages[0].parts.len(), 1);
        assert!(matches!(&messages[0].parts[0], Part::Text(t) if t == "hello"));
    }

    #[test]
    fn apply_anchored_nudges_message_with_no_text_parts() {
        let mut messages = vec![msg(
            "msg1",
            Role::User,
            vec![Part::Reasoning("thinking".into())],
        )];
        let mut by_raw = HashMap::new();
        by_raw.insert("a1".into(), "msg1".into());
        let state = make_state(by_raw);

        apply_anchored_nudges(&mut messages, &state, "nudge", &["a1".into()]);

        // Should add new Text part with nudge
        assert_eq!(messages[0].parts.len(), 2);
        assert!(matches!(&messages[0].parts[0], Part::Reasoning(_)));
        assert!(matches!(&messages[0].parts[1], Part::Text(t) if t == "nudge"));
    }

    #[test]
    fn apply_anchored_nudges_append_to_last_text() {
        let mut messages = vec![msg(
            "msg1",
            Role::User,
            vec![
                Part::Text("first".into()),
                Part::Reasoning("think".into()),
                Part::Text("last".into()),
            ],
        )];
        let mut by_raw = HashMap::new();
        by_raw.insert("a1".into(), "msg1".into());
        let state = make_state(by_raw);

        apply_anchored_nudges(&mut messages, &state, "nudge", &["a1".into()]);

        // Should append to the last Text part (third part)
        assert_eq!(messages[0].parts.len(), 3);
        assert!(matches!(&messages[0].parts[0], Part::Text(t) if t == "first"));
        assert!(matches!(&messages[0].parts[1], Part::Reasoning(_)));
        assert!(matches!(&messages[0].parts[2], Part::Text(t) if t == "last\n\nnudge"));
    }

    #[test]
    fn apply_anchored_nudges_empty_messages_slice() {
        let mut messages: Vec<Message> = vec![];
        let mut by_raw = HashMap::new();
        by_raw.insert("a1".into(), "msg1".into());
        let state = make_state(by_raw);

        // Should not panic with empty messages
        apply_anchored_nudges(&mut messages, &state, "nudge", &["a1".into()]);

        assert!(messages.is_empty());
    }

    #[test]
    fn apply_anchored_nudges_ref_not_in_messages() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let mut by_raw = HashMap::new();
        by_raw.insert("a1".into(), "msg999".into()); // msg999 doesn't exist
        let state = make_state(by_raw);

        // Should not panic
        apply_anchored_nudges(&mut messages, &state, "nudge", &["a1".into()]);

        assert_eq!(messages[0].parts.len(), 1);
        assert!(matches!(&messages[0].parts[0], Part::Text(t) if t == "hello"));
    }
}
