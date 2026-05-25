#![forbid(unsafe_code)]
//! `dynamic_context_pruning` — umbrella crate.
//!
//! This is the **only** crate hosts should depend on. It re-exports the
//! curated public API from `dcp-core`, `dcp-types`, `dcp-traits`, and
//! `dcp-config`, hiding the workspace's internal layout.
//!
//! ```ignore
//! use dynamic_context_pruning::{ContextPruner, Config};
//!
//! let mut pruner = ContextPruner::new(Config::load_default()?)?;
//! let pruned = pruner.transform_messages(messages)?;
//! ```
//!
//! See `PLAN.md` §4 for the full public API surface and `SPEC.md` for
//! behavioral requirements. Phase 0 scaffold: the umbrella exposes no
//! items yet; concrete re-exports land alongside the facade in Phase 5.
