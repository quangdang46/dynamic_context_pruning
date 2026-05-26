#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
//! `dcp-core` — orchestration crate that glues every other `dcp-*`
//! crate into the public [`ContextPruner`] facade.
//!
//! Owns:
//!
//! * The [`ContextPruner`] struct and its [`ContextPrunerBuilder`].
//! * The 10-phase [`ContextPruner::transform_messages`] pipeline
//!   (PLAN.md §6.4 / SPEC.md §5.4).
//! * `cache_stability_mode` gating, `pending_prune` state, and
//!   `force_apply` semantics.
//! * The compress tool dispatcher (range + message mode) and the
//!   `/dcp …` slash-command router ([`commands`]).
//! * The [`Error`] enum surfaced through the umbrella crate.
//!
//! An optional async facade ([`ContextPrunerAsync`]) is gated behind
//! the `async` feature.
//!
//! # Quick start
//!
//! ```rust
//! use dcp_core::{ContextPruner, Config, Message};
//!
//! let mut pruner = ContextPruner::new(Config::default()).unwrap();
//! let messages = vec![
//!     Message::user_text("u1", 0, "hello"),
//!     Message::assistant_text("a1", 0, "hi there"),
//! ];
//! let pruned = pruner.transform_messages(messages).unwrap();
//! assert!(!pruned.is_empty());
//! ```

pub mod commands;
pub mod error;
pub(crate) mod pipeline;
pub mod pruner;
pub mod tokenizer;

pub(crate) mod strip;
#[cfg(feature = "async")]
pub mod async_facade;

// ─────────────────────────────────────────────────────────────────────────
// Public surface
// ─────────────────────────────────────────────────────────────────────────

pub use commands::CommandOutcome;
pub use error::Error;
pub use pipeline::ToolSchema;
pub use pruner::{ContextPruner, ContextPrunerBuilder, DecompressResult, RecompressResult};
pub use tokenizer::Char4Tokenizer;

#[cfg(feature = "async")]
pub use async_facade::ContextPrunerAsync;

// Re-export the canonical types every `ContextPruner` user reaches for,
// so the umbrella crate can `pub use dcp_core::*;` and end up with the
// PLAN.md §4.4 surface.
pub use dcp_compress::{
    CompressArgs, CompressError, CompressResult, MessageEntry, NotificationEntry, RangeEntry,
};
pub use dcp_config::{
    CacheStabilityMode, CommandsConfig, CompressMode, Config, ConfigError, DeduplicationConfig,
    ExperimentalConfig, InjectionMode, LimitValue, ManualModeConfig, NotificationConfig,
    NotificationKind, NotificationLevel, NudgeForce, Permission, PurgeErrorsConfig,
    StaleFileReadsConfig, StrategiesConfig, TurnProtectionConfig,
};
pub use dcp_nudges::NudgeKind;
pub use dcp_prompts::{PromptError, PromptStore, Prompts};
pub use dcp_telemetry::{Event, EventKind, Observer, Telemetry, TelemetrySnapshot};
pub use dcp_traits::{
    CacheAccountant, CacheEvent, MemoryRetriever, PersistedState, PersistedStateV1,
    PersistenceError, PruneError, PruneOutcome, PruneStrategy, RetrievalError, RetrievedMemory,
    StatePersistence, Tokenizer,
};
pub use dcp_traits::defaults::{NoopMemoryRetriever, NoopStorage};
pub use dcp_storage::{FileStateStore, InMemoryStateStore, default_storage_dir};
pub use dcp_types::{
    BlockId, CompressionBlock, CompressionMode, CompressPermission, ManualMode, Message,
    MessageRef, MessageRefKind, MessageRefParseError, Part, Role, RunId, SessionState, Stats,
    ToolStatus,
};
