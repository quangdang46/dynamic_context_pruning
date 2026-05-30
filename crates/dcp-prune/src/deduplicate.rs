//! The deduplicate strategy — SPEC.md §5.1.

use std::collections::HashMap;

use dcp_types::{SessionState, ToolStatus};

use crate::PruneOutcome;
use crate::config::PruneConfig;

/// Run the deduplicate strategy.
///
/// Returns the outcome (`name`, `pruned_count`, `tokens_saved`, optional
/// `skipped_reason`). All mutations to `state.prune.tools` and
/// `stats.dedup_pruned` happen in-place.
///
/// The strategy is total — given a well-formed `state` (per
/// `dcp_state::sync_tool_cache`) it never panics and never produces an
/// inconsistent intermediate state.
pub fn run<C: PruneConfig + ?Sized>(state: &mut SessionState, config: &C) -> PruneOutcome {
    if !config.dedup_enabled() {
        return PruneOutcome::skipped("deduplicate", "disabled");
    }
    if config.manual_mode_enabled() && !config.manual_mode_automatic_strategies() {
        return PruneOutcome::skipped("deduplicate", "manual_mode");
    }

    let dedup_protected = config.dedup_protected_tools();
    let path_protected = config.protected_paths();

    // signature -> ordered list of call_ids in tool_id_list order
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();

    for id in &state.tool_id_list {
        if state.prune.tools.contains_key(id) {
            continue;
        }
        let Some(entry) = state.tool_parameters.get(id) else {
            continue;
        };
        if dedup_protected.is_protected(&entry.tool) {
            continue;
        }
        if entry.paths.iter().any(|p| path_protected.is_protected(p)) {
            continue;
        }
        if entry.status != Some(ToolStatus::Completed) {
            continue;
        }
        groups
            .entry(entry.signature.clone())
            .or_default()
            .push(id.clone());
    }

    let mut pruned: Vec<(String, u64)> = Vec::new();
    for ids in groups.values() {
        if ids.len() < 2 {
            continue;
        }
        // All except the last are duplicates.
        for id in &ids[..ids.len() - 1] {
            let tokens = state
                .tool_parameters
                .get(id)
                .and_then(|e| e.token_count)
                .unwrap_or(0);
            pruned.push((id.clone(), tokens));
        }
    }

    let pruned_ids: Vec<String> = pruned.iter().map(|(id, _)| id.clone()).collect();
    let mut tokens_saved: u64 = 0;
    for (id, tokens) in pruned {
        state.prune.tools.insert(id, tokens);
        tokens_saved = tokens_saved.saturating_add(tokens);
    }

    state.stats.dedup_pruned = state
        .stats
        .dedup_pruned
        .saturating_add(pruned_ids.len() as u32);

    PruneOutcome {
        strategy: "deduplicate".into(),
        pruned_ids,
        tokens_saved,
        reason_skipped: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StaticPruneConfig;
    use dcp_protected::ToolProtection;
    use dcp_state::{StaticConfigLike, default_tracked_tools, sync_tool_cache};
    use dcp_types::{Message, Part, Role, ToolStatus};
    use serde_json::json;

    fn assist_call(id: &str, call: &str, tool: &str, input: serde_json::Value) -> Message {
        Message::new(
            id,
            Role::Assistant,
            vec![Part::tool_call(call, tool, input)],
            0,
        )
    }
    fn user_result(id: &str, call: &str, status: ToolStatus) -> Message {
        Message::new(
            id,
            Role::User,
            vec![Part::tool_result(call, status, Some("ok".into()), None)],
            0,
        )
    }

    fn build_state(messages: &[Message]) -> SessionState {
        let cfg = StaticConfigLike {
            tracked_tools: default_tracked_tools(),
            ..StaticConfigLike::default()
        };
        let mut state = SessionState::default();
        sync_tool_cache(&mut state, &cfg, messages);
        state
    }

    #[test]
    fn prunes_all_but_latest_in_signature_group() {
        let messages = vec![
            assist_call("a1", "c1", "read", json!({"path": "x"})),
            user_result("u1", "c1", ToolStatus::Completed),
            assist_call("a2", "c2", "read", json!({"path": "x"})),
            user_result("u2", "c2", ToolStatus::Completed),
            assist_call("a3", "c3", "read", json!({"path": "x"})),
            user_result("u3", "c3", ToolStatus::Completed),
        ];
        let mut state = build_state(&messages);
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        assert_eq!(out.strategy, "deduplicate");
        assert_eq!(out.pruned_ids.len(), 2);
        assert!(state.prune.tools.contains_key("c1"));
        assert!(state.prune.tools.contains_key("c2"));
        assert!(!state.prune.tools.contains_key("c3"));
        assert_eq!(state.stats.dedup_pruned, 2);
    }

    #[test]
    fn single_occurrence_is_not_pruned() {
        let messages = vec![
            assist_call("a1", "c1", "read", json!({"path": "x"})),
            user_result("u1", "c1", ToolStatus::Completed),
        ];
        let mut state = build_state(&messages);
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        assert!(out.pruned_ids.is_empty());
        assert!(state.prune.tools.is_empty());
    }

    #[test]
    fn errored_calls_are_skipped() {
        let messages = vec![
            assist_call("a1", "c1", "read", json!({"path": "x"})),
            user_result("u1", "c1", ToolStatus::Error),
            assist_call("a2", "c2", "read", json!({"path": "x"})),
            user_result("u2", "c2", ToolStatus::Error),
        ];
        let mut state = build_state(&messages);
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        assert!(
            out.pruned_ids.is_empty(),
            "errored calls must not be deduped"
        );
    }

    #[test]
    fn protected_tools_are_skipped() {
        let messages = vec![
            assist_call("a1", "c1", "task", json!({"id": "x"})),
            user_result("u1", "c1", ToolStatus::Completed),
            assist_call("a2", "c2", "task", json!({"id": "x"})),
            user_result("u2", "c2", ToolStatus::Completed),
        ];
        let mut state = build_state(&messages);
        let config = StaticPruneConfig {
            dedup_protected_tools: ToolProtection::new_exact(["task"]),
            ..StaticPruneConfig::defaults_enabled()
        };
        let out = run(&mut state, &config);
        assert!(out.pruned_ids.is_empty());
    }

    #[test]
    fn protected_paths_are_skipped() {
        let messages = vec![
            assist_call("a1", "c1", "read", json!({"path": "Cargo.toml"})),
            user_result("u1", "c1", ToolStatus::Completed),
            assist_call("a2", "c2", "read", json!({"path": "Cargo.toml"})),
            user_result("u2", "c2", ToolStatus::Completed),
        ];
        let mut state = build_state(&messages);
        let config = StaticPruneConfig {
            protected_paths: dcp_protected::PathProtection::compile(&["Cargo.toml".into()])
                .unwrap(),
            ..StaticPruneConfig::defaults_enabled()
        };
        let out = run(&mut state, &config);
        assert!(out.pruned_ids.is_empty());
    }

    #[test]
    fn skipped_when_disabled() {
        let mut state = SessionState::default();
        let config = StaticPruneConfig {
            dedup_enabled: false,
            ..StaticPruneConfig::defaults_enabled()
        };
        let out = run(&mut state, &config);
        assert_eq!(out.reason_skipped.as_deref(), Some("disabled"));
    }

    #[test]
    fn skipped_in_manual_mode_without_automatic_strategies() {
        let mut state = SessionState::default();
        let config = StaticPruneConfig {
            manual_mode_enabled: true,
            manual_mode_automatic_strategies: false,
            ..StaticPruneConfig::defaults_enabled()
        };
        let out = run(&mut state, &config);
        assert_eq!(out.reason_skipped.as_deref(), Some("manual_mode"));
    }

    #[test]
    fn already_pruned_calls_are_skipped() {
        let messages = vec![
            assist_call("a1", "c1", "read", json!({"path": "x"})),
            user_result("u1", "c1", ToolStatus::Completed),
            assist_call("a2", "c2", "read", json!({"path": "x"})),
            user_result("u2", "c2", ToolStatus::Completed),
        ];
        let mut state = build_state(&messages);
        state.prune.tools.insert("c1".into(), 0);
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        // Only c2 left in the group, single occurrence — no further pruning.
        assert!(out.pruned_ids.is_empty());
    }
}
