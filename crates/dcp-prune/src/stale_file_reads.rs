//! The stale-file-reads strategy — SPEC.md §5.3.

use std::collections::HashMap;

use dcp_types::{SessionState, ToolStatus};

use crate::PruneOutcome;
use crate::config::PruneConfig;

/// Run the stale-file-reads strategy.
///
/// For each path observed across `tracked_tools`, keep only the most
/// recent call/result pair (in `tool_id_list` order) and mark the
/// earlier ones for pruning. Skips error/pending statuses (those are
/// either purge-errors candidates or not yet stale).
pub fn run<C: PruneConfig + ?Sized>(state: &mut SessionState, config: &C) -> PruneOutcome {
    if !config.stale_file_reads_enabled() {
        return PruneOutcome::skipped("stale_file_reads", "disabled");
    }
    if config.manual_mode_enabled() && !config.manual_mode_automatic_strategies() {
        return PruneOutcome::skipped("stale_file_reads", "manual_mode");
    }

    let tracked = config.stale_file_reads_tracked_tools();
    let protected_tools = config.stale_file_reads_protected_tools();
    let protected_paths = config.protected_paths();

    let is_tracked = |tool: &str| tracked.iter().any(|t| t == tool);

    // path -> ordered list of call_ids (in tool_id_list order)
    let mut by_path: HashMap<String, Vec<String>> = HashMap::new();

    for id in &state.tool_id_list {
        if state.prune.tools.contains_key(id) {
            continue;
        }
        let Some(entry) = state.tool_parameters.get(id) else {
            continue;
        };
        if !is_tracked(&entry.tool) {
            continue;
        }
        if protected_tools.is_protected(&entry.tool) {
            continue;
        }
        if entry.status != Some(ToolStatus::Completed) {
            continue;
        }
        for path in &entry.paths {
            if protected_paths.is_protected(path) {
                continue;
            }
            by_path.entry(path.clone()).or_default().push(id.clone());
        }
    }

    // Per SPEC §5.3 step 4: for each path with two or more ids, mark all
    // but the last for pruning. A `multiedit` referencing several paths
    // can become "the latest" for each; we therefore deduplicate the
    // pruned-id set.
    let mut pruned_set: std::collections::BTreeSet<String> = Default::default();
    let mut keep_set: std::collections::HashSet<String> = Default::default();
    for ids in by_path.values() {
        if ids.is_empty() {
            continue;
        }
        if let Some(last) = ids.last() {
            keep_set.insert(last.clone());
        }
    }
    for ids in by_path.values() {
        for id in &ids[..ids.len().saturating_sub(1)] {
            // A call that is the latest for *some* other path must not be
            // pruned. SPEC §5.3 edge case: multiedit with multiple paths.
            if keep_set.contains(id) {
                continue;
            }
            pruned_set.insert(id.clone());
        }
    }

    let mut pruned_ids: Vec<String> = Vec::with_capacity(pruned_set.len());
    let mut tokens_saved: u64 = 0;
    for id in pruned_set {
        let tokens = state
            .tool_parameters
            .get(&id)
            .and_then(|e| e.token_count)
            .unwrap_or(0);
        state.prune.tools.insert(id.clone(), tokens);
        tokens_saved = tokens_saved.saturating_add(tokens);
        pruned_ids.push(id);
    }

    state.stats.stale_file_reads_pruned = state
        .stats
        .stale_file_reads_pruned
        .saturating_add(pruned_ids.len() as u32);

    PruneOutcome {
        strategy: "stale_file_reads".into(),
        pruned_ids,
        tokens_saved,
        reason_skipped: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StaticPruneConfig;
    use dcp_protected::PathProtection;
    use dcp_state::{StaticConfigLike, default_tracked_tools, sync_tool_cache};
    use dcp_types::{Message, Part, Role, ToolStatus};
    use serde_json::json;

    fn assist(id: &str, call: &str, tool: &str, input: serde_json::Value) -> Message {
        Message::new(
            id,
            Role::Assistant,
            vec![Part::tool_call(call, tool, input)],
            0,
        )
    }
    fn user_done(id: &str, call: &str) -> Message {
        Message::new(
            id,
            Role::User,
            vec![Part::tool_result(
                call,
                ToolStatus::Completed,
                Some("ok".into()),
                None,
            )],
            0,
        )
    }

    fn build(messages: &[Message]) -> SessionState {
        let cfg = StaticConfigLike {
            tracked_tools: default_tracked_tools(),
            ..StaticConfigLike::default()
        };
        let mut state = SessionState::default();
        sync_tool_cache(&mut state, &cfg, messages);
        state
    }

    #[test]
    fn prunes_older_reads_of_same_path() {
        let messages = vec![
            assist("a1", "c1", "read", json!({"path": "src/main.rs"})),
            user_done("u1", "c1"),
            assist("a2", "c2", "read", json!({"path": "src/main.rs"})),
            user_done("u2", "c2"),
            assist("a3", "c3", "read", json!({"path": "src/main.rs"})),
            user_done("u3", "c3"),
        ];
        let mut state = build(&messages);
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        assert_eq!(out.pruned_ids.len(), 2);
        assert!(state.prune.tools.contains_key("c1"));
        assert!(state.prune.tools.contains_key("c2"));
        assert!(!state.prune.tools.contains_key("c3"));
    }

    #[test]
    fn keeps_latest_per_path_independently() {
        let messages = vec![
            assist("a1", "c1", "read", json!({"path": "a"})),
            user_done("u1", "c1"),
            assist("a2", "c2", "read", json!({"path": "b"})),
            user_done("u2", "c2"),
            assist("a3", "c3", "read", json!({"path": "a"})),
            user_done("u3", "c3"),
        ];
        let mut state = build(&messages);
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        assert_eq!(out.pruned_ids, vec!["c1".to_string()]);
    }

    #[test]
    fn write_then_read_prunes_write() {
        let messages = vec![
            assist("a1", "c1", "write", json!({"path": "x"})),
            user_done("u1", "c1"),
            assist("a2", "c2", "read", json!({"path": "x"})),
            user_done("u2", "c2"),
        ];
        let mut state = build(&messages);
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        assert_eq!(out.pruned_ids, vec!["c1".to_string()]);
    }

    #[test]
    fn protected_paths_exempt_all_calls() {
        let messages = vec![
            assist("a1", "c1", "read", json!({"path": "Cargo.toml"})),
            user_done("u1", "c1"),
            assist("a2", "c2", "read", json!({"path": "Cargo.toml"})),
            user_done("u2", "c2"),
        ];
        let mut state = build(&messages);
        let config = StaticPruneConfig {
            protected_paths: PathProtection::compile(&["Cargo.toml".into()]).unwrap(),
            ..StaticPruneConfig::defaults_enabled()
        };
        let out = run(&mut state, &config);
        assert!(out.pruned_ids.is_empty());
    }

    #[test]
    fn untracked_tools_are_ignored() {
        let messages = vec![
            assist("a1", "c1", "bash", json!({"path": "x"})),
            user_done("u1", "c1"),
            assist("a2", "c2", "bash", json!({"path": "x"})),
            user_done("u2", "c2"),
        ];
        let mut state = build(&messages);
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        assert!(out.pruned_ids.is_empty(), "bash not in trackedTools");
    }

    #[test]
    fn errored_calls_are_skipped() {
        let messages = vec![
            assist("a1", "c1", "read", json!({"path": "x"})),
            Message::new(
                "u1",
                Role::User,
                vec![Part::tool_result(
                    "c1",
                    ToolStatus::Error,
                    None,
                    Some("e".into()),
                )],
                0,
            ),
            assist("a2", "c2", "read", json!({"path": "x"})),
            user_done("u2", "c2"),
        ];
        let mut state = build(&messages);
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        // Only the completed pair forms a single-element by_path entry —
        // nothing to prune.
        assert!(out.pruned_ids.is_empty());
    }

    #[test]
    fn multiedit_kept_when_latest_for_any_path() {
        let messages = vec![
            // Older read of "a".
            assist("a1", "c1", "read", json!({"path": "a"})),
            user_done("u1", "c1"),
            // Multiedit covers both "a" and "b" — latest for "a" and "b".
            assist(
                "a2",
                "c2",
                "multiedit",
                json!({"path": "a", "edits": [{"path": "b"}]}),
            ),
            user_done("u2", "c2"),
            // Another read of "a" comes later than the multiedit.
            assist("a3", "c3", "read", json!({"path": "a"})),
            user_done("u3", "c3"),
        ];
        let mut state = build(&messages);
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        // c2 (multiedit) is still latest for "b" — must not be pruned.
        // c1 is older for "a" and must be pruned.
        assert!(state.prune.tools.contains_key("c1"));
        assert!(!state.prune.tools.contains_key("c2"));
        assert!(!state.prune.tools.contains_key("c3"));
        assert_eq!(out.pruned_ids, vec!["c1".to_string()]);
    }

    #[test]
    fn skipped_when_disabled() {
        let mut state = SessionState::default();
        let config = StaticPruneConfig {
            stale_file_reads_enabled: false,
            ..StaticPruneConfig::defaults_enabled()
        };
        let out = run(&mut state, &config);
        assert_eq!(out.reason_skipped.as_deref(), Some("disabled"));
    }
}
