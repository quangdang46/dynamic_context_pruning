//! Subagent result expansion — port of lib/subagents/subagent-results.ts.
//!
//! Provides: get_sub_agent_id, build_subagent_result_text, merge_subagent_result,
//! inject_extended_sub_agent_results.

use std::collections::HashMap;

use dcp_types::{Message, Part, Role, SessionState};

// ============================================================================
// get_sub_agent_id
// ============================================================================

/// Extract the agent ID from a message's parts.
///
/// The TS upstream reads `part.state.metadata.sessionId` from ToolResult parts.
/// Since the Rust `Part` type (dcp-types) does not carry a `state` field with
/// metadata, this function accepts an optional agent name from the caller.
///
/// Returns `Some(id)` if `agent_id` is provided and non-empty, `None` otherwise.
pub fn get_sub_agent_id(agent_id: Option<&str>) -> Option<String> {
    agent_id.filter(|s| !s.is_empty()).map(String::from)
}

// ============================================================================
// build_subagent_result_text
// ============================================================================

/// Build the result text for a subagent from the cache.
///
/// Looks up `sub_agent_id` in the cache. Returns a formatted string like:
/// "Subagent result: {id}\n{result}" if found, or empty string if not found.
pub fn build_subagent_result_text(cache: &HashMap<String, String>, sub_agent_id: &str) -> String {
    match cache.get(sub_agent_id) {
        Some(result) => format!("Subagent result: {}\n{}", sub_agent_id, result),
        None => String::new(),
    }
}

// ============================================================================
// merge_subagent_result
// ============================================================================

/// Merge a subagent result into the session state.
///
/// Stores/updates `result` in `state.sub_agent_result_cache` keyed by `sub_agent_id`.
pub fn merge_subagent_result(state: &mut SessionState, sub_agent_id: &str, result: &str) {
    state
        .subagent_result_cache
        .insert(sub_agent_id.to_string(), result.to_string());
}

// ============================================================================
// inject_extended_sub_agent_results
// ============================================================================

/// Inject extended subagent results into assistant messages.
///
/// Iterates through `messages`. For each assistant message, uses `agent_for`
/// closure to obtain the agent ID, then injects the cached result text.
///
/// Returns the count of messages that were modified.
pub fn inject_extended_sub_agent_results(
    state: &SessionState,
    messages: &mut [Message],
    agent_for: impl Fn(&Message) -> Option<String>,
) -> usize {
    let mut count = 0;

    for msg in messages.iter_mut() {
        if msg.role != Role::Assistant {
            continue;
        }

        let Some(agent_id) = agent_for(msg) else {
            continue;
        };

        let Some(result) = state.subagent_result_cache.get(&agent_id) else {
            continue;
        };

        let inject_text = format!("\n\nSubagent result: {}\n{}", agent_id, result);
        inject_into_text_parts(msg, &inject_text);
        count += 1;
    }

    count
}

/// Inject text into all Text parts of a message.
fn inject_into_text_parts(msg: &mut Message, inject_text: &str) {
    for part in msg.parts.iter_mut() {
        if let Part::Text(text) = part {
            text.push_str(inject_text);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::MessageIdState;
    use std::collections::HashMap;

    // ─────────────────────────────────────────────────────────────────
    // Helper constructors
    // ─────────────────────────────────────────────────────────────────

    fn make_state(subagent_result_cache: HashMap<String, String>) -> SessionState {
        SessionState {
            session_id: None,
            is_subagent: false,
            manual_mode: dcp_types::ManualMode::default(),
            compress_permission: dcp_types::CompressPermission::default(),
            pending_manual_trigger: None,
            prune: dcp_types::Prune::default(),
            nudges: dcp_types::Nudges::default(),
            stats: dcp_types::Stats::default(),
            compression_timing: dcp_types::CompressionTimingState::default(),
            tool_parameters: HashMap::new(),
            subagent_result_cache,
            tool_id_list: Vec::new(),
            message_ids: MessageIdState::default(),
            last_compaction: 0,
            current_turn: 0,
            model_context_limit: None,
            system_prompt_tokens: None,
            last_message_was_assistant_text: false,
            pending_prune: None,
            last_apply_turn: None,
            force_apply_requested: false,
        }
    }

    fn msg(id: &str, role: Role, parts: Vec<Part>) -> Message {
        Message {
            id: id.to_string(),
            role,
            parts,
            time: 0,
            ignored: false,
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // get_sub_agent_id tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn get_sub_agent_id_returns_some_when_provided() {
        assert_eq!(
            get_sub_agent_id(Some("agent-123")),
            Some("agent-123".to_string())
        );
    }

    #[test]
    fn get_sub_agent_id_returns_none_when_empty() {
        assert!(get_sub_agent_id(Some("")).is_none());
    }

    #[test]
    fn get_sub_agent_id_returns_none_when_not_provided() {
        assert!(get_sub_agent_id(None).is_none());
    }

    // ─────────────────────────────────────────────────────────────────
    // build_subagent_result_text tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn build_subagent_result_text_found() {
        let mut cache = HashMap::new();
        cache.insert(
            "agent-123".to_string(),
            "completed successfully".to_string(),
        );

        let result = build_subagent_result_text(&cache, "agent-123");

        assert_eq!(result, "Subagent result: agent-123\ncompleted successfully");
    }

    #[test]
    fn build_subagent_result_text_not_found() {
        let cache = HashMap::new();

        let result = build_subagent_result_text(&cache, "agent-456");

        assert_eq!(result, "");
    }

    #[test]
    fn build_subagent_result_text_empty_result() {
        let mut cache = HashMap::new();
        cache.insert("agent-789".to_string(), String::new());

        let result = build_subagent_result_text(&cache, "agent-789");

        // Empty result still produces output with just the id
        assert_eq!(result, "Subagent result: agent-789\n");
    }

    #[test]
    fn build_subagent_result_text_with_newlines_in_result() {
        let mut cache = HashMap::new();
        cache.insert("agent-multi".to_string(), "line1\nline2\nline3".to_string());

        let result = build_subagent_result_text(&cache, "agent-multi");

        assert!(result.contains("line1\nline2\nline3"));
        assert!(result.starts_with("Subagent result: agent-multi\n"));
    }

    // ─────────────────────────────────────────────────────────────────
    // merge_subagent_result tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn merge_subagent_result_inserts_new() {
        let mut state = make_state(HashMap::new());

        merge_subagent_result(&mut state, "agent-new", "result data");

        assert_eq!(
            state.subagent_result_cache.get("agent-new"),
            Some(&"result data".to_string())
        );
    }

    #[test]
    fn merge_subagent_result_overwrites_existing() {
        let mut cache = HashMap::new();
        cache.insert("agent-old".to_string(), "old result".to_string());
        let mut state = make_state(cache);

        merge_subagent_result(&mut state, "agent-old", "new result");

        assert_eq!(
            state.subagent_result_cache.get("agent-old"),
            Some(&"new result".to_string())
        );
    }

    #[test]
    fn merge_subagent_result_multiple_agents() {
        let mut state = make_state(HashMap::new());

        merge_subagent_result(&mut state, "agent-a", "result a");
        merge_subagent_result(&mut state, "agent-b", "result b");
        merge_subagent_result(&mut state, "agent-c", "result c");

        assert_eq!(state.subagent_result_cache.len(), 3);
        assert_eq!(
            state.subagent_result_cache.get("agent-a"),
            Some(&"result a".to_string())
        );
        assert_eq!(
            state.subagent_result_cache.get("agent-b"),
            Some(&"result b".to_string())
        );
        assert_eq!(
            state.subagent_result_cache.get("agent-c"),
            Some(&"result c".to_string())
        );
    }

    // ─────────────────────────────────────────────────────────────────
    // inject_extended_sub_agent_results tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn inject_extended_sub_agent_results_empty_messages() {
        let state = make_state(HashMap::new());
        let mut messages: Vec<Message> = vec![];

        let count = inject_extended_sub_agent_results(&state, &mut messages, |_| None);

        assert_eq!(count, 0);
    }

    #[test]
    fn inject_extended_sub_agent_results_no_cache() {
        let state = make_state(HashMap::new());
        let mut messages = vec![
            msg("m1", Role::User, vec![Part::text("hello")]),
            msg("m2", Role::Assistant, vec![Part::text("world")]),
        ];

        let count = inject_extended_sub_agent_results(&state, &mut messages, |_| None);

        assert_eq!(count, 0);
        assert_eq!(messages[0].parts[0].as_text().unwrap(), "hello");
        assert_eq!(messages[1].parts[0].as_text().unwrap(), "world");
    }

    #[test]
    fn inject_extended_sub_agent_results_skips_user_messages() {
        let mut cache = HashMap::new();
        cache.insert("agent-user".to_string(), "user result".to_string());
        let state = make_state(cache);
        let mut messages = vec![msg("m1", Role::User, vec![Part::text("hello")])];

        let count = inject_extended_sub_agent_results(&state, &mut messages, |_| None);

        assert_eq!(count, 0);
    }

    #[test]
    fn inject_extended_sub_agent_results_returns_count() {
        let mut cache = HashMap::new();
        cache.insert("agent-1".to_string(), "result 1".to_string());
        cache.insert("agent-2".to_string(), "result 2".to_string());
        let state = make_state(cache);

        let mut messages = vec![msg("m1", Role::Assistant, vec![Part::text("original")])];

        let count = inject_extended_sub_agent_results(&state, &mut messages, |_| None);

        assert_eq!(count, 0);
    }

    #[test]
    fn inject_extended_sub_agent_results_injects_into_text_parts() {
        let mut cache = HashMap::new();
        cache.insert("test-agent".to_string(), "completed".to_string());
        let state = make_state(cache);

        let mut messages = vec![msg("m1", Role::Assistant, vec![Part::text("hello")])];

        let count = inject_extended_sub_agent_results(&state, &mut messages, |msg| {
            if msg.id == "m1" {
                Some("test-agent".to_string())
            } else {
                None
            }
        });

        assert_eq!(count, 1);
        assert!(
            messages[0].parts[0]
                .as_text()
                .unwrap()
                .contains("completed")
        );
    }

    // Helper trait for tests
    trait AsText {
        fn as_text(&self) -> Option<&str>;
    }

    impl AsText for Part {
        fn as_text(&self) -> Option<&str> {
            match self {
                Part::Text(s) => Some(s),
                _ => None,
            }
        }
    }
}
