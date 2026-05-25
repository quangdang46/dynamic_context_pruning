#![forbid(unsafe_code)]
//! `dcp-prune` — the three deterministic prune strategies.
//!
//! Strategies (see SPEC.md §5):
//!
//! 1. **Deduplicate** — collapse repeat tool calls with the same
//!    `(tool_name, normalized_input)` signature, keeping only the latest.
//! 2. **Purge errored tool inputs** — after `N` turns, drop the input of
//!    tool calls whose status is `error`, while keeping the error message.
//! 3. **Stale file reads** — within the same file path, keep only the most
//!    recent `read`/`write`/`edit`/`multiedit` output.
//!
//! Plus the prune-to-messages applier that materializes pending decisions
//! into the outgoing message stream while preserving tool call/result
//! pairing.
//!
//! Phase 0 scaffold: algorithms will land in Phase 3.
