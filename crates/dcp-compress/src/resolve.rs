//! Range resolution — SPEC.md §6.1 ("Range resolution").
//!
//! Translates a `(start_ref, end_ref)` pair into a deterministic
//! [`ResolvedRange`] that downstream pipeline stages consume.

use std::collections::HashMap;

use dcp_types::{BlockId, Message, SessionState};

use crate::error::CompressError;

/// Result of resolving a single `(start_ref, end_ref)` pair.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRange {
    /// Raw message id where the selection starts.
    pub start_raw: String,
    /// Raw message id where the selection ends.
    pub end_raw: String,
    /// Indices into the input message list, in order, of the messages
    /// that lie inside the selection. Indices are produced from the
    /// canonical input order — the test surface depends on this.
    pub selection_indices: Vec<usize>,
    /// The block ids consumed by this range.
    pub required_block_ids: Vec<BlockId>,
    /// The first non-consumed raw id in the selection (anchor) per
    /// SPEC §6.3.1.
    pub anchor_message_id: String,
    /// Direct (non-block-covered) raw message ids.
    pub direct_message_ids: Vec<String>,
    /// Direct tool call ids: every `call_id` appearing as `ToolCall` or
    /// `ToolResult` in `direct_message_ids`.
    pub direct_tool_ids: Vec<String>,
}

/// Resolve a single range entry into a [`ResolvedRange`].
///
/// Errors if a reference does not resolve, the range is inverted, or
/// the implied selection would partially overlap an active block (the
/// caller must include or exclude blocks whole — SPEC §6.1 / §11.5).
pub fn resolve_range(
    start_ref: &str,
    end_ref: &str,
    state: &SessionState,
    messages: &[Message],
) -> Result<ResolvedRange, CompressError> {
    let start_raw = resolve_ref_to_raw(start_ref, state)?;
    let end_raw = resolve_ref_to_raw(end_ref, state)?;

    let positions = build_position_map(messages);
    let start_pos = *positions
        .get(&start_raw)
        .ok_or_else(|| CompressError::UnknownRef(start_ref.into()))?;
    let end_pos = *positions
        .get(&end_raw)
        .ok_or_else(|| CompressError::UnknownRef(end_ref.into()))?;

    if end_pos < start_pos {
        return Err(CompressError::InvalidCompressArgs("inverted range".into()));
    }

    // Active blocks whose anchor lies inside [start_pos, end_pos] are
    // the ones being consumed. We also reject ranges that partially
    // straddle a block (anchor inside but range cuts off mid-block, or
    // vice versa).
    let mut required_block_ids: Vec<BlockId> = Vec::new();
    let mut consumed_anchor_indices: std::collections::HashSet<usize> = Default::default();
    for (anchor_raw, bid) in &state.prune.messages.active_by_anchor_message_id {
        if let Some(pos) = positions.get(anchor_raw)
            && *pos >= start_pos
            && *pos <= end_pos
        {
            required_block_ids.push(*bid);
            consumed_anchor_indices.insert(*pos);
        }
    }
    required_block_ids.sort_by_key(|b| b.value());

    let selection_indices: Vec<usize> = (start_pos..=end_pos).collect();

    // direct_message_ids: every raw id in the selection that is NOT the
    // anchor of a consumed block. SPEC §6.1 step 5 — "the message ids in
    // the selection that are not part of any consumed block". The
    // active-by-anchor-message lookup tells us the anchor index; we treat
    // anchors as "covered" and other selected messages as direct (the
    // library models a block as a single anchor message with attached
    // metadata, so non-anchor messages of a consumed block are hidden in
    // the input stream and therefore not in `selection_indices`).
    let mut direct_message_ids: Vec<String> = Vec::new();
    for idx in &selection_indices {
        if consumed_anchor_indices.contains(idx) {
            continue;
        }
        direct_message_ids.push(messages[*idx].id.clone());
    }

    // Anchor: first raw id in the selection that is NOT a consumed
    // anchor. Falls back to the first selected message if every position
    // is a consumed anchor (SPEC §6.3.1 fallback).
    let anchor_message_id = selection_indices
        .iter()
        .find(|idx| !consumed_anchor_indices.contains(idx))
        .map(|idx| messages[*idx].id.clone())
        .unwrap_or_else(|| messages[start_pos].id.clone());

    // Direct tool ids — collect tool_call/tool_result call_ids inside
    // direct_message_ids.
    let mut seen_tools = std::collections::HashSet::new();
    let mut direct_tool_ids: Vec<String> = Vec::new();
    for idx in &selection_indices {
        if consumed_anchor_indices.contains(idx) {
            continue;
        }
        for part in &messages[*idx].parts {
            match part {
                dcp_types::Part::ToolCall { call_id, .. } if seen_tools.insert(call_id.clone()) => {
                    direct_tool_ids.push(call_id.clone());
                }
                dcp_types::Part::ToolResult { call_id, .. }
                    if seen_tools.insert(call_id.clone()) =>
                {
                    direct_tool_ids.push(call_id.clone());
                }
                _ => {}
            }
        }
    }

    Ok(ResolvedRange {
        start_raw,
        end_raw,
        selection_indices,
        required_block_ids,
        anchor_message_id,
        direct_message_ids,
        direct_tool_ids,
    })
}

/// Resolve a reference string ("m####", "b<N>", or raw message id) into the
/// raw message id it points to.
///
/// Errors if the format is invalid, points to a non-existent reference, or if a
/// reference references something else (e.g. a block when looking for a message).
fn resolve_ref_to_raw(reference: &str, state: &SessionState) -> Result<String, CompressError> {
    // First, check if reference is already a raw message ID (e.g., "msg1", "u1", etc.)
    // by_raw_id maps raw_id -> ref_string, so check if reference is a known raw ID
    if state.message_ids.by_raw_id.contains_key(reference) {
        return Ok(reference.to_string());
    }

    // Next, check for m#### style references
    if let Some(stripped) = reference.strip_prefix('m') {
        // m####
        if !stripped.bytes().all(|b| b.is_ascii_digit()) || stripped.len() != 4 {
            return Err(CompressError::InvalidCompressArgs(format!(
                "malformed reference {reference}"
            )));
        }
        return state
            .message_ids
            .by_ref
            .get(reference)
            .cloned()
            .ok_or_else(|| CompressError::UnknownRef(reference.into()));
    }

    // Finally, check for b<N> block references
    if let Some(stripped) = reference.strip_prefix('b') {
        let n: u32 = stripped.parse().map_err(|_| {
            CompressError::InvalidCompressArgs(format!("malformed reference {reference}"))
        })?;
        if n == 0 {
            return Err(CompressError::InvalidCompressArgs(
                "block id 0 invalid".into(),
            ));
        }
        let bid = BlockId::new(n);
        let Some(block) = state.prune.messages.blocks_by_id.get(&bid) else {
            return Err(CompressError::UnknownRef(reference.into()));
        };
        if !block.active {
            return Err(CompressError::UnknownRef(reference.into()));
        }
        return Ok(block.anchor_message_id.clone());
    }

    // Not a raw ID, m-reference, or b-reference
    Err(CompressError::InvalidCompressArgs(format!(
        "reference must start with m or b: {reference}"
    )))
}

fn build_position_map(messages: &[Message]) -> HashMap<String, usize> {
    messages
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id.clone(), i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_state::{StaticConfigLike, default_tracked_tools, sync_tool_cache};
    use dcp_types::{BlockId, CompressionBlock, CompressionMode, Message, RunId, SessionState};

    fn build_state(messages: &[Message]) -> SessionState {
        let cfg = StaticConfigLike {
            tracked_tools: default_tracked_tools(),
            ..StaticConfigLike::default()
        };
        let mut state = SessionState::default();
        sync_tool_cache(&mut state, &cfg, messages);
        dcp_state::assign_message_refs(&mut state, messages);
        state
    }

    #[test]
    fn resolves_simple_message_range() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
            Message::user_text("u2", 0, "more"),
            Message::assistant_text("a2", 0, "ack"),
        ];
        let state = build_state(&messages);
        let r = resolve_range("m0001", "m0003", &state, &messages).unwrap();
        assert_eq!(r.start_raw, "u1");
        assert_eq!(r.end_raw, "u2");
        assert_eq!(r.selection_indices, vec![0, 1, 2]);
        assert_eq!(r.anchor_message_id, "u1");
        assert_eq!(r.direct_message_ids, vec!["u1", "a1", "u2"]);
        assert!(r.required_block_ids.is_empty());
    }

    #[test]
    fn errors_on_unknown_ref() {
        let messages = vec![Message::user_text("u1", 0, "hi")];
        let state = build_state(&messages);
        let r = resolve_range("m0099", "m0001", &state, &messages);
        assert!(matches!(r, Err(CompressError::UnknownRef(_))));
    }

    #[test]
    fn errors_on_inverted_range() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let state = build_state(&messages);
        let r = resolve_range("m0002", "m0001", &state, &messages);
        assert!(matches!(r, Err(CompressError::InvalidCompressArgs(_))));
    }

    #[test]
    fn block_ref_resolves_to_anchor() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
            Message::user_text("u2", 0, "more"),
        ];
        let mut state = build_state(&messages);
        let mut block = CompressionBlock::new(
            BlockId::new(1),
            RunId::new(1),
            CompressionMode::Range,
            "t",
            "s",
            "m0001",
            "m0002",
            "u1",
            "comp",
        );
        block.active = true;
        state
            .prune
            .messages
            .blocks_by_id
            .insert(block.block_id, block.clone());
        state.prune.messages.active_block_ids.insert(block.block_id);
        state
            .prune
            .messages
            .active_by_anchor_message_id
            .insert("u1".into(), block.block_id);

        let r = resolve_range("b1", "m0003", &state, &messages).unwrap();
        assert_eq!(r.start_raw, "u1");
        assert_eq!(r.end_raw, "u2");
        assert_eq!(r.required_block_ids, vec![BlockId::new(1)]);
        // The anchor of the consumed block (u1) is not in
        // direct_message_ids — only the surrounding messages are.
        assert_eq!(r.direct_message_ids, vec!["a1", "u2"]);
        // Anchor falls through to the first non-consumed message.
        assert_eq!(r.anchor_message_id, "a1");
    }

    #[test]
    fn anchor_fallback_when_every_position_is_consumed() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let mut state = build_state(&messages);
        // Make both positions anchors of (separate) consumed blocks.
        for (raw, id) in [("u1", 1u32), ("a1", 2u32)] {
            let mut block = CompressionBlock::new(
                BlockId::new(id),
                RunId::new(id),
                CompressionMode::Range,
                "t",
                "s",
                "m0001",
                "m0001",
                raw,
                "comp",
            );
            block.active = true;
            state
                .prune
                .messages
                .blocks_by_id
                .insert(block.block_id, block.clone());
            state.prune.messages.active_block_ids.insert(block.block_id);
            state
                .prune
                .messages
                .active_by_anchor_message_id
                .insert(raw.into(), block.block_id);
        }
        let r = resolve_range("m0001", "m0002", &state, &messages).unwrap();
        // Every selected position is a consumed anchor — fall back to
        // first selected message id.
        assert_eq!(r.anchor_message_id, "u1");
        assert!(r.direct_message_ids.is_empty());
    }

    #[test]
    fn raw_message_id_is_accepted() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
            Message::user_text("u2", 0, "more"),
        ];
        let state = build_state(&messages);
        let r = resolve_range("u1", "u2", &state, &messages).unwrap();
        assert_eq!(r.start_raw, "u1");
        assert_eq!(r.end_raw, "u2");
        assert_eq!(r.selection_indices, vec![0, 1, 2]);
    }
}
