#![forbid(unsafe_code)]
//! `dcp-types` — canonical internal representation (IR) for the
//! `dynamic_context_pruning` library.
//!
//! This crate is the leaf of the workspace dependency graph. It defines the
//! format-agnostic types (`Message`, `Part`, `Role`, `BlockId`, `RunId`,
//! `MessageRef`, `CompressionBlock`, `SessionState`, `Stats`, `Telemetry`,
//! ...) that every other `dcp-*` crate consumes. Provider-specific message
//! formats (Anthropic, OpenAI, Gemini, Bedrock) are converted in and out of
//! these types by host-side adapters.
//!
//! Phase 0 scaffold: this module is intentionally empty. Concrete types will
//! land in Phase 1 (see PLAN.md §10 Phase 1 and SPEC.md §2).
