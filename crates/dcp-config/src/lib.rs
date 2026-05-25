#![forbid(unsafe_code)]
//! `dcp-config` — configuration schema, JSONC parser, and cascade
//! resolution.
//!
//! Resolution order (PLAN.md §8.2):
//!
//! 1. Built-in defaults (compiled in).
//! 2. Global: `$XDG_CONFIG_HOME/dynamic_context_pruning/config.jsonc`.
//! 3. Custom directory: `$DCP_CONFIG_DIR/config.jsonc`.
//! 4. Project: `.dynamic_context_pruning/config.jsonc` in the project root.
//!
//! Exposes `Config`, `CompressConfig`, `StrategiesConfig`,
//! `CacheStabilityMode`, `NudgeForce`, and `InjectionMode`. Field
//! semantics and validation rules are defined in SPEC.md §10.
//!
//! Phase 0 scaffold: structs will land in Phase 5.
