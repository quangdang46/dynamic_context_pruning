//! [`CacheStabilityMode`] — the gate that decides when accumulated
//! prune decisions reach the outgoing message stream (SPEC.md §7).
//!
//! Strategies in this crate always *compute* their decisions on every
//! transform invocation. Whether those decisions are *applied* —
//! materialised into the outgoing `Vec<Message>` via
//! [`crate::apply::apply_prune_to_messages`] — is gated by the
//! [`CacheStabilityMode`] selected by the host. The trade-off is
//! freshness vs. prompt-cache stability: applying every transform busts
//! the LLM provider's prefix cache; applying only at safe boundaries
//! preserves it (SPEC.md §7.1).
//!
//! The enum lives in `dcp-prune` because the gating logic and the apply
//! phase are owned here. `dcp-config` re-exports it so hosts can read
//! it through the canonical configuration surface without depending on
//! `dcp-prune` directly.

use serde::{Deserialize, Serialize};

/// When the apply phase materialises pending prune decisions.
///
/// SPEC.md §7.1 — these three values are exhaustive. The default is
/// [`CacheStabilityMode::AgentMessage`], which applies pruning at turn
/// boundaries (i.e. after the assistant produces its final text turn).
///
/// Serialisation uses kebab-case to match the JSONC config surface
/// (SPEC.md §10.2): `"aggressive"`, `"agent-message"`, `"manual"`.
///
/// # Example
///
/// ```rust
/// use dcp_prune::CacheStabilityMode;
/// assert_eq!(CacheStabilityMode::default(), CacheStabilityMode::AgentMessage);
/// let s = serde_json::to_string(&CacheStabilityMode::AgentMessage).unwrap();
/// assert_eq!(s, "\"agent-message\"");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheStabilityMode {
    /// Apply every transform — maximum freshness, minimum cache hits.
    /// Intended for development and debugging.
    Aggressive,
    /// Apply at turn boundaries — the default. Pruning decisions
    /// accumulate during tool turns and flush once the assistant emits
    /// a final text response.
    #[default]
    AgentMessage,
    /// Never apply automatically — the host triggers application via
    /// `force_apply()`. Strategies still compute and accumulate.
    Manual,
}

impl CacheStabilityMode {
    /// Stable string identifier matching the JSONC config form.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_prune::CacheStabilityMode;
    /// assert_eq!(CacheStabilityMode::AgentMessage.as_str(), "agent-message");
    /// assert_eq!(CacheStabilityMode::Aggressive.as_str(), "aggressive");
    /// assert_eq!(CacheStabilityMode::Manual.as_str(), "manual");
    /// ```
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aggressive => "aggressive",
            Self::AgentMessage => "agent-message",
            Self::Manual => "manual",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_agent_message() {
        assert_eq!(
            CacheStabilityMode::default(),
            CacheStabilityMode::AgentMessage
        );
    }

    #[test]
    fn serde_kebab_case_roundtrip() {
        for (mode, expected) in [
            (CacheStabilityMode::Aggressive, "\"aggressive\""),
            (CacheStabilityMode::AgentMessage, "\"agent-message\""),
            (CacheStabilityMode::Manual, "\"manual\""),
        ] {
            let s = serde_json::to_string(&mode).unwrap();
            assert_eq!(s, expected);
            let back: CacheStabilityMode = serde_json::from_str(&s).unwrap();
            assert_eq!(back, mode);
        }
    }

    #[test]
    fn as_str_matches_serde_form() {
        for mode in [
            CacheStabilityMode::Aggressive,
            CacheStabilityMode::AgentMessage,
            CacheStabilityMode::Manual,
        ] {
            let s = format!("\"{}\"", mode.as_str());
            let parsed: CacheStabilityMode = serde_json::from_str(&s).unwrap();
            assert_eq!(parsed, mode);
        }
    }
}
