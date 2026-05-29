//! Host-permission snapshot and glob-based resolution.
//!
//! Port of `lib/host-permissions.ts`.

use std::collections::HashMap;

/// What the host allows for a particular action.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum PermissionAction {
    /// The host must be asked before proceeding.
    #[default]
    Ask,
    /// The action is allowed.
    Allow,
    /// The action is denied.
    Deny,
}

/// Snapshot of the host's permission state.
///
/// Mirrors the shape described in the host-permissions spec:
/// global permissions plus per-agent overrides.
#[derive(Clone, Debug, Default)]
pub struct HostPermissionSnapshot {
    /// Global permission map. Key is action name (e.g. `"compress"`).
    pub global: HashMap<String, PermissionAction>,
    /// Per-agent permission maps. Outer key is agent name.
    pub agents: HashMap<String, HashMap<String, PermissionAction>>,
}

/// Escape regex metacharacters *except* `*` and `?`.
#[allow(dead_code)]
fn escape_regex(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        match c {
            '*' | '?' => result.push(c),
            '\\' => {
                // Normalize backslash to forward slash on Windows paths
                result.push('/');
            }
            c if regex::escape(&c.to_string()).len() == 1 => {
                // It's a single character that needs escaping
                result.push_str(&regex::escape(&c.to_string()));
            }
            _ => result.push(c),
        }
    }
    result
}

/// Returns `true` if `value` matches `pattern` using glob rules.
///
/// - `*` matches any sequence of characters (converted to `.*` in regex)
/// - `?` matches any single character (converted to `.` in regex)
/// - `\` is normalized to `/` (Windows path handling)
/// - Matching is case-insensitive
pub fn wildcard_match(value: &str, pattern: &str) -> bool {
    let value_normalized = value.replace('\\', "/");
    let pattern_normalized = pattern.replace('\\', "/");

    // Convert glob pattern to regex
    let mut regex_pattern = String::new();
    regex_pattern.push('^');
    for c in pattern_normalized.chars() {
        match c {
            '*' => regex_pattern.push_str(".*"),
            '?' => regex_pattern.push('.'),
            other => regex_pattern.push_str(&regex::escape(&other.to_string())),
        }
    }
    regex_pattern.push('$');

    // Use regex with case-insensitive flag
    let re = regex::RegexBuilder::new(&regex_pattern)
        .case_insensitive(true)
        .build();

    match re {
        Ok(re) => re.is_match(&value_normalized),
        Err(_) => false,
    }
}

/// Returns `true` if *any* config in `configs` has "compress" denied with
/// pattern `"*"`.
pub fn compress_disabled_by_opencode(configs: &[&HashMap<String, PermissionAction>]) -> bool {
    for config in configs {
        if let Some(action) = config.get("compress") {
            if *action == PermissionAction::Deny {
                // Check if there's a wildcard pattern entry
                // Since we store exact strings as keys, a "deny" entry means
                // the host denied compress with the catch-all pattern
                return true;
            }
        }
    }
    false
}

/// Resolve the effective permission for an action given the base permission
/// and the host snapshot.
///
/// Resolution rules (last-match-wins):
/// 1. If `base` is `Deny`, return `Deny` immediately.
/// 2. Check agent-specific permissions (if `agent_name` is `Some`).
/// 3. Fall back to global permissions.
/// 4. Return `base` if no host permission is set.
///
/// `agent_name` selects the per-agent map; when `None`, only global is used.
pub fn resolve_effective_permission(
    base: PermissionAction,
    host: &HostPermissionSnapshot,
    agent_name: Option<&str>,
) -> PermissionAction {
    // Step 1: base deny always wins
    if base == PermissionAction::Deny {
        return PermissionAction::Deny;
    }

    // Step 2: check agent-specific permissions
    if let Some(name) = agent_name {
        if let Some(agent_map) = host.agents.get(name) {
            if let Some(action) = agent_map.get("compress") {
                return action.clone();
            }
        }
    }

    // Step 3: fall back to global
    if let Some(action) = host.global.get("compress") {
        return action.clone();
    }

    // Step 4: no host permission set, use base
    base
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── wildcard_match ────────────────────────────────────────────────────────

    #[test]
    fn test_wildcard_match_star() {
        assert!(wildcard_match("anything", "*"));
        assert!(wildcard_match("foo/bar/baz.txt", "*"));
        assert!(wildcard_match("", "*"));
    }

    #[test]
    fn test_wildcard_match_question() {
        assert!(wildcard_match("compress", "co?press"));
        assert!(wildcard_match("compress", "compress"));
        assert!(wildcard_match("coXpress", "co?press"));
        assert!(!wildcard_match("coXXpress", "co?press"));
    }

    #[test]
    fn test_wildcard_match_literal() {
        assert!(wildcard_match("compress", "compress"));
        assert!(!wildcard_match("compressx", "compress"));
    }

    #[test]
    fn test_wildcard_match_windows_paths() {
        // Backslash normalized to forward slash
        assert!(wildcard_match("foo\\bar", "foo/bar"));
        assert!(wildcard_match("foo/bar", "foo\\bar"));
    }

    #[test]
    fn test_wildcard_match_case_insensitive() {
        assert!(wildcard_match("COMPRESS", "compress"));
        assert!(wildcard_match("Compress", "compress"));
    }

    #[test]
    fn test_wildcard_match_complex_glob() {
        assert!(wildcard_match("src/lib.rs", "src/*.rs"));
        assert!(wildcard_match("src/main.rs", "src/*.rs"));
        assert!(!wildcard_match("src/lib.js", "src/*.rs"));
    }

    #[test]
    fn test_wildcard_match_no_match() {
        assert!(!wildcard_match("compress", "deny"));
        assert!(!wildcard_match("foo", "bar"));
    }

    // ── compress_disabled_by_opencode ────────────────────────────────────────

    #[test]
    fn test_compress_disabled_by_opencode_denied() {
        let mut config: HashMap<String, PermissionAction> = HashMap::new();
        config.insert("compress".to_string(), PermissionAction::Deny);
        assert!(compress_disabled_by_opencode(&[&config]));
    }

    #[test]
    fn test_compress_disabled_by_opencode_allowed() {
        let mut config: HashMap<String, PermissionAction> = HashMap::new();
        config.insert("compress".to_string(), PermissionAction::Allow);
        assert!(!compress_disabled_by_opencode(&[&config]));
    }

    #[test]
    fn test_compress_disabled_by_opencode_ask() {
        let mut config: HashMap<String, PermissionAction> = HashMap::new();
        config.insert("compress".to_string(), PermissionAction::Ask);
        assert!(!compress_disabled_by_opencode(&[&config]));
    }

    #[test]
    fn test_compress_disabled_by_opencode_multiple_configs() {
        let mut config1: HashMap<String, PermissionAction> = HashMap::new();
        config1.insert("other".to_string(), PermissionAction::Allow);

        let mut config2: HashMap<String, PermissionAction> = HashMap::new();
        config2.insert("compress".to_string(), PermissionAction::Deny);

        assert!(compress_disabled_by_opencode(&[&config1, &config2]));
    }

    #[test]
    fn test_compress_disabled_by_opencode_empty_config() {
        let config: HashMap<String, PermissionAction> = HashMap::new();
        assert!(!compress_disabled_by_opencode(&[&config]));
    }

    // ── resolve_effective_permission ─────────────────────────────────────────

    #[test]
    fn test_resolve_effective_deny_base() {
        let host = HostPermissionSnapshot::default();
        let result = resolve_effective_permission(PermissionAction::Deny, &host, None);
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn test_resolve_effective_deny_base_with_host_allow() {
        let mut global: HashMap<String, PermissionAction> = HashMap::new();
        global.insert("compress".to_string(), PermissionAction::Allow);
        let host = HostPermissionSnapshot {
            global,
            ..Default::default()
        };
        // Base deny should still win
        let result = resolve_effective_permission(PermissionAction::Deny, &host, None);
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn test_resolve_effective_global_allow() {
        let mut global: HashMap<String, PermissionAction> = HashMap::new();
        global.insert("compress".to_string(), PermissionAction::Allow);
        let host = HostPermissionSnapshot {
            global,
            ..Default::default()
        };
        let result = resolve_effective_permission(PermissionAction::Ask, &host, None);
        assert_eq!(result, PermissionAction::Allow);
    }

    #[test]
    fn test_resolve_effective_global_deny() {
        let mut global: HashMap<String, PermissionAction> = HashMap::new();
        global.insert("compress".to_string(), PermissionAction::Deny);
        let host = HostPermissionSnapshot {
            global,
            ..Default::default()
        };
        let result = resolve_effective_permission(PermissionAction::Ask, &host, None);
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn test_resolve_effective_agent_allow() {
        let mut agent_map: HashMap<String, PermissionAction> = HashMap::new();
        agent_map.insert("compress".to_string(), PermissionAction::Allow);

        let mut agents: HashMap<String, HashMap<String, PermissionAction>> = HashMap::new();
        agents.insert("my-agent".to_string(), agent_map);

        let host = HostPermissionSnapshot {
            global: HashMap::new(),
            agents,
        };
        let result = resolve_effective_permission(PermissionAction::Ask, &host, Some("my-agent"));
        assert_eq!(result, PermissionAction::Allow);
    }

    #[test]
    fn test_resolve_effective_agent_deny_overrides_global_allow() {
        let mut global: HashMap<String, PermissionAction> = HashMap::new();
        global.insert("compress".to_string(), PermissionAction::Allow);

        let mut agent_map: HashMap<String, PermissionAction> = HashMap::new();
        agent_map.insert("compress".to_string(), PermissionAction::Deny);

        let mut agents: HashMap<String, HashMap<String, PermissionAction>> = HashMap::new();
        agents.insert("my-agent".to_string(), agent_map);

        let host = HostPermissionSnapshot { global, agents };
        // Agent-specific deny should override global allow
        let result = resolve_effective_permission(PermissionAction::Ask, &host, Some("my-agent"));
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn test_resolve_effective_no_host_permission_uses_base() {
        let host = HostPermissionSnapshot::default();
        let result = resolve_effective_permission(PermissionAction::Ask, &host, Some("my-agent"));
        assert_eq!(result, PermissionAction::Ask);
    }

    #[test]
    fn test_resolve_effective_unknown_agent_uses_global() {
        let mut global: HashMap<String, PermissionAction> = HashMap::new();
        global.insert("compress".to_string(), PermissionAction::Allow);

        let host = HostPermissionSnapshot {
            global,
            ..Default::default()
        };
        // Unknown agent, fall back to global
        let result =
            resolve_effective_permission(PermissionAction::Ask, &host, Some("unknown-agent"));
        assert_eq!(result, PermissionAction::Allow);
    }
}
