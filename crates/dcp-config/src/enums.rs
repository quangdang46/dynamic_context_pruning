//! Top-level enum types for the configuration schema (SPEC.md §10.2).
//!
//! These mirror the JSONC surface exactly: every variant name maps to
//! the lowercase / kebab-case form documented in SPEC.md §10.2.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// Re-exports of types owned by other crates so callers can reach them
// through the canonical configuration API.
pub use dcp_prompts::NudgeForce;
pub use dcp_prune::CacheStabilityMode;

/// How the active `compress` tool operates (SPEC.md §10.2:
/// `compress.mode`).
///
/// `Range` covers a contiguous span of messages; `Message` covers an
/// individual non-contiguous message. The variants intentionally match
/// [`dcp_types::CompressionMode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CompressMode {
    /// A contiguous range of messages.
    #[default]
    Range,
    /// A single message.
    Message,
}

impl From<CompressMode> for dcp_types::CompressionMode {
    fn from(value: CompressMode) -> Self {
        match value {
            CompressMode::Range => Self::Range,
            CompressMode::Message => Self::Message,
        }
    }
}

impl From<dcp_types::CompressionMode> for CompressMode {
    fn from(value: dcp_types::CompressionMode) -> Self {
        match value {
            dcp_types::CompressionMode::Range => Self::Range,
            dcp_types::CompressionMode::Message => Self::Message,
            _ => Self::Range,
        }
    }
}

/// Whether the host has granted permission to run the `compress` tool
/// (SPEC.md §10.2: `compress.permission`).
///
/// Variant names match [`dcp_types::CompressPermission`]; `Allow` is the
/// default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    /// The host must be prompted before each compression.
    Ask,
    /// Compression may run without further consent.
    #[default]
    Allow,
    /// Compression is not allowed.
    Deny,
}

impl From<Permission> for dcp_types::CompressPermission {
    fn from(value: Permission) -> Self {
        match value {
            Permission::Ask => Self::Ask,
            Permission::Allow => Self::Allow,
            Permission::Deny => Self::Deny,
        }
    }
}

impl From<dcp_types::CompressPermission> for Permission {
    fn from(value: dcp_types::CompressPermission) -> Self {
        match value {
            dcp_types::CompressPermission::Ask => Self::Ask,
            dcp_types::CompressPermission::Allow => Self::Allow,
            dcp_types::CompressPermission::Deny => Self::Deny,
        }
    }
}

/// How rendered nudge text is glued onto the anchor message
/// (SPEC.md §10.2, §8 — see also [`dcp_nudges::InjectionMode`]).
///
/// Two modes are defined in SPEC.md §8:
///
/// * `wrap_block` — wrap in `<dcp-nudge> … </dcp-nudge>` tags then append (default).
/// * `append_text` — append raw nudge text after the existing body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum InjectionMode {
    /// Wrap the nudge in `<dcp-nudge> … </dcp-nudge>` tags and append
    /// after the existing body. Default.
    #[default]
    #[serde(rename = "wrap_block")]
    WrapBlock,
    /// Append the rendered nudge after the existing body.
    #[serde(rename = "append_text")]
    AppendText,
}

impl From<InjectionMode> for dcp_nudges::InjectionMode {
    fn from(value: InjectionMode) -> Self {
        match value {
            InjectionMode::WrapBlock => Self::WrapBlock,
            InjectionMode::AppendText => Self::AppendText,
        }
    }
}

impl From<dcp_nudges::InjectionMode> for InjectionMode {
    fn from(value: dcp_nudges::InjectionMode) -> Self {
        match value {
            dcp_nudges::InjectionMode::WrapBlock => Self::WrapBlock,
            dcp_nudges::InjectionMode::AppendText => Self::AppendText,
        }
    }
}

/// Verbosity level of host-facing notifications
/// (SPEC.md §10.2: `notification.level`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NotificationLevel {
    /// Notifications are suppressed entirely.
    Off,
    /// One-line summaries only.
    Minimal,
    /// Full structured notifications (default).
    #[default]
    Detailed,
}

/// Surface where notifications are rendered
/// (SPEC.md §10.2: `notification.kind`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NotificationKind {
    /// Inline chat notification (default).
    #[default]
    Chat,
    /// Out-of-band toast / popup.
    Toast,
}

/// Tokenizer selection (SPEC.md §10.2 -- `tokenizer.kind`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum TokenizerKind {
    /// Fast heuristic: `chars / 4` (default). No external deps.
    #[default]
    #[serde(rename = "chars_div_4")]
    CharsDiv4,
    /// OpenAI `tiktoken` (requires `tiktoken-rs` feature).
    #[serde(rename = "tiktoken")]
    Tiktoken,
    /// HuggingFace tokenizer (requires `tokenizers` feature).
    #[serde(rename = "hf")]
    Hf,
    /// Anthropic Claude tokenizer (requires `claude` feature).
    #[serde(rename = "claude")]
    Claude,
    /// User-supplied tokenizer via the [`Tokenizer`] trait.
    #[serde(rename = "custom")]
    Custom,
}

// ============================================================================
// Schema phantom for `CacheStabilityMode`
// ============================================================================
//
// `CacheStabilityMode` lives in `dcp-prune`, which does not depend on
// `schemars`. This phantom enum has identical serde shape and is used
// only when generating the JSON schema; serde still uses the real type.

/// Internal phantom type used purely by [`schemars::JsonSchema`] when
/// generating the JSON schema for a [`crate::Config`] field of type
/// [`CacheStabilityMode`]. Callers should never use this type directly.
#[doc(hidden)]
#[derive(JsonSchema)]
#[serde(rename_all = "kebab-case", rename = "CacheStabilityMode")]
#[allow(dead_code)]
pub enum CacheStabilityModeSchema {
    /// Apply on every transform — debug-only.
    Aggressive,
    /// Apply at turn boundaries — default.
    AgentMessage,
    /// Apply only when the host calls `force_apply()`.
    Manual,
}

// ============================================================================
// Schema phantom for `NudgeForce`
// ============================================================================
//
// `NudgeForce` lives in `dcp-prompts`, which does not depend on
// `schemars`. Same trick as for `CacheStabilityMode`.

/// Internal phantom type used purely by [`schemars::JsonSchema`] when
/// generating the JSON schema for a [`crate::CompressConfig`] field of
/// type [`NudgeForce`]. Callers should never use this type directly.
#[doc(hidden)]
#[derive(JsonSchema)]
#[serde(rename_all = "lowercase", rename = "NudgeForce")]
#[allow(dead_code)]
pub enum NudgeForceSchema {
    /// Strong nudge tone.
    Strong,
    /// Soft nudge tone (default).
    Soft,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_mode_serde_lowercase() {
        assert_eq!(
            serde_json::to_string(&CompressMode::Range).unwrap(),
            "\"range\""
        );
        assert_eq!(
            serde_json::to_string(&CompressMode::Message).unwrap(),
            "\"message\""
        );
    }

    #[test]
    fn permission_serde_lowercase() {
        assert_eq!(serde_json::to_string(&Permission::Ask).unwrap(), "\"ask\"");
        assert_eq!(
            serde_json::to_string(&Permission::Allow).unwrap(),
            "\"allow\""
        );
        assert_eq!(
            serde_json::to_string(&Permission::Deny).unwrap(),
            "\"deny\""
        );
    }

    #[test]
    fn injection_mode_serde_spec_names() {
        assert_eq!(
            serde_json::to_string(&InjectionMode::WrapBlock).unwrap(),
            "\"wrap_block\""
        );
        assert_eq!(
            serde_json::to_string(&InjectionMode::AppendText).unwrap(),
            "\"append_text\""
        );
    }

    #[test]
    fn notification_level_serde_lowercase() {
        for (lvl, expected) in [
            (NotificationLevel::Off, "\"off\""),
            (NotificationLevel::Minimal, "\"minimal\""),
            (NotificationLevel::Detailed, "\"detailed\""),
        ] {
            assert_eq!(serde_json::to_string(&lvl).unwrap(), expected);
        }
    }

    #[test]
    fn notification_kind_serde_lowercase() {
        for (kind, expected) in [
            (NotificationKind::Chat, "\"chat\""),
            (NotificationKind::Toast, "\"toast\""),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), expected);
        }
    }

    #[test]
    fn defaults_match_spec() {
        assert_eq!(CompressMode::default(), CompressMode::Range);
        assert_eq!(Permission::default(), Permission::Allow);
        assert_eq!(InjectionMode::default(), InjectionMode::WrapBlock);
        assert_eq!(NotificationLevel::default(), NotificationLevel::Detailed);
        assert_eq!(NotificationKind::default(), NotificationKind::Chat);
    }

    #[test]
    fn tokenizer_kind_default() {
        assert_eq!(TokenizerKind::default(), TokenizerKind::CharsDiv4);
    }

    #[test]
    fn tokenizer_kind_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&TokenizerKind::CharsDiv4).unwrap(),
            "\"chars_div_4\""
        );
        assert_eq!(
            serde_json::to_string(&TokenizerKind::Tiktoken).unwrap(),
            "\"tiktoken\""
        );
        assert_eq!(serde_json::to_string(&TokenizerKind::Hf).unwrap(), "\"hf\"");
        assert_eq!(
            serde_json::to_string(&TokenizerKind::Claude).unwrap(),
            "\"claude\""
        );
        assert_eq!(
            serde_json::to_string(&TokenizerKind::Custom).unwrap(),
            "\"custom\""
        );
    }

    #[test]
    fn cache_stability_mode_reexport_default() {
        assert_eq!(
            CacheStabilityMode::default(),
            CacheStabilityMode::AgentMessage
        );
    }

    #[test]
    fn nudge_force_reexport_default() {
        assert_eq!(NudgeForce::default(), NudgeForce::Soft);
    }

    #[test]
    fn compress_mode_into_compression_mode() {
        let m: dcp_types::CompressionMode = CompressMode::Range.into();
        assert_eq!(m, dcp_types::CompressionMode::Range);
    }

    #[test]
    fn permission_into_compress_permission() {
        let p: dcp_types::CompressPermission = Permission::Allow.into();
        assert_eq!(p, dcp_types::CompressPermission::Allow);
    }
}
