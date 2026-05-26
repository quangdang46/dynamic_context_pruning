//! Helpers for the [`crate::ContextPruner::transform_messages`] pipeline.
//!
//! Per SPEC.md §5.4 / PLAN.md §6.4, `transform_messages` is a 10-phase
//! pipeline. The phases are split into stand-alone functions here so the
//! main facade implementation reads top-down without long inline
//! sequences.

use std::collections::{HashMap, HashSet};

use dcp_compress::CompressConfig as CompressCfg;
use dcp_compress::filter_compressed_ranges;
use dcp_config::Config;
use dcp_nudges::{
    NudgeConfig, NudgeKind, build_priority_map, inject_extended_subagent_results, inject_message_ids,
    inject_nudges,
};
use dcp_prompts::Prompts;
use dcp_prune::apply::{PruneKind, apply_prune_to_messages};
use dcp_prune::{deduplicate, purge_errors, stale_file_reads};
use dcp_state::{
    assign_message_refs, count_turns, find_last_compaction_timestamp, sync_tool_cache,
};
use dcp_telemetry::{EventKind, Telemetry};
use dcp_traits::{PruneOutcome, Tokenizer};
use dcp_types::{Message, Part, PendingPrune, Role, SessionState, ToolStatus};

// ─────────────────────────────────────────────────────────────────────────
// Validation (SPEC.md §2.5)
// ─────────────────────────────────────────────────────────────────────────

/// Filter the input message list to only those that satisfy SPEC.md §2.5.
///
/// Validation rules:
///
/// * Non-empty parts.
/// * Role consistency — assistants do not emit `tool_result`; users do
///   not emit `tool_call` / `reasoning`.
/// * Id uniqueness — duplicate ids are dropped (first wins).
///
/// Tool-result orphan filtering is deferred to [`sync_tool_cache`], which
/// counts orphans on `state.stats.orphan_tool_results`. We keep the
/// envelope here so the apply phase can still see the result; the result
/// is later removed if its sibling tool_call gets pruned.
pub(crate) fn filter_valid_messages(
    messages: Vec<Message>,
    state: &mut SessionState,
) -> Vec<Message> {
    let mut seen_ids: HashSet<String> = HashSet::with_capacity(messages.len());
    let mut dropped: u32 = 0;
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());

    for msg in messages {
        if msg.parts.is_empty() {
            dropped = dropped.saturating_add(1);
            continue;
        }
        if !role_consistent(&msg) {
            dropped = dropped.saturating_add(1);
            continue;
        }
        if !seen_ids.insert(msg.id.clone()) {
            dropped = dropped.saturating_add(1);
            continue;
        }
        out.push(msg);
    }

    state.stats.dropped_invalid = state.stats.dropped_invalid.saturating_add(dropped);
    out
}

fn role_consistent(msg: &Message) -> bool {
    match msg.role {
        Role::Assistant => msg
            .parts
            .iter()
            .all(|p| !matches!(p, Part::ToolResult { .. })),
        Role::User => msg
            .parts
            .iter()
            .all(|p| !matches!(p, Part::ToolCall { .. } | Part::Reasoning(_))),
        // System messages are accepted as-is. The library does not emit
        // them but tolerates host-supplied compaction markers.
        Role::System => true,
        _ => true,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Compaction detection (SPEC.md §3.3)
// ─────────────────────────────────────────────────────────────────────────

/// Detect the SPEC.md §3.3 compaction signature (the host has elided the
/// most recent live message ids). On detection, mutate `state` per the
/// spec.
pub(crate) fn detect_compaction(state: &mut SessionState, messages: &[Message], now_ms: i64) {
    if state.message_ids.by_raw_id.is_empty() {
        return;
    }
    let live: Vec<&str> = state
        .message_ids
        .by_raw_id
        .keys()
        .map(String::as_str)
        .collect();
    if live.is_empty() {
        return;
    }
    let seen: HashSet<&str> = messages.iter().map(|m| m.id.as_str()).collect();

    // Heuristic: walk the input messages backwards via tool_id_list as a
    // proxy for "recent". When that's not informative, look at the last
    // three live ids the library knows about (per the spec).
    //
    // For the implementation, "recent" = the last three keys in
    // `message_ids.by_ref` ordered by reference number.
    let mut sorted_refs: Vec<(u32, &str)> = state
        .message_ids
        .by_ref
        .iter()
        .filter_map(|(r, raw)| r.strip_prefix('m').and_then(|n| n.parse::<u32>().ok()).map(|n| (n, raw.as_str())))
        .collect();
    sorted_refs.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    let recent: Vec<&str> = sorted_refs.iter().take(3).map(|(_, r)| *r).collect();
    if recent.is_empty() {
        return;
    }
    if recent.iter().any(|id| seen.contains(id)) {
        return;
    }

    dcp_state::reset_on_compaction_at(state, now_ms);
}

// ─────────────────────────────────────────────────────────────────────────
// Sync phase (SPEC.md §3 + §4)
// ─────────────────────────────────────────────────────────────────────────

/// Run the synchronisation half of `transform_messages` — populate the
/// state from the validated message stream. Mirrors PLAN.md §6.4 phase
/// 1, executed before any strategy runs.
pub(crate) fn sync_state(state: &mut SessionState, config: &Config, messages: &[Message]) {
    sync_tool_cache(state, config, messages);
    assign_message_refs(state, messages);
    state.current_turn = count_turns(state, messages);
    state.last_message_was_assistant_text = is_last_assistant_text(messages);
    state.last_compaction = state
        .last_compaction
        .max(find_last_compaction_timestamp(messages));

    // SPEC §3.1: when the host has not assigned a session id, derive one
    // deterministically from the last message's raw id so persistence can
    // round-trip without explicit setup.
    if state.session_id.is_none()
        && let Some(last) = messages.last()
    {
        state.session_id = Some(last.id.clone());
    }
}

fn is_last_assistant_text(messages: &[Message]) -> bool {
    let Some(last) = messages.last() else {
        return false;
    };
    if last.role != Role::Assistant {
        return false;
    }
    let has_text = last.parts.iter().any(|p| matches!(p, Part::Text(_)));
    let has_open_tool = last
        .parts
        .iter()
        .any(|p| matches!(p, Part::ToolCall { .. }));
    has_text && !has_open_tool
}

// ─────────────────────────────────────────────────────────────────────────
// Cache stability gate (SPEC.md §7.3)
// ─────────────────────────────────────────────────────────────────────────

/// Decide whether the apply phase should run on this transform.
///
/// SPEC.md §7.3 — `force_apply_requested` always wins, otherwise the
/// configured [`dcp_prune::CacheStabilityMode`] decides.
pub(crate) fn should_apply_now(state: &SessionState, config: &Config) -> bool {
    use dcp_prune::CacheStabilityMode::*;
    if state.force_apply_requested {
        return true;
    }
    match config.cache_stability_mode {
        Aggressive => true,
        AgentMessage => state.last_message_was_assistant_text,
        Manual => false,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Strategy invocation
// ─────────────────────────────────────────────────────────────────────────

/// Run every built-in strategy and return the new pruned `(call_id,
/// kind)` decisions accumulated during this transform.
pub(crate) fn run_strategies(
    state: &mut SessionState,
    config: &Config,
    telemetry: &mut Telemetry,
) -> Vec<PruneOutcome> {
    let outcomes = vec![
        run_named(state, config, telemetry, "deduplicate", deduplicate::run),
        run_named(state, config, telemetry, "purge_errors", purge_errors::run),
        run_named(
            state,
            config,
            telemetry,
            "stale_file_reads",
            stale_file_reads::run,
        ),
    ];
    outcomes
}

fn run_named<F>(
    state: &mut SessionState,
    config: &Config,
    telemetry: &mut Telemetry,
    name: &str,
    f: F,
) -> PruneOutcome
where
    F: FnOnce(&mut SessionState, &Config) -> PruneOutcome,
{
    let outcome = f(state, config);
    if outcome.reason_skipped.is_none() {
        telemetry.record(EventKind::Prune {
            strategy: name.to_string(),
        });
    }
    outcome
}

// ─────────────────────────────────────────────────────────────────────────
// Apply phase
// ─────────────────────────────────────────────────────────────────────────

/// Map the pruned `call_id` set in `state.prune.tools` into a per-call
/// [`PruneKind`] and call the applier.
///
/// The kind is derived from the recorded tool status — errored calls
/// keep their envelopes (PurgeError), everything else is dropped (Drop).
/// This matches SPEC.md §5.4's pruning rule.
pub(crate) fn apply_prune(state: &SessionState, messages: &[Message]) -> Vec<Message> {
    if state.prune.tools.is_empty() {
        return messages.to_vec();
    }
    let mut decisions: HashMap<String, PruneKind> = HashMap::with_capacity(state.prune.tools.len());
    for call_id in state.prune.tools.keys() {
        let kind = match state.tool_parameters.get(call_id) {
            Some(entry) if entry.status == Some(ToolStatus::Error) => PruneKind::PurgeError,
            _ => PruneKind::Drop,
        };
        decisions.insert(call_id.clone(), kind);
    }
    apply_prune_to_messages(messages, &decisions)
}

// ─────────────────────────────────────────────────────────────────────────
// Pending prune snapshot
// ─────────────────────────────────────────────────────────────────────────

/// Capture a snapshot of the currently-pending prune decisions when the
/// apply phase is gated off.
pub(crate) fn accumulate_pending(state: &mut SessionState, before_size: usize) {
    if state.prune.tools.len() == before_size {
        return;
    }
    let mut tool_ids: Vec<String> = state.prune.tools.keys().cloned().collect();
    tool_ids.sort();
    let cumulative_tokens: u64 = state.prune.tools.values().sum();
    state.pending_prune = Some(PendingPrune {
        tool_ids,
        cumulative_tokens,
        accumulated_at_turn: state.current_turn,
    });
}

// ─────────────────────────────────────────────────────────────────────────
// Compression block expansion
// ─────────────────────────────────────────────────────────────────────────

/// Apply [`filter_compressed_ranges`] to `messages`.
pub(crate) fn expand_compressed(messages: Vec<Message>, state: &SessionState) -> Vec<Message> {
    filter_compressed_ranges(&messages, state)
}

// ─────────────────────────────────────────────────────────────────────────
// Subagent result inlining
// ─────────────────────────────────────────────────────────────────────────

/// Wrap [`inject_extended_subagent_results`] with the
/// `experimental.allowSubagents` gate so the facade does not have to
/// know about the experimental flag plumbing.
pub(crate) fn inject_subagent_results(
    state: &SessionState,
    config: &Config,
    messages: Vec<Message>,
) -> Vec<Message> {
    inject_extended_subagent_results(state, messages, config.experimental.allow_subagents)
}

// ─────────────────────────────────────────────────────────────────────────
// Nudge + message-id injection
// ─────────────────────────────────────────────────────────────────────────

/// Build the nudge config from the host's [`Config`] (the runtime view
/// `dcp-nudges` needs).
pub(crate) fn nudge_config_from(config: &Config) -> NudgeConfig {
    NudgeConfig {
        injection_mode: config_injection_mode(config),
        nudge_frequency: config.compress.nudge_frequency,
        iteration_nudge_threshold: config.compress.iteration_nudge_threshold,
        nudge_force: config.compress.nudge_force,
        max_context_limit: config
            .compress
            .max_context_limit
            .resolve(config_state_model_limit(None)),
        min_context_limit: config
            .compress
            .min_context_limit
            .resolve(config_state_model_limit(None)),
    }
}

fn config_injection_mode(_config: &Config) -> dcp_nudges::InjectionMode {
    // SPEC.md §10.2 documents `injectionMode` as a top-level field, but
    // the existing `dcp_config::Config` keeps the default `Append`
    // behaviour by not surfacing the knob. Until the schema gains the
    // field, the facade always uses the default.
    dcp_nudges::InjectionMode::default()
}

fn config_state_model_limit(model: Option<u64>) -> Option<u64> {
    model
}

/// Compute the priority map and inject both nudges and `<dcp-message-id>`
/// tags into `messages`. Mutates `state.nudges` counters.
pub(crate) fn inject_nudges_and_ids(
    state: &mut SessionState,
    config: &Config,
    prompts: &Prompts,
    tokenizer: &dyn Tokenizer,
    messages: &mut [Message],
    telemetry: &mut Telemetry,
) {
    let nudge_cfg = nudge_config_from(config);
    let total_tokens = total_token_count(tokenizer, messages);
    let priorities = build_priority_map(&nudge_cfg, state, messages, total_tokens);
    record_nudge_telemetry(&priorities, telemetry);

    inject_nudges(state, &nudge_cfg, messages, prompts, &priorities);
    inject_message_ids(state, &nudge_cfg, messages, &priorities);
}

fn record_nudge_telemetry(priorities: &HashMap<String, NudgeKind>, telemetry: &mut Telemetry) {
    for kind in priorities.values() {
        let label = match kind {
            NudgeKind::ContextLimit { .. } => "context_limit",
            NudgeKind::Turn => "turn",
            NudgeKind::Iteration { .. } => "iteration",
            _ => "other",
        };
        telemetry.record(EventKind::Nudge {
            kind: label.to_string(),
        });
    }
}

fn total_token_count(tokenizer: &dyn Tokenizer, messages: &[Message]) -> u64 {
    let mut total: u64 = 0;
    for msg in messages {
        for part in &msg.parts {
            match part {
                Part::Text(t) | Part::Reasoning(t) => {
                    total = total.saturating_add(tokenizer.count(t) as u64);
                }
                Part::ToolCall { tool, input, .. } => {
                    total = total.saturating_add(tokenizer.count(tool) as u64);
                    total = total.saturating_add(
                        tokenizer.count(&serde_json::to_string(input).unwrap_or_default()) as u64,
                    );
                }
                Part::ToolResult { output, error, .. } => {
                    if let Some(o) = output {
                        total = total.saturating_add(tokenizer.count(o) as u64);
                    }
                    if let Some(e) = error {
                        total = total.saturating_add(tokenizer.count(e) as u64);
                    }
                }
                Part::Image { .. } => {
                    total = total.saturating_add(85);
                }
                _ => {}
            }
        }
    }
    total
}

// ─────────────────────────────────────────────────────────────────────────
// Strip stale metadata
// ─────────────────────────────────────────────────────────────────────────

/// Strip transient metadata before returning messages to the host.
///
/// At the moment the only "metadata" the library ever leaves on a
/// message is the `<dcp-*>` tags it injected itself, and those are part
/// of the contract — keep them. This function exists for symmetry with
/// the SPEC.md §5.4 pseudocode and so the facade has a hook to tighten
/// later without touching every caller.
pub(crate) fn strip_internal_metadata(_messages: &mut [Message]) {
    // Intentionally empty for now. See doc comment above.
}

// ─────────────────────────────────────────────────────────────────────────
// Persistence helper
// ─────────────────────────────────────────────────────────────────────────

/// Build a [`dcp_traits::PersistedState`] envelope from the live
/// [`SessionState`].
pub(crate) fn build_persisted(state: &SessionState) -> dcp_traits::PersistedState {
    use dcp_traits::{PersistedState, PersistedStateV1};

    let v1 = PersistedStateV1 {
        session_name: None,
        session_id: state.session_id.clone().unwrap_or_default(),
        last_updated: now_iso8601(),
        current_turn: state.current_turn,
        frontier_message_ref: state.prune.messages.frontier_message_ref.clone(),
        next_block_id: state.prune.messages.next_block_id.value(),
        next_run_id: state.prune.messages.next_run_id.value(),
        next_message_ref: state.message_ids.next_ref,
        stats: serde_json::to_value(&state.stats).unwrap_or(serde_json::Value::Null),
        nudges: serde_json::to_value(&state.nudges).unwrap_or(serde_json::Value::Null),
        prune: serde_json::json!({
            "tools": state.prune.tools,
        }),
        tool_index: serde_json::to_value(&state.tool_parameters)
            .unwrap_or(serde_json::Value::Null),
        message_id_map: serde_json::json!({
            "by_raw_id": state.message_ids.by_raw_id,
            "by_ref": state.message_ids.by_ref,
        }),
        compaction: serde_json::json!({
            "last_compaction_at": state.last_compaction,
            "compactions_observed": state.stats.compactions_observed,
        }),
    };
    PersistedState::V1(v1)
}

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ─────────────────────────────────────────────────────────────────────────
// Tokenizer pass-through
// ─────────────────────────────────────────────────────────────────────────

/// Add the configured compress system-prompt addendum (`<dcp-protected-tools>`
/// listing + manual-mode + sub-agent notes) to `system`.
pub(crate) fn render_system_addendum(prompts: &Prompts, config: &Config) -> String {
    let extension =
        dcp_prompts::build_protected_tools_extension(&config.compress.protected_tools);
    let manual_mode = config.manual_mode.enabled;
    let allow_subagents = config.experimental.allow_subagents;
    dcp_prompts::render_system_prompt(prompts, &extension, manual_mode, allow_subagents)
}

// ─────────────────────────────────────────────────────────────────────────
// Compress tool schema rendering
// ─────────────────────────────────────────────────────────────────────────

/// Tool schema returned to the host so it can register the `compress`
/// tool with the LLM. The shape matches SPEC.md §6.1 / §6.2.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolSchema {
    /// Tool name (always `"compress"`).
    pub name: String,
    /// Description rendered from the active mode's prompt template.
    pub description: String,
    /// JSON-schema fragment for the tool's `parameters` field.
    pub parameters: serde_json::Value,
}

/// Build the [`ToolSchema`] that `compress_tool_schema()` returns, taking
/// the active mode from `config.compress.mode`.
pub(crate) fn compress_tool_schema(prompts: &Prompts, config: &Config) -> ToolSchema {
    use dcp_config::CompressMode;

    let (description_template, parameters) = match config.compress.mode {
        CompressMode::Range => (
            prompts.compress_range.clone(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "topic": { "type": "string" },
                    "content": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "startId": { "type": "string" },
                                "endId":   { "type": "string" },
                                "summary": { "type": "string" }
                            },
                            "required": ["startId", "endId", "summary"]
                        }
                    }
                },
                "required": ["topic", "content"]
            }),
        ),
        CompressMode::Message => (
            prompts.compress_message.clone(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "topic": { "type": "string" },
                    "content": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "messageId": { "type": "string" },
                                "topic":     { "type": "string" },
                                "summary":   { "type": "string" }
                            },
                            "required": ["messageId", "topic", "summary"]
                        }
                    }
                },
                "required": ["topic", "content"]
            }),
        ),
    };
    ToolSchema {
        name: "compress".into(),
        description: description_template,
        parameters,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tokenizer-aware system prompt cache
// ─────────────────────────────────────────────────────────────────────────

/// Cache the token count of the rendered system prompt addendum on the
/// state, so the nudge / context-limit logic can read it without
/// recomputing each turn.
pub(crate) fn cache_system_prompt_tokens(
    state: &mut SessionState,
    tokenizer: &dyn Tokenizer,
    rendered: &str,
) {
    state.system_prompt_tokens = Some(tokenizer.count(rendered) as u64);
}

// Suppress the dead-code warning for `CompressCfg` — the import is
// retained so callers see the trait it parameterises in scope when
// reading this module top-down.
#[allow(dead_code)]
const _: Option<&dyn CompressCfg> = None;
