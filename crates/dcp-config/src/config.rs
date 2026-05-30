//! Root [`Config`] type, validation, JSON-schema generation, and
//! ConfigLike trait implementations.
//!
//! See SPEC.md §10 and PLAN.md §8 for the overall shape.

use dcp_protected::{PathProtection, ToolProtection};
use schemars::JsonSchema;
use schemars::r#gen::SchemaGenerator;
use schemars::schema::{Metadata, RootSchema};
use serde::{Deserialize, Serialize};

use crate::cascade::load_default;
use crate::enums::{CacheStabilityMode, CacheStabilityModeSchema, NudgeForce};
use crate::error::ConfigError;
use crate::sub_configs::{
    CommandsConfig, CompressConfig, ExperimentalConfig, ManualModeConfig, NotificationConfig,
    StrategiesConfig, TurnProtectionConfig,
};

// ============================================================================
// Cached protection sets
// ============================================================================

#[derive(Clone, Debug, Default)]
struct CachedProtections {
    dedup: ToolProtection,
    purge_errors: ToolProtection,
    stale_file_reads: ToolProtection,
    compress: ToolProtection,
    commands: ToolProtection,
    paths: PathProtection,
}

// ============================================================================
// Config
// ============================================================================

/// Top-level configuration for `dynamic_context_pruning`.
///
/// Mirrors SPEC.md §10.2. Keys serialise in camelCase to match the
/// JSONC surface; in-memory access uses snake_case Rust field names.
///
/// # Construction
///
/// * [`Config::default`] — built-in defaults from SPEC.md §10.2.
/// * [`Config::load_default`] — cascade resolution per SPEC.md §10.1.
/// * Programmatic mutation followed by [`Config::validate`].
///
/// # Trait surface
///
/// `Config` implements [`dcp_state::ConfigLike`], [`dcp_prune::PruneConfig`],
/// and [`dcp_compress::CompressConfig`] so downstream crates can read
/// only the slices they need.
///
/// # Example
///
/// ```rust
/// use dcp_config::Config;
/// let cfg = Config::default();
/// assert!(cfg.enabled);
/// assert!(!cfg.debug);
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Master switch (SPEC.md §10.2).
    pub enabled: bool,
    /// Verbose telemetry / debug log output.
    pub debug: bool,
    /// Cache stability gate (`"aggressive" | "agent-message" | "manual"`).
    #[schemars(with = "CacheStabilityModeSchema")]
    pub cache_stability_mode: CacheStabilityMode,
    /// Notification preferences.
    pub notification: NotificationConfig,
    /// Manual mode flags.
    pub manual_mode: ManualModeConfig,
    /// Turn-protection knobs.
    pub turn_protection: TurnProtectionConfig,
    /// Project-wide protected file globs (SPEC.md §5.1, §5.3).
    pub protected_file_patterns: Vec<String>,
    /// Compress tool configuration.
    pub compress: CompressConfig,
    /// Strategy configuration.
    pub strategies: StrategiesConfig,
    /// Slash-command configuration.
    pub commands: CommandsConfig,
    /// Experimental flags.
    pub experimental: ExperimentalConfig,
    /// Cached, compiled protection sets. Rebuilt after every load and
    /// after any programmatic mutation.
    #[serde(skip)]
    #[schemars(skip)]
    cached: CachedProtections,
}

impl Default for Config {
    fn default() -> Self {
        let mut cfg = Self {
            enabled: true,
            debug: false,
            cache_stability_mode: CacheStabilityMode::AgentMessage,
            notification: NotificationConfig::default(),
            manual_mode: ManualModeConfig::default(),
            turn_protection: TurnProtectionConfig::default(),
            protected_file_patterns: Vec::new(),
            compress: CompressConfig::default(),
            strategies: StrategiesConfig::default(),
            commands: CommandsConfig::default(),
            experimental: ExperimentalConfig::default(),
            cached: CachedProtections::default(),
        };
        // The default Config is known-valid by construction, so any
        // failure here is a programming error.
        cfg.rebuild_cache()
            .expect("default Config must build a valid cache");
        cfg
    }
}

impl Config {
    /// Resolve the configuration cascade per SPEC.md §10.1.
    ///
    /// Order: built-in defaults → global → `$DCP_CONFIG_DIR` →
    /// project (`.dynamic_context_pruning/config.jsonc`).
    pub fn load_default() -> Result<Self, ConfigError> {
        load_default()
    }

    /// Validate this configuration per SPEC.md §10.3.
    ///
    /// Returns the first violation encountered. Successful validation
    /// is a precondition for using a [`Config`] inside the library.
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate(self)
    }

    /// Recompute the cached [`ToolProtection`] / [`PathProtection`]
    /// values from the raw configuration. Must be called after any
    /// mutation to a `protected*` field before passing the config to
    /// the rest of the library.
    pub fn rebuild_cache(&mut self) -> Result<(), ConfigError> {
        self.cached.dedup = {
            let (exact, glob): (Vec<_>, Vec<_>) = self
                .strategies
                .deduplication
                .protected_tools
                .iter()
                .partition(|t| !t.contains('*') && !t.contains('?'));
            ToolProtection::new(exact.into_iter().cloned(), glob.into_iter().cloned())
        };
        self.cached.purge_errors = {
            let (exact, glob): (Vec<_>, Vec<_>) = self
                .strategies
                .purge_errors
                .protected_tools
                .iter()
                .partition(|t| !t.contains('*') && !t.contains('?'));
            ToolProtection::new(exact.into_iter().cloned(), glob.into_iter().cloned())
        };
        self.cached.stale_file_reads = {
            let (exact, glob): (Vec<_>, Vec<_>) = self
                .strategies
                .stale_file_reads
                .protected_tools
                .iter()
                .partition(|t| !t.contains('*') && !t.contains('?'));
            ToolProtection::new(exact.into_iter().cloned(), glob.into_iter().cloned())
        };
        self.cached.compress = {
            let (exact, glob): (Vec<_>, Vec<_>) = self
                .compress
                .protected_tools
                .iter()
                .partition(|t| !t.contains('*') && !t.contains('?'));
            ToolProtection::new(exact.into_iter().cloned(), glob.into_iter().cloned())
        };
        self.cached.commands = {
            let (exact, glob): (Vec<_>, Vec<_>) = self
                .commands
                .protected_tools
                .iter()
                .partition(|t| !t.contains('*') && !t.contains('?'));
            ToolProtection::new(exact.into_iter().cloned(), glob.into_iter().cloned())
        };
        self.cached.paths =
            PathProtection::compile(&self.protected_file_patterns).map_err(|e| match e {
                dcp_protected::ProtectionError::InvalidGlob { pattern, source } => {
                    ConfigError::InvalidGlob {
                        pattern,
                        message: source.to_string(),
                    }
                }
            })?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Public read accessors used by ConfigLike-style trait impls.
    // ------------------------------------------------------------------

    /// Compiled protected-file globs (`protectedFilePatterns`).
    pub fn protected_paths(&self) -> &PathProtection {
        &self.cached.paths
    }

    /// Compiled compress-protected tools (`compress.protectedTools`).
    pub fn compress_protected_tools(&self) -> &ToolProtection {
        &self.cached.compress
    }

    /// Compiled deduplication-protected tools.
    pub fn dedup_protected_tools(&self) -> &ToolProtection {
        &self.cached.dedup
    }

    /// Compiled purge-errors-protected tools.
    pub fn purge_errors_protected_tools(&self) -> &ToolProtection {
        &self.cached.purge_errors
    }

    /// Compiled stale-file-reads-protected tools.
    pub fn stale_file_reads_protected_tools(&self) -> &ToolProtection {
        &self.cached.stale_file_reads
    }

    /// Compiled commands-protected tools (`commands.protectedTools`).
    pub fn commands_protected_tools(&self) -> &ToolProtection {
        &self.cached.commands
    }

    /// True when `nudgeForce == Strong`.
    pub fn nudge_force(&self) -> NudgeForce {
        self.compress.nudge_force
    }
}

impl PartialEq for Config {
    fn eq(&self, other: &Self) -> bool {
        // Cached protections are deterministic from raw fields; ignore
        // them to keep semantic equality intuitive.
        self.enabled == other.enabled
            && self.debug == other.debug
            && self.cache_stability_mode == other.cache_stability_mode
            && self.notification == other.notification
            && self.manual_mode == other.manual_mode
            && self.turn_protection == other.turn_protection
            && self.protected_file_patterns == other.protected_file_patterns
            && self.compress == other.compress
            && self.strategies == other.strategies
            && self.commands == other.commands
            && self.experimental == other.experimental
    }
}

// ============================================================================
// Validation (SPEC.md §10.3)
// ============================================================================

/// Top-level validator implementing SPEC.md §10.3 rules.
pub fn validate(config: &Config) -> Result<(), ConfigError> {
    // The enums (`CacheStabilityMode`, `Permission`, `CompressMode`,
    // `NotificationLevel`, `NotificationKind`) are statically constrained
    // by their Rust type, so the "is-one-of-the-known-values" rules
    // collapse into compile-time guarantees and we only assert ranges /
    // dependencies here.

    // compress.maxContextLimit
    match config.compress.max_context_limit {
        crate::limits::LimitValue::Number(n) => {
            if n < 1_000 {
                return Err(ConfigError::validation("maxContextLimit too small"));
            }
        }
        crate::limits::LimitValue::Percent(_) => { /* parser already enforced 0 < p <= 100 */ }
    }

    // compress.minContextLimit
    if let crate::limits::LimitValue::Number(n) = config.compress.min_context_limit
        && n < 1_000
    {
        return Err(ConfigError::validation("minContextLimit too small"));
    }

    // min < max in numeric form (SPEC.md §10.3).
    let max = config.compress.max_context_limit.resolve(None);
    let min = config.compress.min_context_limit.resolve(None);
    if min >= max {
        return Err(ConfigError::validation(
            "minContextLimit must be < maxContextLimit",
        ));
    }

    // Per-model overrides — same shape rules.
    for (model, lv) in &config.compress.model_max_limits {
        if let crate::limits::LimitValue::Number(n) = lv
            && *n < 1_000
        {
            return Err(ConfigError::validation(format!(
                "modelMaxLimits[{model}] is below 1000 tokens"
            )));
        }
    }
    for (model, lv) in &config.compress.model_min_limits {
        if let crate::limits::LimitValue::Number(n) = lv
            && *n < 1_000
        {
            return Err(ConfigError::validation(format!(
                "modelMinLimits[{model}] is below 1000 tokens"
            )));
        }
    }

    // compress.nudgeFrequency in 1..=100
    if !(1..=100).contains(&config.compress.nudge_frequency) {
        return Err(ConfigError::validation(format!(
            "nudgeFrequency must be in 1..=100 (was {})",
            config.compress.nudge_frequency
        )));
    }

    // compress.iterationNudgeThreshold in 2..=100
    if !(2..=100).contains(&config.compress.iteration_nudge_threshold) {
        return Err(ConfigError::validation(format!(
            "iterationNudgeThreshold must be in 2..=100 (was {})",
            config.compress.iteration_nudge_threshold
        )));
    }

    // compress.maxSummaryChars in 1024..=262144
    if !(1024..=262_144).contains(&config.compress.max_summary_chars) {
        return Err(ConfigError::validation(format!(
            "maxSummaryChars must be in 1024..=262144 (was {})",
            config.compress.max_summary_chars
        )));
    }

    // strategies.purgeErrors.turns in 1..=100
    if !(1..=100).contains(&config.strategies.purge_errors.turns) {
        return Err(ConfigError::validation(format!(
            "strategies.purgeErrors.turns must be in 1..=100 (was {})",
            config.strategies.purge_errors.turns
        )));
    }

    // strategies.staleFileReads.trackedTools non-empty when enabled
    if config.strategies.stale_file_reads.enabled
        && config.strategies.stale_file_reads.tracked_tools.is_empty()
    {
        return Err(ConfigError::validation(
            "trackedTools cannot be empty when staleFileReads is enabled",
        ));
    }

    // turnProtection.turns in 0..=100
    if config.turn_protection.turns > 100 {
        return Err(ConfigError::validation(format!(
            "turnProtection.turns must be in 0..=100 (was {})",
            config.turn_protection.turns
        )));
    }

    // protectedFilePatterns must compile (cached path protection
    // already attempted compilation; surface the error here for
    // parity with SPEC §10.3 semantics).
    PathProtection::compile(&config.protected_file_patterns).map_err(|e| match e {
        dcp_protected::ProtectionError::InvalidGlob { pattern, source } => {
            ConfigError::InvalidGlob {
                pattern,
                message: source.to_string(),
            }
        }
    })?;

    Ok(())
}

// ============================================================================
// JSON schema export
// ============================================================================

/// Return the canonical JSON schema for [`Config`] as a
/// `serde_json::Value`. Hosts can publish the result alongside the
/// repo to enable IDE autocomplete (PLAN.md §8.3).
///
/// # Example
///
/// ```rust
/// let v = dcp_config::json_schema();
/// assert!(v.get("$schema").is_some());
/// ```
pub fn json_schema() -> serde_json::Value {
    let mut generator = SchemaGenerator::default();
    let mut root: RootSchema = generator.root_schema_for::<Config>();
    let title = "dynamic_context_pruning configuration";
    let metadata = root
        .schema
        .metadata
        .get_or_insert_with(|| Box::new(Metadata::default()));
    metadata.title = Some(title.into());
    metadata.description = Some("JSON Schema for the dynamic_context_pruning library".into());
    serde_json::to_value(&root).expect("schemars must produce a JSON-serialisable RootSchema")
}

// ============================================================================
// ConfigLike trait implementations
// ============================================================================

impl dcp_state::ConfigLike for Config {
    fn is_tracked_tool(&self, name: &str) -> bool {
        self.strategies
            .stale_file_reads
            .tracked_tools
            .iter()
            .any(|t| t == name)
    }

    fn protected_tools(&self) -> &[String] {
        &self.compress.protected_tools
    }

    fn turn_protection_enabled(&self) -> bool {
        self.turn_protection.enabled
    }

    fn turn_protection_turns(&self) -> u32 {
        self.turn_protection.turns
    }
}

impl dcp_prune::PruneConfig for Config {
    fn dedup_enabled(&self) -> bool {
        self.strategies.deduplication.enabled
    }
    fn dedup_protected_tools(&self) -> &ToolProtection {
        &self.cached.dedup
    }
    fn purge_errors_enabled(&self) -> bool {
        self.strategies.purge_errors.enabled
    }
    fn purge_errors_turns(&self) -> u32 {
        self.strategies.purge_errors.turns
    }
    fn purge_errors_protected_tools(&self) -> &ToolProtection {
        &self.cached.purge_errors
    }
    fn stale_file_reads_enabled(&self) -> bool {
        self.strategies.stale_file_reads.enabled
    }
    fn stale_file_reads_tracked_tools(&self) -> &[String] {
        &self.strategies.stale_file_reads.tracked_tools
    }
    fn stale_file_reads_protected_tools(&self) -> &ToolProtection {
        &self.cached.stale_file_reads
    }
    fn protected_paths(&self) -> &PathProtection {
        &self.cached.paths
    }
    fn manual_mode_enabled(&self) -> bool {
        self.manual_mode.enabled
    }
    fn manual_mode_automatic_strategies(&self) -> bool {
        self.manual_mode.automatic_strategies
    }
}

impl dcp_compress::CompressConfig for Config {
    fn max_summary_chars(&self) -> usize {
        self.compress.max_summary_chars as usize
    }
    fn protected_tools(&self) -> &ToolProtection {
        &self.cached.compress
    }
    fn protect_user_messages(&self) -> bool {
        self.compress.protect_user_messages
    }
    fn show_compression(&self) -> bool {
        self.compress.show_compression
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::LimitValue;
    use dcp_state::ConfigLike;

    #[test]
    fn default_matches_spec_field_by_field() {
        let c = Config::default();
        assert!(c.enabled);
        assert!(!c.debug);
        assert_eq!(c.cache_stability_mode, CacheStabilityMode::AgentMessage);
        assert_eq!(c.notification, NotificationConfig::default());
        assert_eq!(c.manual_mode, ManualModeConfig::default());
        assert_eq!(c.turn_protection, TurnProtectionConfig::default());
        assert!(c.protected_file_patterns.is_empty());
        assert_eq!(c.compress, CompressConfig::default());
        assert_eq!(c.strategies, StrategiesConfig::default());
        assert_eq!(c.commands, CommandsConfig::default());
        assert_eq!(c.experimental, ExperimentalConfig::default());
    }

    #[test]
    fn default_is_valid() {
        let c = Config::default();
        c.validate().unwrap();
    }

    #[test]
    fn config_serde_camel_case_keys() {
        let c = Config::default();
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"cacheStabilityMode\""));
        assert!(s.contains("\"protectedFilePatterns\""));
        assert!(s.contains("\"manualMode\""));
        assert!(s.contains("\"turnProtection\""));
        assert!(s.contains("\"agent-message\""));
    }

    #[test]
    fn config_serde_round_trip() {
        let c = Config::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let c: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn config_like_accessors_match_spec() {
        let c = Config::default();
        assert!(c.is_tracked_tool("read"));
        assert!(c.is_tracked_tool("multiedit"));
        assert!(!c.is_tracked_tool("bash"));
        assert_eq!(c.protected_tools(), ["task", "skill"]);
        assert!(!c.turn_protection_enabled());
        assert_eq!(c.turn_protection_turns(), 4);
    }

    #[test]
    fn prune_config_accessors_match_spec() {
        use dcp_prune::PruneConfig;
        let c = Config::default();
        assert!(c.dedup_enabled());
        assert!(c.purge_errors_enabled());
        assert!(c.stale_file_reads_enabled());
        assert_eq!(c.purge_errors_turns(), 4);
        assert_eq!(
            c.stale_file_reads_tracked_tools(),
            ["read", "write", "edit", "multiedit"]
        );
        assert!(!c.manual_mode_enabled());
        assert!(c.manual_mode_automatic_strategies());
    }

    #[test]
    fn compress_config_accessors_match_spec() {
        use dcp_compress::CompressConfig as CC;
        let c = Config::default();
        assert_eq!(CC::max_summary_chars(&c), 32_768);
        assert!(CC::protected_tools(&c).is_protected("task"));
        assert!(CC::protected_tools(&c).is_protected("skill"));
        assert!(!CC::protect_user_messages(&c));
        assert!(!CC::show_compression(&c));
    }

    #[test]
    fn rebuild_cache_compiles_globs() {
        let mut c = Config {
            protected_file_patterns: vec!["**/*.config.ts".into(), "Cargo.toml".into()],
            ..Config::default()
        };
        c.rebuild_cache().unwrap();
        assert!(c.protected_paths().is_protected("Cargo.toml"));
        assert!(c.protected_paths().is_protected("src/x.config.ts"));
    }

    #[test]
    fn rebuild_cache_invalid_glob_is_error() {
        let mut c = Config {
            protected_file_patterns: vec!["src/[".into()],
            ..Config::default()
        };
        match c.rebuild_cache() {
            Err(ConfigError::InvalidGlob { pattern, .. }) => assert_eq!(pattern, "src/["),
            other => panic!("expected InvalidGlob, got {other:?}"),
        }
    }

    // ----- validation -----

    #[test]
    fn validate_rejects_max_context_limit_too_small() {
        let mut c = Config::default();
        c.compress.max_context_limit = LimitValue::Number(500);
        // minContextLimit is also 50000 default — reduce to keep
        // the failure path on the "too small" rule.
        c.compress.min_context_limit = LimitValue::Number(100);
        let err = c.validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::Validation(m) if m.contains("maxContextLimit too small"))
        );
    }

    #[test]
    fn validate_rejects_min_geq_max() {
        let mut c = Config::default();
        c.compress.min_context_limit = LimitValue::Number(200_000);
        let err = c.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(m) if m.contains("minContextLimit must be")));
    }

    #[test]
    fn validate_rejects_nudge_frequency_out_of_range() {
        let mut c = Config::default();
        c.compress.nudge_frequency = 0;
        let err = c.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(m) if m.contains("nudgeFrequency")));

        let mut c = Config::default();
        c.compress.nudge_frequency = 101;
        let err = c.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(m) if m.contains("nudgeFrequency")));
    }

    #[test]
    fn validate_rejects_iteration_threshold_below_2() {
        let mut c = Config::default();
        c.compress.iteration_nudge_threshold = 1;
        let err = c.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(m) if m.contains("iterationNudgeThreshold")));
    }

    #[test]
    fn validate_rejects_max_summary_chars_below_1024() {
        let mut c = Config::default();
        c.compress.max_summary_chars = 100;
        let err = c.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(m) if m.contains("maxSummaryChars")));
    }

    #[test]
    fn validate_rejects_purge_errors_turns_zero() {
        let mut c = Config::default();
        c.strategies.purge_errors.turns = 0;
        let err = c.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(m) if m.contains("purgeErrors.turns")));
    }

    #[test]
    fn validate_rejects_empty_tracked_tools_when_enabled() {
        let mut c = Config::default();
        c.strategies.stale_file_reads.tracked_tools.clear();
        let err = c.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(m) if m.contains("trackedTools")));
    }

    #[test]
    fn validate_allows_empty_tracked_tools_when_disabled() {
        let mut c = Config::default();
        c.strategies.stale_file_reads.enabled = false;
        c.strategies.stale_file_reads.tracked_tools.clear();
        c.validate().unwrap();
    }

    #[test]
    fn validate_rejects_turn_protection_over_100() {
        let mut c = Config::default();
        c.turn_protection.turns = 101;
        let err = c.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(m) if m.contains("turnProtection.turns")));
    }

    #[test]
    fn validate_rejects_invalid_glob() {
        let mut c = Config {
            protected_file_patterns: vec!["src/[".into()],
            ..Config::default()
        };
        // rebuild_cache surfaces the same error before validate even
        // runs — guard against either signalling style.
        match c.rebuild_cache() {
            Err(ConfigError::InvalidGlob { .. }) => {}
            Ok(()) => match c.validate() {
                Err(ConfigError::InvalidGlob { .. }) => {}
                other => panic!("expected InvalidGlob, got {other:?}"),
            },
            other => panic!("expected InvalidGlob, got {other:?}"),
        }
    }

    // ----- schema generation -----

    #[test]
    fn schema_generation_produces_object_schema() {
        let v = json_schema();
        let obj = v.as_object().expect("schema must be an object");
        assert!(obj.get("$schema").and_then(|s| s.as_str()).is_some());
        assert_eq!(
            obj.get("title").and_then(|s| s.as_str()),
            Some("dynamic_context_pruning configuration")
        );
        // Top-level type should be "object".
        assert_eq!(obj.get("type").and_then(|s| s.as_str()), Some("object"));
        // Properties block should at least mention the documented keys.
        let props = obj.get("properties").and_then(|p| p.as_object()).unwrap();
        for key in [
            "enabled",
            "debug",
            "cacheStabilityMode",
            "notification",
            "manualMode",
            "turnProtection",
            "protectedFilePatterns",
            "compress",
            "strategies",
            "commands",
            "experimental",
        ] {
            assert!(props.contains_key(key), "missing property {key}");
        }
    }

    #[test]
    fn schema_is_valid_json() {
        let v = json_schema();
        let s = serde_json::to_string(&v).unwrap();
        let _: serde_json::Value = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn programmatic_mutation_then_validate() {
        let mut c = Config {
            protected_file_patterns: vec!["**/*.lock".into()],
            ..Config::default()
        };
        c.compress.protected_tools.push("custom".into());
        c.rebuild_cache().unwrap();
        c.validate().unwrap();
        assert!(c.compress_protected_tools().is_protected("custom"));
        assert!(c.protected_paths().is_protected("Cargo.lock"));
    }
}
