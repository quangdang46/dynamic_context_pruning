//! Compress entry point — `handle_compress` (SPEC.md §6.1 and §6.2).
//!
//! Orchestrates: validate → resolve → placeholder check → wrap →
//! commit_block → frontier advance → return [`CompressResult`].

use std::time::SystemTime;

use dcp_types::{BlockId, CompressionBlock, CompressionMode, Message, RunId, SessionState};

fn timestamp_now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

use crate::block::{commit_block, maybe_advance_frontier};
use crate::config::CompressConfig;
use crate::error::CompressError;
use crate::filter::filter_compressed_ranges;
use crate::placeholder::{
    append_missing_block_summaries, inject_placeholder_expansions, parse_placeholders,
    validate_placeholders,
};
use crate::resolve::{ResolvedRange, resolve_range};
use crate::timing::resolve_compression_duration;
use crate::types::{CompressArgs, CompressResult, NotificationEntry, RangeEntry};
use crate::validate::{validate_non_overlapping, validate_topic_and_content};
use crate::wrap::{
    append_protected_tool_outputs, append_protected_user_messages, compute_effective,
    compute_included, estimate_compressed_tokens, estimate_summary_tokens, wrap_compressed_summary,
};

/// Allocate the next block id from `state`, promoting `0` to `1` per
/// SPEC.md §2.4.
fn allocate_block_id(state: &mut SessionState) -> BlockId {
    let mut next = state.prune.messages.next_block_id.value();
    if next == 0 {
        next = 1;
    }
    state.prune.messages.next_block_id = BlockId::new(next.saturating_add(1));
    BlockId::new(next)
}

/// Allocate the next run id, promoting `0` to `1`.
fn allocate_run_id(state: &mut SessionState) -> RunId {
    let mut next = state.prune.messages.next_run_id.value();
    if next == 0 {
        next = 1;
    }
    state.prune.messages.next_run_id = RunId::new(next.saturating_add(1));
    RunId::new(next)
}

/// Process a [`CompressArgs`] invocation and return a
/// [`CompressResult`].
///
/// On error, no state mutations happen. Mutations include:
///
/// * `next_block_id`, `next_run_id` advance
/// * New blocks inserted via [`commit_block`]
/// * Frontier advance via [`maybe_advance_frontier`]
/// * `stats.compress_runs` increment
///
/// The optional `now_ms` argument lets callers pin the wall-clock time
/// (essential for deterministic tests).
pub fn handle_compress<C: CompressConfig + ?Sized>(
    args: CompressArgs,
    state: &mut SessionState,
    messages: &[Message],
    config: &C,
    now_ms: i64,
) -> Result<CompressResult, CompressError> {
    validate_topic_and_content(&args, config)?;

    match args {
        CompressArgs::Range { topic, content } => {
            handle_range(&topic, &content, state, messages, config)
        }
        CompressArgs::Message { topic, content } => {
            handle_message(&topic, &content, state, messages, config, now_ms)
        }
    }
}

fn handle_range<C: CompressConfig + ?Sized>(
    topic: &str,
    entries: &[RangeEntry],
    state: &mut SessionState,
    messages: &[Message],
    config: &C,
) -> Result<CompressResult, CompressError> {
    // ── Phase A: resolve all ranges and validate non-overlap ─────────
    let mut plans: Vec<(ResolvedRange, &RangeEntry)> = Vec::with_capacity(entries.len());
    for entry in entries {
        let plan = resolve_range(&entry.start_id, &entry.end_id, state, messages)?;
        plans.push((plan, entry));
    }
    let plan_only: Vec<ResolvedRange> = plans.iter().map(|(p, _)| p.clone()).collect();
    validate_non_overlapping(&plan_only)?;

    // ── Phase B: validate placeholders for every entry ───────────────
    for (plan, entry) in &plans {
        let placeholders = parse_placeholders(&entry.summary);
        validate_placeholders(&placeholders, &plan.required_block_ids)?;
    }

    // ── Phase C: assemble each block under one run id ────────────────
    let run_id = allocate_run_id(state);

    let mut new_blocks_meta: Vec<NotificationEntry> = Vec::with_capacity(plans.len());
    let mut compressed_messages_total: usize = 0;

    // We need to commit serially so each commit's effect is visible to
    // the next placeholder expansion (a later range can `{{block:b<N>}}`
    // a block from an earlier range in the same call). Compute the
    // wrapped summary, allocate a block id, build the block, commit,
    // advance the frontier.
    let started_at = timestamp_now();
    for (plan, entry) in plans {
        let now_ms = timestamp_now();
        let mentioned = parse_placeholders(&entry.summary);

        let injected = inject_placeholder_expansions(&entry.summary, state)?;
        let with_user = append_protected_user_messages(&injected, &plan, messages, config);
        let with_tools = append_protected_tool_outputs(&with_user, &plan, messages, state, config);
        let with_blocks = append_missing_block_summaries(
            &with_tools,
            &plan.required_block_ids,
            &mentioned,
            state,
        )?;

        let block_id = allocate_block_id(state);
        let wrapped = wrap_compressed_summary(block_id, topic, &with_blocks, config);

        let summary_tokens = estimate_summary_tokens(&wrapped);
        let compressed_tokens = estimate_compressed_tokens(&plan, messages);

        let (effective_messages, effective_tools) = compute_effective(
            &plan.direct_message_ids,
            &plan.direct_tool_ids,
            &plan.required_block_ids,
            &state.prune.messages.blocks_by_id,
        );
        let included =
            compute_included(&plan.required_block_ids, &state.prune.messages.blocks_by_id);

        let mut block = CompressionBlock {
            block_id,
            run_id,
            mode: CompressionMode::Range,
            topic: topic.to_string(),
            batch_topic: None,
            summary: wrapped.clone(),
            start_id: entry.start_id.clone(),
            end_id: entry.end_id.clone(),
            anchor_message_id: plan.anchor_message_id.clone(),
            compress_message_id: String::new(),
            compress_call_id: None,
            included_block_ids: included,
            consumed_block_ids: plan.required_block_ids.clone(),
            parent_block_ids: Vec::new(),
            direct_message_ids: plan.direct_message_ids.clone(),
            direct_tool_ids: plan.direct_tool_ids.clone(),
            effective_message_ids: effective_messages,
            effective_tool_ids: effective_tools,
            compressed_tokens,
            summary_tokens,
            duration_ms: resolve_compression_duration(started_at, now_ms, 0) as u64,
            active: true,
            deactivated_by_user: false,
            created_at: now_ms,
            deactivated_at: None,
            deactivated_by_block_id: None,
        };

        compressed_messages_total += plan.direct_message_ids.len();
        commit_block(state, block.clone(), now_ms);
        // After commit, refresh the block reference so frontier advance
        // sees the canonical version (active flag was already true; the
        // post-commit state owns it).
        block.active = true;
        maybe_advance_frontier(state, &block);

        new_blocks_meta.push(NotificationEntry {
            block_id: block_id.value(),
            run_id: run_id.value(),
            summary: wrapped,
            summary_tokens,
            topic: block.topic.clone(),
            compressed_tokens: block.compressed_tokens,
            direct_message_count: plan.direct_message_ids.len(),
            direct_tool_count: plan.direct_tool_ids.len(),
        });
    }

    state.stats.compress_runs = state.stats.compress_runs.saturating_add(1);

    Ok(CompressResult {
        compressed_messages: compressed_messages_total,
        blocks: new_blocks_meta,
    })
}

fn handle_message<C: CompressConfig + ?Sized>(
    topic: &str,
    entries: &[crate::types::MessageEntry],
    state: &mut SessionState,
    messages: &[Message],
    config: &C,
    now_ms: i64,
) -> Result<CompressResult, CompressError> {
    let positions: std::collections::HashMap<String, usize> = messages
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id.clone(), i))
        .collect();

    // ── Resolve per-entry plans first ────────────────────────────────
    let mut plans: Vec<(ResolvedRange, &crate::types::MessageEntry)> =
        Vec::with_capacity(entries.len());
    for entry in entries {
        let raw_id = state
            .message_ids
            .by_ref
            .get(&entry.message_id)
            .cloned()
            .ok_or_else(|| CompressError::UnknownRef(entry.message_id.clone()))?;
        let pos = *positions
            .get(&raw_id)
            .ok_or_else(|| CompressError::UnknownRef(entry.message_id.clone()))?;

        // Reject if the message is already inside an active block
        // (anchor or otherwise).
        if state
            .prune
            .messages
            .active_by_anchor_message_id
            .contains_key(&raw_id)
        {
            return Err(CompressError::MessageAlreadyCompressed(
                entry.message_id.clone(),
            ));
        }
        for block in state.prune.messages.blocks_by_id.values() {
            if block.active && block.direct_message_ids.iter().any(|id| id == &raw_id) {
                return Err(CompressError::MessageAlreadyCompressed(
                    entry.message_id.clone(),
                ));
            }
        }

        let mut direct_tool_ids: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for part in &messages[pos].parts {
            match part {
                dcp_types::Part::ToolCall { call_id, .. } if seen.insert(call_id.clone()) => {
                    direct_tool_ids.push(call_id.clone());
                }
                dcp_types::Part::ToolResult { call_id, .. } if seen.insert(call_id.clone()) => {
                    direct_tool_ids.push(call_id.clone());
                }
                _ => {}
            }
        }
        plans.push((
            ResolvedRange {
                start_raw: raw_id.clone(),
                end_raw: raw_id.clone(),
                selection_indices: vec![pos],
                required_block_ids: vec![],
                anchor_message_id: raw_id.clone(),
                direct_message_ids: vec![raw_id],
                direct_tool_ids,
            },
            entry,
        ));
    }

    let run_id = allocate_run_id(state);
    let mut new_blocks_meta: Vec<NotificationEntry> = Vec::with_capacity(plans.len());
    let started_at = timestamp_now();

    for (plan, entry) in plans {
        // Message mode: no placeholders, no missing-blocks step.
        let with_user = append_protected_user_messages(&entry.summary, &plan, messages, config);
        let with_tools = append_protected_tool_outputs(&with_user, &plan, messages, state, config);

        let block_id = allocate_block_id(state);
        let wrapped = wrap_compressed_summary(block_id, topic, &with_tools, config);
        let summary_tokens = estimate_summary_tokens(&wrapped);
        let compressed_tokens = estimate_compressed_tokens(&plan, messages);

        let mut block = CompressionBlock {
            block_id,
            run_id,
            mode: CompressionMode::Message,
            topic: topic.to_string(),
            batch_topic: Some(entry.topic.clone()),
            summary: wrapped.clone(),
            start_id: entry.message_id.clone(),
            end_id: entry.message_id.clone(),
            anchor_message_id: plan.anchor_message_id.clone(),
            compress_message_id: String::new(),
            compress_call_id: None,
            included_block_ids: Vec::new(),
            consumed_block_ids: Vec::new(),
            parent_block_ids: Vec::new(),
            direct_message_ids: plan.direct_message_ids.clone(),
            direct_tool_ids: plan.direct_tool_ids.clone(),
            effective_message_ids: plan.direct_message_ids.clone(),
            effective_tool_ids: plan.direct_tool_ids.clone(),
            compressed_tokens,
            summary_tokens,
            duration_ms: resolve_compression_duration(started_at, now_ms, 0) as u64,
            active: true,
            deactivated_by_user: false,
            created_at: now_ms,
            deactivated_at: None,
            deactivated_by_block_id: None,
        };
        commit_block(state, block.clone(), now_ms);
        block.active = true;
        maybe_advance_frontier(state, &block);

        new_blocks_meta.push(NotificationEntry {
            block_id: block_id.value(),
            run_id: run_id.value(),
            summary: wrapped,
            summary_tokens,
            topic: block.topic.clone(),
            compressed_tokens: block.compressed_tokens,
            direct_message_count: plan.direct_message_ids.len(),
            direct_tool_count: plan.direct_tool_ids.len(),
        });
    }

    state.stats.compress_runs = state.stats.compress_runs.saturating_add(1);

    Ok(CompressResult {
        compressed_messages: entries.len(),
        blocks: new_blocks_meta,
    })
}

/// Convenience wrapper: run [`handle_compress`] and then immediately
/// apply [`filter_compressed_ranges`] to produce the post-compress
/// outgoing message stream. Useful for end-to-end testing.
pub fn compress_and_apply<C: CompressConfig + ?Sized>(
    args: CompressArgs,
    state: &mut SessionState,
    messages: &[Message],
    config: &C,
    now_ms: i64,
) -> Result<(CompressResult, Vec<Message>), CompressError> {
    let result = handle_compress(args, state, messages, config, now_ms)?;
    let out = filter_compressed_ranges(messages, state);
    Ok((result, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StaticCompressConfig;
    use crate::types::{MessageEntry, RangeEntry};
    use dcp_state::{StaticConfigLike, default_tracked_tools, sync_tool_cache};
    use dcp_types::{Message, Part, Role, ToolStatus};
    use serde_json::json;

    fn build_state(messages: &[Message]) -> SessionState {
        let cfg = StaticConfigLike {
            tracked_tools: default_tracked_tools(),
            ..StaticConfigLike::default()
        };
        let mut state = SessionState::default();
        sync_tool_cache(&mut state, &cfg, messages);
        dcp_state::assign_message_refs(&mut state, messages);
        state
    }

    #[test]
    fn range_mode_basic_commit() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
            Message::user_text("u2", 0, "more"),
            Message::assistant_text("a2", 0, "ack"),
        ];
        let mut state = build_state(&messages);
        let cfg = StaticCompressConfig::defaults();
        let args = CompressArgs::Range {
            topic: "topic".into(),
            content: vec![RangeEntry {
                start_id: "m0001".into(),
                end_id: "m0003".into(),
                summary: "Summary text".into(),
            }],
        };
        let result = handle_compress(args, &mut state, &messages, &cfg, 1_000).unwrap();
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.compressed_messages, 3);
        assert_eq!(state.prune.messages.active_block_ids.len(), 1);
        assert_eq!(state.stats.compress_runs, 1);
        assert_eq!(state.stats.compress_blocks_committed, 1);
    }

    #[test]
    fn range_mode_consumes_existing_block() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
            Message::user_text("u2", 0, "more"),
        ];
        let mut state = build_state(&messages);
        let cfg = StaticCompressConfig::defaults();

        // First compress a small range.
        let r1 = handle_compress(
            CompressArgs::Range {
                topic: "t1".into(),
                content: vec![RangeEntry {
                    start_id: "m0001".into(),
                    end_id: "m0002".into(),
                    summary: "small".into(),
                }],
            },
            &mut state,
            &messages,
            &cfg,
            1_000,
        )
        .unwrap();
        let inner_id = dcp_types::BlockId::new(r1.blocks[0].block_id);

        // Then a larger range that engulfs the inner block.
        handle_compress(
            CompressArgs::Range {
                topic: "t2".into(),
                content: vec![RangeEntry {
                    start_id: "m0001".into(),
                    end_id: "m0003".into(),
                    summary: format!("see {{{{block:b{}}}}}", inner_id.value()),
                }],
            },
            &mut state,
            &messages,
            &cfg,
            2_000,
        )
        .unwrap();

        // Inner block deactivated; outer is the only active.
        assert!(!state.prune.messages.blocks_by_id[&inner_id].active);
        let active: Vec<u32> = state
            .prune
            .messages
            .active_block_ids
            .iter()
            .map(|b| b.value())
            .collect();
        assert_eq!(active.len(), 1);
        assert!(!active.contains(&inner_id.value()));
    }

    #[test]
    fn range_mode_validation_failures_no_state_change() {
        let messages = vec![Message::user_text("u1", 0, "hi")];
        let mut state = build_state(&messages);
        let snapshot = state.clone();
        let cfg = StaticCompressConfig::defaults();
        let _ = handle_compress(
            CompressArgs::Range {
                topic: "  ".into(),
                content: vec![],
            },
            &mut state,
            &messages,
            &cfg,
            0,
        );
        assert_eq!(state, snapshot, "validation must not mutate state");
    }

    #[test]
    fn message_mode_basic_commit() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let mut state = build_state(&messages);
        let cfg = StaticCompressConfig::defaults();
        let result = handle_compress(
            CompressArgs::Message {
                topic: "batch".into(),
                content: vec![MessageEntry {
                    message_id: "m0002".into(),
                    topic: "entry".into(),
                    summary: "msg summary".into(),
                }],
            },
            &mut state,
            &messages,
            &cfg,
            1_000,
        )
        .unwrap();
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(state.prune.messages.active_block_ids.len(), 1);
    }

    #[test]
    fn message_mode_rejects_already_compressed_target() {
        let messages = vec![Message::assistant_text("a1", 0, "hello")];
        let mut state = build_state(&messages);
        let cfg = StaticCompressConfig::defaults();
        // First compress.
        handle_compress(
            CompressArgs::Message {
                topic: "x".into(),
                content: vec![MessageEntry {
                    message_id: "m0001".into(),
                    topic: "t".into(),
                    summary: "summary".into(),
                }],
            },
            &mut state,
            &messages,
            &cfg,
            1_000,
        )
        .unwrap();
        // Second attempt should be rejected.
        let err = handle_compress(
            CompressArgs::Message {
                topic: "x".into(),
                content: vec![MessageEntry {
                    message_id: "m0001".into(),
                    topic: "t".into(),
                    summary: "again".into(),
                }],
            },
            &mut state,
            &messages,
            &cfg,
            2_000,
        )
        .unwrap_err();
        assert!(matches!(err, CompressError::MessageAlreadyCompressed(_)));
    }

    #[test]
    fn end_to_end_compress_and_apply_drops_inner_messages() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
            Message::user_text("u2", 0, "more"),
            Message::assistant_text("a2", 0, "ack"),
            Message::user_text("u3", 0, "after"),
        ];
        let mut state = build_state(&messages);
        let cfg = StaticCompressConfig::defaults();
        let (_, after) = compress_and_apply(
            CompressArgs::Range {
                topic: "x".into(),
                content: vec![RangeEntry {
                    start_id: "m0001".into(),
                    end_id: "m0004".into(),
                    summary: "wrapped".into(),
                }],
            },
            &mut state,
            &messages,
            &cfg,
            1_000,
        )
        .unwrap();
        let ids: Vec<&str> = after.iter().map(|m| m.id.as_str()).collect();
        // u1 (anchor) and u3 (outside range) only.
        assert_eq!(ids, vec!["u1", "u3"]);
    }

    #[test]
    fn frontier_advances_on_oversized_summary() {
        // A 4-message range whose verbatim text is small; we use a
        // deliberately oversized summary to force the frontier to move.
        let messages = vec![
            Message::user_text("u1", 0, "x"),
            Message::assistant_text("a1", 0, "y"),
        ];
        let mut state = build_state(&messages);
        let cfg = StaticCompressConfig::defaults();
        let huge_summary = "z".repeat(4 * 1024); // ~1024 tokens via /4
        handle_compress(
            CompressArgs::Range {
                topic: "x".into(),
                content: vec![RangeEntry {
                    start_id: "m0001".into(),
                    end_id: "m0002".into(),
                    summary: huge_summary,
                }],
            },
            &mut state,
            &messages,
            &cfg,
            1_000,
        )
        .unwrap();
        assert!(state.prune.messages.frontier_message_ref.is_some());
        assert_eq!(state.stats.compress_oversized, 1);
    }

    #[test]
    fn placeholder_referencing_unknown_block_rejected() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let mut state = build_state(&messages);
        let cfg = StaticCompressConfig::defaults();
        let err = handle_compress(
            CompressArgs::Range {
                topic: "x".into(),
                content: vec![RangeEntry {
                    start_id: "m0001".into(),
                    end_id: "m0002".into(),
                    summary: "see {{block:b9}} body".into(),
                }],
            },
            &mut state,
            &messages,
            &cfg,
            0,
        )
        .unwrap_err();
        assert!(matches!(err, CompressError::PlaceholderMismatch(_)));
    }

    #[test]
    fn protected_tool_output_is_appended_to_summary() {
        let messages = vec![
            Message::new(
                "a1",
                Role::Assistant,
                vec![Part::tool_call("c1", "task", json!({"id": "x"}))],
                0,
            ),
            Message::new(
                "u1",
                Role::User,
                vec![Part::tool_result(
                    "c1",
                    ToolStatus::Completed,
                    Some("important task output".into()),
                    None,
                )],
                0,
            ),
            Message::assistant_text("a2", 0, "done"),
        ];
        let mut state = build_state(&messages);
        let cfg = StaticCompressConfig::defaults();
        let result = handle_compress(
            CompressArgs::Range {
                topic: "x".into(),
                content: vec![RangeEntry {
                    start_id: "m0001".into(),
                    end_id: "m0003".into(),
                    summary: "compressed".into(),
                }],
            },
            &mut state,
            &messages,
            &cfg,
            1_000,
        )
        .unwrap();
        let block_summary = &result.blocks[0].summary;
        assert!(block_summary.contains("important task output"));
        assert!(block_summary.contains("<dcp-protected-tools>"));
    }
}
