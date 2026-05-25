#![forbid(unsafe_code)]
//! `dcp-compress` — LLM-driven compression machinery.
//!
//! Provides:
//!
//! - The `compress` tool schema (range and message modes; SPEC.md §6).
//! - Range/message-mode handlers that allocate `block_id` / `run_id`,
//!   write summaries into anchor messages, and update `SessionState`.
//! - Block bookkeeping: `included_block_ids`, `consumed_block_ids`,
//!   `parent_block_ids`, `effective_message_ids`, plus the
//!   active/deactivated lifecycle.
//! - Nesting and consumption rules when a new range covers an older
//!   block.
//! - The frontier mechanism that prevents repeatedly compressing a
//!   range whose summary turned out to be larger than the raw content.
//!
//! Phase 0 scaffold: implementations will land in Phase 4.
