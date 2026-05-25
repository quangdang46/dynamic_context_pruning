#![forbid(unsafe_code)]
//! `dcp-prompts` — the 6 default prompts plus an override loader.
//!
//! Prompts cover:
//!
//! - The `compress` tool description (range mode).
//! - The `compress` tool description (message mode).
//! - System-prompt addendum installed via `transform_system`.
//! - Context-limit, turn, and iteration nudges.
//!
//! All prompts are embedded at compile time; the host may override any of
//! them via the configured `Prompts` struct.
//!
//! Phase 0 scaffold: prompt content will be embedded in Phase 4.
