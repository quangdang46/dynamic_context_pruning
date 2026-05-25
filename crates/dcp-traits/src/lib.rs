#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! `dcp-traits` — pluggable traits the host implements (or accepts the
//! defaults for) when wiring `dynamic_context_pruning` into an agent.
//!
//! The traits exposed here are:
//!
//! - [`Tokenizer`] — count tokens for a string under the host's chosen
//!   encoder.
//! - [`StatePersistence`] — load/save the per-session [`PersistedState`]
//!   blob.
//! - [`MemoryRetriever`] — optional cross-session memory lookup hook.
//! - [`CacheAccountant`] — optional prompt-cache cost observer.
//! - [`PruneStrategy`] — extension point for custom strategies beyond the
//!   three deterministic ones shipped in `dcp-prune`.
//!
//! The crate also defines the versioned [`PersistedState`] schema used by
//! every storage backend (per SPEC.md §9.1) and the corresponding error
//! types (`PersistenceError`, `RetrievalError`, `PruneError`).
//!
//! Per PLAN.md §5.2, this crate sits one level above `dcp-types` in the
//! dependency graph and is depended on by every implementation crate.
//!
//! # Sync-only (decision D2)
//!
//! Phase 1 is sync-only. `async-trait` is intentionally not a dependency; an
//! async facade arrives later via the `async` feature on `dcp-core` (see
//! PLAN.md §3.3 / §10 Phase 5).

use std::collections::BTreeMap;
use std::sync::Mutex;

use dcp_types::{Message, SessionState};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────────
// Tokenizer
// ─────────────────────────────────────────────────────────────────────────

/// Counts tokens for arbitrary text under a host-chosen encoder.
///
/// Implementations are expected to be fast and side-effect-free; the
/// pruning pipeline calls `count` many times per transform.
pub trait Tokenizer: Send + Sync {
    /// Return the number of tokens in `text` under this tokenizer's
    /// encoding scheme.
    fn count(&self, text: &str) -> usize;

    /// Convenience: token count for a batch of strings. The default
    /// implementation simply sums the per-string [`count`](Self::count)
    /// values; backends with batch-friendly encoders may override for
    /// performance.
    fn count_batch(&self, texts: &[&str]) -> usize {
        texts.iter().map(|t| self.count(t)).sum()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// StatePersistence + PersistedState schema
// ─────────────────────────────────────────────────────────────────────────

/// Errors returned by [`StatePersistence`] backends.
#[derive(Debug, Error)]
pub enum PersistenceError {
    /// Underlying I/O failure.
    #[error("persistence i/o error: {0}")]
    Io(String),
    /// Serialization or deserialization failure.
    #[error("persistence serde error: {0}")]
    Serde(String),
    /// The on-disk schema version is newer than this library understands
    /// (per SPEC.md §9.2).
    #[error("unsupported schema version: {0}")]
    UnsupportedSchema(String),
    /// Migration from an older schema failed.
    #[error("schema migration failed: {0}")]
    MigrationFailed(String),
    /// Generic backend-specific error.
    #[error("persistence backend error: {0}")]
    Backend(String),
}

/// Pluggable storage backend for [`PersistedState`] blobs, keyed by
/// `session_id`. SPEC.md §9 describes the on-disk format and the atomic
/// write protocol used by the bundled file backend.
pub trait StatePersistence: Send + Sync {
    /// Load the persisted state for `session_id`. `Ok(None)` means the
    /// session has never been saved; an error is reserved for actual
    /// failures (I/O, corruption, schema mismatch).
    fn load(&self, session_id: &str) -> Result<Option<PersistedState>, PersistenceError>;

    /// Save the persisted state for `session_id`. Implementations should
    /// be atomic (write-temp-then-rename for file backends).
    fn save(&self, session_id: &str, state: &PersistedState) -> Result<(), PersistenceError>;

    /// List every `session_id` currently held by the backend.
    fn list_sessions(&self) -> Result<Vec<String>, PersistenceError>;

    /// Remove the persisted state for `session_id`. Idempotent: deleting a
    /// non-existent session is a no-op.
    fn delete(&self, session_id: &str) -> Result<(), PersistenceError>;
}

/// Versioned, serde-tagged persisted state envelope.
///
/// Serialization uses `#[serde(tag = "schema_version")]`, so a V1 document
/// looks like:
///
/// ```jsonc
/// {
///     "schema_version": "1",
///     "session_id": "...",
///     "current_turn": 0,
///     // ... rest of PersistedStateV1 fields
/// }
/// ```
///
/// New variants (`V2`, `V3`, …) are added as the schema evolves; the
/// loader runs migrations to bring older documents up to the current
/// variant (SPEC.md §9.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "schema_version")]
pub enum PersistedState {
    /// Schema version 1 — the initial release format.
    #[serde(rename = "1")]
    V1(PersistedStateV1),
}

/// Concrete schema-V1 payload. Mirrors SPEC.md §9.1.
///
/// The well-known scalar fields are typed; the complex nested objects
/// (`stats`, `nudges`, `prune`, `tool_index`, `message_id_map`,
/// `compaction`) are currently held as `serde_json::Value` so that this
/// crate does not pull in the full canonical IR before `dcp-types` and
/// `dcp-state` land. The on-disk JSON shape is unchanged when those
/// fields are later retyped.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistedStateV1 {
    /// Optional human-readable name for the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    /// Opaque session id (key into the storage backend).
    pub session_id: String,
    /// RFC3339 timestamp of the last successful save.
    pub last_updated: String,
    /// Number of completed turns observed so far.
    pub current_turn: u32,
    /// Latest message-reference (`m####`) currently treated as the
    /// pruning frontier, or `None` for a fresh session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontier_message_ref: Option<String>,
    /// Next available compression-block id.
    pub next_block_id: u32,
    /// Next available compression-run id.
    pub next_run_id: u32,
    /// Next available message-reference numeric component (`m####`).
    pub next_message_ref: u32,
    /// Cumulative session statistics. Shape will be retyped to
    /// `dcp_types::Stats` when that type lands.
    #[serde(default)]
    pub stats: serde_json::Value,
    /// Nudge bookkeeping (`context_limit_counter`, `turn_nudged_pairs`).
    #[serde(default)]
    pub nudges: serde_json::Value,
    /// Prune dictionaries: `tools` and `messages.{blocks, active_block_ids}`.
    #[serde(default)]
    pub prune: serde_json::Value,
    /// Per-call_id tool-tracking entries (SPEC.md §4.1).
    #[serde(default)]
    pub tool_index: serde_json::Value,
    /// Bidirectional map between raw host ids and library `m####` refs.
    #[serde(default)]
    pub message_id_map: serde_json::Value,
    /// Compaction observation timestamps and counter.
    #[serde(default)]
    pub compaction: serde_json::Value,
}

// ─────────────────────────────────────────────────────────────────────────
// MemoryRetriever
// ─────────────────────────────────────────────────────────────────────────

/// Errors returned by [`MemoryRetriever`] implementations.
#[derive(Debug, Error)]
pub enum RetrievalError {
    /// The query was malformed or unsupported.
    #[error("invalid retrieval query: {0}")]
    InvalidQuery(String),
    /// Generic backend failure (e.g. vector store unreachable).
    #[error("retrieval backend error: {0}")]
    Backend(String),
}

/// One result row returned by [`MemoryRetriever::retrieve`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievedMemory {
    /// Memory text (already detokenized / human-readable).
    pub content: String,
    /// Backend-specific relevance score; higher = more relevant. Callers
    /// must not assume any particular range or distribution.
    pub score: f32,
    /// Optional provenance label (e.g. document id, file path).
    pub source: Option<String>,
}

/// Optional cross-session memory lookup hook.
///
/// The library calls this to enrich pruned/compressed contexts with
/// relevant prior knowledge, when the host opts in. A no-op default is
/// provided in [`defaults::NoopMemoryRetriever`].
pub trait MemoryRetriever: Send + Sync {
    /// Return up to `k` memories ranked by relevance to `query`.
    /// Implementations must respect `k` as an upper bound but may return
    /// fewer rows.
    fn retrieve(&self, query: &str, k: usize) -> Result<Vec<RetrievedMemory>, RetrievalError>;
}

// ─────────────────────────────────────────────────────────────────────────
// CacheAccountant
// ─────────────────────────────────────────────────────────────────────────

/// Prompt-cache lifecycle event observed by a [`CacheAccountant`].
///
/// `#[non_exhaustive]` so future variants (e.g. a `Renewed` event) can be
/// added without breaking downstream matches.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CacheEvent {
    /// The provider reported a cache hit covering `tokens` tokens.
    Hit {
        /// Number of cached tokens served by the provider.
        tokens: u64,
    },
    /// The provider reported a cache miss covering `tokens` tokens.
    Miss {
        /// Number of tokens that were re-encoded due to the miss.
        tokens: u64,
    },
    /// The cache was busted — typically because the prefix changed in a
    /// way that invalidated existing entries.
    Bust {
        /// Free-form reason string (e.g. `"prompt_changed"`,
        /// `"system_prompt_diff"`, `"tools_changed"`).
        reason: String,
    },
}

/// Pluggable observer for prompt-cache cost accounting.
///
/// Implementations let the host compare hit/miss patterns under different
/// cache-stability modes (SPEC.md §7.2) without tying the library itself to
/// a specific provider's pricing.
pub trait CacheAccountant: Send + Sync {
    /// Estimated cost (in arbitrary units, typically dollars) for
    /// re-encoding `tokens` tokens that missed the cache.
    fn cost_per_cache_miss_tokens(&self, tokens: usize) -> f64;

    /// Record an observed cache lifecycle event.
    fn record_event(&mut self, event: CacheEvent);
}

// ─────────────────────────────────────────────────────────────────────────
// PruneStrategy
// ─────────────────────────────────────────────────────────────────────────

/// Errors returned by [`PruneStrategy::apply`].
#[derive(Debug, Error)]
pub enum PruneError {
    /// The session state was not in a shape the strategy could handle
    /// (e.g. missing required fields).
    #[error("invalid state for strategy: {0}")]
    InvalidState(String),
    /// The strategy encountered an unrecoverable internal error.
    #[error("strategy internal error: {0}")]
    Internal(String),
    /// The host configuration was malformed for this strategy.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Result of a single [`PruneStrategy::apply`] invocation.
///
/// Mirrors SPEC.md §5 (the per-strategy `PruneOutcome` table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PruneOutcome {
    /// Strategy name (e.g. `"deduplicate"`, `"purge_errors"`).
    pub strategy: String,
    /// Ids (call_ids, message refs, etc.) newly marked for pruning by
    /// this run.
    pub pruned_ids: Vec<String>,
    /// Sum of `token_count` across the newly pruned entries.
    pub tokens_saved: u64,
    /// Set when the strategy declined to run; `None` when it executed.
    /// Examples: `"disabled"`, `"manual_mode"`, `"already_pruned"`.
    pub reason_skipped: Option<String>,
}

impl PruneOutcome {
    /// Convenience constructor for the "strategy did not run" case.
    pub fn skipped(strategy: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            strategy: strategy.into(),
            pruned_ids: Vec::new(),
            tokens_saved: 0,
            reason_skipped: Some(reason.into()),
        }
    }
}

/// Extension point for custom pruning strategies beyond the three
/// deterministic ones shipped in `dcp-prune`.
///
/// # Type parameter
///
/// The strategy is generic over its `Config` type `C`. This avoids a
/// circular dependency: the canonical `Config` lives in `dcp-config`,
/// which itself depends on `dcp-traits` (PLAN.md §5.2). Concrete
/// implementations in `dcp-prune` instantiate `C = dcp_config::Config`;
/// custom host strategies can pick whatever config type they prefer.
///
/// `?Sized` is allowed so trait objects (`&dyn ConfigTrait`) work as the
/// config type.
pub trait PruneStrategy<C: ?Sized>: Send + Sync {
    /// Stable identifier used in telemetry, logs, and `PruneOutcome`.
    fn name(&self) -> &str;

    /// Apply the strategy. Implementations may mutate `state` (e.g. add
    /// entries to `state.prune.tools`) but must preserve the invariants
    /// documented per strategy in SPEC.md §5.
    fn apply(
        &self,
        state: &mut SessionState,
        messages: &[Message],
        config: &C,
    ) -> Result<PruneOutcome, PruneError>;
}

// ─────────────────────────────────────────────────────────────────────────
// defaults — minimal, dependency-free implementations for tests and
// "turn the feature off" host wiring.
// ─────────────────────────────────────────────────────────────────────────

/// Minimal, dependency-free trait implementations useful in tests and as
/// the "feature off" wiring for hosts that do not need persistence or
/// memory retrieval.
pub mod defaults {
    use super::{
        BTreeMap, MemoryRetriever, Mutex, PersistedState, PersistenceError, RetrievalError,
        RetrievedMemory, StatePersistence,
    };

    /// In-memory [`StatePersistence`] backend.
    ///
    /// State is kept in a `BTreeMap` behind a `Mutex`, so a single
    /// `NoopStorage` instance can be shared across threads. Process exit
    /// erases all sessions — this backend is intended for tests and for
    /// hosts that explicitly opt out of disk persistence.
    #[derive(Debug)]
    pub struct NoopStorage {
        sessions: Mutex<BTreeMap<String, PersistedState>>,
    }

    impl NoopStorage {
        /// Create an empty in-memory store.
        pub fn new() -> Self {
            Self {
                sessions: Mutex::new(BTreeMap::new()),
            }
        }
    }

    impl Default for NoopStorage {
        fn default() -> Self {
            Self::new()
        }
    }

    impl StatePersistence for NoopStorage {
        fn load(&self, session_id: &str) -> Result<Option<PersistedState>, PersistenceError> {
            let guard = self
                .sessions
                .lock()
                .map_err(|e| PersistenceError::Backend(format!("mutex poisoned: {e}")))?;
            Ok(guard.get(session_id).cloned())
        }

        fn save(&self, session_id: &str, state: &PersistedState) -> Result<(), PersistenceError> {
            let mut guard = self
                .sessions
                .lock()
                .map_err(|e| PersistenceError::Backend(format!("mutex poisoned: {e}")))?;
            guard.insert(session_id.to_string(), state.clone());
            Ok(())
        }

        fn list_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            let guard = self
                .sessions
                .lock()
                .map_err(|e| PersistenceError::Backend(format!("mutex poisoned: {e}")))?;
            Ok(guard.keys().cloned().collect())
        }

        fn delete(&self, session_id: &str) -> Result<(), PersistenceError> {
            let mut guard = self
                .sessions
                .lock()
                .map_err(|e| PersistenceError::Backend(format!("mutex poisoned: {e}")))?;
            guard.remove(session_id);
            Ok(())
        }
    }

    /// [`MemoryRetriever`] that always returns an empty result set.
    ///
    /// Use this when the host does not wire a real retrieval backend; the
    /// rest of the pipeline still type-checks.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct NoopMemoryRetriever;

    impl MemoryRetriever for NoopMemoryRetriever {
        fn retrieve(
            &self,
            _query: &str,
            _k: usize,
        ) -> Result<Vec<RetrievedMemory>, RetrievalError> {
            Ok(Vec::new())
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use defaults::{NoopMemoryRetriever, NoopStorage};

    /// A char-count tokenizer used to verify the default `count_batch`.
    struct CharTokenizer;
    impl Tokenizer for CharTokenizer {
        fn count(&self, text: &str) -> usize {
            text.chars().count()
        }
    }

    fn sample_state(session_id: &str) -> PersistedState {
        PersistedState::V1(PersistedStateV1 {
            session_name: Some("test session".into()),
            session_id: session_id.into(),
            last_updated: "2024-01-01T00:00:00Z".into(),
            current_turn: 0,
            frontier_message_ref: None,
            next_block_id: 1,
            next_run_id: 1,
            next_message_ref: 1,
            stats: serde_json::Value::Null,
            nudges: serde_json::Value::Null,
            prune: serde_json::Value::Null,
            tool_index: serde_json::Value::Null,
            message_id_map: serde_json::Value::Null,
            compaction: serde_json::Value::Null,
        })
    }

    #[test]
    fn tokenizer_count_batch_default_sums_per_string_counts() {
        let tok = CharTokenizer;
        assert_eq!(tok.count_batch(&[]), 0);
        assert_eq!(tok.count_batch(&["abc"]), 3);
        assert_eq!(tok.count_batch(&["abc", "de", ""]), 5);
    }

    #[test]
    fn noop_storage_load_save_roundtrip() {
        let store = NoopStorage::new();
        let state = sample_state("s1");

        // initially empty
        assert!(store.load("s1").unwrap().is_none());

        // save -> load
        store.save("s1", &state).unwrap();
        assert_eq!(store.load("s1").unwrap(), Some(state.clone()));

        // overwrite
        let mut state2 = state.clone();
        let PersistedState::V1(ref mut v) = state2;
        v.current_turn = 7;
        store.save("s1", &state2).unwrap();
        assert_eq!(store.load("s1").unwrap(), Some(state2));
    }

    #[test]
    fn noop_storage_list_sessions_returns_all_keys() {
        let store = NoopStorage::new();
        store.save("alpha", &sample_state("alpha")).unwrap();
        store.save("beta", &sample_state("beta")).unwrap();

        let mut sessions = store.list_sessions().unwrap();
        sessions.sort();
        assert_eq!(sessions, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn noop_storage_delete_is_idempotent() {
        let store = NoopStorage::new();
        store.save("x", &sample_state("x")).unwrap();
        assert!(store.load("x").unwrap().is_some());

        store.delete("x").unwrap();
        assert!(store.load("x").unwrap().is_none());

        // deleting a missing session is a no-op
        store.delete("x").unwrap();
        store.delete("never-existed").unwrap();
    }

    #[test]
    fn noop_storage_default_matches_new() {
        let _ = NoopStorage::default();
    }

    #[test]
    fn noop_memory_retriever_returns_empty() {
        let r = NoopMemoryRetriever;
        assert_eq!(r.retrieve("anything", 10).unwrap(), Vec::new());
        assert_eq!(r.retrieve("", 0).unwrap(), Vec::new());
    }

    #[test]
    fn persisted_state_serialises_with_schema_version_tag() {
        let state = sample_state("abc");
        let s = serde_json::to_string(&state).unwrap();
        // The serde tag must appear at the top level.
        assert!(s.contains(r#""schema_version":"1""#), "got: {s}");
        assert!(s.contains(r#""session_id":"abc""#));
    }

    #[test]
    fn persisted_state_serde_roundtrip_preserves_value() {
        let state = PersistedState::V1(PersistedStateV1 {
            session_name: None,
            session_id: "rt".into(),
            last_updated: "2024-06-01T12:34:56Z".into(),
            current_turn: 4,
            frontier_message_ref: Some("m0042".into()),
            next_block_id: 3,
            next_run_id: 2,
            next_message_ref: 11,
            stats: serde_json::json!({ "messages_seen": 10 }),
            nudges: serde_json::json!({
                "context_limit_counter": 0,
                "turn_nudged_pairs": []
            }),
            prune: serde_json::json!({ "tools": {} }),
            tool_index: serde_json::Value::Null,
            message_id_map: serde_json::Value::Null,
            compaction: serde_json::Value::Null,
        });
        let s = serde_json::to_string(&state).unwrap();
        let back: PersistedState = serde_json::from_str(&s).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn cache_event_constructs_all_variants() {
        // Make sure the variants compile and serialize.
        let hit = CacheEvent::Hit { tokens: 100 };
        let miss = CacheEvent::Miss { tokens: 50 };
        let bust = CacheEvent::Bust {
            reason: "prompt_changed".into(),
        };

        for ev in [hit, miss, bust] {
            let s = serde_json::to_string(&ev).unwrap();
            assert!(s.contains("\"event\""), "got: {s}");
        }
    }

    /// Minimal `CacheAccountant` to verify trait shape and `record_event`
    /// mutability.
    struct CountingAccountant {
        events: Vec<CacheEvent>,
    }
    impl CacheAccountant for CountingAccountant {
        fn cost_per_cache_miss_tokens(&self, tokens: usize) -> f64 {
            tokens as f64 * 0.000_003
        }
        fn record_event(&mut self, event: CacheEvent) {
            self.events.push(event);
        }
    }

    #[test]
    fn cache_accountant_records_events() {
        let mut a = CountingAccountant { events: vec![] };
        a.record_event(CacheEvent::Hit { tokens: 1 });
        a.record_event(CacheEvent::Miss { tokens: 2 });
        assert_eq!(a.events.len(), 2);
        assert!((a.cost_per_cache_miss_tokens(1_000_000) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn prune_outcome_skipped_constructor_matches_shape() {
        let o = PruneOutcome::skipped("deduplicate", "disabled");
        assert_eq!(o.strategy, "deduplicate");
        assert_eq!(o.pruned_ids, Vec::<String>::new());
        assert_eq!(o.tokens_saved, 0);
        assert_eq!(o.reason_skipped.as_deref(), Some("disabled"));
    }

    /// Minimal `PruneStrategy` impl over a unit config to verify the trait
    /// is object-safe and generic-instantiable.
    struct AlwaysSkip;
    impl PruneStrategy<()> for AlwaysSkip {
        fn name(&self) -> &str {
            "always_skip"
        }
        fn apply(
            &self,
            _state: &mut SessionState,
            _messages: &[Message],
            _config: &(),
        ) -> Result<PruneOutcome, PruneError> {
            Ok(PruneOutcome::skipped(self.name(), "test"))
        }
    }

    #[test]
    fn prune_strategy_can_be_invoked_via_trait_object() {
        let strat: Box<dyn PruneStrategy<()>> = Box::new(AlwaysSkip);
        let mut state = SessionState::default();
        let outcome = strat.apply(&mut state, &[], &()).unwrap();
        assert_eq!(outcome.strategy, "always_skip");
        assert_eq!(outcome.reason_skipped.as_deref(), Some("test"));
    }

    /// Compile-time assertion: all trait-bearing types are `Send + Sync`
    /// (the trait bounds say so, but the assertion catches accidental
    /// regressions when default impls are added).
    #[test]
    fn defaults_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NoopStorage>();
        assert_send_sync::<NoopMemoryRetriever>();
    }
}
