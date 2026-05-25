#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
//! `dcp-config` — configuration schema, JSONC parser, and cascade
//! resolution for `dynamic_context_pruning`.
//!
//! This crate owns the canonical [`Config`] type (SPEC.md §10.2),
//! resolves it from disk via the cascade in SPEC.md §10.1, runs the
//! validation rules in SPEC.md §10.3, and exposes a JSON schema for
//! IDE / tooling integration (PLAN.md §8.3).
//!
//! # Cascade order (SPEC.md §10.1)
//!
//! 1. Built-in defaults (compiled in).
//! 2. Global: `$XDG_CONFIG_HOME/dynamic_context_pruning/config.jsonc`
//!    (with `~/.config/dynamic_context_pruning/config.jsonc` fallback).
//! 3. Custom: `$DCP_CONFIG_DIR/config.jsonc` if the env var is set.
//! 4. Project: `.dynamic_context_pruning/config.jsonc` in the working
//!    directory or any ancestor up to a marker (`.git`, `Cargo.toml`,
//!    `pyproject.toml`, `package.json`).
//!
//! Object fields are deep-merged (later wins per key); arrays / scalars
//! are replaced wholesale.
//!
//! # Trait implementations
//!
//! [`Config`] implements [`dcp_state::ConfigLike`],
//! [`dcp_prune::PruneConfig`], and [`dcp_compress::CompressConfig`] so
//! downstream crates can read only the slices they need without
//! depending on `dcp-config`.
//!
//! # Quick start
//!
//! ```no_run
//! use dcp_config::Config;
//!
//! let cfg = Config::load_default()?;
//! cfg.validate()?;
//! # Ok::<(), dcp_config::ConfigError>(())
//! ```

mod cascade;
mod config;
mod enums;
mod error;
mod limits;
mod sub_configs;

pub use cascade::{
    CONFIG_FILE_NAME, ENV_DCP_CONFIG_DIR, PROJECT_DIR_NAME, PROJECT_MARKERS, ResolvedPaths,
    load_default, load_default_at, load_with_paths, parse_jsonc_value,
};
pub use config::{Config, json_schema, validate};
pub use enums::{
    CacheStabilityMode, CompressMode, InjectionMode, NotificationKind, NotificationLevel,
    NudgeForce, Permission,
};
pub use error::ConfigError;
pub use limits::LimitValue;
pub use sub_configs::{
    CommandsConfig, CompressConfig, DeduplicationConfig, ExperimentalConfig, ManualModeConfig,
    NotificationConfig, PurgeErrorsConfig, StaleFileReadsConfig, StrategiesConfig,
    TurnProtectionConfig,
};
