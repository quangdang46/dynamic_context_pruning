//! Public input/output types for the `compress` tool — SPEC.md §6.1 / §6.2.

use serde::{Deserialize, Serialize};

/// Top-level compress invocation submitted by the model.
///
/// The `mode` decides whether `content` carries [`RangeEntry`] values
/// (range mode, SPEC §6.1) or [`MessageEntry`] values (message mode,
/// SPEC §6.2).
#[derive(Clone, Debug, PartialEq)]
pub enum CompressArgs {
    /// Range-mode invocation.
    Range {
        /// Batch-level topic.
        topic: String,
        /// Ranges to compress. Each becomes one block.
        content: Vec<RangeEntry>,
    },
    /// Message-mode invocation.
    Message {
        /// Batch-level topic.
        topic: String,
        /// Individual messages to compress.
        content: Vec<MessageEntry>,
    },
}

/// One range in a range-mode compress call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RangeEntry {
    /// Reference of the first message or block in the range (`m####` or `b#`).
    pub start_id: String,
    /// Reference of the last message or block. Must be at-or-after
    /// `start_id` in conversation order.
    pub end_id: String,
    /// Self-contained summary of the range. May contain
    /// `{{block:b#}}` placeholders.
    pub summary: String,
}

/// One entry in a message-mode compress call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageEntry {
    /// Reference of the message to compress (`m####`).
    pub message_id: String,
    /// Per-entry topic (used as the in-block heading).
    pub topic: String,
    /// Self-contained summary.
    pub summary: String,
}

/// One block reported in [`CompressResult::blocks`] (the
/// `NotificationEntry` from SPEC §6.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotificationEntry {
    /// Newly allocated block id.
    pub block_id: u32,
    /// Run id shared by every block produced by this invocation.
    pub run_id: u32,
    /// Wrapped summary stored on the block.
    pub summary: String,
    /// Token count of the wrapped summary.
    pub summary_tokens: u64,
}

/// Result returned by [`crate::handle_compress`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompressResult {
    /// Total count of `direct_message_ids` across every new block.
    pub compressed_messages: usize,
    /// One entry per newly committed block.
    pub blocks: Vec<NotificationEntry>,
}
