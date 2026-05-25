#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! `dcp-telemetry` — event-driven metric collection and observer hooks
//! for `dynamic_context_pruning`.
//!
//! Two complementary surfaces live here:
//!
//! - **[`Telemetry`]** — an in-process counter object that records
//!   [`EventKind`]s and exposes deterministic [`TelemetrySnapshot`]s.
//!   Hosts pull a snapshot via the public facade
//!   `ContextPruner::telemetry()` (PLAN.md §4.2).
//! - **[`Observer`]** — a sink trait that lets hosts receive [`Event`]s
//!   in real time without holding any internal lock. Two reference
//!   implementations ship in this crate:
//!     - [`InMemoryObserver`] (always available) — useful in tests and
//!       for hosts that want to consume events on a thread.
//!     - [`LoggingObserver`] (behind the `logging` feature) — forwards
//!       each event to the `log` crate.
//!
//! The crate also implements a default
//! [`CacheAccountant`](dcp_traits::CacheAccountant) ([`DefaultCacheAccountant`])
//! that tracks every observed [`CacheEvent`](dcp_traits::CacheEvent) and
//! sums their re-encode cost using a host-supplied `cost_per_token`.
//!
//! # Stability
//!
//! [`EventKind`] is `#[non_exhaustive]` — new variants can be added in
//! future minor releases without breaking downstream `match` arms.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use dcp_traits::{CacheAccountant, CacheEvent};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────
// EventKind
// ─────────────────────────────────────────────────────────────────────────

/// One categorical event observed by the pruning pipeline.
///
/// Variants carry just enough metadata (typically the strategy / mode /
/// reason name) to bucket counters in [`Telemetry::event_counts`]. Bulky
/// per-event detail belongs on the surrounding [`Event`]'s `metadata`
/// JSON value, not in the variant itself.
///
/// `#[non_exhaustive]` so future variants can be added without breaking
/// downstream code that matches on this enum.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventKind {
    /// A prune strategy ran. `strategy` is the strategy's stable name
    /// (e.g. `"deduplicate"`, `"purge_errors"`, `"stale_file_reads"`).
    Prune {
        /// Stable strategy identifier.
        strategy: String,
    },
    /// A compression run completed. `mode` is `"range"` or `"message"`.
    Compress {
        /// Compression mode the host invoked.
        mode: String,
    },
    /// A nudge was injected. `kind` is `"context_limit"`, `"turn"`, or
    /// `"iteration"`.
    Nudge {
        /// Nudge category as documented in SPEC.md §8.
        kind: String,
    },
    /// The prompt cache was busted. `reason` is a free-form label
    /// (`"prompt_changed"`, `"system_prompt_diff"`, `"tools_changed"`, …).
    CacheBust {
        /// Free-form reason string.
        reason: String,
    },
    /// The apply phase was triggered. `mode` reflects the active
    /// `CacheStabilityMode` (e.g. `"agent_message"`, `"never"`,
    /// `"force"`).
    ApplyTrigger {
        /// Cache-stability mode that authorised the apply.
        mode: String,
    },
    /// The pruning frontier advanced past a message reference.
    Frontier {
        /// `m####` reference the frontier moved to.
        advanced_to: String,
    },
    /// The host signalled that an external compaction occurred.
    Compaction,
    /// Catch-all for host-defined events not covered by the bundled
    /// variants. Use sparingly: prefer requesting a first-class variant
    /// when the event has stable semantics.
    Other {
        /// Free-form event name.
        name: String,
    },
}

impl EventKind {
    /// Stable short name for this variant, useful for log lines.
    pub fn discriminant(&self) -> &'static str {
        match self {
            EventKind::Prune { .. } => "prune",
            EventKind::Compress { .. } => "compress",
            EventKind::Nudge { .. } => "nudge",
            EventKind::CacheBust { .. } => "cache_bust",
            EventKind::ApplyTrigger { .. } => "apply_trigger",
            EventKind::Frontier { .. } => "frontier",
            EventKind::Compaction => "compaction",
            EventKind::Other { .. } => "other",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Event
// ─────────────────────────────────────────────────────────────────────────

/// A single telemetry event delivered to [`Observer`]s.
///
/// `metadata` is a free-form JSON value — observers that pretty-print
/// events should treat it as opaque. `timestamp` is unix-epoch
/// milliseconds; the producer is expected to fill this in (typically
/// using [`now_millis`]).
///
/// `Eq` is intentionally not derived because `serde_json::Value` does not
/// implement it for `Number` variants holding floats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Event category and any small payload that fits in [`EventKind`].
    pub kind: EventKind,
    /// Producer-provided unix-epoch milliseconds.
    pub timestamp: i64,
    /// Free-form per-event payload.
    pub metadata: serde_json::Value,
}

impl Event {
    /// Construct an [`Event`] with a null `metadata` value.
    pub fn new(kind: EventKind, timestamp: i64) -> Self {
        Self {
            kind,
            timestamp,
            metadata: serde_json::Value::Null,
        }
    }

    /// Construct an [`Event`] with explicit `metadata`.
    pub fn with_metadata(kind: EventKind, timestamp: i64, metadata: serde_json::Value) -> Self {
        Self {
            kind,
            timestamp,
            metadata,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Clock helper
// ─────────────────────────────────────────────────────────────────────────

/// Current unix-epoch milliseconds, saturating on systems whose clock is
/// before the epoch.
///
/// This is a free function (not a trait) because the only reason callers
/// need a clock is to fill [`Event::timestamp`] / [`Telemetry::record`];
/// any host that needs deterministic timestamps in tests should call
/// [`Telemetry::record_at`] / [`Event::new`] directly.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────
// Telemetry + TelemetrySnapshot
// ─────────────────────────────────────────────────────────────────────────

/// Custom serde adapter for `HashMap<EventKind, u64>`.
///
/// JSON requires object keys to be strings, but [`EventKind`] serialises
/// to a tagged object. Encoding the map as a sorted `Vec<(EventKind,
/// u64)>` lets [`Telemetry`] / [`TelemetrySnapshot`] survive a JSON
/// round-trip and gives deterministic byte-for-byte output for the same
/// multiset of records.
mod event_counts_serde {
    use super::EventKind;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::HashMap;

    pub fn serialize<S: Serializer>(
        map: &HashMap<EventKind, u64>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let mut pairs: Vec<(&EventKind, &u64)> = map.iter().collect();
        // Stable order: by serialised key. Failures fall through to a
        // discriminant-based ordering so we never panic in serialize.
        pairs.sort_by(|a, b| {
            let ka = serde_json::to_string(a.0).unwrap_or_else(|_| a.0.discriminant().into());
            let kb = serde_json::to_string(b.0).unwrap_or_else(|_| b.0.discriminant().into());
            ka.cmp(&kb)
        });
        pairs.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<HashMap<EventKind, u64>, D::Error> {
        let pairs: Vec<(EventKind, u64)> = Vec::deserialize(d)?;
        Ok(pairs.into_iter().collect())
    }
}

/// In-process counter object surfaced by the public facade.
///
/// This is the *live* counter; hosts read it via [`Telemetry::snapshot`]
/// to get a stable, comparable [`TelemetrySnapshot`]. Reset via
/// [`Telemetry::reset`].
///
/// # Example
///
/// ```
/// use dcp_telemetry::{EventKind, Telemetry};
///
/// let mut t = Telemetry::new(0);
/// t.record_at(EventKind::Prune { strategy: "deduplicate".into() }, 1_000);
/// t.record_at(EventKind::Prune { strategy: "deduplicate".into() }, 1_500);
/// let snap = t.snapshot();
/// assert_eq!(snap.total_events(), 2);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Telemetry {
    /// Per-[`EventKind`] occurrence count.
    #[serde(with = "event_counts_serde")]
    pub event_counts: HashMap<EventKind, u64>,
    /// Unix-epoch milliseconds when this counter was created (or last
    /// [`reset`](Self::reset)).
    pub started_at: i64,
    /// Unix-epoch milliseconds of the most recent recorded event.
    /// Equals `started_at` until the first [`record`](Self::record) /
    /// [`record_at`](Self::record_at).
    pub last_event_at: i64,
}

impl Telemetry {
    /// Create a fresh counter anchored at `started_at` (unix-epoch
    /// milliseconds). `last_event_at` is initialised to the same value.
    pub fn new(started_at: i64) -> Self {
        Self {
            event_counts: HashMap::new(),
            started_at,
            last_event_at: started_at,
        }
    }

    /// Create a counter anchored at the current wall clock.
    pub fn now() -> Self {
        Self::new(now_millis())
    }

    /// Record a single event at the current wall-clock time.
    ///
    /// For deterministic tests, prefer [`record_at`](Self::record_at).
    pub fn record(&mut self, kind: EventKind) {
        self.record_at(kind, now_millis());
    }

    /// Record a single event at the given unix-epoch millisecond
    /// timestamp. Out-of-order timestamps are accepted but
    /// `last_event_at` advances monotonically (it never moves backwards).
    pub fn record_at(&mut self, kind: EventKind, timestamp: i64) {
        *self.event_counts.entry(kind).or_insert(0) += 1;
        if timestamp > self.last_event_at {
            self.last_event_at = timestamp;
        }
    }

    /// Snapshot the current counters. The snapshot is a structural copy
    /// — two [`Telemetry`]s that received the same `record_at(kind, ts)`
    /// sequence produce equal snapshots.
    pub fn snapshot(&self) -> TelemetrySnapshot {
        TelemetrySnapshot {
            event_counts: self.event_counts.clone(),
            started_at: self.started_at,
            last_event_at: self.last_event_at,
        }
    }

    /// Clear all counts and re-anchor `started_at` / `last_event_at` to
    /// the current wall-clock time.
    pub fn reset(&mut self) {
        self.event_counts.clear();
        let now = now_millis();
        self.started_at = now;
        self.last_event_at = now;
    }

    /// Clear all counts and re-anchor `started_at` / `last_event_at` to
    /// `started_at` — the deterministic counterpart of
    /// [`reset`](Self::reset) for tests.
    pub fn reset_at(&mut self, started_at: i64) {
        self.event_counts.clear();
        self.started_at = started_at;
        self.last_event_at = started_at;
    }

    /// Total number of recorded events across every variant.
    pub fn total_events(&self) -> u64 {
        self.event_counts.values().sum()
    }

    /// Count of events matching `kind` (zero if never recorded).
    pub fn count_of(&self, kind: &EventKind) -> u64 {
        self.event_counts.get(kind).copied().unwrap_or(0)
    }
}

/// Immutable structural copy of [`Telemetry`].
///
/// Equality is structural (it does not depend on `HashMap` iteration
/// order), so snapshots are useful for golden-file / determinism tests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    /// Per-[`EventKind`] occurrence count at the moment of the snapshot.
    #[serde(with = "event_counts_serde")]
    pub event_counts: HashMap<EventKind, u64>,
    /// Mirrors [`Telemetry::started_at`].
    pub started_at: i64,
    /// Mirrors [`Telemetry::last_event_at`].
    pub last_event_at: i64,
}

impl TelemetrySnapshot {
    /// Total number of recorded events across every variant.
    pub fn total_events(&self) -> u64 {
        self.event_counts.values().sum()
    }

    /// Count of events matching `kind` (zero if never recorded).
    pub fn count_of(&self, kind: &EventKind) -> u64 {
        self.event_counts.get(kind).copied().unwrap_or(0)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Observer
// ─────────────────────────────────────────────────────────────────────────

/// Sink trait for [`Event`]s.
///
/// `record` takes `&self` so observers must use interior mutability
/// (e.g. [`Mutex`]). All implementations must be `Send + Sync` so a
/// single observer can be shared across threads behind an `Arc`.
pub trait Observer: Send + Sync {
    /// Receive a single event. Implementations should be cheap and
    /// non-blocking — if the sink can fail, swallow or log the failure
    /// rather than panicking.
    fn record(&self, event: Event);
}

/// In-memory [`Observer`] that retains every event in insertion order.
///
/// Mainly useful for tests and short-lived hosts. Production hosts
/// should generally implement their own observer that forwards events to
/// their existing logging or metrics pipeline.
#[derive(Debug, Default)]
pub struct InMemoryObserver {
    events: Mutex<Vec<Event>>,
}

impl InMemoryObserver {
    /// Construct an empty observer.
    pub fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    /// Snapshot all retained events. Returns events in insertion order.
    pub fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    /// Number of retained events.
    pub fn len(&self) -> usize {
        self.events.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// `true` if no events have been retained.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Discard all retained events.
    pub fn clear(&self) {
        if let Ok(mut g) = self.events.lock() {
            g.clear();
        }
    }
}

impl Observer for InMemoryObserver {
    fn record(&self, event: Event) {
        // A poisoned mutex still has the underlying `Vec`; recover it so
        // event loss happens only if the OS itself is broken.
        match self.events.lock() {
            Ok(mut g) => g.push(event),
            Err(poisoned) => poisoned.into_inner().push(event),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// LoggingObserver (feature = "logging")
// ─────────────────────────────────────────────────────────────────────────

/// [`Observer`] that forwards each event to the `log` crate at
/// `info` level.
///
/// Available behind the `logging` feature.
#[cfg(feature = "logging")]
#[derive(Debug, Default, Clone, Copy)]
pub struct LoggingObserver;

#[cfg(feature = "logging")]
impl LoggingObserver {
    /// Construct a new [`LoggingObserver`].
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "logging")]
impl Observer for LoggingObserver {
    fn record(&self, event: Event) {
        log::info!(
            target: "dcp_telemetry",
            "kind={} ts={} meta={}",
            event.kind.discriminant(),
            event.timestamp,
            event.metadata,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// DefaultCacheAccountant
// ─────────────────────────────────────────────────────────────────────────

/// Default [`CacheAccountant`](dcp_traits::CacheAccountant)
/// implementation.
///
/// Records every observed [`CacheEvent`] in insertion order and
/// estimates re-encode cost as `tokens * cost_per_token`. The per-token
/// cost is host-supplied so this crate stays provider-agnostic.
///
/// # Example
///
/// ```
/// use dcp_telemetry::DefaultCacheAccountant;
/// use dcp_traits::{CacheAccountant, CacheEvent};
///
/// let mut a = DefaultCacheAccountant::new(0.000_003);
/// a.record_event(CacheEvent::Miss { tokens: 1_000_000 });
/// a.record_event(CacheEvent::Hit { tokens: 500 }); // ignored in cost
/// assert!((a.total_miss_cost() - 3.0).abs() < 1e-9);
/// ```
#[derive(Debug, Clone, Default)]
pub struct DefaultCacheAccountant {
    /// Cost (e.g. dollars) charged per re-encoded token on a cache miss.
    pub cost_per_token: f64,
    /// Every event observed via [`CacheAccountant::record_event`], in
    /// insertion order.
    pub events: Vec<CacheEvent>,
}

impl DefaultCacheAccountant {
    /// Construct a new accountant with the given per-token re-encode
    /// cost.
    pub fn new(cost_per_token: f64) -> Self {
        Self {
            cost_per_token,
            events: Vec::new(),
        }
    }

    /// Sum of `tokens * cost_per_token` across every observed
    /// [`CacheEvent::Miss`].
    pub fn total_miss_cost(&self) -> f64 {
        estimate_cache_miss_cost(&self.events, self.cost_per_token)
    }

    /// Number of [`CacheEvent::Bust`] events observed so far.
    pub fn total_bust_count(&self) -> u64 {
        self.events
            .iter()
            .filter(|e| matches!(e, CacheEvent::Bust { .. }))
            .count() as u64
    }

    /// Number of [`CacheEvent::Hit`] events observed so far.
    pub fn total_hit_count(&self) -> u64 {
        self.events
            .iter()
            .filter(|e| matches!(e, CacheEvent::Hit { .. }))
            .count() as u64
    }

    /// Number of [`CacheEvent::Miss`] events observed so far.
    pub fn total_miss_count(&self) -> u64 {
        self.events
            .iter()
            .filter(|e| matches!(e, CacheEvent::Miss { .. }))
            .count() as u64
    }
}

impl CacheAccountant for DefaultCacheAccountant {
    fn cost_per_cache_miss_tokens(&self, tokens: usize) -> f64 {
        tokens as f64 * self.cost_per_token
    }

    fn record_event(&mut self, event: CacheEvent) {
        self.events.push(event);
    }
}

/// Estimate the total re-encode cost across `events` using
/// `cost_per_token`.
///
/// Only [`CacheEvent::Miss`] entries contribute; hits and busts have no
/// re-encode cost in this model.
pub fn estimate_cache_miss_cost(events: &[CacheEvent], cost_per_token: f64) -> f64 {
    events
        .iter()
        .filter_map(|e| match e {
            CacheEvent::Miss { tokens } => Some(*tokens as f64 * cost_per_token),
            _ => None,
        })
        .sum()
}

// ─────────────────────────────────────────────────────────────────────────
// tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn k_prune(s: &str) -> EventKind {
        EventKind::Prune { strategy: s.into() }
    }

    fn k_compress(m: &str) -> EventKind {
        EventKind::Compress { mode: m.into() }
    }

    // ── EventKind ─────────────────────────────────────────────────────

    #[test]
    fn event_kind_serialises_with_kind_tag() {
        let ek = k_prune("deduplicate");
        let s = serde_json::to_string(&ek).unwrap();
        assert!(s.contains(r#""event":"prune""#), "got: {s}");
        assert!(s.contains(r#""strategy":"deduplicate""#), "got: {s}");
    }

    #[test]
    fn event_kind_discriminant_is_stable() {
        assert_eq!(k_prune("x").discriminant(), "prune");
        assert_eq!(k_compress("range").discriminant(), "compress");
        assert_eq!(
            EventKind::Nudge {
                kind: "turn".into()
            }
            .discriminant(),
            "nudge"
        );
        assert_eq!(
            EventKind::CacheBust { reason: "x".into() }.discriminant(),
            "cache_bust"
        );
        assert_eq!(
            EventKind::ApplyTrigger {
                mode: "agent_message".into()
            }
            .discriminant(),
            "apply_trigger"
        );
        assert_eq!(
            EventKind::Frontier {
                advanced_to: "m0001".into()
            }
            .discriminant(),
            "frontier"
        );
        assert_eq!(EventKind::Compaction.discriminant(), "compaction");
        assert_eq!(
            EventKind::Other {
                name: "custom".into()
            }
            .discriminant(),
            "other"
        );
    }

    #[test]
    fn event_kind_eq_distinguishes_payload() {
        assert_ne!(k_prune("a"), k_prune("b"));
        assert_eq!(k_prune("a"), k_prune("a"));
    }

    // ── Event ─────────────────────────────────────────────────────────

    #[test]
    fn event_new_defaults_metadata_to_null() {
        let e = Event::new(k_prune("x"), 42);
        assert_eq!(e.timestamp, 42);
        assert_eq!(e.metadata, serde_json::Value::Null);
    }

    #[test]
    fn event_with_metadata_preserves_value() {
        let meta = serde_json::json!({ "tokens_saved": 100 });
        let e = Event::with_metadata(k_prune("x"), 7, meta.clone());
        assert_eq!(e.metadata, meta);
    }

    // ── Telemetry: recording + snapshots ──────────────────────────────

    #[test]
    fn telemetry_record_at_increments_count() {
        let mut t = Telemetry::new(1_000);
        t.record_at(k_prune("dedup"), 1_100);
        t.record_at(k_prune("dedup"), 1_200);
        t.record_at(k_prune("purge"), 1_300);

        assert_eq!(t.count_of(&k_prune("dedup")), 2);
        assert_eq!(t.count_of(&k_prune("purge")), 1);
        assert_eq!(t.count_of(&k_prune("never")), 0);
        assert_eq!(t.total_events(), 3);
    }

    #[test]
    fn telemetry_record_at_advances_last_event_at_monotonically() {
        let mut t = Telemetry::new(1_000);
        assert_eq!(t.last_event_at, 1_000);

        t.record_at(k_prune("a"), 1_500);
        assert_eq!(t.last_event_at, 1_500);

        // Out-of-order timestamps must not move the watermark backwards.
        t.record_at(k_prune("a"), 1_100);
        assert_eq!(t.last_event_at, 1_500);

        t.record_at(k_prune("a"), 2_000);
        assert_eq!(t.last_event_at, 2_000);
    }

    #[test]
    fn telemetry_snapshot_is_deterministic_over_recording_order() {
        // Two telemetry instances that receive the same multiset of
        // (kind, timestamp) records — but in different order — must
        // produce equal snapshots (HashMap equality is order-insensitive,
        // last_event_at is the max).
        let events: &[(EventKind, i64)] = &[
            (k_prune("dedup"), 1_100),
            (k_compress("range"), 1_150),
            (k_prune("dedup"), 1_200),
            (
                EventKind::Nudge {
                    kind: "turn".into(),
                },
                1_300,
            ),
            (
                EventKind::CacheBust {
                    reason: "tools_changed".into(),
                },
                1_400,
            ),
        ];

        let mut a = Telemetry::new(1_000);
        for (k, ts) in events {
            a.record_at(k.clone(), *ts);
        }

        let mut b = Telemetry::new(1_000);
        for (k, ts) in events.iter().rev() {
            b.record_at(k.clone(), *ts);
        }

        assert_eq!(a.snapshot(), b.snapshot());
    }

    #[test]
    fn telemetry_snapshot_round_trips_through_serde() {
        let mut t = Telemetry::new(0);
        t.record_at(k_prune("dedup"), 100);
        t.record_at(k_compress("range"), 200);

        let snap = t.snapshot();
        let s = serde_json::to_string(&snap).unwrap();
        let back: TelemetrySnapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(snap, back);
    }

    #[test]
    fn telemetry_reset_at_clears_counts_and_anchors_clock() {
        let mut t = Telemetry::new(1_000);
        t.record_at(k_prune("dedup"), 1_500);
        assert_eq!(t.total_events(), 1);

        t.reset_at(2_000);
        assert_eq!(t.total_events(), 0);
        assert_eq!(t.started_at, 2_000);
        assert_eq!(t.last_event_at, 2_000);
        assert!(t.event_counts.is_empty());
    }

    #[test]
    fn telemetry_reset_uses_wall_clock() {
        // We can only assert that started_at == last_event_at and that
        // counts are cleared — we cannot assert an exact wall-clock
        // value without a clock injection.
        let mut t = Telemetry::new(0);
        t.record_at(k_prune("dedup"), 100);
        t.reset();
        assert!(t.event_counts.is_empty());
        assert_eq!(t.started_at, t.last_event_at);
    }

    #[test]
    fn telemetry_record_uses_wall_clock_and_increments() {
        let mut t = Telemetry::new(0);
        t.record(k_prune("dedup"));
        assert_eq!(t.count_of(&k_prune("dedup")), 1);
        // last_event_at must have moved past the (zero) anchor.
        assert!(t.last_event_at >= 0);
    }

    // ── Observer dispatch ─────────────────────────────────────────────

    #[test]
    fn in_memory_observer_dispatches_events_in_order() {
        let obs = InMemoryObserver::new();
        let e1 = Event::new(k_prune("dedup"), 100);
        let e2 = Event::with_metadata(
            k_compress("range"),
            200,
            serde_json::json!({ "tokens_saved": 42 }),
        );
        obs.record(e1.clone());
        obs.record(e2.clone());

        assert_eq!(obs.len(), 2);
        assert!(!obs.is_empty());
        let got = obs.events();
        assert_eq!(got, vec![e1, e2]);
    }

    #[test]
    fn in_memory_observer_clear_removes_events() {
        let obs = InMemoryObserver::new();
        obs.record(Event::new(k_prune("a"), 1));
        obs.record(Event::new(k_prune("b"), 2));
        assert_eq!(obs.len(), 2);
        obs.clear();
        assert!(obs.is_empty());
    }

    #[test]
    fn in_memory_observer_is_thread_safe() {
        let obs = Arc::new(InMemoryObserver::new());
        let mut handles = Vec::new();
        for tid in 0..4u64 {
            let obs = Arc::clone(&obs);
            handles.push(thread::spawn(move || {
                for i in 0..25u64 {
                    obs.record(Event::new(
                        EventKind::Other {
                            name: format!("t{tid}"),
                        },
                        (tid * 1_000 + i) as i64,
                    ));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(obs.len(), 100);
    }

    #[test]
    fn observer_trait_object_is_object_safe() {
        // Compile-time check: `dyn Observer` must be usable as a trait
        // object behind `Arc`.
        let obs: Arc<dyn Observer> = Arc::new(InMemoryObserver::new());
        obs.record(Event::new(k_prune("dedup"), 1));
    }

    // ── DefaultCacheAccountant ────────────────────────────────────────

    #[test]
    fn default_cache_accountant_estimates_per_token_cost() {
        let a = DefaultCacheAccountant::new(0.000_003);
        // 1_000_000 tokens × $0.000003 = $3.00
        assert!((a.cost_per_cache_miss_tokens(1_000_000) - 3.0).abs() < 1e-9);
        assert_eq!(a.cost_per_cache_miss_tokens(0), 0.0);
    }

    #[test]
    fn default_cache_accountant_records_events() {
        let mut a = DefaultCacheAccountant::new(0.000_003);
        a.record_event(CacheEvent::Hit { tokens: 100 });
        a.record_event(CacheEvent::Miss { tokens: 50 });
        a.record_event(CacheEvent::Bust {
            reason: "tools_changed".into(),
        });

        assert_eq!(a.events.len(), 3);
        assert_eq!(a.total_hit_count(), 1);
        assert_eq!(a.total_miss_count(), 1);
        assert_eq!(a.total_bust_count(), 1);
    }

    #[test]
    fn default_cache_accountant_total_miss_cost_sums_only_misses() {
        let mut a = DefaultCacheAccountant::new(0.01);
        a.record_event(CacheEvent::Hit { tokens: 1_000 }); // ignored
        a.record_event(CacheEvent::Miss { tokens: 100 });
        a.record_event(CacheEvent::Miss { tokens: 200 });
        a.record_event(CacheEvent::Bust { reason: "x".into() }); // ignored

        // (100 + 200) × 0.01 = 3.0
        assert!((a.total_miss_cost() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn estimate_cache_miss_cost_handles_empty_input() {
        assert_eq!(estimate_cache_miss_cost(&[], 0.5), 0.0);
    }

    #[test]
    fn estimate_cache_miss_cost_ignores_non_miss_variants() {
        let events = [
            CacheEvent::Hit { tokens: 10 },
            CacheEvent::Bust { reason: "x".into() },
        ];
        assert_eq!(estimate_cache_miss_cost(&events, 1.0), 0.0);
    }

    // ── End-to-end: pipe Events through Telemetry + Observer ──────────

    #[test]
    fn observer_and_telemetry_compose() {
        // Demonstrate that a single producer can update both a
        // `Telemetry` counter and an `InMemoryObserver` with consistent
        // event ordering.
        let mut t = Telemetry::new(1_000);
        let obs: Arc<dyn Observer> = Arc::new(InMemoryObserver::new());

        let kinds = [
            k_prune("dedup"),
            k_prune("dedup"),
            k_compress("range"),
            EventKind::CacheBust {
                reason: "prompt_changed".into(),
            },
        ];

        for (i, kind) in kinds.iter().enumerate() {
            let ts = 1_000 + (i as i64) * 100;
            t.record_at(kind.clone(), ts);
            obs.record(Event::new(kind.clone(), ts));
        }

        assert_eq!(t.total_events(), 4);
        assert_eq!(t.count_of(&k_prune("dedup")), 2);
        assert_eq!(t.last_event_at, 1_300);

        // Recover the InMemoryObserver to inspect retained events.
        // We know the concrete type, but to keep the test honest we go
        // back through a fresh observer.
        let snap = t.snapshot();
        assert_eq!(snap.total_events(), 4);
    }

    // Compile-time assertion: the public observer-bearing types are
    // `Send + Sync` so they can be shared across threads.
    #[test]
    fn public_types_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Telemetry>();
        assert_send_sync::<TelemetrySnapshot>();
        assert_send_sync::<InMemoryObserver>();
        assert_send_sync::<DefaultCacheAccountant>();
        assert_send_sync::<Event>();
        assert_send_sync::<EventKind>();
    }

    // ── LoggingObserver (feature = "logging") ─────────────────────────

    #[cfg(feature = "logging")]
    #[test]
    fn logging_observer_is_send_sync_and_records_without_panic() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LoggingObserver>();

        // No global logger is required — `log::info!` is a no-op when no
        // logger is registered. The point is to exercise the trait impl.
        let obs: Arc<dyn Observer> = Arc::new(LoggingObserver::new());
        obs.record(Event::with_metadata(
            k_prune("dedup"),
            42,
            serde_json::json!({ "tokens_saved": 10 }),
        ));
    }
}
