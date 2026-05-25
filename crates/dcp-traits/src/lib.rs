#![forbid(unsafe_code)]
//! `dcp-traits` — pluggable traits the host implements (or accepts the
//! defaults for) when wiring `dynamic_context_pruning` into an agent.
//!
//! The traits exposed here are:
//!
//! - `Tokenizer` — count tokens for a string under the host's chosen encoder.
//! - `StatePersistence` — load/save the per-session persisted state blob.
//! - `MemoryRetriever` — optional cross-session memory lookup hook.
//! - `CacheAccountant` — optional prompt-cache cost observer.
//! - `PruneStrategy` — extension point for custom strategies beyond the
//!   three deterministic ones shipped in `dcp-prune`.
//!
//! This crate has no internal `dcp-*` dependencies other than `dcp-types`,
//! so it sits one level above the leaf in the dependency graph (see
//! PLAN.md §5.2).
//!
//! Phase 0 scaffold: types will be filled in during Phase 1.
