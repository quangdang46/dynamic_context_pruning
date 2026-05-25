#![forbid(unsafe_code)]
//! `dcp-tokens` — `Tokenizer` implementations for
//! `dynamic_context_pruning`.
//!
//! - Default (no feature): `Char4Tokenizer` — `char_count / 4` heuristic, zero
//!   dependencies, suitable for budget estimation.
//! - `tokenizers` feature: HuggingFace `tokenizers` backend (universal,
//!   accurate; loads any `tokenizer.json`).
//! - `tiktoken-fast` feature: the `tiktoken` crate (fast OpenAI-style BPE).
//! - `claude-tokens` feature: `claude-tokenizer` crate.
//!
//! Phase 0 scaffold: implementations will land in Phase 1.
