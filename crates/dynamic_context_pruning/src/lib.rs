#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
//! `dynamic_context_pruning` — umbrella crate for the
//! [DCP](https://github.com/quangdang46/dynamic_context_pruning) library.
//!
//! This is the **only** crate hosts depend on. It re-exports the curated
//! public API from every internal `dcp-*` crate so the workspace's
//! layout stays an implementation detail.
//!
//! See `PLAN.md` §4 for the full public API surface and `SPEC.md` for
//! behavioural requirements.
//!
//! # Quick start
//!
//! ```rust
//! use std::sync::Arc;
//! use dynamic_context_pruning::{ContextPruner, Config, Message};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // 1. Construct a pruner with bundled defaults.
//! let mut pruner = ContextPruner::new(Config::default())?;
//!
//! // 2. Transform messages before sending them to the LLM.
//! let messages = vec![
//!     Message::user_text("u1", 0, "hello"),
//!     Message::assistant_text("a1", 0, "hi there"),
//! ];
//! let pruned = pruner.transform_messages(messages)?;
//! assert!(!pruned.is_empty());
//!
//! // 3. Append the system-prompt addendum.
//! let mut system = String::from("You are a helpful assistant.");
//! pruner.transform_system(&mut system);
//! assert!(system.contains("Context-pruning support"));
//! # let _ = Arc::new(0u8);
//! # Ok(())
//! # }
//! ```
//!
//! # Full builder
//!
//! ```rust
//! use std::sync::Arc;
//! use dynamic_context_pruning::{
//!     ContextPruner, Config, default_tokenizer, FileStateStore,
//!     default_storage_dir,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let pruner = ContextPruner::builder()
//!     .config(Config::default())
//!     .tokenizer(default_tokenizer())
//!     .storage(Arc::new(FileStateStore::new(default_storage_dir())))
//!     .build()?;
//! let _ = pruner;
//! # Ok(())
//! # }
//! ```
//!
//! # Module map
//!
//! Re-exports are grouped by source crate below; each section comments
//! map directly onto PLAN.md §4.4.

// ─────────────────────────────────────────────────────────────────────────
// dcp-types — canonical IR (PLAN.md §3.6, §4.4)
// ─────────────────────────────────────────────────────────────────────────

pub use dcp_types::{
    BlockId, CompressionBlock, CompressionMode, Message, MessageRef, MessageRefKind,
    MessageRefParseError, Part, Role, RunId, SessionState, Stats, Telemetry as TypesTelemetry,
    ToolStatus,
};

// ─────────────────────────────────────────────────────────────────────────
// dcp-config — Config + sub-configs + enums (PLAN.md §4.4, §8)
// ─────────────────────────────────────────────────────────────────────────

pub use dcp_config::{
    CacheStabilityMode, CommandsConfig, CompressConfig, CompressMode, Config, ConfigError,
    DeduplicationConfig, ExperimentalConfig, InjectionMode, LimitValue, ManualModeConfig,
    NotificationConfig, NotificationKind, NotificationLevel, NudgeForce, Permission,
    PurgeErrorsConfig, StaleFileReadsConfig, StrategiesConfig, TurnProtectionConfig,
};

// ─────────────────────────────────────────────────────────────────────────
// dcp-traits — pluggable host interfaces (PLAN.md §3.4, §4.4)
// ─────────────────────────────────────────────────────────────────────────

pub use dcp_traits::{
    CacheAccountant, CacheEvent, MemoryRetriever, PersistedState, PersistedStateV1,
    PersistenceError, PruneError, PruneOutcome, PruneStrategy, RetrievalError, RetrievedMemory,
    StatePersistence, Tokenizer,
};

/// Default trait implementations bundled with `dcp-traits`
/// (`NoopMemoryRetriever`, `NoopStorage`).
pub use dcp_traits::defaults;

// ─────────────────────────────────────────────────────────────────────────
// dcp-core — facade (PLAN.md §4.2, §4.3, §4.5)
// ─────────────────────────────────────────────────────────────────────────

pub use dcp_core::{
    CommandOutcome, ContextPruner, ContextPrunerBuilder, DecompressResult, Error, RecompressResult,
    ToolSchema,
};

// dcp-compress lives behind dcp-core; re-export the args / result shapes
// so the umbrella exposes the full PLAN.md §4.2 surface without forcing
// users to depend on `dcp-compress` directly.
pub use dcp_core::{CompressArgs, CompressResult, MessageEntry, NotificationEntry, RangeEntry};

// Async facade ships behind the `async` feature (PLAN.md §3.3 / §10).
#[cfg(feature = "async")]
pub use dcp_core::ContextPrunerAsync;

// ─────────────────────────────────────────────────────────────────────────
// dcp-tokens — tokenizer backends (PLAN.md §2.5, §4.4)
// ─────────────────────────────────────────────────────────────────────────

pub use dcp_tokens::{default_tokenizer, Char4Tokenizer};

// Feature-gated re-exports that match the umbrella's optional features.
#[cfg(feature = "claude")]
pub use dcp_tokens::ClaudeTokenizer;
#[cfg(feature = "tokenizers")]
pub use dcp_tokens::HuggingFaceTokenizer;
#[cfg(feature = "tiktoken")]
pub use dcp_tokens::TiktokenTokenizer;

// ─────────────────────────────────────────────────────────────────────────
// dcp-storage — persistence backends (PLAN.md §4.4, §7.3)
// ─────────────────────────────────────────────────────────────────────────

pub use dcp_storage::{default_storage_dir, FileStateStore, InMemoryStateStore};

// ─────────────────────────────────────────────────────────────────────────
// dcp-prompts — bundled prompts + override loader (PLAN.md §4.4)
// ─────────────────────────────────────────────────────────────────────────

pub use dcp_prompts::{PromptError, PromptStore, Prompts};

// ─────────────────────────────────────────────────────────────────────────
// dcp-telemetry — events, observers, accountants (PLAN.md §4.4)
// ─────────────────────────────────────────────────────────────────────────

pub use dcp_telemetry::{
    DefaultCacheAccountant, Event, EventKind, InMemoryObserver, Observer, Telemetry,
    TelemetrySnapshot,
};
