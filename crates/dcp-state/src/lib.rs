#![forbid(unsafe_code)]
//! `dcp-state` — owns the in-memory `SessionState` and its transitions.
//!
//! Responsibilities:
//!
//! - Apply prune decisions and compression-block bookkeeping to state.
//! - Allocate stable `m####` message references and `b#` block references.
//! - Provide an idempotent rebuild path so that
//!   `rebuild(messages, persisted_blocks)` reproduces the same pruning
//!   decisions a fresh run would have made (see SPEC.md §11).
//! - Track tool-call signatures, turn boundaries, and pending-prune state
//!   used by cache-stability mode.
//!
//! Phase 0 scaffold: types will land in Phase 2.
