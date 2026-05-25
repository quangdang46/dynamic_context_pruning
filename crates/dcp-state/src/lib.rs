#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
//! `dcp-state` — owns the in-memory [`SessionState`] and its transitions.
//!
//! This crate is the *operations* layer for the canonical state shape
//! defined in [`dcp_types`]. Every function is total (no panics on legal
//! inputs) and deterministic — given the same `(messages, persisted
//! blocks, config)` triple, the same `SessionState` comes out, which is
//! what makes [`session::rebuild_from_messages`] sound (SPEC.md §11.4).
//!
//! # Public surface
//!
//! Lifecycle (SPEC.md §3):
//!
//! * [`create_session_state`] — construct a fresh, empty state.
//! * [`reset_session_state`] — replace every field with its default.
//! * [`check_session`] — react to a session-id change by reinitialising.
//! * [`ensure_session_initialized`] — load persisted state and restore
//!   prune bookkeeping.
//! * [`find_last_compaction_timestamp`] — derive the most recent
//!   compaction marker from the message stream.
//! * [`count_turns`] — count turn-ends per SPEC §3.2.
//! * [`reset_on_compaction`] — apply SPEC §3.3 mutations once a
//!   compaction event has been detected.
//! * [`rebuild_from_messages`] — KEY idempotent rebuild (PLAN §7.4).
//! * [`get_active_summary_token_usage`] — sum of `summary_tokens` across
//!   currently-active blocks.
//!
//! Tool tracking (SPEC.md §4):
//!
//! * [`tool_cache::sync_tool_cache`] — populate `tool_parameters` and
//!   `tool_id_list` from a message stream.
//!
//! Message-reference allocation (SPEC.md §2.4):
//!
//! * [`message_refs::assign_message_refs`] — allocate `m####` references
//!   in first-seen order.
//!
//! Nudges (SPEC.md §8.2):
//!
//! * [`nudges::collect_turn_nudge_anchors`] — set of assistant message
//!   ids eligible for a turn nudge.
//!
//! # Re-exports
//!
//! Types from [`dcp_types`] used in this crate's public API are re-exported
//! for caller convenience. The re-exports are not new types — they are
//! the same items from [`dcp_types`].
//!
//! # Decoupling from `dcp-config`
//!
//! Per PLAN.md §5.2, `dcp-state` must not depend on `dcp-config`. The
//! [`config_like::ConfigLike`] trait abstracts the slice of configuration
//! the state operations consume. `dcp-config` will provide a blanket
//! implementation when it lands.

pub mod config_like;
pub mod message_refs;
pub mod nudges;
pub mod session;
pub mod tool_cache;

pub use config_like::{ConfigLike, StaticConfigLike, default_tracked_tools};
pub use message_refs::assign_message_refs;
pub use nudges::collect_turn_nudge_anchors;
pub use session::{
    EnsureInitError, check_session, count_turns, count_turns_through, create_session_state,
    ensure_session_initialized, find_last_compaction_timestamp, get_active_summary_token_usage,
    rebuild_from_messages, reset_on_compaction, reset_on_compaction_at, reset_session_state,
};
pub use tool_cache::sync_tool_cache;

// Re-export the canonical types every consumer of this crate also needs,
// so they don't have to depend on `dcp-types` separately.
pub use dcp_types::{
    BlockId, CompressionBlock, CompressionMode, Message, MessageRef, Nudges, Part, Prune,
    PruneMessagesState, Role, RunId, SessionState, Stats, Telemetry, ToolParameterEntry,
    ToolStatus,
};
