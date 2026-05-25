//! The purge-errored-tool-inputs strategy — SPEC.md §5.2.

use dcp_types::{SessionState, ToolStatus};

use crate::PruneOutcome;
use crate::config::PruneConfig;

/// Run the purge-errors strategy.
///
/// Marks every errored tool call older than `purge_errors_turns()` turns
/// for input-removal. The apply phase later replaces the input field
/// with a placeholder string while leaving the error message in the
/// outgoing stream so the model still sees what went wrong.
pub fn run<C: PruneConfig + ?Sized>(state: &mut SessionState, config: &C) -> PruneOutcome {
    if !config.purge_errors_enabled() {
        return PruneOutcome::skipped("purge_errors", "disabled");
    }
    if config.manual_mode_enabled() && !config.manual_mode_automatic_strategies() {
        return PruneOutcome::skipped("purge_errors", "manual_mode");
    }

    // SPEC §5.2: clamp configured `0` to `1`.
    let threshold = config.purge_errors_turns().max(1);
    let protected = config.purge_errors_protected_tools();

    let mut pruned: Vec<(String, u64)> = Vec::new();

    for id in &state.tool_id_list {
        if state.prune.tools.contains_key(id) {
            continue;
        }
        let Some(entry) = state.tool_parameters.get(id) else {
            continue;
        };
        if entry.status != Some(ToolStatus::Error) {
            continue;
        }
        if protected.is_protected(&entry.tool) {
            continue;
        }
        let age = state.current_turn.saturating_sub(entry.turn);
        if age < threshold {
            continue;
        }
        pruned.push((id.clone(), entry.token_count.unwrap_or(0)));
    }

    let pruned_ids: Vec<String> = pruned.iter().map(|(id, _)| id.clone()).collect();
    let mut tokens_saved: u64 = 0;
    for (id, tokens) in pruned {
        state.prune.tools.insert(id, tokens);
        tokens_saved = tokens_saved.saturating_add(tokens);
    }

    state.stats.purge_errors_pruned = state
        .stats
        .purge_errors_pruned
        .saturating_add(pruned_ids.len() as u32);

    PruneOutcome {
        strategy: "purge_errors".into(),
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
    fn user_err(id: &str, call: &str) -> Message {
        Message::new(
            id,
            Role::User,
            vec![Part::tool_result(
                call,
                ToolStatus::Error,
                None,
                Some("boom".into()),
            )],
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
    fn prunes_errored_call_older_than_threshold() {
        let messages = vec![
            assist_call("a1", "c1", "read", json!({"path": "x"})),
            user_err("u1", "c1"),
        ];
        let mut state = build(&messages);
        // Simulate enough time has passed: turn captured at 0, current at 4.
        state.current_turn = 4;
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        assert_eq!(out.pruned_ids, vec!["c1".to_string()]);
        assert_eq!(state.stats.purge_errors_pruned, 1);
    }

    #[test]
    fn does_not_prune_recent_errors() {
        let messages = vec![
            assist_call("a1", "c1", "read", json!({"path": "x"})),
            user_err("u1", "c1"),
        ];
        let mut state = build(&messages);
        state.current_turn = 1; // age = 1, threshold = 4
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        assert!(out.pruned_ids.is_empty());
    }

    #[test]
    fn skips_completed_calls() {
        let messages = vec![
            assist_call("a1", "c1", "read", json!({"path": "x"})),
            user_done("u1", "c1"),
        ];
        let mut state = build(&messages);
        state.current_turn = 100;
        let config = StaticPruneConfig::defaults_enabled();
        let out = run(&mut state, &config);
        assert!(out.pruned_ids.is_empty());
    }

    #[test]
    fn protected_tools_are_skipped() {
        let messages = vec![
            assist_call("a1", "c1", "task", json!({})),
            user_err("u1", "c1"),
        ];
        let mut state = build(&messages);
        state.current_turn = 100;
        let config = StaticPruneConfig {
            purge_errors_protected_tools: ToolProtection::new(["task"]),
            ..StaticPruneConfig::defaults_enabled()
        };
        let out = run(&mut state, &config);
        assert!(out.pruned_ids.is_empty());
    }

    #[test]
    fn zero_threshold_clamped_to_one() {
        let messages = vec![
            assist_call("a1", "c1", "read", json!({"path": "x"})),
            user_err("u1", "c1"),
        ];
        let mut state = build(&messages);
        state.current_turn = 1; // age = 1, clamped threshold = 1
        let config = StaticPruneConfig {
            purge_errors_turns: 0,
            ..StaticPruneConfig::defaults_enabled()
        };
        let out = run(&mut state, &config);
        assert_eq!(out.pruned_ids, vec!["c1".to_string()]);
    }

    #[test]
    fn skipped_when_disabled() {
        let mut state = SessionState::default();
        let config = StaticPruneConfig {
            purge_errors_enabled: false,
            ..StaticPruneConfig::defaults_enabled()
        };
        let out = run(&mut state, &config);
        assert_eq!(out.reason_skipped.as_deref(), Some("disabled"));
    }
}
