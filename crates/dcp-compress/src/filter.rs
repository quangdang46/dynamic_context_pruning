//! `filter_compressed_ranges` — turn active blocks into outgoing
//! messages (SPEC.md §6.4).
//!
//! The function walks the input message list and:
//!
//! 1. Drops every raw id in any active block's `direct_message_ids`
//!    *except* its `anchor_message_id`.
//! 2. For each anchor, replaces the message's text/reasoning parts with
//!    a single `Text` part containing the wrapped block summary, and
//!    strips tool_call/tool_result parts whose `call_id` is in the
//!    block's `effective_tool_ids`.

use std::collections::{HashMap, HashSet};

use dcp_types::{Message, Part, SessionState};

/// Apply compression-block expansion to `messages`.
pub fn filter_compressed_ranges(messages: &[Message], state: &SessionState) -> Vec<Message> {
    let mut skip_ids: HashSet<&str> = HashSet::new();
    let mut anchor_to_block: HashMap<&str, &dcp_types::CompressionBlock> = HashMap::new();

    for bid in &state.prune.messages.active_block_ids {
        let Some(block) = state.prune.messages.blocks_by_id.get(bid) else {
            continue;
        };
        for raw in &block.direct_message_ids {
            if raw.as_str() != block.anchor_message_id.as_str() {
                skip_ids.insert(raw.as_str());
            }
        }
        anchor_to_block.insert(block.anchor_message_id.as_str(), block);
    }

    let mut out = Vec::with_capacity(messages.len());
    for msg in messages {
        if skip_ids.contains(msg.id.as_str()) {
            continue;
        }
        match anchor_to_block.get(msg.id.as_str()) {
            Some(block) => {
                let new = render_block_anchor(msg, block);
                out.push(new);
            }
            None => out.push(msg.clone()),
        }
    }
    out
}

fn render_block_anchor(msg: &Message, block: &dcp_types::CompressionBlock) -> Message {
    let effective_tools: HashSet<&str> = block
        .effective_tool_ids
        .iter()
        .map(String::as_str)
        .collect();

    let mut new_parts: Vec<Part> = Vec::with_capacity(msg.parts.len() + 1);
    new_parts.push(Part::Text(block.summary.clone()));
    for part in &msg.parts {
        match part {
            // The first text/reasoning is replaced; subsequent ones are
            // dropped. SPEC §6.4 — the wrapped summary is the canonical
            // text for the anchor.
            Part::Text(_) | Part::Reasoning(_) => {}
            Part::ToolCall { call_id, .. } | Part::ToolResult { call_id, .. } => {
                if !effective_tools.contains(call_id.as_str()) {
                    new_parts.push(part.clone());
                }
            }
            other => new_parts.push(other.clone()),
        }
    }

    Message {
        id: msg.id.clone(),
        role: msg.role,
        parts: new_parts,
        time: msg.time,
        ignored: msg.ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{
        BlockId, CompressionBlock, CompressionMode, Message, Part, Role, RunId, SessionState,
        ToolStatus,
    };
    use serde_json::json;

    fn install_block(state: &mut SessionState, mut block: CompressionBlock) {
        let bid = block.block_id;
        let anchor = block.anchor_message_id.clone();
        block.active = true;
        state.prune.messages.blocks_by_id.insert(bid, block);
        state.prune.messages.active_block_ids.insert(bid);
        state
            .prune
            .messages
            .active_by_anchor_message_id
            .insert(anchor, bid);
    }

    #[test]
    fn replaces_anchor_text_and_drops_other_direct_ids() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
            Message::user_text("u2", 0, "more"),
            Message::assistant_text("a2", 0, "ack"),
            Message::user_text("u3", 0, "after"),
        ];
        let mut state = SessionState::default();
        let mut block = CompressionBlock::new(
            BlockId::new(1),
            RunId::new(1),
            CompressionMode::Range,
            "t",
            "WRAPPED SUMMARY",
            "m0001",
            "m0004",
            "u1",
            "comp",
        );
        block.direct_message_ids = vec!["u1".into(), "a1".into(), "u2".into(), "a2".into()];
        install_block(&mut state, block);

        let out = filter_compressed_ranges(&messages, &state);
        let ids: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["u1", "u3"]);
        // u1's parts have been replaced with the wrapped summary.
        match &out[0].parts[0] {
            Part::Text(t) => assert_eq!(t, "WRAPPED SUMMARY"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn strips_tool_parts_in_effective_tool_ids() {
        let messages = vec![Message::new(
            "a1",
            Role::Assistant,
            vec![
                Part::text("inner thinking"),
                Part::tool_call("c1", "read", json!({})),
                Part::tool_call("c2", "read", json!({})),
            ],
            0,
        )];
        let mut state = SessionState::default();
        let mut block = CompressionBlock::new(
            BlockId::new(1),
            RunId::new(1),
            CompressionMode::Message,
            "t",
            "wrapped",
            "m0001",
            "m0001",
            "a1",
            "comp",
        );
        block.direct_message_ids = vec!["a1".into()];
        block.effective_tool_ids = vec!["c1".into()];
        install_block(&mut state, block);

        let out = filter_compressed_ranges(&messages, &state);
        let parts = &out[0].parts;
        // First part is the wrapped summary text.
        assert!(matches!(&parts[0], Part::Text(t) if t == "wrapped"));
        // Tool call c1 is dropped; c2 survives.
        let remaining_calls: Vec<&str> = parts
            .iter()
            .filter_map(|p| {
                if let Part::ToolCall { call_id, .. } = p {
                    Some(call_id.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(remaining_calls, vec!["c2"]);
    }

    #[test]
    fn keeps_tool_result_when_call_kept() {
        let messages = vec![
            Message::new(
                "a1",
                Role::Assistant,
                vec![Part::tool_call("c1", "read", json!({}))],
                0,
            ),
            Message::new(
                "u1",
                Role::User,
                vec![Part::tool_result(
                    "c1",
                    ToolStatus::Completed,
                    Some("ok".into()),
                    None,
                )],
                0,
            ),
        ];
        let mut state = SessionState::default();
        let mut block = CompressionBlock::new(
            BlockId::new(1),
            RunId::new(1),
            CompressionMode::Message,
            "t",
            "wrapped",
            "m0001",
            "m0001",
            "a1",
            "comp",
        );
        block.direct_message_ids = vec!["a1".into()];
        // No tool ids are "covered" by this block.
        install_block(&mut state, block);

        let out = filter_compressed_ranges(&messages, &state);
        // Anchor's tool_call is kept (not in effective_tool_ids); u1
        // still flows through unchanged.
        let kept_a1_calls: Vec<&str> = out[0]
            .parts
            .iter()
            .filter_map(|p| {
                if let Part::ToolCall { call_id, .. } = p {
                    Some(call_id.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(kept_a1_calls, vec!["c1"]);
        assert_eq!(out[1].id, "u1");
    }

    #[test]
    fn passthrough_when_no_blocks() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let state = SessionState::default();
        let out = filter_compressed_ranges(&messages, &state);
        assert_eq!(out, messages);
    }
}
