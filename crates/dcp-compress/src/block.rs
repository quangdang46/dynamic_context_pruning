//! Block bookkeeping — `commit_block` (SPEC.md §6.4) and the frontier
//! advance rule (SPEC.md §6.5).

use dcp_types::{CompressionBlock, MessageRef, MessageRefKind, SessionState};

/// Commit `block` into `state`: insert it into `blocks_by_id`, register
/// the active anchor, and deactivate every `consumed_block_ids` entry
/// (rewriting `parent_block_ids`, `deactivated_at`,
/// `deactivated_by_block_id`, and the `active_by_anchor_message_id`
/// lookup).
pub fn commit_block(state: &mut SessionState, mut block: CompressionBlock, now_ms: i64) {
    let block_id = block.block_id;
    let anchor = block.anchor_message_id.clone();
    let consumed = block.consumed_block_ids.clone();

    if block.created_at == 0 {
        block.created_at = now_ms;
    }

    // Insert the new block first; consumed updates reference it.
    state
        .prune
        .messages
        .blocks_by_id
        .insert(block_id, block.clone());
    state.prune.messages.active_block_ids.insert(block_id);
    state
        .prune
        .messages
        .active_by_anchor_message_id
        .insert(anchor.clone(), block_id);

    for cid in &consumed {
        if let Some(consumed_block) = state.prune.messages.blocks_by_id.get_mut(cid) {
            consumed_block.active = false;
            consumed_block.deactivated_at = Some(now_ms);
            consumed_block.deactivated_by_block_id = Some(block_id);
            if !consumed_block.parent_block_ids.contains(&block_id) {
                consumed_block.parent_block_ids.push(block_id);
            }
            // Remove the consumed anchor lookup *only* if it still
            // points at the consumed id. SPEC §6.4 — the parent already
            // overwrote the entry in the loop above; this guards against
            // a stale leftover when the consumed and parent anchors
            // coincide.
            let consumed_anchor = consumed_block.anchor_message_id.clone();
            if state
                .prune
                .messages
                .active_by_anchor_message_id
                .get(&consumed_anchor)
                == Some(cid)
            {
                state
                    .prune
                    .messages
                    .active_by_anchor_message_id
                    .remove(&consumed_anchor);
            }
        }
        state.prune.messages.active_block_ids.remove(cid);
    }

    state.stats.compress_blocks_committed = state.stats.compress_blocks_committed.saturating_add(1);
}

/// Possibly advance `state.prune.messages.frontier_message_ref` after a
/// block commits. SPEC §6.5: the frontier advances when the new block's
/// `summary_tokens >= compressed_tokens`, i.e. compression yielded no
/// benefit. Returns `true` when the frontier moved.
pub fn maybe_advance_frontier(state: &mut SessionState, committed: &CompressionBlock) -> bool {
    if committed.summary_tokens < committed.compressed_tokens {
        // Beneficial — leave the frontier alone, but record the win in
        // stats.
        state.stats.compress_useful = state.stats.compress_useful.saturating_add(1);
        return false;
    }

    state.stats.compress_oversized = state.stats.compress_oversized.saturating_add(1);

    let new_ref_str = committed.end_id.clone();
    let new_ref = match MessageRef::parse(&new_ref_str) {
        Ok(r) => r,
        Err(_) => return false,
    };

    let advanced = match &state.prune.messages.frontier_message_ref {
        None => true,
        Some(prev) => match (MessageRef::parse(prev), new_ref.kind()) {
            (Ok(prev_ref), _) => ref_lt(prev_ref.kind(), new_ref.kind()),
            (Err(_), _) => true,
        },
    };
    if advanced {
        state.prune.messages.frontier_message_ref = Some(new_ref_str);
    }
    advanced
}

fn ref_lt(a: MessageRefKind, b: MessageRefKind) -> bool {
    match (a, b) {
        (MessageRefKind::Message(x), MessageRefKind::Message(y)) => x < y,
        (MessageRefKind::Block(x), MessageRefKind::Block(y)) => x.value() < y.value(),
        // Mixed-kind comparison: block refs sort after message refs (a
        // block always represents a span; treat it as "later" than any
        // m####).
        (MessageRefKind::Message(_), MessageRefKind::Block(_)) => true,
        (MessageRefKind::Block(_), MessageRefKind::Message(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{BlockId, CompressionMode, RunId, SessionState};

    fn fresh_block(id: u32, anchor: &str) -> CompressionBlock {
        let mut b = CompressionBlock::new(
            BlockId::new(id),
            RunId::new(id),
            CompressionMode::Range,
            "t",
            "summary",
            "m0001",
            "m0002",
            anchor,
            "comp",
        );
        b.active = true;
        b
    }

    #[test]
    fn commit_inserts_and_marks_active() {
        let mut state = SessionState::default();
        let block = fresh_block(1, "raw1");
        commit_block(&mut state, block.clone(), 1_000);
        assert!(
            state
                .prune
                .messages
                .blocks_by_id
                .contains_key(&block.block_id)
        );
        assert!(
            state
                .prune
                .messages
                .active_block_ids
                .contains(&block.block_id)
        );
        assert_eq!(
            state.prune.messages.active_by_anchor_message_id["raw1"],
            block.block_id
        );
        assert_eq!(state.stats.compress_blocks_committed, 1);
    }

    #[test]
    fn commit_deactivates_consumed_block() {
        let mut state = SessionState::default();
        let child = fresh_block(1, "raw_child");
        commit_block(&mut state, child.clone(), 1_000);

        let mut parent = fresh_block(2, "raw_parent");
        parent.consumed_block_ids = vec![BlockId::new(1)];
        commit_block(&mut state, parent.clone(), 2_000);

        let consumed = &state.prune.messages.blocks_by_id[&BlockId::new(1)];
        assert!(!consumed.active);
        assert_eq!(consumed.deactivated_at, Some(2_000));
        assert_eq!(consumed.deactivated_by_block_id, Some(BlockId::new(2)));
        assert_eq!(consumed.parent_block_ids, vec![BlockId::new(2)]);
        // Active set excludes the consumed id and contains the parent.
        assert!(
            !state
                .prune
                .messages
                .active_block_ids
                .contains(&BlockId::new(1))
        );
        assert!(
            state
                .prune
                .messages
                .active_block_ids
                .contains(&BlockId::new(2))
        );
        // Anchor lookup for the consumed anchor is gone.
        assert!(
            !state
                .prune
                .messages
                .active_by_anchor_message_id
                .contains_key("raw_child")
        );
    }

    #[test]
    fn frontier_advances_on_oversized_block() {
        let mut state = SessionState::default();
        let mut block = fresh_block(1, "raw");
        block.compressed_tokens = 100;
        block.summary_tokens = 200;
        block.end_id = "m0042".into();
        let advanced = maybe_advance_frontier(&mut state, &block);
        assert!(advanced);
        assert_eq!(
            state.prune.messages.frontier_message_ref.as_deref(),
            Some("m0042")
        );
        assert_eq!(state.stats.compress_oversized, 1);
        assert_eq!(state.stats.compress_useful, 0);
    }

    #[test]
    fn frontier_unchanged_on_useful_block() {
        let mut state = SessionState::default();
        let mut block = fresh_block(1, "raw");
        block.compressed_tokens = 200;
        block.summary_tokens = 100;
        block.end_id = "m0042".into();
        assert!(!maybe_advance_frontier(&mut state, &block));
        assert!(state.prune.messages.frontier_message_ref.is_none());
        assert_eq!(state.stats.compress_useful, 1);
    }

    #[test]
    fn frontier_only_moves_forward() {
        let mut state = SessionState::default();
        state.prune.messages.frontier_message_ref = Some("m0050".into());
        let mut block = fresh_block(1, "raw");
        block.compressed_tokens = 100;
        block.summary_tokens = 200;
        block.end_id = "m0010".into();
        assert!(!maybe_advance_frontier(&mut state, &block));
        assert_eq!(
            state.prune.messages.frontier_message_ref.as_deref(),
            Some("m0050")
        );
    }

    #[test]
    fn frontier_breakeven_treated_as_oversized() {
        let mut state = SessionState::default();
        let mut block = fresh_block(1, "raw");
        block.compressed_tokens = 100;
        block.summary_tokens = 100;
        block.end_id = "m0001".into();
        assert!(maybe_advance_frontier(&mut state, &block));
        assert_eq!(state.stats.compress_oversized, 1);
    }
}
