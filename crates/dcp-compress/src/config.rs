//! [`CompressConfig`] — the slice of configuration `dcp-compress` reads.
//!
//! Per PLAN.md §5.2, this crate cannot depend on `dcp-config`. The trait
//! exposes only the fields the compress pipeline consumes; production
//! callers wire their real config through a thin impl, while tests use
//! [`StaticCompressConfig`].

use dcp_protected::ToolProtection;

/// Default per-entry summary character cap (SPEC.md §6.1 + §10.2:
/// `compress.maxSummaryChars`, default 32 KiB).
pub const DEFAULT_MAX_SUMMARY_CHARS: usize = 32 * 1024;

/// Plain-data trait abstracting the slice of config the compress
/// pipeline consumes.
pub trait CompressConfig {
    /// Per-entry summary length cap in characters
    /// (`compress.maxSummaryChars`).
    fn max_summary_chars(&self) -> usize {
        DEFAULT_MAX_SUMMARY_CHARS
    }

    /// Tool names whose verbatim output should be preserved when the
    /// surrounding range is compressed
    /// (`compress.protectedTools`).
    fn protected_tools(&self) -> &ToolProtection;

    /// Whether user messages are protected from compression
    /// (`compress.protectUserMessages`).
    fn protect_user_messages(&self) -> bool;

    /// Whether the summary should include the wrapping `<dcp-block>`
    /// tags shown in SPEC §6.3.3 (`compress.showCompression`).
    fn show_compression(&self) -> bool {
        true
    }
}

/// Plain-data implementation suitable for tests and short-lived
/// callers.
#[derive(Clone, Debug, Default)]
pub struct StaticCompressConfig {
    /// `compress.maxSummaryChars`.
    pub max_summary_chars: usize,
    /// `compress.protectedTools` (compiled).
    pub protected_tools: ToolProtection,
    /// `compress.protectUserMessages`.
    pub protect_user_messages: bool,
    /// `compress.showCompression`.
    pub show_compression: bool,
}

impl StaticCompressConfig {
    /// Build a [`StaticCompressConfig`] with SPEC defaults applied.
    pub fn defaults() -> Self {
        Self {
            max_summary_chars: DEFAULT_MAX_SUMMARY_CHARS,
            protected_tools: ToolProtection::new_exact(["task", "skill"]),
            protect_user_messages: false,
            show_compression: true,
        }
    }
}

impl CompressConfig for StaticCompressConfig {
    fn max_summary_chars(&self) -> usize {
        if self.max_summary_chars == 0 {
            DEFAULT_MAX_SUMMARY_CHARS
        } else {
            self.max_summary_chars
        }
    }
    fn protected_tools(&self) -> &ToolProtection {
        &self.protected_tools
    }
    fn protect_user_messages(&self) -> bool {
        self.protect_user_messages
    }
    fn show_compression(&self) -> bool {
        self.show_compression
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = StaticCompressConfig::defaults();
        assert_eq!(c.max_summary_chars(), DEFAULT_MAX_SUMMARY_CHARS);
        assert!(c.protected_tools().is_protected("task"));
        assert!(c.protected_tools().is_protected("skill"));
        assert!(!c.protect_user_messages());
        assert!(c.show_compression());
    }

    #[test]
    fn zero_max_summary_chars_falls_back_to_default() {
        let c = StaticCompressConfig {
            max_summary_chars: 0,
            ..StaticCompressConfig::default()
        };
        assert_eq!(c.max_summary_chars(), DEFAULT_MAX_SUMMARY_CHARS);
    }
}
