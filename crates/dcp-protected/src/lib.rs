#![forbid(unsafe_code)]
//! `dcp-protected` — protection helpers used by every prune strategy.
//!
//! Provides:
//!
//! - Tool-name protection (e.g. never prune calls to `task` / `skill`).
//! - File-path protection via glob patterns (e.g. always keep
//!   `Cargo.toml`, `**/*.config.ts`).
//! - Tag-based and user-message protection knobs.
//!
//! The matcher is built on `globset` (battle-tested via the `ignore` crate).
//!
//! Phase 0 scaffold: implementations will land in Phase 1.
