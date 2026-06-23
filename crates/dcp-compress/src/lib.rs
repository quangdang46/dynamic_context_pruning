#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
//! `dcp-compress` — LLM-driven compression machinery (SPEC.md §6).
//!
//! Provides the `compress` tool's range and message modes, block
//! bookkeeping (`commit_block`, `effective_message_ids`,
//! `effective_tool_ids`), the `filter_compressed_ranges` apply step,
//! and the frontier mechanism that suppresses repeated nudges for
//! oversized compressions.
//!
//! # Public surface
//!
//! * [`handle_compress`] — top-level entry point used by the host's
//!   tool layer.
//! * [`compress_and_apply`] — convenience wrapper that runs
//!   [`handle_compress`] followed by [`filter_compressed_ranges`].
//! * [`filter_compressed_ranges`] — apply step (SPEC §6.4).
//! * [`commit_block`] / [`maybe_advance_frontier`] — fine-grained block
//!   bookkeeping for callers that have built their own pipelines.
//! * Type re-exports: [`CompressArgs`], [`CompressResult`],
//!   [`RangeEntry`], [`MessageEntry`], [`NotificationEntry`],
//!   [`CompressError`], [`CompressConfig`], [`StaticCompressConfig`].
//!
//! # Decoupling
//!
//! Per PLAN.md §5.2 this crate does not depend on `dcp-config`. The
//! [`config::CompressConfig`] trait abstracts the slice it needs.

pub mod block;
pub mod config;
pub mod error;
pub mod filter;
pub mod handler;
pub mod placeholder;
pub mod resolve;
pub mod timing;
pub mod types;
pub mod validate;
pub mod wrap;

pub use block::{commit_block, maybe_advance_frontier};
pub use config::{CompressConfig, DEFAULT_MAX_SUMMARY_CHARS, StaticCompressConfig};
pub use error::CompressError;
pub use filter::filter_compressed_ranges;
pub use handler::{compress_and_apply, handle_compress};
pub use placeholder::{
    append_missing_block_summaries, inject_placeholder_expansions, parse_placeholders,
    validate_placeholders,
};
pub use resolve::{ResolvedRange, resolve_range};
pub use timing::{build_compression_timing_key, resolve_compression_duration};
pub use types::{CompressArgs, CompressResult, MessageEntry, NotificationEntry, RangeEntry};
pub use validate::{validate_non_overlapping, validate_topic_and_content};
pub use wrap::{
    PROTECTED_USER_TRUNCATE_BYTES, append_protected_tag_content, append_protected_tool_outputs,
    append_protected_user_messages, compute_effective, compute_included,
    extract_protected_tag_sections, estimate_compressed_tokens, estimate_summary_tokens,
    maybe_buffer_summary, wrap_compressed_summary,
};
