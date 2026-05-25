#![forbid(unsafe_code)]
//! `dcp-core` — orchestration crate that glues every other `dcp-*`
//! crate into the public `ContextPruner` facade.
//!
//! Owns:
//!
//! - The `ContextPruner` struct and its `ContextPrunerBuilder`.
//! - The 10-phase `transform_messages` pipeline (PLAN.md §6.4).
//! - `CacheStabilityMode` gating, `pending_prune` state, and
//!   `force_apply` semantics.
//! - The `compress` tool dispatcher and slash-command router.
//! - The `Error` enum surfaced through the umbrella crate.
//!
//! An optional async facade is gated behind the `async` feature.
//!
//! Phase 0 scaffold: the facade itself lands in Phase 5; earlier phases
//! populate the underlying crates.
