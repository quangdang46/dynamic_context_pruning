//! Tool tracking per SPEC.md §4.
//!
//! [`sync_tool_cache`] walks a validated message list and rebuilds:
//!
//! * [`SessionState::tool_parameters`] — keyed by `call_id`, populated
//!   with the canonical signature (§4.4–4.5), call/result pairing
//!   (§4.2), per-call turn snapshot (§4.3), and extracted file paths
//!   (§4.6).
//! * [`SessionState::tool_id_list`] — first-seen insertion-ordered list of
//!   `call_id`s (§4.7).
//!
//! The function is total and idempotent — repeated invocations with the
//! same `(messages, config)` produce identical output. That is what makes
//! [`super::session::rebuild_from_messages`] sound.
//!
//! Counters that this module increments live on `state.stats`:
//! `orphan_tool_results`, `invalid_status_transitions`,
//! `normalize_depth_clamped`, `path_null_byte_stripped`. Other counters
//! are touched elsewhere.

use std::collections::HashSet;

use dcp_types::{Message, Part, Role, SessionState, ToolParameterEntry, ToolStatus};
use serde_json::Value as JsonValue;

use crate::config_like::ConfigLike;
use crate::session::count_turns_through;

/// Maximum recursion depth allowed during JSON parameter normalization.
/// SPEC.md §4.4 — beyond this depth, the original value is returned and
/// `stats.normalize_depth_clamped` is incremented.
pub const NORMALIZE_MAX_DEPTH: u32 = 1000;

/// Maximum byte length for a single extracted file path. SPEC.md §4.6 —
/// longer paths are truncated to this many UTF-8-safe bytes.
pub const PATH_MAX_BYTES: usize = 4096;

/// Rebuild [`SessionState::tool_parameters`] and
/// [`SessionState::tool_id_list`] from `messages`.
///
/// Behavior matches SPEC.md §4 end-to-end:
///
/// * Re-running with the same inputs produces identical output (the spec's
///   idempotence requirement, SPEC.md §11.4).
/// * Tool calls are collected in first-seen order.
/// * Each entry's `turn` snapshot is the value `count_turns_through` would
///   return for the prefix ending immediately before the assistant message
///   that emitted the call.
/// * Status transitions follow the table in SPEC.md §4.3.
/// * Path extraction respects the `tracked_tools` list from
///   [`ConfigLike`].
///
/// # Example
///
/// ```rust
/// use dcp_state::config_like::{StaticConfigLike, default_tracked_tools};
/// use dcp_state::tool_cache::sync_tool_cache;
/// use dcp_types::{Message, Part, Role, SessionState, ToolStatus};
///
/// let messages = vec![
///     Message::new(
///         "a1", Role::Assistant,
///         vec![Part::tool_call("c1", "read", serde_json::json!({"path": "x"}))],
///         0,
///     ),
///     Message::new(
///         "u1", Role::User,
///         vec![Part::tool_result("c1", ToolStatus::Completed, Some("ok".into()), None)],
///         0,
///     ),
/// ];
/// let cfg = StaticConfigLike { tracked_tools: default_tracked_tools(), ..Default::default() };
/// let mut state = SessionState::default();
/// sync_tool_cache(&mut state, &cfg, &messages);
///
/// assert_eq!(state.tool_id_list, vec!["c1".to_string()]);
/// let entry = &state.tool_parameters["c1"];
/// assert_eq!(entry.tool, "read");
/// assert_eq!(entry.status, Some(ToolStatus::Completed));
/// assert_eq!(entry.paths, vec!["x".to_string()]);
/// ```
pub fn sync_tool_cache<C: ConfigLike + ?Sized>(
    state: &mut SessionState,
    config: &C,
    messages: &[Message],
) {
    // SPEC §4.7: full rebuild, not incremental.
    state.tool_parameters.clear();
    state.tool_id_list.clear();

    let mut seen_calls: HashSet<String> = HashSet::new();
    let mut depth_clamped: u32 = 0;
    let mut path_null_bytes: u32 = 0;
    let mut invalid_transitions: u32 = 0;
    let mut orphan_results: u32 = 0;

    for (idx, msg) in messages.iter().enumerate() {
        match msg.role {
            Role::Assistant => {
                for part in &msg.parts {
                    if let Part::ToolCall {
                        call_id,
                        tool,
                        input,
                    } = part
                    {
                        // First-seen list (SPEC §4.7)
                        if seen_calls.insert(call_id.clone()) {
                            state.tool_id_list.push(call_id.clone());
                        }
                        // Build/refresh entry (SPEC §4.1)
                        let normalized = normalize_value(input, 0, config, &mut depth_clamped);
                        let signature = signature_string(tool, &normalized);
                        let paths = extract_file_paths(tool, input, config, &mut path_null_bytes);
                        let turn_snapshot = count_turns_through(&messages[..idx]);
                        let entry = state.tool_parameters.entry(call_id.clone()).or_insert(
                            ToolParameterEntry {
                                tool: tool.clone(),
                                signature: signature.clone(),
                                status: None,
                                turn: turn_snapshot,
                                message_id: msg.id.clone(),
                                result_message_id: None,
                                paths: paths.clone(),
                                token_count: None,
                            },
                        );
                        // Re-emission of the same call_id should not change
                        // the captured turn (SPEC §4.1: "at the time the
                        // tool call was *first* observed").
                        entry.tool = tool.clone();
                        entry.signature = signature;
                        entry.paths = paths;
                        // message_id always points at the most recent
                        // assistant message that re-emitted the call so
                        // result-pairing search has the right anchor; in
                        // practice hosts do not duplicate calls.
                        entry.message_id = msg.id.clone();
                    }
                }
            }
            Role::User => {
                for part in &msg.parts {
                    if let Part::ToolResult {
                        call_id, status, ..
                    } = part
                    {
                        match state.tool_parameters.get_mut(call_id) {
                            Some(entry) => {
                                if can_transition(entry.status, *status) {
                                    entry.status = Some(*status);
                                } else {
                                    invalid_transitions += 1;
                                }
                                entry.result_message_id = Some(msg.id.clone());
                            }
                            None => {
                                // SPEC §4.2: orphan result.
                                orphan_results += 1;
                            }
                        }
                    }
                }
            }
            Role::System => {}
            // `Role` is `#[non_exhaustive]`; ignore unknown roles.
            _ => {}
        }
    }

    state.stats.normalize_depth_clamped = state
        .stats
        .normalize_depth_clamped
        .saturating_add(depth_clamped);
    state.stats.path_null_byte_stripped = state
        .stats
        .path_null_byte_stripped
        .saturating_add(path_null_bytes);
    state.stats.invalid_status_transitions = state
        .stats
        .invalid_status_transitions
        .saturating_add(invalid_transitions);
    state.stats.orphan_tool_results = state
        .stats
        .orphan_tool_results
        .saturating_add(orphan_results);
}

// ────────────────────────────────────────────────────────────────────────
// Normalization (SPEC §4.4)
// ────────────────────────────────────────────────────────────────────────

/// Recursive normalize-and-canonicalize. Returns a new owned [`JsonValue`]
/// with object keys sorted lexicographically and `null` values dropped per
/// the configured `drop_null_keys` list.
fn normalize_value<C: ConfigLike + ?Sized>(
    value: &JsonValue,
    depth: u32,
    config: &C,
    depth_clamped: &mut u32,
) -> JsonValue {
    if depth >= NORMALIZE_MAX_DEPTH {
        *depth_clamped = depth_clamped.saturating_add(1);
        return value.clone();
    }
    match value {
        JsonValue::Object(map) => {
            let mut entries: Vec<(String, JsonValue)> = Vec::with_capacity(map.len());
            for (k, v) in map {
                if v.is_null() && is_drop_null_key(k, config) {
                    continue;
                }
                entries.push((
                    k.clone(),
                    normalize_value(v, depth + 1, config, depth_clamped),
                ));
            }
            // Lexicographic key sort (UTF-8 byte order).
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = serde_json::Map::with_capacity(entries.len());
            for (k, v) in entries {
                out.insert(k, v);
            }
            JsonValue::Object(out)
        }
        JsonValue::Array(arr) => JsonValue::Array(
            arr.iter()
                .map(|v| normalize_value(v, depth + 1, config, depth_clamped))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn is_drop_null_key<C: ConfigLike + ?Sized>(key: &str, config: &C) -> bool {
    config.drop_null_keys().iter().any(|k| k == key)
}

/// Compute the canonical signature `<tool>::<canonical_json>` per
/// SPEC.md §4.5.
///
/// Caller passes an *already-normalized* value; this function only
/// emits the canonical JSON form. `serde_json::to_string` produces
/// keys in insertion order and no whitespace, which after
/// [`normalize_value`] satisfies the spec.
pub fn signature_string(tool: &str, normalized: &JsonValue) -> String {
    let canonical = serde_json::to_string(normalized).unwrap_or_else(|_| "null".to_string());
    format!("{tool}::{canonical}")
}

// ────────────────────────────────────────────────────────────────────────
// Path extraction (SPEC §4.6)
// ────────────────────────────────────────────────────────────────────────

/// Extract file paths referenced by a tool call's input.
///
/// Public so that downstream crates (notably `dcp-prune`'s
/// stale-file-reads strategy) can re-use the canonical extraction.
/// Behavior:
///
/// * Returns empty for tools not in the configured `tracked_tools` list.
/// * Honors the SPEC's priority order: `path`, `file_path`, `filename`.
/// * Special-cases `multiedit` to also collect `edits[].path`.
/// * De-duplicates while preserving first occurrence.
/// * Strips embedded null bytes and reports them via the out-parameter.
/// * Truncates to `PATH_MAX_BYTES` at a UTF-8 codepoint boundary.
/// * Strips a single leading `./` and collapses repeated `/`.
pub fn extract_file_paths<C: ConfigLike + ?Sized>(
    tool: &str,
    parameters: &JsonValue,
    config: &C,
    path_null_bytes: &mut u32,
) -> Vec<String> {
    if !config.is_tracked_tool(tool) {
        return Vec::new();
    }

    let mut out: Vec<String> = Vec::new();

    if let JsonValue::Object(map) = parameters {
        for key in ["path", "file_path", "filename"] {
            if let Some(JsonValue::String(s)) = map.get(key) {
                if !s.is_empty() {
                    push_path(&mut out, s, path_null_bytes);
                }
            }
        }
        if tool == "multiedit" {
            if let Some(JsonValue::Array(edits)) = map.get("edits") {
                for edit in edits {
                    if let JsonValue::Object(em) = edit
                        && let Some(JsonValue::String(p)) = em.get("path")
                        && !p.is_empty()
                    {
                        push_path(&mut out, p, path_null_bytes);
                    }
                }
            }
        }
    }

    dedup_preserve_order(&mut out);
    out
}

fn push_path(out: &mut Vec<String>, raw: &str, path_null_bytes: &mut u32) {
    let stripped = if raw.contains('\0') {
        *path_null_bytes = path_null_bytes.saturating_add(1);
        raw.replace('\0', "")
    } else {
        raw.to_string()
    };
    let normalized = normalize_path(&stripped);
    let truncated = truncate_utf8(&normalized, PATH_MAX_BYTES);
    out.push(truncated);
}

/// Strip a single leading `./` and collapse runs of `/` (SPEC §4.6 step 5).
fn normalize_path(p: &str) -> String {
    let trimmed = p.strip_prefix("./").unwrap_or(p);
    // Collapse consecutive '/' into one. We deliberately do not touch '\\'.
    let mut out = String::with_capacity(trimmed.len());
    let mut last_was_slash = false;
    for ch in trimmed.chars() {
        if ch == '/' {
            if !last_was_slash {
                out.push(ch);
            }
            last_was_slash = true;
        } else {
            out.push(ch);
            last_was_slash = false;
        }
    }
    out
}

/// Truncate `s` to at most `max_bytes`, respecting UTF-8 codepoint
/// boundaries (SPEC §11.3).
fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn dedup_preserve_order(v: &mut Vec<String>) {
    let mut seen: HashSet<String> = HashSet::with_capacity(v.len());
    v.retain(|item| seen.insert(item.clone()));
}

// ────────────────────────────────────────────────────────────────────────
// Status transitions (SPEC §4.3)
// ────────────────────────────────────────────────────────────────────────

/// True when transitioning from `current` to `next` is allowed.
///
/// SPEC.md §4.3 transition table. `None` is the implicit initial state.
fn can_transition(current: Option<ToolStatus>, next: ToolStatus) -> bool {
    use ToolStatus::*;
    match (current, next) {
        (None, _) => true,
        (Some(Pending), _) => true,
        (Some(Running), Pending) => false,
        (Some(Running), _) => true,
        (Some(Completed), Completed) => true,
        (Some(Completed), _) => false,
        (Some(Error), Error) => true,
        (Some(Error), _) => false,
        // `ToolStatus` is `#[non_exhaustive]`; conservatively reject
        // transitions involving unknown variants.
        (Some(_), _) => false,
    }
}

// ────────────────────────────────────────────────────────────────────────
// Helpers exposed for assertions in `session::tests`.
// ────────────────────────────────────────────────────────────────────────

#[doc(hidden)]
pub fn _normalize_for_test(v: &JsonValue) -> JsonValue {
    let mut clamped = 0;
    let cfg = crate::config_like::StaticConfigLike::default();
    normalize_value(v, 0, &cfg, &mut clamped)
}

// ────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_like::{StaticConfigLike, default_tracked_tools};
    use dcp_types::{Message, Part, Role, ToolStatus};
    use serde_json::json;

    fn cfg_with_tracked() -> StaticConfigLike {
        StaticConfigLike {
            tracked_tools: default_tracked_tools(),
            ..StaticConfigLike::default()
        }
    }

    fn assistant_call(id: &str, call_id: &str, tool: &str, input: serde_json::Value) -> Message {
        Message::new(
            id,
            Role::Assistant,
            vec![Part::tool_call(call_id, tool, input)],
            0,
        )
    }

    fn user_result(
        id: &str,
        call_id: &str,
        status: ToolStatus,
        out: Option<&str>,
        err: Option<&str>,
    ) -> Message {
        Message::new(
            id,
            Role::User,
            vec![Part::tool_result(
                call_id,
                status,
                out.map(String::from),
                err.map(String::from),
            )],
            0,
        )
    }

    // ----- normalize_value -----

    #[test]
    fn normalize_sorts_keys() {
        let mut clamped = 0;
        let cfg = StaticConfigLike::default();
        let v = json!({"b": 1, "a": 2});
        let n = normalize_value(&v, 0, &cfg, &mut clamped);
        let s = serde_json::to_string(&n).unwrap();
        assert_eq!(s, r#"{"a":2,"b":1}"#);
    }

    #[test]
    fn normalize_recurses_into_arrays_and_objects() {
        let mut clamped = 0;
        let cfg = StaticConfigLike::default();
        let v = json!({"x": [{"b": 1, "a": 2}], "y": {"c": 3, "b": [{"z": 1, "y": 2}]}});
        let n = normalize_value(&v, 0, &cfg, &mut clamped);
        let s = serde_json::to_string(&n).unwrap();
        // All keys at every level are sorted.
        assert_eq!(
            s,
            r#"{"x":[{"a":2,"b":1}],"y":{"b":[{"y":2,"z":1}],"c":3}}"#
        );
    }

    #[test]
    fn normalize_preserves_array_order() {
        let mut clamped = 0;
        let cfg = StaticConfigLike::default();
        let v = json!([3, 1, 2]);
        let n = normalize_value(&v, 0, &cfg, &mut clamped);
        assert_eq!(serde_json::to_string(&n).unwrap(), "[3,1,2]");
    }

    #[test]
    fn normalize_keeps_null_by_default() {
        let mut clamped = 0;
        let cfg = StaticConfigLike::default();
        let v = json!({"a": null, "b": 1});
        let n = normalize_value(&v, 0, &cfg, &mut clamped);
        let s = serde_json::to_string(&n).unwrap();
        assert_eq!(s, r#"{"a":null,"b":1}"#);
    }

    #[test]
    fn normalize_drops_null_when_configured() {
        let mut clamped = 0;
        let cfg = StaticConfigLike {
            drop_null_keys: vec!["a".into()],
            ..StaticConfigLike::default()
        };
        let v = json!({"a": null, "b": null, "c": 1});
        let n = normalize_value(&v, 0, &cfg, &mut clamped);
        let s = serde_json::to_string(&n).unwrap();
        assert_eq!(s, r#"{"b":null,"c":1}"#);
    }

    #[test]
    fn normalize_clamps_at_max_depth() {
        // Build {"a": {"a": ... 1010 deep }}.
        let mut v = json!(1);
        for _ in 0..1010 {
            let mut m = serde_json::Map::new();
            m.insert("a".into(), v);
            v = JsonValue::Object(m);
        }
        let mut clamped = 0;
        let cfg = StaticConfigLike::default();
        let _ = normalize_value(&v, 0, &cfg, &mut clamped);
        assert!(clamped >= 1, "expected at least one clamp event");
    }

    // ----- signature -----

    #[test]
    fn signature_format_matches_spec() {
        let normalized = json!({"a": 1, "b": 2});
        let sig = signature_string("read", &normalized);
        assert_eq!(sig, r#"read::{"a":1,"b":2}"#);
    }

    // ----- extract_file_paths -----

    #[test]
    fn extract_path_priority_order() {
        let cfg = cfg_with_tracked();
        let mut nb = 0;
        let v = json!({"path": "p1", "file_path": "p2", "filename": "p3"});
        assert_eq!(
            extract_file_paths("read", &v, &cfg, &mut nb),
            vec!["p1".to_string(), "p2".to_string(), "p3".to_string()]
        );
    }

    #[test]
    fn extract_path_skips_non_tracked_tool() {
        let cfg = cfg_with_tracked();
        let mut nb = 0;
        let v = json!({"path": "p"});
        assert!(extract_file_paths("bash", &v, &cfg, &mut nb).is_empty());
    }

    #[test]
    fn extract_multiedit_collects_edits() {
        let cfg = cfg_with_tracked();
        let mut nb = 0;
        let v = json!({"path": "main", "edits": [{"path": "a"}, {"path": "b"}, {"other": "x"}]});
        let paths = extract_file_paths("multiedit", &v, &cfg, &mut nb);
        assert_eq!(paths, vec!["main", "a", "b"]);
    }

    #[test]
    fn extract_dedups_preserving_order() {
        let cfg = cfg_with_tracked();
        let mut nb = 0;
        let v = json!({"path": "p1", "file_path": "p1", "filename": "p2"});
        let paths = extract_file_paths("read", &v, &cfg, &mut nb);
        assert_eq!(paths, vec!["p1", "p2"]);
    }

    #[test]
    fn extract_strips_null_byte() {
        let cfg = cfg_with_tracked();
        let mut nb = 0;
        let v = json!({"path": "a\0b"});
        let paths = extract_file_paths("read", &v, &cfg, &mut nb);
        assert_eq!(paths, vec!["ab"]);
        assert_eq!(nb, 1);
    }

    #[test]
    fn extract_normalizes_leading_dot_slash_and_double_slash() {
        let cfg = cfg_with_tracked();
        let mut nb = 0;
        let v = json!({"path": "./a//b///c"});
        let paths = extract_file_paths("read", &v, &cfg, &mut nb);
        assert_eq!(paths, vec!["a/b/c"]);
    }

    #[test]
    fn extract_truncates_long_paths_at_codepoint_boundary() {
        let cfg = cfg_with_tracked();
        let mut nb = 0;
        // 1500 copies of 4-byte emoji = 6000 bytes; should truncate to ≤ 4096.
        let long: String = "🦀".repeat(1500);
        let v = json!({"path": long});
        let paths = extract_file_paths("read", &v, &cfg, &mut nb);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].len() <= PATH_MAX_BYTES);
        // No partial codepoints.
        assert!(paths[0].chars().all(|c| c == '🦀'));
    }

    #[test]
    fn extract_ignores_empty_string_path() {
        let cfg = cfg_with_tracked();
        let mut nb = 0;
        let v = json!({"path": ""});
        assert!(extract_file_paths("read", &v, &cfg, &mut nb).is_empty());
    }

    // ----- can_transition -----

    #[test]
    fn transition_table_matches_spec() {
        use ToolStatus::*;
        // (none) → all allowed
        for s in [Pending, Running, Completed, Error] {
            assert!(can_transition(None, s));
        }
        // pending → all allowed
        for s in [Pending, Running, Completed, Error] {
            assert!(can_transition(Some(Pending), s));
        }
        // running → not pending
        assert!(!can_transition(Some(Running), Pending));
        assert!(can_transition(Some(Running), Running));
        assert!(can_transition(Some(Running), Completed));
        assert!(can_transition(Some(Running), Error));
        // completed → only completed
        for s in [Pending, Running, Error] {
            assert!(!can_transition(Some(Completed), s));
        }
        assert!(can_transition(Some(Completed), Completed));
        // error → only error
        for s in [Pending, Running, Completed] {
            assert!(!can_transition(Some(Error), s));
        }
        assert!(can_transition(Some(Error), Error));
    }

    // ----- sync_tool_cache -----

    #[test]
    fn sync_builds_tool_id_list_in_first_seen_order() {
        let cfg = cfg_with_tracked();
        let messages = vec![
            assistant_call("a1", "c1", "read", json!({"path": "a"})),
            user_result("u1", "c1", ToolStatus::Completed, Some("ok"), None),
            assistant_call("a2", "c2", "read", json!({"path": "b"})),
            assistant_call("a3", "c1", "read", json!({"path": "a"})), // re-emit
        ];
        let mut state = SessionState::default();
        sync_tool_cache(&mut state, &cfg, &messages);
        assert_eq!(state.tool_id_list, vec!["c1".to_string(), "c2".to_string()]);
    }

    #[test]
    fn sync_pairs_call_with_result_and_records_status() {
        let cfg = cfg_with_tracked();
        let messages = vec![
            assistant_call("a1", "c1", "read", json!({"path": "x"})),
            user_result("u1", "c1", ToolStatus::Completed, Some("data"), None),
        ];
        let mut state = SessionState::default();
        sync_tool_cache(&mut state, &cfg, &messages);
        let entry = &state.tool_parameters["c1"];
        assert_eq!(entry.tool, "read");
        assert_eq!(entry.status, Some(ToolStatus::Completed));
        assert_eq!(entry.result_message_id.as_deref(), Some("u1"));
    }

    #[test]
    fn sync_orphan_result_is_dropped_and_counted() {
        let cfg = cfg_with_tracked();
        let messages = vec![user_result(
            "u1",
            "missing",
            ToolStatus::Completed,
            Some("x"),
            None,
        )];
        let mut state = SessionState::default();
        sync_tool_cache(&mut state, &cfg, &messages);
        assert!(state.tool_parameters.is_empty());
        assert_eq!(state.stats.orphan_tool_results, 1);
    }

    #[test]
    fn sync_invalid_transition_is_counted_and_state_preserved() {
        let cfg = cfg_with_tracked();
        let messages = vec![
            assistant_call("a1", "c1", "read", json!({"path": "x"})),
            user_result("u1", "c1", ToolStatus::Completed, Some("ok"), None),
            // completed → error: invalid.
            user_result("u2", "c1", ToolStatus::Error, None, Some("boom")),
        ];
        let mut state = SessionState::default();
        sync_tool_cache(&mut state, &cfg, &messages);
        let entry = &state.tool_parameters["c1"];
        assert_eq!(entry.status, Some(ToolStatus::Completed));
        assert_eq!(state.stats.invalid_status_transitions, 1);
    }

    #[test]
    fn sync_signature_is_canonical_for_logically_equivalent_inputs() {
        let cfg = cfg_with_tracked();
        let m1 = assistant_call("a1", "c1", "read", json!({"a": 1, "b": 2}));
        let m2 = assistant_call("a2", "c2", "read", json!({"b": 2, "a": 1}));
        let messages = vec![m1, m2];
        let mut state = SessionState::default();
        sync_tool_cache(&mut state, &cfg, &messages);
        let s1 = &state.tool_parameters["c1"].signature;
        let s2 = &state.tool_parameters["c2"].signature;
        assert_eq!(s1, s2, "signatures should be byte-equal");
    }

    #[test]
    fn sync_idempotent_under_repeat() {
        let cfg = cfg_with_tracked();
        let messages = vec![
            assistant_call("a1", "c1", "read", json!({"path": "x"})),
            user_result("u1", "c1", ToolStatus::Completed, Some("ok"), None),
            assistant_call("a2", "c2", "write", json!({"path": "y"})),
        ];
        let mut s1 = SessionState::default();
        sync_tool_cache(&mut s1, &cfg, &messages);
        let mut s2 = SessionState::default();
        sync_tool_cache(&mut s2, &cfg, &messages);
        sync_tool_cache(&mut s2, &cfg, &messages); // run twice
        assert_eq!(s1.tool_parameters, s2.tool_parameters);
        assert_eq!(s1.tool_id_list, s2.tool_id_list);
    }

    #[test]
    fn sync_paths_only_for_tracked_tools() {
        let cfg = StaticConfigLike {
            tracked_tools: vec!["read".into()],
            ..StaticConfigLike::default()
        };
        let messages = vec![
            assistant_call("a1", "c1", "read", json!({"path": "x"})),
            assistant_call("a2", "c2", "bash", json!({"path": "x"})),
        ];
        let mut state = SessionState::default();
        sync_tool_cache(&mut state, &cfg, &messages);
        assert_eq!(state.tool_parameters["c1"].paths, vec!["x"]);
        assert!(state.tool_parameters["c2"].paths.is_empty());
    }

    // ----- helpers -----

    #[test]
    fn truncate_utf8_at_boundary() {
        // 4-byte emoji at byte index 3 — must back up to 0.
        let s = "🦀abc";
        let t = truncate_utf8(s, 3);
        assert_eq!(t, ""); // codepoint boundary at 0 (next is 4)
        let t = truncate_utf8(s, 4);
        assert_eq!(t, "🦀");
        let t = truncate_utf8(s, 5);
        assert_eq!(t, "🦀a");
    }

    #[test]
    fn dedup_preserve_order_keeps_first() {
        let mut v = vec![
            "a".to_string(),
            "b".to_string(),
            "a".to_string(),
            "c".to_string(),
        ];
        dedup_preserve_order(&mut v);
        assert_eq!(v, vec!["a", "b", "c"]);
    }
}
