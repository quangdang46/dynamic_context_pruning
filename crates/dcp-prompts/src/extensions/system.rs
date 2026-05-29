//! System prompt extension helpers for manual mode and subagent modes.

/// Manual mode system prompt extension (wrapped in `<dcp>` tags).
///
/// # Example
///
/// ```rust
/// use dcp_prompts::manual_mode_extension;
/// let ext = manual_mode_extension();
/// assert!(ext.contains("<dcp>"));
/// assert!(ext.contains("</dcp>"));
/// assert!(ext.contains("manual") || ext.contains("Manual"));
/// ```
pub fn manual_mode_extension() -> String {
    r#"<dcp>
Manual mode is enabled. The library will not run pruning strategies
automatically; the user drives compression and pruning via slash
commands. Treat any nudges as advisory only.
</dcp>"#
        .to_string()
}

/// Subagent mode system prompt extension (wrapped in `<dcp>` tags).
///
/// # Example
///
/// ```rust
/// use dcp_prompts::subagent_mode_extension;
/// let ext = subagent_mode_extension();
/// assert!(ext.contains("<dcp>"));
/// assert!(ext.contains("</dcp>"));
/// assert!(ext.contains("subagent") || ext.contains("sub-agent"));
/// ```
pub fn subagent_mode_extension() -> String {
    r#"<dcp>
Sub-agent support is enabled. Results from sub-agent runs may be
folded back into this conversation; treat them as if you produced
them yourself.
</dcp>"#
        .to_string()
}

/// Builds protected tools listing.
///
/// If `protected_tools` is empty, returns an empty string.
/// Otherwise returns `"Protected tools: `tool1`, `tool2`."` format.
///
/// # Example
///
/// ```rust
/// use dcp_prompts::build_protected_tools_extension;
/// assert!(build_protected_tools_extension(&[]).is_empty());
/// let ext = build_protected_tools_extension(&["read".into(), "write".into()]);
/// assert!(ext.contains("read"));
/// assert!(ext.contains("write"));
/// ```
pub fn build_protected_tools_extension(protected_tools: &[String]) -> String {
    if protected_tools.is_empty() {
        return String::new();
    }

    let list: Vec<String> = protected_tools
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("`{}`", s.trim()))
        .collect();

    if list.is_empty() {
        return String::new();
    }

    format!("Protected tools: {}.", list.join(", "))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manual_mode_extension_contains_dcp_tags() {
        let ext = manual_mode_extension();
        assert!(ext.contains("<dcp>"));
        assert!(ext.contains("</dcp>"));
        assert!(ext.to_lowercase().contains("manual"));
    }

    #[test]
    fn test_subagent_mode_extension_contains_dcp_tags() {
        let ext = subagent_mode_extension();
        assert!(ext.contains("<dcp>"));
        assert!(ext.contains("</dcp>"));
        assert!(ext.to_lowercase().contains("sub-agent") || ext.to_lowercase().contains("subagent"));
    }

    #[test]
    fn test_protected_tools_empty() {
        let ext = build_protected_tools_extension(&[]);
        assert!(ext.is_empty());
    }

    #[test]
    fn test_protected_tools_with_items() {
        let tools = vec!["read".to_string(), "write".to_string(), "task".to_string()];
        let ext = build_protected_tools_extension(&tools);
        assert!(ext.contains("`read`"));
        assert!(ext.contains("`write`"));
        assert!(ext.contains("`task`"));
        assert!(ext.starts_with("Protected tools:"));
        assert!(ext.ends_with('.'));
    }

    #[test]
    fn test_protected_tools_skips_blank_entries() {
        let tools = vec!["read".to_string(), "".to_string(), "  ".to_string()];
        let ext = build_protected_tools_extension(&tools);
        assert!(ext.contains("`read`"));
        assert!(!ext.contains("``"));
    }

    #[test]
    fn test_protected_tools_all_blank_returns_empty() {
        let tools = vec!["  ".to_string(), "\t".to_string()];
        let ext = build_protected_tools_extension(&tools);
        assert!(ext.is_empty());
    }
}