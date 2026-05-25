//! Errors returned by the compress pipeline — SPEC.md §6.1 validation.

use thiserror::Error;

/// Compress-pipeline errors.
///
/// Marked `#[non_exhaustive]` so additional variants can be added in
/// future minor versions without breaking downstream `match`.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompressError {
    /// `topic` was empty or whitespace-only, or a per-entry field was
    /// malformed (missing/wrong-type).
    #[error("invalid compress args: {0}")]
    InvalidCompressArgs(String),
    /// A reference (`m####` / `b#`) did not resolve to anything known.
    #[error("invalid compress args: unknown ref: {0}")]
    UnknownRef(String),
    /// Two ranges in the same call cover the same non-anchor message id.
    #[error("range overlap: {0}")]
    RangeOverlap(String),
    /// A `{{block:b#}}` placeholder mentioned a block id that was not in
    /// the entry's `required_block_ids`, or referenced a non-existent
    /// block.
    #[error("placeholder mismatch: {0}")]
    PlaceholderMismatch(String),
    /// The model attempted to compress a message that is already inside
    /// an active block (message mode).
    #[error("message {0} is already inside an active block")]
    MessageAlreadyCompressed(String),
}
