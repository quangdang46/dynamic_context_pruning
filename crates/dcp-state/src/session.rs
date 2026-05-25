//! Session lifecycle operations per SPEC.md §3.
//!
//! Top-level entry points:
//!
//! * [`create_session_state`] — fresh empty state.
//! * [`reset_session_state`] — replace every field with default.
//! * [`check_session`] — react to a session-id change by reinitialising.
//! * [`ensure_session_initialized`] — load persisted state for a session
//!   id and restore prune bookkeeping.
//! * [`find_last_compaction_timestamp`] — derive the most recent
//!   compaction time from the message stream.
//! * [`count_turns`] — count turn-ends per SPEC §3.2.
//! * [`reset_on_compaction`] — apply SPEC §3.3 mutations after detecting
//!   a compaction event.
//! * [`rebuild_from_messages`] — KEY idempotent rebuild (PLAN §7.4 +
//!   SPEC §11.4).
//! * [`get_active_summary_token_usage`] — sum of `summary_tokens` across
//!   currently-active blocks.

use dcp_traits::{PersistedState, PersistedStateV1, StatePersistence};
use dcp_types::{BlockId, CompressionBlock, Message, Part, Role, RunId, SessionState};
use serde_json::Value as JsonValue;

use crate::config_like::ConfigLike;
use crate::message_refs::assign_message_refs;
use crate::nudges::{has_open_tool_call, has_text};
use crate::tool_cache::sync_tool_cache;

/// Construct a fresh [`SessionState`].
///
/// Equivalent to `SessionState::default()` but provided as a named
/// function for symmetry with [`reset_session_state`].
///
/// # Example
///
/// ```rust
/// let s = dcp_state::create_session_state();
/// assert_eq!(s.current_turn, 0);
/// ```
pub fn create_session_state() -> SessionState {
    SessionState::default()
}

/// Reset `state` to the default, post-construction shape.
///
/// SPEC.md §3.4 (`reset()`) — every field is replaced; the storage
/// backend is **not** touched.
///
/// # Example
///
/// ```rust
/// use dcp_state::{create_session_state, reset_session_state};
/// let mut s = create_session_state();
/// s.current_turn = 7;
/// reset_session_state(&mut s);
/// assert_eq!(s.current_turn, 0);
/// ```
pub fn reset_session_state(state: &mut SessionState) {
    *state = SessionState::default();
}

/// Detect a session-id transition.
///
/// If `incoming_session_id` differs from `state.session_id`, the state is
/// reset and the new session id is stored. Returns `true` when a change
/// occurred. Calling with the same id (or the first time, when state has
/// no id yet) records the id without clearing other bookkeeping.
///
/// # Example
///
/// ```rust
/// use dcp_state::{check_session, create_session_state};
/// let mut s = create_session_state();
/// assert!(check_session(&mut s, "sess-A"));     // first set
/// assert!(!check_session(&mut s, "sess-A"));    // unchanged
/// assert!(check_session(&mut s, "sess-B"));     // reinitialise
/// assert_eq!(s.session_id.as_deref(), Some("sess-B"));
/// ```
pub fn check_session(state: &mut SessionState, incoming_session_id: &str) -> bool {
    match state.session_id.as_deref() {
        Some(prev) if prev == incoming_session_id => false,
        Some(_) => {
            *state = SessionState::default();
            state.session_id = Some(incoming_session_id.to_string());
            true
        }
        None => {
            state.session_id = Some(incoming_session_id.to_string());
            true
        }
    }
}

/// Errors returned by [`ensure_session_initialized`].
#[derive(Debug, thiserror::Error)]
pub enum EnsureInitError {
    /// Persistence backend reported a load failure.
    #[error("persistence load failed: {0}")]
    Persistence(#[from] dcp_traits::PersistenceError),
}

/// Load the persisted state for `session_id` (if any) and restore the
/// prune bookkeeping into `state`.
///
/// SPEC.md §3.1 — performs steps 1–6 of session start and *not* the
/// message-derived steps 7–9. Callers use [`rebuild_from_messages`] (or a
/// downstream pipeline) to finish session start.
///
/// Restoration scope:
///
/// * `state.session_id` is set to `session_id`.
/// * `next_block_id`, `next_run_id`, `next_message_ref` are restored (and
///   promoted to `1` if a brand-new session has them at `0`).
/// * `state.prune.messages.frontier_message_ref` is restored.
/// * `state.last_compaction` is restored.
/// * Persisted [`Stats`] are taken verbatim where convertible; failures
///   silently fall back to default.
///
/// The function deliberately does *not* repopulate `tool_parameters` /
/// `tool_id_list` / `message_ids`. Those are derived from the message
/// stream by [`rebuild_from_messages`], guaranteeing the spec's
/// idempotence property.
///
/// Returns `Ok(true)` when persisted state was found and applied,
/// `Ok(false)` when the backend had nothing for the session id.
pub fn ensure_session_initialized<S: StatePersistence + ?Sized>(
    state: &mut SessionState,
    storage: &S,
    session_id: &str,
) -> Result<bool, EnsureInitError> {
    state.session_id = Some(session_id.to_string());
    let loaded = storage.load(session_id)?;
    let Some(persisted) = loaded else {
        return Ok(false);
    };
    apply_persisted(state, persisted);
    Ok(true)
}

fn apply_persisted(state: &mut SessionState, persisted: PersistedState) {
    let PersistedState::V1(v1) = persisted;
    let PersistedStateV1 {
        session_name: _,
        session_id,
        last_updated: _,
        current_turn,
        frontier_message_ref,
        next_block_id,
        next_run_id,
        next_message_ref,
        stats,
        nudges: _,
        prune: _,
        tool_index: _,
        message_id_map: _,
        compaction,
    } = v1;

    state.session_id = Some(session_id);
    state.current_turn = current_turn;
    state.prune.messages.frontier_message_ref = frontier_message_ref;
    state.prune.messages.next_block_id = BlockId::new(next_block_id.max(1));
    state.prune.messages.next_run_id = RunId::new(next_run_id.max(1));
    state.message_ids.next_ref = if next_message_ref == 0 {
        1
    } else {
        next_message_ref
    };

    if let Ok(s) = serde_json::from_value(stats) {
        state.stats = s;
    }
    if let JsonValue::Object(map) = compaction {
        if let Some(JsonValue::Number(n)) = map.get("last_compaction_at")
            && let Some(ms) = n.as_i64()
        {
            state.last_compaction = ms;
        }
        if let Some(JsonValue::Number(n)) = map.get("compactions_observed")
            && let Some(c) = n.as_u64()
        {
            state.stats.compactions_observed = c as u32;
        }
    }
}

/// Walk `messages` and return the timestamp of the most recent message
/// that looks like a compaction event marker.
///
/// The library never *creates* compaction markers — compaction is detected
/// in [`reset_on_compaction`] via id discontinuity (SPEC.md §3.3). This
/// helper covers the ancillary case where the host has injected a
/// `system`-role message describing the compaction; returning its
/// timestamp lets callers seed `state.last_compaction` from the stream.
///
/// Heuristic:
///
/// * Walk messages in reverse.
/// * If a message has `Role::System` and any `Text` part contains the
///   substring `"compact"` (case-insensitive), return its `time`.
/// * Otherwise return `0`.
///
/// # Example
///
/// ```rust
/// use dcp_state::find_last_compaction_timestamp;
/// use dcp_types::{Message, Role};
///
/// let messages = vec![
///     Message::system_text("s1", 1_000, "[Compaction] earlier history removed"),
///     Message::user_text("u1", 2_000, "hi"),
/// ];
/// assert_eq!(find_last_compaction_timestamp(&messages), 1_000);
/// ```
pub fn find_last_compaction_timestamp(messages: &[Message]) -> i64 {
    for m in messages.iter().rev() {
        if m.role != Role::System {
            continue;
        }
        for part in &m.parts {
            if let Part::Text(t) = part
                && t.to_ascii_lowercase().contains("compact")
            {
                return m.time;
            }
        }
    }
    0
}

/// Count completed turns observable in `messages` per SPEC.md §3.2.
///
/// A turn ends at every assistant message that has at least one `Text`
/// part and zero unmatched `ToolCall` parts. The function is total — it
/// runs in a single forward pass and never mutates state.
///
/// `state` is currently unused but kept in the signature so callers can
/// later honour memoisation flags without churn.
///
/// # Example
///
/// ```rust
/// use dcp_state::{count_turns, create_session_state};
/// use dcp_types::{Message, Part, Role};
///
/// let messages = vec![
///     Message::user_text("u1", 0, "hi"),
///     Message::assistant_text("a1", 0, "hello"), // turn end
///     Message::user_text("u2", 0, "more"),
///     Message::new("a2", Role::Assistant, vec![Part::reasoning("…")], 0), // not turn end
/// ];
/// let state = create_session_state();
/// assert_eq!(count_turns(&state, &messages), 1);
/// ```
pub fn count_turns(_state: &SessionState, messages: &[Message]) -> u32 {
    count_turns_through(messages)
}

/// Count turn-ends in `messages` ignoring any prior state. Used by
/// [`count_turns`] and by `tool_cache` to snapshot the per-call turn.
pub fn count_turns_through(messages: &[Message]) -> u32 {
    let mut count: u32 = 0;
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role != Role::Assistant {
            continue;
        }
        if has_text(msg) && !has_open_tool_call(msg, &messages[idx + 1..]) {
            count = count.saturating_add(1);
        }
    }
    count
}

/// Apply SPEC.md §3.3 mutations after a compaction event has been
/// detected. The detection itself happens upstream (cache-stability /
/// session pipeline) and is not this crate's responsibility.
///
/// Mutations:
///
/// 1. Clear `tool_parameters`, `tool_id_list`.
/// 2. Clear `message_ids` and reset `next_ref` to `1`.
/// 3. Mark every active block as inactive (audit trail preserved).
/// 4. Clear `prune.tools`.
/// 5. Set `last_compaction` to `now` (the caller supplies the timestamp;
///    callers that don't need a real wall-clock can pass `0`).
/// 6. Increment `stats.compactions_observed`.
pub fn reset_on_compaction(state: &mut SessionState) {
    let now = chrono::Utc::now().timestamp_millis();
    reset_on_compaction_at(state, now);
}

/// Same as [`reset_on_compaction`] but the caller controls the timestamp
/// (essential for deterministic tests).
pub fn reset_on_compaction_at(state: &mut SessionState, now_ms: i64) {
    state.tool_parameters.clear();
    state.tool_id_list.clear();
    state.message_ids.by_raw_id.clear();
    state.message_ids.by_ref.clear();
    state.message_ids.next_ref = 1;

    let active_ids: Vec<BlockId> = state
        .prune
        .messages
        .active_block_ids
        .iter()
        .copied()
        .collect();
    for id in active_ids {
        if let Some(block) = state.prune.messages.blocks_by_id.get_mut(&id) {
            block.active = false;
            block.deactivated_at = Some(now_ms);
            block.deactivated_by_block_id = None;
        }
    }
    state.prune.messages.active_block_ids.clear();
    state.prune.messages.active_by_anchor_message_id.clear();

    state.prune.tools.clear();
    state.last_compaction = now_ms;
    state.stats.compactions_observed = state.stats.compactions_observed.saturating_add(1);
}

/// **Idempotent rebuild** — PLAN.md §7.4 / SPEC.md §11.4.
///
/// Given the message stream and the persisted compression blocks, produce
/// a `SessionState` whose pruning decisions match what an in-progress
/// session would have computed. Pruning correctness depends on this
/// invariant: the property test
/// `prop_rebuild_idempotent` exercises it on random inputs.
///
/// What gets rebuilt:
///
/// * Every persisted block is inserted into `blocks_by_id` and (if active)
///   into `active_block_ids` and `active_by_anchor_message_id`.
/// * `next_block_id` and `next_run_id` advance past the maximum
///   loaded value.
/// * Tool cache (`tool_parameters`, `tool_id_list`) is rebuilt from the
///   message stream via [`sync_tool_cache`].
/// * Message references are reassigned in first-seen order.
/// * `current_turn` is recomputed via [`count_turns`].
/// * `session_id` is heuristically set to the last message's id (matching
///   the spec's "derived deterministically from the last message's id"
///   rule).
///
/// What is **not** restored from blocks: pending in-flight work,
/// cache-stability snapshots, or any timing telemetry. SPEC.md §11.4
/// explicitly excludes these from the rebuild contract.
pub fn rebuild_from_messages<C: ConfigLike + ?Sized>(
    messages: &[Message],
    persisted_blocks: Vec<CompressionBlock>,
    config: &C,
) -> SessionState {
    let mut state = SessionState {
        session_id: messages.last().map(|m| m.id.clone()),
        ..SessionState::default()
    };

    let mut max_block: u32 = 0;
    let mut max_run: u32 = 0;
    for block in persisted_blocks {
        max_block = max_block.max(block.block_id.value());
        max_run = max_run.max(block.run_id.value());

        let bid = block.block_id;
        let active = block.active;
        let anchor = block.anchor_message_id.clone();
        state.prune.messages.blocks_by_id.insert(bid, block);
        if active {
            state.prune.messages.active_block_ids.insert(bid);
            state
                .prune
                .messages
                .active_by_anchor_message_id
                .insert(anchor, bid);
        }
    }
    state.prune.messages.next_block_id = BlockId::new(max_block.saturating_add(1).max(1));
    state.prune.messages.next_run_id = RunId::new(max_run.saturating_add(1).max(1));

    sync_tool_cache(&mut state, config, messages);
    assign_message_refs(&mut state, messages);
    state.current_turn = count_turns(&state, messages);
    state.last_message_was_assistant_text = is_last_assistant_text(messages);
    state.last_compaction = find_last_compaction_timestamp(messages);

    state
}

fn is_last_assistant_text(messages: &[Message]) -> bool {
    let Some(last) = messages.last() else {
        return false;
    };
    if last.role != Role::Assistant {
        return false;
    }
    has_text(last) && !has_open_tool_call(last, &[])
}

/// Sum `summary_tokens` across blocks whose `active` flag is true.
///
/// Mirrors the "active summary token usage" intuition — how many tokens
/// the model is currently *seeing* via committed summaries (vs the raw
/// content they replaced).
///
/// # Example
///
/// ```rust
/// use dcp_state::get_active_summary_token_usage;
/// use dcp_types::{BlockId, CompressionBlock, CompressionMode, RunId, SessionState};
///
/// let mut state = SessionState::default();
/// let mut block = CompressionBlock::new(
///     BlockId::new(1), RunId::new(1), CompressionMode::Range,
///     "t", "s", "m0001", "m0002", "raw1", "raw2",
/// );
/// block.summary_tokens = 100;
/// state.prune.messages.blocks_by_id.insert(block.block_id, block.clone());
/// state.prune.messages.active_block_ids.insert(block.block_id);
///
/// assert_eq!(get_active_summary_token_usage(&state), 100);
/// ```
pub fn get_active_summary_token_usage(state: &SessionState) -> u64 {
    state
        .prune
        .messages
        .active_block_ids
        .iter()
        .filter_map(|id| state.prune.messages.blocks_by_id.get(id))
        .map(|b| b.summary_tokens)
        .sum()
}

// ────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_like::{StaticConfigLike, default_tracked_tools};
    use dcp_traits::defaults::NoopStorage;
    use dcp_types::{CompressionMode, Message, Part, Role, ToolStatus};
    use serde_json::json;

    fn cfg() -> StaticConfigLike {
        StaticConfigLike {
            tracked_tools: default_tracked_tools(),
            ..StaticConfigLike::default()
        }
    }

    // ----- create / reset / check_session -----

    #[test]
    fn create_session_state_is_default() {
        let s = create_session_state();
        assert_eq!(s, SessionState::default());
    }

    #[test]
    fn reset_session_state_clears_fields() {
        let mut s = SessionState {
            current_turn: 9,
            last_compaction: 1_000,
            ..SessionState::default()
        };
        reset_session_state(&mut s);
        assert_eq!(s, SessionState::default());
    }

    #[test]
    fn check_session_records_first_id() {
        let mut s = create_session_state();
        assert!(check_session(&mut s, "sess-A"));
        assert_eq!(s.session_id.as_deref(), Some("sess-A"));
        assert!(!check_session(&mut s, "sess-A"));
    }

    #[test]
    fn check_session_resets_on_change() {
        let mut s = create_session_state();
        check_session(&mut s, "sess-A");
        s.current_turn = 42;
        s.tool_id_list.push("c1".into());
        assert!(check_session(&mut s, "sess-B"));
        assert_eq!(s.session_id.as_deref(), Some("sess-B"));
        assert_eq!(s.current_turn, 0);
        assert!(s.tool_id_list.is_empty());
    }

    // ----- ensure_session_initialized -----

    #[test]
    fn ensure_session_initialized_no_persisted_returns_false() {
        let mut s = create_session_state();
        let store = NoopStorage::new();
        assert!(!ensure_session_initialized(&mut s, &store, "sess-1").unwrap());
        assert_eq!(s.session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn ensure_session_initialized_restores_persisted_fields() {
        let mut s = create_session_state();
        let store = NoopStorage::new();
        let stats = dcp_types::Stats {
            total_prune_tokens: 123,
            ..dcp_types::Stats::default()
        };
        let persisted = PersistedState::V1(PersistedStateV1 {
            session_name: None,
            session_id: "sess-1".into(),
            last_updated: "2024-01-01T00:00:00Z".into(),
            current_turn: 7,
            frontier_message_ref: Some("m0042".into()),
            next_block_id: 5,
            next_run_id: 3,
            next_message_ref: 11,
            stats: serde_json::to_value(&stats).unwrap(),
            nudges: serde_json::Value::Null,
            prune: serde_json::Value::Null,
            tool_index: serde_json::Value::Null,
            message_id_map: serde_json::Value::Null,
            compaction: serde_json::json!({
                "last_compaction_at": 1_700_000_000_i64,
                "compactions_observed": 4,
            }),
        });
        store.save("sess-1", &persisted).unwrap();

        assert!(ensure_session_initialized(&mut s, &store, "sess-1").unwrap());
        assert_eq!(s.session_id.as_deref(), Some("sess-1"));
        assert_eq!(s.current_turn, 7);
        assert_eq!(s.prune.messages.next_block_id.value(), 5);
        assert_eq!(s.prune.messages.next_run_id.value(), 3);
        assert_eq!(s.message_ids.next_ref, 11);
        assert_eq!(
            s.prune.messages.frontier_message_ref.as_deref(),
            Some("m0042")
        );
        assert_eq!(s.stats.total_prune_tokens, 123);
        assert_eq!(s.last_compaction, 1_700_000_000);
        assert_eq!(s.stats.compactions_observed, 4);
    }

    // ----- find_last_compaction_timestamp -----

    #[test]
    fn finds_compaction_in_system_text() {
        let messages = vec![
            Message::system_text("s1", 1_000, "regular system prompt"),
            Message::user_text("u1", 2_000, "hi"),
            Message::system_text("s2", 3_000, "[Compaction] truncated"),
            Message::user_text("u2", 4_000, "more"),
        ];
        assert_eq!(find_last_compaction_timestamp(&messages), 3_000);
    }

    #[test]
    fn returns_zero_when_no_compaction_marker() {
        let messages = vec![
            Message::user_text("u1", 1_000, "hi"),
            Message::assistant_text("a1", 2_000, "hello"),
        ];
        assert_eq!(find_last_compaction_timestamp(&messages), 0);
    }

    // ----- count_turns -----

    #[test]
    fn count_turns_basic_pair() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        assert_eq!(count_turns(&SessionState::default(), &messages), 1);
    }

    #[test]
    fn count_turns_skips_open_tool_call() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::new(
                "a1",
                Role::Assistant,
                vec![
                    Part::text("doing"),
                    Part::tool_call("c1", "read", json!({"path": "x"})),
                ],
                0,
            ),
        ];
        assert_eq!(count_turns(&SessionState::default(), &messages), 0);
    }

    #[test]
    fn count_turns_with_paired_tool_completes() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::new(
                "a1",
                Role::Assistant,
                vec![
                    Part::text("doing"),
                    Part::tool_call("c1", "read", json!({"path": "x"})),
                ],
                0,
            ),
            Message::new(
                "u2",
                Role::User,
                vec![Part::tool_result(
                    "c1",
                    ToolStatus::Completed,
                    Some("ok".into()),
                    None,
                )],
                0,
            ),
            Message::assistant_text("a2", 0, "done"),
        ];
        assert_eq!(count_turns(&SessionState::default(), &messages), 2);
    }

    // ----- reset_on_compaction -----

    #[test]
    fn reset_on_compaction_clears_caches_and_deactivates_blocks() {
        let mut state = SessionState::default();
        state.tool_id_list.push("c1".into());
        state.tool_parameters.insert(
            "c1".into(),
            dcp_types::ToolParameterEntry {
                tool: "read".into(),
                signature: "read::{}".into(),
                status: Some(ToolStatus::Completed),
                turn: 1,
                message_id: "raw".into(),
                result_message_id: None,
                paths: vec![],
                token_count: None,
            },
        );
        state
            .message_ids
            .by_raw_id
            .insert("u1".into(), "m0001".into());
        state.message_ids.by_ref.insert("m0001".into(), "u1".into());
        state.message_ids.next_ref = 4;
        state.prune.tools.insert("c1".into(), 100);

        let mut block = CompressionBlock::new(
            BlockId::new(1),
            RunId::new(1),
            CompressionMode::Range,
            "t",
            "s",
            "m0001",
            "m0002",
            "raw1",
            "raw2",
        );
        block.active = true;
        state
            .prune
            .messages
            .blocks_by_id
            .insert(block.block_id, block.clone());
        state.prune.messages.active_block_ids.insert(block.block_id);
        state
            .prune
            .messages
            .active_by_anchor_message_id
            .insert("raw1".into(), block.block_id);

        reset_on_compaction_at(&mut state, 5_000);

        assert!(state.tool_parameters.is_empty());
        assert!(state.tool_id_list.is_empty());
        assert!(state.message_ids.by_raw_id.is_empty());
        assert!(state.message_ids.by_ref.is_empty());
        assert_eq!(state.message_ids.next_ref, 1);
        assert!(state.prune.tools.is_empty());
        assert!(state.prune.messages.active_block_ids.is_empty());
        assert!(state.prune.messages.active_by_anchor_message_id.is_empty());

        // Block is preserved in audit trail with active=false.
        let audited = &state.prune.messages.blocks_by_id[&BlockId::new(1)];
        assert!(!audited.active);
        assert_eq!(audited.deactivated_at, Some(5_000));
        assert_eq!(audited.deactivated_by_block_id, None);

        assert_eq!(state.last_compaction, 5_000);
        assert_eq!(state.stats.compactions_observed, 1);
    }

    // ----- rebuild_from_messages -----

    fn make_block(id: u32, anchor: &str, summary_tokens: u64, active: bool) -> CompressionBlock {
        let mut b = CompressionBlock::new(
            BlockId::new(id),
            RunId::new(id),
            CompressionMode::Range,
            "topic",
            "summary",
            "m0001",
            "m0002",
            anchor,
            "raw-compress",
        );
        b.active = active;
        b.summary_tokens = summary_tokens;
        b
    }

    #[test]
    fn rebuild_assigns_session_id_to_last_message() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let state = rebuild_from_messages(&messages, vec![], &cfg());
        assert_eq!(state.session_id.as_deref(), Some("a1"));
    }

    #[test]
    fn rebuild_sets_allocators_past_max_loaded_block() {
        let blocks = vec![
            make_block(3, "raw-a", 5, true),
            make_block(7, "raw-b", 10, false),
        ];
        let state = rebuild_from_messages(&[], blocks, &cfg());
        assert_eq!(state.prune.messages.next_block_id.value(), 8);
        assert_eq!(state.prune.messages.next_run_id.value(), 8);
    }

    #[test]
    fn rebuild_only_active_blocks_register_anchor_lookup() {
        let blocks = vec![
            make_block(1, "raw-a", 5, true),
            make_block(2, "raw-b", 5, false),
        ];
        let state = rebuild_from_messages(&[], blocks, &cfg());
        assert!(
            state
                .prune
                .messages
                .active_block_ids
                .contains(&BlockId::new(1))
        );
        assert!(
            !state
                .prune
                .messages
                .active_block_ids
                .contains(&BlockId::new(2))
        );
        assert_eq!(
            state.prune.messages.active_by_anchor_message_id["raw-a"],
            BlockId::new(1)
        );
        assert!(
            !state
                .prune
                .messages
                .active_by_anchor_message_id
                .contains_key("raw-b")
        );
    }

    #[test]
    fn rebuild_recomputes_turn_count_and_tool_cache() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::new(
                "a1",
                Role::Assistant,
                vec![
                    Part::text("doing"),
                    Part::tool_call("c1", "read", json!({"path": "x"})),
                ],
                0,
            ),
            Message::new(
                "u2",
                Role::User,
                vec![Part::tool_result(
                    "c1",
                    ToolStatus::Completed,
                    Some("ok".into()),
                    None,
                )],
                0,
            ),
            Message::assistant_text("a2", 0, "done"),
        ];
        let state = rebuild_from_messages(&messages, vec![], &cfg());
        assert_eq!(state.current_turn, 2);
        assert_eq!(state.tool_id_list, vec!["c1".to_string()]);
        assert!(state.last_message_was_assistant_text);
        assert_eq!(state.message_ids.by_raw_id["u1"], "m0001");
        assert_eq!(state.message_ids.by_raw_id["a1"], "m0002");
        assert_eq!(state.message_ids.by_raw_id["a2"], "m0004");
    }

    #[test]
    fn rebuild_from_empty_inputs_is_default_with_alloc_at_one() {
        let state = rebuild_from_messages(&[], vec![], &cfg());
        assert_eq!(state.prune.messages.next_block_id.value(), 1);
        assert_eq!(state.prune.messages.next_run_id.value(), 1);
        assert_eq!(state.current_turn, 0);
        assert!(state.session_id.is_none());
    }

    // ----- get_active_summary_token_usage -----

    #[test]
    fn get_active_summary_token_usage_sums_active_only() {
        let mut state = SessionState::default();
        let b1 = make_block(1, "a", 100, true);
        let b2 = make_block(2, "b", 50, true);
        let b3 = make_block(3, "c", 999, false); // inactive — excluded
        state.prune.messages.active_block_ids.insert(b1.block_id);
        state.prune.messages.active_block_ids.insert(b2.block_id);
        state.prune.messages.blocks_by_id.insert(b1.block_id, b1);
        state.prune.messages.blocks_by_id.insert(b2.block_id, b2);
        state.prune.messages.blocks_by_id.insert(b3.block_id, b3);
        assert_eq!(get_active_summary_token_usage(&state), 150);
    }

    #[test]
    fn get_active_summary_token_usage_zero_when_no_active() {
        let s = SessionState::default();
        assert_eq!(get_active_summary_token_usage(&s), 0);
    }

    #[test]
    fn rebuild_idempotent_double_call() {
        // Sanity check: rebuilding twice from the same inputs gives the
        // same SessionState (modulo HashMap ordering, which `PartialEq`
        // handles).
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::new(
                "a1",
                Role::Assistant,
                vec![
                    Part::text("doing"),
                    Part::tool_call("c1", "read", json!({"path": "x"})),
                ],
                0,
            ),
            Message::new(
                "u2",
                Role::User,
                vec![Part::tool_result(
                    "c1",
                    ToolStatus::Completed,
                    Some("ok".into()),
                    None,
                )],
                0,
            ),
            Message::assistant_text("a2", 0, "done"),
        ];
        let blocks = vec![make_block(1, "u1", 50, true)];
        let s1 = rebuild_from_messages(&messages, blocks.clone(), &cfg());
        let s2 = rebuild_from_messages(&messages, blocks, &cfg());
        assert_eq!(s1, s2);
    }
}
