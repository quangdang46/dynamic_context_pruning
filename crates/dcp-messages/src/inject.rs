//! Main injection orchestrator — port of lib/messages/inject/inject.ts.
//!
//! Provides: inject_compress_nudges, inject_message_ids.

#[allow(unused_imports)]
use dcp_types::{Message, Part, Role, SessionState};

use crate::inject_utils::apply_anchored_nudges;
use crate::utils::DCP_TAG_NAME;

// ============================================================================
// InjectParams
// ============================================================================

/// Parameters controlling nudge and message ID injection.
#[derive(Clone, Debug, Default)]
pub struct InjectParams {
    /// Whether to inject priority guidance nudges.
    pub inject_priority_guidance: bool,
    /// References to include in priority nudges.
    pub priority_refs: Vec<String>,
    /// Index into the messages slice at which to anchor priority guidance.
    pub priority_anchor_index: usize,
    /// Whether to emit informational/debug nudges.
    pub debug: bool,
}

// ============================================================================
// inject_compress_nudges
// ============================================================================

/// Determine which nudge tier applies given token usage.
///
/// Returns the nudge tier:
/// - Tier 1 (Strong): usage_ratio >= 1.0 → context limit exceeded
/// - Tier 2 (Soft): usage_ratio >= 0.8 → approaching limit
/// - Tier 3 (Info): debug && usage_ratio >= 0.5 → informational
/// - None: below all thresholds
fn determine_nudge_tier(usage_ratio: f64, debug: bool) -> Option<&'static str> {
    if usage_ratio >= 1.0 {
        Some("strong")
    } else if usage_ratio >= 0.8 {
        Some("soft")
    } else if debug && usage_ratio >= 0.5 {
        Some("info")
    } else {
        None
    }
}

/// Build the nudge text for a given tier.
fn build_nudge_text(tier: &str, usage_ratio: f64, mrefs: &[String]) -> String {
    match tier {
        "strong" => {
            let base = "⚠️ Context limit exceeded. Consider compressing.";
            if mrefs.is_empty() {
                base.to_string()
            } else {
                format!("{}\n\nAffected messages: {}", base, mrefs.join(", "))
            }
        }
        "soft" => {
            let pct = (usage_ratio * 100.0).round() as i32;
            let base = format!("ℹ️ Approaching context limit ({}% used)", pct);
            if mrefs.is_empty() {
                base
            } else {
                format!("{}\n\nAffected messages: {}", base, mrefs.join(", "))
            }
        }
        "info" => {
            // Tier 3 is informational only, doesn't include mrefs
            format!("📊 Context usage: {:.0}%", usage_ratio * 100.0)
        }
        _ => String::new(),
    }
}

/// Inject compression nudges into the message stream.
///
/// This implements a 3-tier nudge system:
/// - **Tier 1 (Strong)**: Context limit exceeded (ratio >= 1.0)
/// - **Tier 2 (Soft)**: Approaching limit (ratio >= 0.8)
/// - **Tier 3 (Informational)**: Debug mode only (ratio >= 0.5)
///
/// Returns the nudge text that was applied, or empty string if no nudge was applied.
pub fn inject_compress_nudges(
    messages: &mut [Message],
    state: &SessionState,
    working_memory_tokens: u64,
    context_limit: u64,
    debug: bool,
    priority_refs: &[String],
) -> String {
    // Early return for empty messages slice
    if messages.is_empty() {
        return String::new();
    }

    // Determine tier based on usage ratio
    let usage_ratio = if context_limit == 0 {
        0.0
    } else {
        working_memory_tokens as f64 / context_limit as f64
    };

    let tier = match determine_nudge_tier(usage_ratio, debug) {
        Some(t) => t,
        None => return String::new(),
    };

    // Build appropriate nudge text based on tier
    let nudge_text = build_nudge_text(tier, usage_ratio, priority_refs);

    // Apply via apply_anchored_nudges
    apply_anchored_nudges(messages, state, &nudge_text, priority_refs);

    nudge_text
}

// ============================================================================
// inject_message_ids
// ============================================================================

/// Inject message ID XML tags into messages that don't already have them.
///
/// The XML tag format is `<dcp ref="m####">` where `####` is the zero-padded
/// four-digit message reference.
///
/// Returns the count of messages that received ID tags.
pub fn inject_message_ids(
    messages: &mut [Message],
    state: &SessionState,
    params: &InjectParams,
) -> usize {
    let mut count = 0;

    let _params = params; // Currently unused but part of public API

    for msg in messages.iter_mut() {
        // Skip ignored messages
        if msg.ignored {
            continue;
        }

        // Look up this message's reference in the state
        let msg_ref = state.message_ids.by_raw_id.get(&msg.id);

        let Some(ref_id) = msg_ref else {
            continue;
        };

        // Build the tag for this message
        let tag = format!("<{} ref=\"{}\">", DCP_TAG_NAME, ref_id);

        // Check if this exact tag is already prepended to the first Text part
        let needs_tag = match msg.parts.first() {
            Some(Part::Text(t)) => !t.starts_with(&tag),
            _ => true,
        };

        if needs_tag {
            prepend_unique_tag(msg, &tag);
            count += 1;
        }
    }

    count
}

/// Prepend a unique tag to the message's first Text part.
///
/// If the message has no Text parts, inserts a new Text part at position 0.
/// If the tag is already present in the first Text part, does nothing (idempotent).
fn prepend_unique_tag(msg: &mut Message, tag: &str) {
    // Check first text part for tag presence
    if let Some(Part::Text(text)) = msg.parts.first_mut() {
        if text.contains(tag) {
            return; // Already tagged, idempotent
        }
        if text.is_empty() {
            *text = tag.to_string();
        } else {
            let original = std::mem::take(text);
            *text = format!("{}\n{}", tag, original);
        }
    } else {
        // No Text part exists, insert new one at position 0
        msg.parts.insert(0, Part::Text(tag.to_string()));
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

    fn make_state(by_raw_id: HashMap<String, String>) -> SessionState {
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
            subagent_result_cache: HashMap::new(),
            tool_id_list: Vec::new(),
            message_ids: MessageIdState {
                by_raw_id,
                by_ref: HashMap::new(),
                next_ref: 0,
            },
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
    // InjectParams tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn inject_params_default() {
        let params = InjectParams::default();
        assert!(!params.inject_priority_guidance);
        assert!(params.priority_refs.is_empty());
        assert_eq!(params.priority_anchor_index, 0);
        assert!(!params.debug);
    }

    // ─────────────────────────────────────────────────────────────────
    // inject_compress_nudges tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn inject_compress_nudges_tier1_exceeded_no_refs() {
        let mut messages = vec![
            msg("msg1", Role::User, vec![Part::Text("hello".into())]),
            msg("msg2", Role::Assistant, vec![Part::Text("world".into())]),
        ];
        let state = make_state(HashMap::new());

        // Tier 1: ratio >= 1.0 (context exceeded)
        let result = inject_compress_nudges(&mut messages, &state, 1000, 800, false, &[]);

        assert!(result.contains("Context limit exceeded"));
        assert!(result.contains("⚠️"));
        // No refs so no "Affected messages" line
        assert!(!result.contains("Affected messages"));
    }

    #[test]
    fn inject_compress_nudges_tier1_exceeded_with_refs() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let mut by_raw = HashMap::new();
        by_raw.insert("anchor1".into(), "msg1".into());
        let state = make_state(by_raw);

        let result =
            inject_compress_nudges(&mut messages, &state, 1000, 800, false, &["anchor1".into()]);

        assert!(result.contains("Context limit exceeded"));
        assert!(result.contains("Affected messages: anchor1"));
    }

    #[test]
    fn inject_compress_nudges_tier2_approaching() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let state = make_state(HashMap::new());

        // Tier 2: ratio >= 0.8 but < 1.0 (approaching)
        let result = inject_compress_nudges(&mut messages, &state, 850, 1000, false, &[]);

        assert!(result.contains("Approaching context limit"));
        assert!(result.contains("85% used"));
    }

    #[test]
    fn inject_compress_nudges_tier2_shows_percentage() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let state = make_state(HashMap::new());

        // Exactly 80%
        let result = inject_compress_nudges(&mut messages, &state, 800, 1000, false, &[]);

        assert!(result.contains("80% used"));
    }

    #[test]
    fn inject_compress_nudges_tier3_debug_only() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let state = make_state(HashMap::new());

        // Tier 3: debug=true and ratio >= 0.5 but < 0.8
        let result = inject_compress_nudges(&mut messages, &state, 600, 1000, true, &[]);

        assert!(result.contains("Context usage: 60%"));
        assert!(result.contains("📊"));
    }

    #[test]
    fn inject_compress_nudges_tier3_not_in_production() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let state = make_state(HashMap::new());

        // Debug = false, so even though ratio >= 0.5, no nudge
        let result = inject_compress_nudges(&mut messages, &state, 600, 1000, false, &[]);

        assert!(result.is_empty());
    }

    #[test]
    fn inject_compress_nudges_no_nudge_below_threshold() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let state = make_state(HashMap::new());

        // ratio = 0.4 (< 0.5), no nudge regardless of debug
        let result = inject_compress_nudges(&mut messages, &state, 400, 1000, true, &[]);

        assert!(result.is_empty());
    }

    #[test]
    fn inject_compress_nudges_zero_context_limit() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let state = make_state(HashMap::new());

        // Zero context limit should not panic, treats as 0 ratio (no nudge)
        let result = inject_compress_nudges(&mut messages, &state, 100, 0, false, &[]);

        assert!(result.is_empty());
    }

    #[test]
    fn inject_compress_nudges_applies_via_anchored_nudges() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let mut by_raw = HashMap::new();
        by_raw.insert("anchor1".into(), "msg1".into());
        let state = make_state(by_raw);

        // Tier 1 with anchor ref
        inject_compress_nudges(&mut messages, &state, 1000, 800, false, &["anchor1".into()]);

        // The nudge should be appended to the message via apply_anchored_nudges
        assert!(messages[0].parts.iter().any(|p| match p {
            Part::Text(t) => t.contains("Context limit exceeded"),
            _ => false,
        }));
    }

    #[test]
    fn inject_compress_nudges_returns_nudge_text() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let state = make_state(HashMap::new());

        let result = inject_compress_nudges(&mut messages, &state, 1000, 800, false, &[]);

        assert!(!result.is_empty());
        assert!(result.contains("⚠️ Context limit exceeded"));
    }

    #[test]
    fn inject_compress_nudges_empty_messages_slice() {
        let mut messages: Vec<Message> = vec![];
        let state = make_state(HashMap::new());

        // Should not panic
        let result = inject_compress_nudges(&mut messages, &state, 1000, 800, false, &[]);

        assert!(result.is_empty());
    }

    // ─────────────────────────────────────────────────────────────────
    // inject_message_ids tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn inject_message_ids_empty_messages() {
        let mut messages: Vec<Message> = vec![];
        let state = make_state(HashMap::new());
        let params = InjectParams::default();

        let count = inject_message_ids(&mut messages, &state, &params);

        assert_eq!(count, 0);
    }

    #[test]
    fn inject_message_ids_no_state_entries() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let state = make_state(HashMap::new()); // no by_raw_id entries
        let params = InjectParams::default();

        let count = inject_message_ids(&mut messages, &state, &params);

        assert_eq!(count, 0);
        // Message should not be modified
        assert!(matches!(&messages[0].parts[0], Part::Text(t) if t == "hello"));
    }

    #[test]
    fn inject_message_ids_single_message() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        let mut by_raw = HashMap::new();
        by_raw.insert("msg1".into(), "m0001".into());
        let state = make_state(by_raw);
        let params = InjectParams::default();

        let count = inject_message_ids(&mut messages, &state, &params);

        assert_eq!(count, 1);
        // Tag should be prepended
        assert!(
            matches!(&messages[0].parts[0], Part::Text(t) if t.starts_with("<dcp ref=\"m0001\">"))
        );
    }

    #[test]
    fn inject_message_ids_idempotent() {
        let mut messages = vec![msg(
            "msg1",
            Role::User,
            vec![Part::Text("<dcp ref=\"m0001\">\nhello".into())],
        )];
        let mut by_raw = HashMap::new();
        by_raw.insert("msg1".into(), "m0001".into());
        let state = make_state(by_raw);
        let params = InjectParams::default();

        // Call twice - message already starts with tag, so both return 0
        let count1 = inject_message_ids(&mut messages, &state, &params);
        let count2 = inject_message_ids(&mut messages, &state, &params);

        assert_eq!(count1, 0); // Already tagged, no injection needed
        assert_eq!(count2, 0);
        // Content should be unchanged
        assert!(messages[0].parts[0].is_text());
        if let Part::Text(t) = &messages[0].parts[0] {
            assert!(t.starts_with("<dcp ref=\"m0001\">"));
            assert!(t.contains("hello"));
        }
    }

    #[test]
    fn inject_message_ids_skips_ignored() {
        let mut messages = vec![msg("msg1", Role::User, vec![Part::Text("hello".into())])];
        messages[0].ignored = true;
        let mut by_raw = HashMap::new();
        by_raw.insert("msg1".into(), "m0001".into());
        let state = make_state(by_raw);
        let params = InjectParams::default();

        let count = inject_message_ids(&mut messages, &state, &params);

        assert_eq!(count, 0);
    }

    #[test]
    fn inject_message_ids_skips_ignored_with_ref() {
        let mut messages = vec![
            msg("msg1", Role::User, vec![Part::Text("hello".into())]),
            msg("msg2", Role::Assistant, vec![Part::Text("world".into())]),
        ];
        messages[0].ignored = true;
        let mut by_raw = HashMap::new();
        by_raw.insert("msg1".into(), "m0001".into());
        by_raw.insert("msg2".into(), "m0002".into());
        let state = make_state(by_raw);
        let params = InjectParams::default();

        let count = inject_message_ids(&mut messages, &state, &params);

        assert_eq!(count, 1);
        // Only msg2 should get the tag
        assert!(
            matches!(&messages[1].parts[0], Part::Text(t) if t.starts_with("<dcp ref=\"m0002\">"))
        );
    }

    #[test]
    fn inject_message_ids_multiple_messages() {
        let mut messages = vec![
            msg("msg1", Role::User, vec![Part::Text("hello".into())]),
            msg("msg2", Role::Assistant, vec![Part::Text("world".into())]),
        ];
        let mut by_raw = HashMap::new();
        by_raw.insert("msg1".into(), "m0001".into());
        by_raw.insert("msg2".into(), "m0002".into());
        let state = make_state(by_raw);
        let params = InjectParams::default();

        let count = inject_message_ids(&mut messages, &state, &params);

        assert_eq!(count, 2);
        assert!(
            matches!(&messages[0].parts[0], Part::Text(t) if t.starts_with("<dcp ref=\"m0001\">"))
        );
        assert!(
            matches!(&messages[1].parts[0], Part::Text(t) if t.starts_with("<dcp ref=\"m0002\">"))
        );
    }

    #[test]
    fn inject_message_ids_no_text_parts() {
        let mut messages = vec![msg(
            "msg1",
            Role::User,
            vec![Part::Reasoning("thinking".into())],
        )];
        let mut by_raw = HashMap::new();
        by_raw.insert("msg1".into(), "m0001".into());
        let state = make_state(by_raw);
        let params = InjectParams::default();

        let count = inject_message_ids(&mut messages, &state, &params);

        assert_eq!(count, 1);
        // Should insert a new Text part at position 0
        assert!(matches!(&messages[0].parts[0], Part::Text(t) if t == "<dcp ref=\"m0001\">"));
    }

    #[test]
    fn inject_message_ids_partial_coverage() {
        let mut messages = vec![
            msg("msg1", Role::User, vec![Part::Text("hello".into())]),
            msg("msg2", Role::Assistant, vec![Part::Text("world".into())]),
            msg("msg3", Role::User, vec![Part::Text("third".into())]),
        ];
        let mut by_raw = HashMap::new();
        by_raw.insert("msg1".into(), "m0001".into());
        // msg2 has no state entry
        by_raw.insert("msg3".into(), "m0003".into());
        let state = make_state(by_raw);
        let params = InjectParams::default();

        let count = inject_message_ids(&mut messages, &state, &params);

        assert_eq!(count, 2);
        assert!(
            matches!(&messages[0].parts[0], Part::Text(t) if t.starts_with("<dcp ref=\"m0001\">"))
        );
        // msg2 unchanged
        assert!(matches!(&messages[1].parts[0], Part::Text(t) if t == "world"));
        assert!(
            matches!(&messages[2].parts[0], Part::Text(t) if t.starts_with("<dcp ref=\"m0003\">"))
        );
    }

    #[test]
    fn inject_message_ids_returns_count() {
        let mut messages = vec![
            msg("msg1", Role::User, vec![Part::Text("hello".into())]),
            msg("msg2", Role::Assistant, vec![Part::Text("world".into())]),
        ];
        let mut by_raw = HashMap::new();
        by_raw.insert("msg1".into(), "m0001".into());
        by_raw.insert("msg2".into(), "m0002".into());
        let state = make_state(by_raw);
        let params = InjectParams::default();

        let count = inject_message_ids(&mut messages, &state, &params);

        assert_eq!(count, 2);
    }

    // ─────────────────────────────────────────────────────────────────
    // prepend_unique_tag tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn prepend_unique_tag_to_empty_text() {
        let mut msg = msg("msg1", Role::User, vec![Part::Text("".into())]);
        prepend_unique_tag(&mut msg, "<dcp ref=\"m0001\">");
        assert_eq!(msg.parts.len(), 1);
        assert!(matches!(&msg.parts[0], Part::Text(t) if t == "<dcp ref=\"m0001\">"));
    }

    #[test]
    fn prepend_unique_tag_idempotent() {
        let mut msg = msg(
            "msg1",
            Role::User,
            vec![Part::Text("<dcp ref=\"m0001\">\ncontent".into())],
        );
        prepend_unique_tag(&mut msg, "<dcp ref=\"m0001\">");
        assert_eq!(msg.parts.len(), 1);
        if let Part::Text(t) = &msg.parts[0] {
            assert!(t.starts_with("<dcp ref=\"m0001\">"));
        }
    }

    #[test]
    fn prepend_unique_tag_no_text_parts() {
        let mut msg = msg("msg1", Role::User, vec![Part::Reasoning("thinking".into())]);
        prepend_unique_tag(&mut msg, "<dcp ref=\"m0001\">");
        assert_eq!(msg.parts.len(), 2);
        assert!(matches!(&msg.parts[0], Part::Text(t) if t == "<dcp ref=\"m0001\">"));
        assert!(matches!(&msg.parts[1], Part::Reasoning(_)));
    }

    #[test]
    fn prepend_unique_tag_preserves_content() {
        let mut msg = msg(
            "msg1",
            Role::User,
            vec![Part::Text("original content".into())],
        );
        prepend_unique_tag(&mut msg, "<dcp ref=\"m0001\">");
        assert_eq!(msg.parts.len(), 1);
        if let Part::Text(t) = &msg.parts[0] {
            assert!(t.starts_with("<dcp ref=\"m0001\">\n"));
            assert!(t.contains("original content"));
        }
    }
}
