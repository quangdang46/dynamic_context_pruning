//! Apply phase — materialise pending prune decisions into the outgoing
//! message stream while preserving tool call/result pairing
//! (SPEC.md §5.4 + §11.1).
//!
//! The applier is parameterised by a per-call [`PruneKind`], which the
//! caller supplies based on which strategy authored the decision:
//!
//! * [`PruneKind::Drop`] — used by deduplicate and stale-file-reads. The
//!   tool_call and matching tool_result parts are removed entirely.
//! * [`PruneKind::PurgeError`] — used by purge_errors. The tool_call's
//!   `input` field is replaced with the placeholder JSON value
//!   `{"removed": "[input removed due to failed tool call]"}` and the
//!   tool_result's `output` is cleared while the `error` field is kept.
//!
//! Messages whose every part is removed and that have no `text` part are
//! dropped from the outgoing list.

use std::collections::HashMap;

use dcp_types::{Message, Part, Role};
use serde_json::{Value as JsonValue, json};

/// What the apply phase does for a single pruned `call_id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PruneKind {
    /// Strip the tool_call and matching tool_result entirely. Used by
    /// the deduplicate and stale-file-reads strategies.
    Drop,
    /// Keep the tool_call envelope but blank out the `input`. Keep the
    /// tool_result envelope and `error` field; drop the `output`. Used
    /// by purge_errors.
    PurgeError,
}

/// Placeholder text inserted into a purged tool_call's `input` JSON.
pub const PURGED_INPUT_PLACEHOLDER: &str = "[input removed due to failed tool call]";

/// Apply pruning decisions to `messages` and return the transformed
/// stream. Inputs are not modified; the function returns a fresh `Vec`.
///
/// `decisions` maps each pruned `call_id` to its [`PruneKind`]. Any
/// `call_id` not present in the map flows through unchanged.
pub fn apply_prune_to_messages(
    messages: &[Message],
    decisions: &HashMap<String, PruneKind>,
) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());

    for msg in messages {
        let new_parts: Vec<Part> = msg
            .parts
            .iter()
            .filter_map(|part| transform_part(msg.role, part, decisions))
            .collect();

        if new_parts.is_empty() {
            // SPEC §5.4: drop messages whose every part was removed.
            continue;
        }

        // Drop messages whose parts are all non-text dross (i.e. only
        // dangling tool envelopes) — SPEC §5.4 only requires "no text"
        // dropping when zero parts remain, so we keep partially-pruned
        // messages even if they no longer carry text.
        out.push(Message {
            id: msg.id.clone(),
            role: msg.role,
            parts: new_parts,
            time: msg.time,
        });
    }

    out
}

fn transform_part(role: Role, part: &Part, decisions: &HashMap<String, PruneKind>) -> Option<Part> {
    match (role, part) {
        (
            Role::Assistant,
            Part::ToolCall {
                call_id,
                tool,
                input,
            },
        ) => match decisions.get(call_id) {
            Some(PruneKind::Drop) => None,
            Some(PruneKind::PurgeError) => Some(Part::ToolCall {
                call_id: call_id.clone(),
                tool: tool.clone(),
                input: purged_input(),
            }),
            None => Some(Part::ToolCall {
                call_id: call_id.clone(),
                tool: tool.clone(),
                input: input.clone(),
            }),
        },
        (
            Role::User,
            Part::ToolResult {
                call_id,
                status,
                output,
                error,
            },
        ) => match decisions.get(call_id) {
            Some(PruneKind::Drop) => None,
            Some(PruneKind::PurgeError) => Some(Part::ToolResult {
                call_id: call_id.clone(),
                status: *status,
                output: None,
                error: error.clone(),
            }),
            None => Some(Part::ToolResult {
                call_id: call_id.clone(),
                status: *status,
                output: output.clone(),
                error: error.clone(),
            }),
        },
        (_, other) => Some(other.clone()),
    }
}

fn purged_input() -> JsonValue {
    json!({"removed": PURGED_INPUT_PLACEHOLDER})
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{Message, Part, Role, ToolStatus};
    use serde_json::json;

    fn assist(id: &str, parts: Vec<Part>) -> Message {
        Message::new(id, Role::Assistant, parts, 0)
    }
    fn user(id: &str, parts: Vec<Part>) -> Message {
        Message::new(id, Role::User, parts, 0)
    }

    #[test]
    fn passthrough_when_no_decisions() {
        let messages = vec![
            user("u1", vec![Part::text("hi")]),
            assist("a1", vec![Part::text("hello")]),
        ];
        let out = apply_prune_to_messages(&messages, &HashMap::new());
        assert_eq!(out, messages);
    }

    #[test]
    fn drop_removes_call_and_result_pair() {
        let messages = vec![
            assist(
                "a1",
                vec![
                    Part::text("doing"),
                    Part::tool_call("c1", "read", json!({"path": "x"})),
                ],
            ),
            user(
                "u1",
                vec![Part::tool_result(
                    "c1",
                    ToolStatus::Completed,
                    Some("ok".into()),
                    None,
                )],
            ),
            assist("a2", vec![Part::text("done")]),
        ];
        let mut decisions = HashMap::new();
        decisions.insert("c1".to_string(), PruneKind::Drop);
        let out = apply_prune_to_messages(&messages, &decisions);

        // Assistant message keeps text but loses the tool_call.
        assert_eq!(out[0].parts.len(), 1);
        assert!(matches!(&out[0].parts[0], Part::Text(t) if t == "doing"));
        // User result message had only the tool_result — dropped entirely.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "a1");
        assert_eq!(out[1].id, "a2");
    }

    #[test]
    fn purge_error_replaces_input_and_clears_output() {
        let messages = vec![
            assist(
                "a1",
                vec![Part::tool_call("c1", "read", json!({"path": "x"}))],
            ),
            user(
                "u1",
                vec![Part::tool_result(
                    "c1",
                    ToolStatus::Error,
                    None,
                    Some("boom".into()),
                )],
            ),
        ];
        let mut decisions = HashMap::new();
        decisions.insert("c1".to_string(), PruneKind::PurgeError);
        let out = apply_prune_to_messages(&messages, &decisions);

        // Call envelope kept, input rewritten.
        match &out[0].parts[0] {
            Part::ToolCall { input, .. } => {
                assert_eq!(input, &purged_input());
            }
            _ => panic!("expected ToolCall, got {:?}", out[0].parts[0]),
        }
        // Result envelope kept, output cleared, error preserved.
        match &out[1].parts[0] {
            Part::ToolResult {
                status,
                output,
                error,
                ..
            } => {
                assert_eq!(*status, ToolStatus::Error);
                assert!(output.is_none());
                assert_eq!(error.as_deref(), Some("boom"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn unrelated_calls_pass_through() {
        let messages = vec![
            assist(
                "a1",
                vec![
                    Part::tool_call("c1", "read", json!({"path": "x"})),
                    Part::tool_call("c2", "read", json!({"path": "y"})),
                ],
            ),
            user(
                "u1",
                vec![
                    Part::tool_result("c1", ToolStatus::Completed, Some("ok".into()), None),
                    Part::tool_result("c2", ToolStatus::Completed, Some("ok".into()), None),
                ],
            ),
        ];
        let mut decisions = HashMap::new();
        decisions.insert("c1".to_string(), PruneKind::Drop);
        let out = apply_prune_to_messages(&messages, &decisions);

        // c1 dropped, c2 kept.
        assert_eq!(out[0].parts.len(), 1);
        match &out[0].parts[0] {
            Part::ToolCall { call_id, .. } => assert_eq!(call_id, "c2"),
            _ => panic!(),
        }
        assert_eq!(out[1].parts.len(), 1);
        match &out[1].parts[0] {
            Part::ToolResult { call_id, .. } => assert_eq!(call_id, "c2"),
            _ => panic!(),
        }
    }

    #[test]
    fn drops_message_whose_only_part_was_pruned() {
        // Result-only user message → dropped entirely after Drop.
        let messages = vec![
            assist("a1", vec![Part::tool_call("c1", "read", json!({}))]),
            user(
                "u1",
                vec![Part::tool_result("c1", ToolStatus::Completed, None, None)],
            ),
            assist("a2", vec![Part::text("ack")]),
        ];
        let mut decisions = HashMap::new();
        decisions.insert("c1".to_string(), PruneKind::Drop);
        let out = apply_prune_to_messages(&messages, &decisions);
        let ids: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["a2"]);
    }
}
