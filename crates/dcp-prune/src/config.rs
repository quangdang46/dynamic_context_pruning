//! Configuration trait for the three deterministic prune strategies.
//!
//! As with [`dcp_state::ConfigLike`], `dcp-prune` does not depend on
//! `dcp-config`. Implementations of [`PruneConfig`] expose only the
//! fields the strategies and apply phase consume, mirroring the SPEC.md
//! §10.2 schema:
//!
//! * `strategies.deduplication.{enabled,protectedTools}`
//! * `strategies.purgeErrors.{enabled,turns,protectedTools}`
//! * `strategies.staleFileReads.{enabled,trackedTools,protectedTools}`
//! * `protectedFilePatterns`
//! * `manualMode.{enabled,automaticStrategies}`

use dcp_protected::{PathProtection, ToolProtection};

/// Plain-data trait abstracting the slice of config the prune strategies
/// consume. New algorithms should extend this trait rather than depend
/// on a concrete config type.
pub trait PruneConfig {
    /// Whether the deduplicate strategy is enabled
    /// (`strategies.deduplication.enabled`).
    fn dedup_enabled(&self) -> bool;

    /// Tools the deduplicate strategy must skip.
    fn dedup_protected_tools(&self) -> &ToolProtection;

    /// Whether the purge-errors strategy is enabled
    /// (`strategies.purgeErrors.enabled`).
    fn purge_errors_enabled(&self) -> bool;

    /// Turn-age threshold for the purge-errors strategy. SPEC.md §5.2
    /// clamps a configured `0` to `1`; that clamping happens inside the
    /// strategy, not here.
    fn purge_errors_turns(&self) -> u32;

    /// Tools the purge-errors strategy must skip.
    fn purge_errors_protected_tools(&self) -> &ToolProtection;

    /// Whether the stale-file-reads strategy is enabled
    /// (`strategies.staleFileReads.enabled`).
    fn stale_file_reads_enabled(&self) -> bool;

    /// Tool names tracked by stale-file-reads (default
    /// `["read","write","edit","multiedit"]`).
    fn stale_file_reads_tracked_tools(&self) -> &[String];

    /// Tools the stale-file-reads strategy must skip.
    fn stale_file_reads_protected_tools(&self) -> &ToolProtection;

    /// Project-wide protected file globs (`protectedFilePatterns`).
    fn protected_paths(&self) -> &PathProtection;

    /// Whether `manualMode.enabled` is set.
    fn manual_mode_enabled(&self) -> bool;

    /// Whether `manualMode.automaticStrategies` is set.
    fn manual_mode_automatic_strategies(&self) -> bool;
}

/// Plain-data implementation of [`PruneConfig`].
///
/// Used by tests and by callers that want a transient config slice. A
/// real config type from `dcp-config` will provide its own (likely
/// blanket) impl.
#[derive(Clone, Debug, Default)]
pub struct StaticPruneConfig {
    /// `strategies.deduplication.enabled`.
    pub dedup_enabled: bool,
    /// `strategies.deduplication.protectedTools`.
    pub dedup_protected_tools: ToolProtection,
    /// `strategies.purgeErrors.enabled`.
    pub purge_errors_enabled: bool,
    /// `strategies.purgeErrors.turns`.
    pub purge_errors_turns: u32,
    /// `strategies.purgeErrors.protectedTools`.
    pub purge_errors_protected_tools: ToolProtection,
    /// `strategies.staleFileReads.enabled`.
    pub stale_file_reads_enabled: bool,
    /// `strategies.staleFileReads.trackedTools`.
    pub stale_file_reads_tracked_tools: Vec<String>,
    /// `strategies.staleFileReads.protectedTools`.
    pub stale_file_reads_protected_tools: ToolProtection,
    /// Compiled `protectedFilePatterns`.
    pub protected_paths: PathProtection,
    /// `manualMode.enabled`.
    pub manual_mode_enabled: bool,
    /// `manualMode.automaticStrategies`.
    pub manual_mode_automatic_strategies: bool,
}

impl StaticPruneConfig {
    /// Build a [`StaticPruneConfig`] with every strategy enabled and
    /// SPEC.md defaults applied.
    pub fn defaults_enabled() -> Self {
        Self {
            dedup_enabled: true,
            dedup_protected_tools: ToolProtection::default(),
            purge_errors_enabled: true,
            purge_errors_turns: 4,
            purge_errors_protected_tools: ToolProtection::default(),
            stale_file_reads_enabled: true,
            stale_file_reads_tracked_tools: vec![
                "read".into(),
                "write".into(),
                "edit".into(),
                "multiedit".into(),
            ],
            stale_file_reads_protected_tools: ToolProtection::default(),
            protected_paths: PathProtection::default(),
            manual_mode_enabled: false,
            manual_mode_automatic_strategies: true,
        }
    }
}

impl PruneConfig for StaticPruneConfig {
    fn dedup_enabled(&self) -> bool {
        self.dedup_enabled
    }
    fn dedup_protected_tools(&self) -> &ToolProtection {
        &self.dedup_protected_tools
    }
    fn purge_errors_enabled(&self) -> bool {
        self.purge_errors_enabled
    }
    fn purge_errors_turns(&self) -> u32 {
        self.purge_errors_turns
    }
    fn purge_errors_protected_tools(&self) -> &ToolProtection {
        &self.purge_errors_protected_tools
    }
    fn stale_file_reads_enabled(&self) -> bool {
        self.stale_file_reads_enabled
    }
    fn stale_file_reads_tracked_tools(&self) -> &[String] {
        &self.stale_file_reads_tracked_tools
    }
    fn stale_file_reads_protected_tools(&self) -> &ToolProtection {
        &self.stale_file_reads_protected_tools
    }
    fn protected_paths(&self) -> &PathProtection {
        &self.protected_paths
    }
    fn manual_mode_enabled(&self) -> bool {
        self.manual_mode_enabled
    }
    fn manual_mode_automatic_strategies(&self) -> bool {
        self.manual_mode_automatic_strategies
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enabled_matches_spec_defaults() {
        let c = StaticPruneConfig::defaults_enabled();
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
}
