#![forbid(unsafe_code)]
//! `dcp-telemetry` — metric collection and observer hooks.
//!
//! Exposes counters and gauges for: pruning decisions, compression
//! events, cache-bust events, nudge fires, and frontier advances. Hosts
//! consume telemetry via the `Telemetry` snapshot returned by
//! `ContextPruner::telemetry()` (see PLAN.md §4.2).
//!
//! The optional `quality` feature enables a regression detector that
//! flags suspicious increases in token consumption or cache-miss rate.
//!
//! Phase 0 scaffold: types will land alongside the facade in Phase 5.
