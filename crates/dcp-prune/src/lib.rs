#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
//! `dcp-prune` — the three deterministic prune strategies.
//!
//! Strategies (SPEC.md §5):
//!
//! 1. **[`deduplicate`]** — collapse repeat tool calls with the same
//!    `(tool_name, normalized_input)` signature, keeping only the most
//!    recent (§5.1).
//! 2. **[`purge_errors`]** — after `N` turns, drop the *input* of tool
//!    calls whose status is `error`, while keeping the error message
//!    (§5.2).
//! 3. **[`stale_file_reads`]** — within the same file path, keep only
//!    the most recent `read`/`write`/`edit`/`multiedit` output (§5.3).
//!
//! Plus the [`apply::apply_prune_to_messages`] applier that materialises
//! pending decisions into the outgoing message stream while preserving
//! tool call/result pairing (SPEC.md §11.1).
//!
//! # Public surface
//!
//! Each strategy exposes a `run(state, config)` free function that
//! returns a [`dcp_traits::PruneOutcome`]. Callers (typically `dcp-core`)
//! invoke them in fixed order — deduplicate → purge_errors →
//! stale_file_reads — and then run [`apply::apply_prune_to_messages`].
//!
//! # Configuration
//!
//! The strategies read their config through the [`config::PruneConfig`]
//! trait so this crate does not depend on `dcp-config`. A
//! ready-to-construct [`config::StaticPruneConfig`] is provided for
//! tests and short-lived callers.

pub mod apply;
pub mod cache_stability;
pub mod config;
pub mod deduplicate;
pub mod purge_errors;
pub mod stale_file_reads;

pub use apply::{PURGED_INPUT_PLACEHOLDER, PruneKind, apply_prune_to_messages};
pub use cache_stability::CacheStabilityMode;
pub use config::{PruneConfig, StaticPruneConfig};
pub use dcp_traits::PruneOutcome;
