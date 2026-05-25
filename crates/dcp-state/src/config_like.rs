//! [`ConfigLike`] — the slice of configuration this crate needs.
//!
//! `dcp-state` deliberately does *not* depend on `dcp-config` (PLAN.md §5.2
//! locks the dependency graph: `dcp-state` is a *consumer* of types and a
//! *producer* of state transitions; the canonical [`Config`] type lives in
//! a sibling crate that depends on both). Instead, this crate defines a
//! narrow trait that any caller can implement — typically by forwarding to
//! its real config.
//!
//! Test code can satisfy the trait with a tiny [`StaticConfigLike`] value.
//!
//! # Surface
//!
//! Only the fields actually consumed inside `dcp-state` appear here. New
//! algorithms that need additional config bits should extend the trait
//! rather than depend on the concrete `dcp-config::Config`.

/// A read-only view over the config knobs `dcp-state` needs.
///
/// Implementations are expected to be cheap; every method may be called
/// many times per `transform_messages` invocation. The trait is
/// dyn-compatible (object-safe) so callers may store
/// `&dyn ConfigLike` if convenient.
pub trait ConfigLike {
    /// True when `name` is in the `staleFileReads.trackedTools` list, i.e.
    /// the library should attempt to extract file paths from that tool's
    /// parameters (SPEC.md §4.6).
    fn is_tracked_tool(&self, name: &str) -> bool;

    /// Tool names whose verbatim output is preserved when their range is
    /// compressed. Mirrors `compress.protectedTools` (SPEC.md §10.2).
    fn protected_tools(&self) -> &[String];

    /// True when `turnProtection.enabled` is set.
    fn turn_protection_enabled(&self) -> bool;

    /// Number of recent turns shielded from compression suggestions when
    /// turn-protection is enabled (`turnProtection.turns`).
    fn turn_protection_turns(&self) -> u32;

    /// Object keys whose value is JSON `null` should be dropped during
    /// parameter normalization (SPEC.md §4.4 step 3a). The default empty
    /// list keeps `null` values, matching the spec's default behavior.
    fn drop_null_keys(&self) -> &[String] {
        &[]
    }
}

/// Plain-data implementation of [`ConfigLike`] useful for tests and for
/// callers that only need a transient config slice.
///
/// # Example
///
/// ```rust
/// use dcp_state::config_like::{ConfigLike, StaticConfigLike};
///
/// let c = StaticConfigLike {
///     tracked_tools: vec!["read".into(), "write".into()],
///     ..StaticConfigLike::default()
/// };
/// assert!(c.is_tracked_tool("read"));
/// assert!(!c.is_tracked_tool("bash"));
/// ```
#[derive(Clone, Debug, Default)]
pub struct StaticConfigLike {
    /// Tool names that should have file-path extraction performed.
    pub tracked_tools: Vec<String>,
    /// `compress.protectedTools` mirror.
    pub protected_tools: Vec<String>,
    /// `turnProtection.enabled` mirror.
    pub turn_protection_enabled: bool,
    /// `turnProtection.turns` mirror.
    pub turn_protection_turns: u32,
    /// `drop_null_keys` mirror; usually empty.
    pub drop_null_keys: Vec<String>,
}

impl ConfigLike for StaticConfigLike {
    fn is_tracked_tool(&self, name: &str) -> bool {
        self.tracked_tools.iter().any(|t| t == name)
    }

    fn protected_tools(&self) -> &[String] {
        &self.protected_tools
    }

    fn turn_protection_enabled(&self) -> bool {
        self.turn_protection_enabled
    }

    fn turn_protection_turns(&self) -> u32 {
        self.turn_protection_turns
    }

    fn drop_null_keys(&self) -> &[String] {
        &self.drop_null_keys
    }
}

/// Default tracked-tools list per SPEC.md §10.2 (`staleFileReads.trackedTools`
/// default).
///
/// Convenience for tests; production callers should source the value from
/// the real config.
pub fn default_tracked_tools() -> Vec<String> {
    vec![
        "read".into(),
        "write".into(),
        "edit".into(),
        "multiedit".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_config_like_round_trip() {
        let c = StaticConfigLike {
            tracked_tools: default_tracked_tools(),
            protected_tools: vec!["task".into(), "skill".into()],
            turn_protection_enabled: true,
            turn_protection_turns: 5,
            drop_null_keys: vec!["null_field".into()],
        };
        assert!(c.is_tracked_tool("read"));
        assert!(c.is_tracked_tool("multiedit"));
        assert!(!c.is_tracked_tool("bash"));
        assert_eq!(c.protected_tools(), ["task", "skill"]);
        assert!(c.turn_protection_enabled());
        assert_eq!(c.turn_protection_turns(), 5);
        assert_eq!(c.drop_null_keys(), ["null_field"]);
    }

    #[test]
    fn default_static_config_like_is_empty() {
        let c = StaticConfigLike::default();
        assert!(!c.is_tracked_tool("read"));
        assert!(c.protected_tools().is_empty());
        assert!(!c.turn_protection_enabled());
        assert_eq!(c.turn_protection_turns(), 0);
        assert!(c.drop_null_keys().is_empty());
    }

    /// Compile-time confirmation the trait is dyn-compatible.
    #[test]
    fn config_like_is_object_safe() {
        let c: Box<dyn ConfigLike> = Box::new(StaticConfigLike::default());
        assert!(!c.is_tracked_tool("read"));
    }
}
