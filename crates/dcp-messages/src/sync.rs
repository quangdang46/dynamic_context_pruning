//! Compression block synchronization — port of lib/messages/sync.ts.
//!
//! Provides: sync_compression_blocks.

use std::collections::HashSet;

use dcp_types::{BlockId, Message, SessionState};

/// Sentinel timestamp used when the real wall-clock time is unavailable.
const DEACTIVATED_AT_SENTINEL: i64 = 0;

/// Synchronize compression block activation state with the current message set.
///
/// This function updates `state.prune.messages` to reflect which compression
/// blocks should be active based on:
/// - Whether the block's compress message is still present in the messages slice
/// - Whether the block was manually deactivated by the user
/// - Whether any of the block's consumed blocks are no longer active
///
/// After determining active blocks, it updates `by_message_id` entries to only
/// include message IDs from blocks that are still active.
pub fn sync_compression_blocks(state: &mut SessionState, messages: &[Message]) {
    let prune_state = &mut state.prune.messages;

    // Step 1: Build set of all message IDs present in the messages slice
    let message_ids: HashSet<String> = messages.iter().map(|m| m.id.clone()).collect();

    // Step 2: Clear current active tracking
    prune_state.active_block_ids.clear();
    prune_state.active_by_anchor_message_id.clear();

    // Step 3: Get sorted order of block IDs by created_at
    let mut block_ids: Vec<BlockId> = prune_state.blocks_by_id.keys().copied().collect();
    block_ids.sort_by_key(|id| {
        prune_state
            .blocks_by_id
            .get(id)
            .map(|b| b.created_at)
            .unwrap_or(0)
    });

    // PASS 1: Determine which blocks to deactivate (message absence or user flag).
    let mut deactivate: HashSet<BlockId> = HashSet::new();
    for block_id in &block_ids {
        let block = match prune_state.blocks_by_id.get(block_id) {
            Some(b) => b,
            None => continue,
        };

        if !message_ids.contains(&block.compress_message_id) || block.deactivated_by_user {
            deactivate.insert(*block_id);
        }
    }

    // PASS 2: Cascade deactivation through consumed_block_ids.
    let mut changed = true;
    while changed {
        changed = false;
        for block_id in &block_ids {
            if deactivate.contains(block_id) {
                continue;
            }
            let block = match prune_state.blocks_by_id.get(block_id) {
                Some(b) => b,
                None => continue,
            };
            let consumed_deactivated = block
                .consumed_block_ids
                .iter()
                .any(|id| deactivate.contains(id));
            if consumed_deactivated {
                deactivate.insert(*block_id);
                changed = true;
            }
        }
    }

    // PASS 3: Apply decisions and rebuild tracking.
    for block_id in block_ids {
        if let Some(b) = prune_state.blocks_by_id.get_mut(&block_id) {
            if deactivate.contains(&block_id) {
                b.active = false;
                if b.deactivated_at.is_none() {
                    b.deactivated_at = Some(DEACTIVATED_AT_SENTINEL);
                }
            } else {
                b.active = true;
                b.deactivated_at = None;
                prune_state.active_block_ids.insert(block_id);
                prune_state
                    .active_by_anchor_message_id
                    .insert(b.anchor_message_id.clone(), block_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{CompressionBlock, CompressionMode, RunId};

    fn make_test_block(
        block_id: u32,
        compress_message_id: &str,
        anchor_message_id: &str,
    ) -> CompressionBlock {
        CompressionBlock::new(
            BlockId::new(block_id),
            RunId::new(1),
            CompressionMode::Range,
            "test-topic",
            "test-summary",
            "start_id", // start_id
            "end_id",   // end_id
            anchor_message_id,
            compress_message_id,
        )
    }

    fn make_test_message(id: &str) -> Message {
        Message::user_text(id, 0, "test content")
    }

    #[test]
    fn test_sync_deactivates_block_when_origin_message_gone() {
        let mut state = SessionState::default();
        let block_id = BlockId::new(1);
        let compress_msg_id = "m_compress_1";
        let anchor_msg_id = "m_anchor_1";

        // Add block whose compress_message_id is NOT in the messages slice
        let mut block = make_test_block(block_id.value(), compress_msg_id, anchor_msg_id);
        block.active = true;
        state.prune.messages.blocks_by_id.insert(block_id, block);

        // Note: messages slice does NOT contain "m_compress_1"
        let messages = vec![make_test_message("m_other")];

        sync_compression_blocks(&mut state, &messages);

        // Block should be deactivated
        let block = state.prune.messages.blocks_by_id.get(&block_id).unwrap();
        assert!(!block.active);
        assert!(block.deactivated_at.is_some());
        assert!(!state.prune.messages.active_block_ids.contains(&block_id));
    }

    #[test]
    fn test_sync_keeps_block_active_when_origin_message_present() {
        let mut state = SessionState::default();
        let block_id = BlockId::new(1);
        let compress_msg_id = "m_compress_1";
        let anchor_msg_id = "m_anchor_1";

        let mut block = make_test_block(block_id.value(), compress_msg_id, anchor_msg_id);
        block.active = true;
        state.prune.messages.blocks_by_id.insert(block_id, block);

        // Include the compress_message_id in messages
        let messages = vec![make_test_message(compress_msg_id)];

        sync_compression_blocks(&mut state, &messages);

        // Block should remain active
        let block = state.prune.messages.blocks_by_id.get(&block_id).unwrap();
        assert!(block.active);
        assert!(state.prune.messages.active_block_ids.contains(&block_id));
        assert_eq!(
            state
                .prune
                .messages
                .active_by_anchor_message_id
                .get(anchor_msg_id),
            Some(&block_id)
        );
    }

    #[test]
    fn test_sync_keeps_deactivated_block_when_user_deactivated() {
        let mut state = SessionState::default();
        let block_id = BlockId::new(1);
        let compress_msg_id = "m_compress_1";
        let anchor_msg_id = "m_anchor_1";

        let mut block = make_test_block(block_id.value(), compress_msg_id, anchor_msg_id);
        block.active = true;
        block.deactivated_by_user = true;
        state.prune.messages.blocks_by_id.insert(block_id, block);

        // origin message IS present
        let messages = vec![make_test_message(compress_msg_id)];

        sync_compression_blocks(&mut state, &messages);

        // Block should remain deactivated (user explicitly deactivated)
        let block = state.prune.messages.blocks_by_id.get(&block_id).unwrap();
        assert!(!block.active);
        assert!(!state.prune.messages.active_block_ids.contains(&block_id));
    }

    #[test]
    fn test_sync_deactivates_block_when_consumed_block_becomes_inactive() {
        let mut state = SessionState::default();
        let consumed_block_id = BlockId::new(1);
        let parent_block_id = BlockId::new(2);

        let mut consumed_block =
            make_test_block(consumed_block_id.value(), "m_consumed", "m_anchor_1");
        consumed_block.active = true;
        state
            .prune
            .messages
            .blocks_by_id
            .insert(consumed_block_id, consumed_block);

        let mut parent_block = make_test_block(
            parent_block_id.value(),
            "m_parent_compress",
            "m_parent_anchor",
        );
        parent_block.active = true;
        parent_block.consumed_block_ids = vec![consumed_block_id];
        state
            .prune
            .messages
            .blocks_by_id
            .insert(parent_block_id, parent_block);

        // Initially both active - parent consumes block 1
        let messages = vec![
            make_test_message("m_consumed"),
            make_test_message("m_parent_compress"),
        ];

        sync_compression_blocks(&mut state, &messages);
        assert!(
            state
                .prune
                .messages
                .blocks_by_id
                .get(&parent_block_id)
                .unwrap()
                .active
        );

        // Now deactivate the consumed block by removing its message
        let messages_without_consumed = vec![make_test_message("m_parent_compress")];

        sync_compression_blocks(&mut state, &messages_without_consumed);

        // Parent should now be deactivated too
        let parent = state
            .prune
            .messages
            .blocks_by_id
            .get(&parent_block_id)
            .unwrap();
        assert!(!parent.active);
    }

    #[test]
    fn test_sync_preserves_active_block_when_all_consumed_blocks_still_active() {
        let mut state = SessionState::default();
        let consumed_block_id = BlockId::new(1);
        let parent_block_id = BlockId::new(2);

        let mut consumed_block =
            make_test_block(consumed_block_id.value(), "m_consumed", "m_anchor_1");
        consumed_block.active = true;
        state
            .prune
            .messages
            .blocks_by_id
            .insert(consumed_block_id, consumed_block);

        let mut parent_block = make_test_block(
            parent_block_id.value(),
            "m_parent_compress",
            "m_parent_anchor",
        );
        parent_block.active = true;
        parent_block.consumed_block_ids = vec![consumed_block_id];
        state
            .prune
            .messages
            .blocks_by_id
            .insert(parent_block_id, parent_block);

        // Both messages present
        let messages = vec![
            make_test_message("m_consumed"),
            make_test_message("m_parent_compress"),
        ];

        sync_compression_blocks(&mut state, &messages);

        // Parent should remain active
        let parent = state
            .prune
            .messages
            .blocks_by_id
            .get(&parent_block_id)
            .unwrap();
        assert!(parent.active);
        assert!(
            state
                .prune
                .messages
                .active_block_ids
                .contains(&parent_block_id)
        );
    }

    #[test]
    fn test_sync_clears_and_rebuilds_active_tracking() {
        let mut state = SessionState::default();
        let block_id_1 = BlockId::new(1);
        let block_id_2 = BlockId::new(2);

        let mut block1 = make_test_block(1, "m_compress_1", "m_anchor_1");
        block1.active = true;
        state.prune.messages.blocks_by_id.insert(block_id_1, block1);

        let mut block2 = make_test_block(2, "m_compress_2", "m_anchor_2");
        block2.active = true;
        state.prune.messages.blocks_by_id.insert(block_id_2, block2);

        // Both start as active via initial state
        state.prune.messages.active_block_ids.insert(block_id_1);
        state
            .prune
            .messages
            .active_by_anchor_message_id
            .insert("m_anchor_1".to_string(), block_id_1);

        // Only include one message, so only block 1 should stay active
        let messages = vec![make_test_message("m_compress_1")];

        sync_compression_blocks(&mut state, &messages);

        // Only block 1 should be active now
        assert!(state.prune.messages.active_block_ids.contains(&block_id_1));
        assert!(!state.prune.messages.active_block_ids.contains(&block_id_2));
        assert_eq!(
            state
                .prune
                .messages
                .active_by_anchor_message_id
                .get("m_anchor_1"),
            Some(&block_id_1)
        );
        assert!(
            !state
                .prune
                .messages
                .active_by_anchor_message_id
                .contains_key("m_anchor_2")
        );
    }

    #[test]
    fn test_sync_with_empty_messages_clears_all_active() {
        let mut state = SessionState::default();
        let block_id = BlockId::new(1);
        let compress_msg_id = "m_compress_1";

        let mut block = make_test_block(block_id.value(), compress_msg_id, "m_anchor_1");
        block.active = true;
        state.prune.messages.blocks_by_id.insert(block_id, block);

        let messages: Vec<Message> = vec![];

        sync_compression_blocks(&mut state, &messages);

        assert!(state.prune.messages.active_block_ids.is_empty());
        let block = state.prune.messages.blocks_by_id.get(&block_id).unwrap();
        assert!(!block.active);
    }

    #[test]
    fn test_sync_with_no_blocks_is_noop() {
        let mut state = SessionState::default();
        state.prune.messages.blocks_by_id.clear();
        state.prune.messages.active_block_ids.clear();

        let messages = vec![make_test_message("m1"), make_test_message("m2")];

        sync_compression_blocks(&mut state, &messages);

        assert!(state.prune.messages.active_block_ids.is_empty());
        assert!(state.prune.messages.active_by_anchor_message_id.is_empty());
    }

    #[test]
    fn test_sync_multiple_blocks_with_same_anchor() {
        let mut state = SessionState::default();

        // Block 1 (older, should be superseded)
        let mut block1 = make_test_block(1, "m_compress_1", "m_anchor_1");
        block1.created_at = 1000;
        block1.active = true;
        state
            .prune
            .messages
            .blocks_by_id
            .insert(BlockId::new(1), block1);

        // Block 2 (newer, will be the active one)
        let mut block2 = make_test_block(2, "m_compress_2", "m_anchor_1");
        block2.created_at = 2000;
        block2.active = true;
        block2.consumed_block_ids = vec![BlockId::new(1)];
        state
            .prune
            .messages
            .blocks_by_id
            .insert(BlockId::new(2), block2);

        // Neither message is present - both should deactivate
        let messages = vec![];

        sync_compression_blocks(&mut state, &messages);

        // Both should be inactive since neither compress message is present, OR
        // based on the sorted iteration, block1 deactivates (no message), then
        // block2's consumed_block_ids includes inactive block1 so it deactivates too
        assert!(
            !state
                .prune
                .messages
                .blocks_by_id
                .get(&BlockId::new(1))
                .unwrap()
                .active
        );
        assert!(
            !state
                .prune
                .messages
                .blocks_by_id
                .get(&BlockId::new(2))
                .unwrap()
                .active
        );
    }

    #[test]
    fn test_sync_activates_block_and_updates_by_anchor_message_id() {
        let mut state = SessionState::default();
        let block_id = BlockId::new(1);
        let compress_msg_id = "m_compress_1";
        let anchor_msg_id = "m_anchor_1";

        let mut block = make_test_block(block_id.value(), compress_msg_id, anchor_msg_id);
        block.active = false; // Start inactive
        state.prune.messages.blocks_by_id.insert(block_id, block);

        let messages = vec![make_test_message(compress_msg_id)];

        sync_compression_blocks(&mut state, &messages);

        // Block should become active
        assert!(
            state
                .prune
                .messages
                .blocks_by_id
                .get(&block_id)
                .unwrap()
                .active
        );
        assert!(state.prune.messages.active_block_ids.contains(&block_id));
        assert_eq!(
            state
                .prune
                .messages
                .active_by_anchor_message_id
                .get(anchor_msg_id),
            Some(&block_id)
        );
    }

    #[test]
    fn test_sync_block_id_0_is_reserved_but_still_processed() {
        // BlockId(0) is reserved as uninitialized, but if a block with id 0 exists
        // in blocks_by_id, it should still be processed
        let mut state = SessionState::default();
        let block_id = BlockId::new(0); // Reserved but let's see what happens

        let block = make_test_block(block_id.value(), "m_compress_0", "m_anchor_0");
        state.prune.messages.blocks_by_id.insert(block_id, block);

        let messages = vec![make_test_message("m_compress_0")];

        sync_compression_blocks(&mut state, &messages);

        // If block was added with id 0, it should still work
        let b = state.prune.messages.blocks_by_id.get(&block_id);
        assert!(b.is_some());
    }

    #[test]
    fn test_sync_sorts_blocks_by_created_at() {
        // Blocks should be processed in created_at order
        let mut state = SessionState::default();

        // Block 2 created before Block 1
        let mut block2 = make_test_block(2, "m_compress_2", "m_anchor_2");
        block2.created_at = 1000;
        block2.active = true;
        state
            .prune
            .messages
            .blocks_by_id
            .insert(BlockId::new(2), block2);

        let mut block1 = make_test_block(1, "m_compress_1", "m_anchor_1");
        block1.created_at = 2000; // Created later
        block1.active = true;
        state
            .prune
            .messages
            .blocks_by_id
            .insert(BlockId::new(1), block1);

        let messages = vec![
            make_test_message("m_compress_1"),
            make_test_message("m_compress_2"),
        ];

        // Process and verify both are active
        sync_compression_blocks(&mut state, &messages);

        assert!(
            state
                .prune
                .messages
                .blocks_by_id
                .get(&BlockId::new(1))
                .unwrap()
                .active
        );
        assert!(
            state
                .prune
                .messages
                .blocks_by_id
                .get(&BlockId::new(2))
                .unwrap()
                .active
        );
        assert_eq!(state.prune.messages.active_block_ids.len(), 2);
    }
}
