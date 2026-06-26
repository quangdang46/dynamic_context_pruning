//! Sub-config structs that mirror SPEC.md §10.2 exactly.
//!
//! Each struct uses `#[serde(rename_all = "camelCase", default)]` so the
//! JSONC keys round-trip without explicit renames per field, and missing
//! fields fall back to the spec's documented defaults.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::enums::{
    CompressMode, NotificationKind, NotificationLevel, NudgeForce, NudgeForceSchema, Permission,
    TokenizerKind,
};
use crate::limits::LimitValue;

// ----------------------------------------------------------------------------
// notification
// ----------------------------------------------------------------------------

/// Host-facing notification preferences (SPEC.md §10.2 — `notification`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationConfig {
    /// Verbosity (`"off" | "minimal" | "detailed"`, default `detailed`).
    pub level: NotificationLevel,
    /// Render surface (`"chat" | "toast"`, default `chat`).
    pub kind: NotificationKind,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            level: NotificationLevel::Detailed,
            kind: NotificationKind::Chat,
        }
    }
}

// ----------------------------------------------------------------------------
// manualMode
// ----------------------------------------------------------------------------

/// Manual mode flags (SPEC.md §10.2 — `manualMode`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ManualModeConfig {
    /// When `true`, the host drives the library through slash commands.
    pub enabled: bool,
    /// When `false` and `enabled == true`, automatic strategies are
    /// suspended.
    pub automatic_strategies: bool,
}

impl Default for ManualModeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            automatic_strategies: true,
        }
    }
}

// ----------------------------------------------------------------------------
// turnProtection
// ----------------------------------------------------------------------------

/// Turn-protection knobs (SPEC.md §10.2 — `turnProtection`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TurnProtectionConfig {
    /// Reserved master switch.
    pub enabled: bool,
    /// Number of recent turns shielded from compression suggestions
    /// when `enabled == true`. Range: `0..=100`, default `4`.
    pub turns: u32,
}

impl Default for TurnProtectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            turns: 4,
        }
    }
}

// ----------------------------------------------------------------------------
// compress
// ----------------------------------------------------------------------------

/// Configuration for the `compress` tool (SPEC.md §10.2 — `compress`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CompressConfig {
    /// Active mode (`"range" | "message"`, default `range`).
    pub mode: CompressMode,
    /// Permission gating (`"ask" | "allow" | "deny"`, default `allow`).
    pub permission: Permission,
    /// When `true`, render `<dcp-block>` wrapper comments for human
    /// inspection.
    pub show_compression: bool,
    /// When `true`, summaries are buffered through the prompt-engineering
    /// append step.
    pub summary_buffer: bool,
    /// Resolved or raw maximum context limit (number or `"X%"`).
    pub max_context_limit: LimitValue,
    /// Resolved or raw minimum context limit.
    pub min_context_limit: LimitValue,
    /// Per-model overrides for `maxContextLimit`.
    pub model_max_limits: BTreeMap<String, LimitValue>,
    /// Per-model overrides for `minContextLimit`.
    pub model_min_limits: BTreeMap<String, LimitValue>,
    /// Re-fire frequency for context-limit nudges. Range: `1..=100`,
    /// default `5`.
    pub nudge_frequency: u32,
    /// Iteration nudge threshold. Range: `2..=100`, default `15`.
    pub iteration_nudge_threshold: u32,
    /// Tone of nudge text (`"strong" | "soft"`, default `soft`).
    #[schemars(with = "NudgeForceSchema")]
    pub nudge_force: NudgeForce,
    /// Tool names whose verbatim output is appended in
    /// `<dcp-protected-tools>` when their range is compressed.
    pub protected_tools: Vec<String>,
    /// When `true`, text inside `<dcp-protected> … </dcp-protected>`
    /// is preserved verbatim across compression.
    pub protect_tags: bool,
    /// When `true`, user messages are protected from compression.
    pub protect_user_messages: bool,
    /// Per-entry summary length cap. Range: `1024..=262144`, default
    /// `32768`.
    pub max_summary_chars: u32,
}

impl Default for CompressConfig {
    fn default() -> Self {
        Self {
            mode: CompressMode::Range,
            permission: Permission::Allow,
            show_compression: false,
            summary_buffer: true,
            max_context_limit: LimitValue::Number(100_000),
            min_context_limit: LimitValue::Number(50_000),
            model_max_limits: BTreeMap::new(),
            model_min_limits: BTreeMap::new(),
            nudge_frequency: 5,
            iteration_nudge_threshold: 15,
            nudge_force: NudgeForce::Soft,
            protected_tools: vec!["task".into(), "skill".into()],
            protect_tags: false,
            protect_user_messages: false,
            max_summary_chars: 32_768,
        }
    }
}

// ----------------------------------------------------------------------------
// strategies
// ----------------------------------------------------------------------------

/// `strategies.deduplication` (SPEC.md §10.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct DeduplicationConfig {
    /// Master switch for the deduplicate strategy.
    pub enabled: bool,
    /// Tools the deduplicate strategy must skip.
    pub protected_tools: Vec<String>,
}

impl Default for DeduplicationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            protected_tools: Vec::new(),
        }
    }
}

/// `strategies.purgeErrors` (SPEC.md §10.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct PurgeErrorsConfig {
    /// Master switch for the purge-errors strategy.
    pub enabled: bool,
    /// Turn-age threshold. Range: `1..=100`, default `4`.
    pub turns: u32,
    /// Tools the purge-errors strategy must skip.
    pub protected_tools: Vec<String>,
}

impl Default for PurgeErrorsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            turns: 4,
            protected_tools: Vec::new(),
        }
    }
}

/// `strategies.staleFileReads` (SPEC.md §10.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct StaleFileReadsConfig {
    /// Master switch for the stale-file-reads strategy.
    pub enabled: bool,
    /// Tools the stale-file-reads strategy must skip.
    pub protected_tools: Vec<String>,
    /// Tool names tracked by stale-file-reads. Default
    /// `["read","write","edit","multiedit"]`.
    pub tracked_tools: Vec<String>,
}

impl Default for StaleFileReadsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            protected_tools: Vec::new(),
            tracked_tools: vec![
                "read".into(),
                "write".into(),
                "edit".into(),
                "multiedit".into(),
            ],
        }
    }
}

/// `strategies` (SPEC.md §10.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct StrategiesConfig {
    /// Deduplication strategy knobs.
    pub deduplication: DeduplicationConfig,
    /// Purge-errors strategy knobs.
    pub purge_errors: PurgeErrorsConfig,
    /// Stale-file-reads strategy knobs.
    pub stale_file_reads: StaleFileReadsConfig,
}

// ----------------------------------------------------------------------------
// commands
// ----------------------------------------------------------------------------

/// Slash-command surface configuration (SPEC.md §10.2 — `commands`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CommandsConfig {
    /// Master switch for the `/dcp …` slash-command surface.
    pub enabled: bool,
    /// Tools the `/dcp sweep` command pre-protects.
    pub protected_tools: Vec<String>,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            protected_tools: Vec::new(),
        }
    }
}

// ----------------------------------------------------------------------------
// experimental
// ----------------------------------------------------------------------------

/// Experimental feature flags (SPEC.md §10.2 — `experimental`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ExperimentalConfig {
    /// Allow experimental sub-agent inlining (SPEC.md §11.6).
    pub allow_subagents: bool,
    /// Allow programmatic prompt overrides.
    pub custom_prompts: bool,
}
// ----------------------------------------------------------------------------
// persistence
// ----------------------------------------------------------------------------

/// Persistence configuration (SPEC.md §9.3 — `persistence`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct PersistenceConfig {
    /// Master switch for state persistence. When `false`, the ephemeral
    /// in-memory [`NoopStorage`] backend is used (SPEC.md §9.3).
    pub enabled: bool,
    /// Interval in seconds between automatic persistence saves. `0`
    /// disables auto-save (SPEC.md §9.3). Range: `0..=3600`.
    pub auto_save_seconds: u32,
    /// When `true`, each save copies the previous `<session>.json` to
    /// `<session>.json.bak` before writing the new file (SPEC.md §9.3).
    pub keep_backup: bool,
    /// Optional custom storage directory. When `None`, the platform's
    /// default data directory is used (e.g. `~/.local/share/...` on Linux,
    /// `~/Library/Application Support/...` on macOS, `%APPDATA%/...` on
    /// Windows).
    pub path: Option<String>,
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_save_seconds: 0,
            keep_backup: true,
            path: None,
        }
    }
}

// ----------------------------------------------------------------------------
// tokenizer
// ----------------------------------------------------------------------------

/// Tokenizer selection and image-cost configuration (SPEC.md §10.2 -- `tokenizer`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TokenizerConfig {
    /// Which tokenizer implementation to use.
    pub kind: TokenizerKind,
    /// Optional model identifier (used by some tokenizer backends).
    pub model: Option<String>,
    /// Estimated token cost per image. Range: `1..=10000`, default `1500`.
    pub image_tokens: u32,
}

impl Default for TokenizerConfig {
    fn default() -> Self {
        Self {
            kind: TokenizerKind::CharsDiv4,
            model: None,
            image_tokens: 1500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_default_matches_spec() {
        let n = NotificationConfig::default();
        assert_eq!(n.level, NotificationLevel::Detailed);
        assert_eq!(n.kind, NotificationKind::Chat);
    }

    #[test]
    fn manual_mode_default_matches_spec() {
        let m = ManualModeConfig::default();
        assert!(!m.enabled);
        assert!(m.automatic_strategies);
    }

    #[test]
    fn turn_protection_default_matches_spec() {
        let t = TurnProtectionConfig::default();
        assert!(!t.enabled);
        assert_eq!(t.turns, 4);
    }

    #[test]
    fn compress_default_matches_spec() {
        let c = CompressConfig::default();
        assert_eq!(c.mode, CompressMode::Range);
        assert_eq!(c.permission, Permission::Allow);
        assert!(!c.show_compression);
        assert!(c.summary_buffer);
        assert_eq!(c.max_context_limit, LimitValue::Number(100_000));
        assert_eq!(c.min_context_limit, LimitValue::Number(50_000));
        assert!(c.model_max_limits.is_empty());
        assert!(c.model_min_limits.is_empty());
        assert_eq!(c.nudge_frequency, 5);
        assert_eq!(c.iteration_nudge_threshold, 15);
        assert_eq!(c.nudge_force, NudgeForce::Soft);
        assert_eq!(c.protected_tools, vec!["task".to_string(), "skill".into()]);
        assert!(!c.protect_tags);
        assert!(!c.protect_user_messages);
        assert_eq!(c.max_summary_chars, 32_768);
    }

    #[test]
    fn strategies_defaults_match_spec() {
        let s = StrategiesConfig::default();
        assert!(s.deduplication.enabled);
        assert!(s.deduplication.protected_tools.is_empty());
        assert!(s.purge_errors.enabled);
        assert_eq!(s.purge_errors.turns, 4);
        assert!(s.purge_errors.protected_tools.is_empty());
        assert!(s.stale_file_reads.enabled);
        assert_eq!(
            s.stale_file_reads.tracked_tools,
            vec![
                "read".to_string(),
                "write".into(),
                "edit".into(),
                "multiedit".into(),
            ]
        );
        assert!(s.stale_file_reads.protected_tools.is_empty());
    }

    #[test]
    fn commands_default_matches_spec() {
        let c = CommandsConfig::default();
        assert!(c.enabled);
        assert!(c.protected_tools.is_empty());
    }

    #[test]
    fn persistence_default_matches_spec() {
        let p = PersistenceConfig::default();
        assert!(p.enabled);
        assert_eq!(p.auto_save_seconds, 0);
        assert!(p.keep_backup);
        assert_eq!(p.path, None);
    }

    #[test]
    fn experimental_default_matches_spec() {
        let e = ExperimentalConfig::default();
        assert!(!e.allow_subagents);
        assert!(!e.custom_prompts);
    }

    #[test]
    fn tokenizer_default_matches_spec() {
        let t = TokenizerConfig::default();
        assert_eq!(t.kind, TokenizerKind::CharsDiv4);
        assert_eq!(t.model, None);
        assert_eq!(t.image_tokens, 1500);
    }

    #[test]
    fn tokenizer_config_camel_case_keys() {
        let t = TokenizerConfig::default();
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"imageTokens\""), "got: {s}");
    }

    #[test]
    fn tokenizer_config_missing_fields_fill_with_defaults() {
        let t: TokenizerConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(t, TokenizerConfig::default());
    }

    #[test]
    fn camel_case_keys_round_trip() {
        let m = ManualModeConfig {
            enabled: true,
            automatic_strategies: false,
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"automaticStrategies\""), "got: {s}");
        let back: ManualModeConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn missing_fields_fill_with_defaults() {
        let c: CompressConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c, CompressConfig::default());
    }
}
