//! Wrapped-summary assembly — SPEC.md §6.3.2 and §6.3.3.
//!
//! The wrapped summary is the string committed to a [`CompressionBlock::summary`]
//! and rendered into the outgoing message stream. Its shape (showCompression
//! mode):
//!
//! ```text
//! <dcp-block id="b<N>" topic="<topic>">
//! <dcp-summary>
//! <expanded summary text>
//! </dcp-summary>
//! <dcp-protected-user>...</dcp-protected-user>      (optional)
//! <dcp-protected-tools>...</dcp-protected-tools>    (optional)
//! </dcp-block>
//! ```

use dcp_types::{BlockId, CompressionBlock, Message, Part, Role};

use crate::config::CompressConfig;
use crate::resolve::ResolvedRange;

/// Per-message protected-user truncation cap (SPEC §6.3.2 — 8 KiB).
pub const PROTECTED_USER_TRUNCATE_BYTES: usize = 8 * 1024;

/// Append `<dcp-protected-user>…</dcp-protected-user>` for every user
/// message inside the range, gated by `config.protect_user_messages`.
/// Each message is truncated to [`PROTECTED_USER_TRUNCATE_BYTES`] at a
/// UTF-8 codepoint boundary; truncated messages are tagged with
/// `[truncated]`.
pub fn append_protected_user_messages<C: CompressConfig + ?Sized>(
    summary: &str,
    plan: &ResolvedRange,
    messages: &[Message],
    config: &C,
) -> String {
    if !config.protect_user_messages() {
        return summary.to_string();
    }
    let mut bodies: Vec<String> = Vec::new();
    for idx in &plan.selection_indices {
        let msg = &messages[*idx];
        if msg.role != Role::User {
            continue;
        }
        for part in &msg.parts {
            if let Part::Text(t) = part {
                let truncated = truncate_utf8_with_marker(t, PROTECTED_USER_TRUNCATE_BYTES);
                bodies.push(truncated);
            }
        }
    }
    if bodies.is_empty() {
        return summary.to_string();
    }
    let mut out = summary.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("<dcp-protected-user>\n");
    for body in bodies {
        out.push_str(&body);
        out.push('\n');
    }
    out.push_str("</dcp-protected-user>");
    out
}

/// Append `<dcp-protected-tools>…</dcp-protected-tools>` for every tool
/// result inside the range whose tool name is in
/// `config.protected_tools()`.
pub fn append_protected_tool_outputs<C: CompressConfig + ?Sized>(
    summary: &str,
    plan: &ResolvedRange,
    messages: &[Message],
    state: &dcp_types::SessionState,
    config: &C,
) -> String {
    let protected = config.protected_tools();
    let mut bodies: Vec<String> = Vec::new();
    for idx in &plan.selection_indices {
        for part in &messages[*idx].parts {
            if let Part::ToolResult {
                call_id,
                output: Some(out),
                ..
            } = part
                && let Some(entry) = state.tool_parameters.get(call_id)
                && protected.is_protected(&entry.tool)
            {
                bodies.push(format!(
                    "<dcp-tool name=\"{}\" call_id=\"{}\">\n{}\n</dcp-tool>",
                    entry.tool, call_id, out
                ));
            }
        }
    }
    if bodies.is_empty() {
        return summary.to_string();
    }
    let mut out = summary.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("<dcp-protected-tools>\n");
    for body in bodies {
        out.push_str(&body);
        out.push('\n');
    }
    out.push_str("</dcp-protected-tools>");
    out
}

/// Wrap an expanded summary into the canonical
/// `<dcp-block id="b<N>" topic="…">…</dcp-block>` form (SPEC §6.3.3).
///
/// When `config.show_compression()` is `false`, the surrounding block
/// envelope is omitted; only the inner content remains.
pub fn wrap_compressed_summary<C: CompressConfig + ?Sized>(
    block_id: BlockId,
    topic: &str,
    inner: &str,
    config: &C,
) -> String {
    let escaped = escape_attr(topic);
    let inner_block = format!("<dcp-summary>\n{inner}\n</dcp-summary>");
    if !config.show_compression() {
        return inner_block;
    }
    format!(
        "<dcp-block id=\"{}\" topic=\"{escaped}\">\n{inner_block}\n</dcp-block>",
        block_id.reference()
    )
}

/// Estimate `compressed_tokens` for a plan as the rough sum of bytes
/// across direct parts (used as a best-effort estimate at commit time
/// per SPEC §6.3 — the host's tokenizer can refine this later).
pub fn estimate_compressed_tokens(plan: &ResolvedRange, messages: &[Message]) -> u64 {
    let mut bytes: u64 = 0;
    for idx in &plan.selection_indices {
        for part in &messages[*idx].parts {
            match part {
                Part::Text(t) | Part::Reasoning(t) => {
                    bytes = bytes.saturating_add(t.len() as u64);
                }
                Part::ToolCall { input, .. } => {
                    bytes = bytes.saturating_add(
                        serde_json::to_string(input)
                            .map(|s| s.len() as u64)
                            .unwrap_or(0),
                    );
                }
                Part::ToolResult {
                    output: Some(o), ..
                } => {
                    bytes = bytes.saturating_add(o.len() as u64);
                }
                _ => {}
            }
        }
    }
    // 4 bytes per token is the SPEC's `chars_div_4` placeholder default.
    bytes / 4
}

/// Estimate `summary_tokens` for a wrapped summary string.
pub fn estimate_summary_tokens(wrapped: &str) -> u64 {
    (wrapped.len() / 4) as u64
}

/// Compute `effective_message_ids` and `effective_tool_ids` for a fresh
/// block (SPEC §6.4) — the closure of `direct_*` over consumed blocks.
pub fn compute_effective(
    direct_message_ids: &[String],
    direct_tool_ids: &[String],
    consumed_block_ids: &[BlockId],
    blocks_by_id: &std::collections::HashMap<BlockId, CompressionBlock>,
) -> (Vec<String>, Vec<String>) {
    let mut messages: Vec<String> = direct_message_ids.to_vec();
    let mut tools: Vec<String> = direct_tool_ids.to_vec();
    let mut seen_msg: std::collections::HashSet<String> = messages.iter().cloned().collect();
    let mut seen_tool: std::collections::HashSet<String> = tools.iter().cloned().collect();

    for cid in consumed_block_ids {
        let Some(consumed) = blocks_by_id.get(cid) else {
            continue;
        };
        for m in &consumed.effective_message_ids {
            if seen_msg.insert(m.clone()) {
                messages.push(m.clone());
            }
        }
        for t in &consumed.effective_tool_ids {
            if seen_tool.insert(t.clone()) {
                tools.push(t.clone());
            }
        }
    }
    (messages, tools)
}

/// Compute `included_block_ids` for a fresh block (SPEC §6.4) — the
/// transitive closure of `consumed_block_ids` over each consumed
/// block's `included_block_ids`.
pub fn compute_included(
    consumed_block_ids: &[BlockId],
    blocks_by_id: &std::collections::HashMap<BlockId, CompressionBlock>,
) -> Vec<BlockId> {
    let mut included: Vec<BlockId> = consumed_block_ids.to_vec();
    let mut seen: std::collections::BTreeSet<BlockId> = included.iter().copied().collect();
    for cid in consumed_block_ids {
        let Some(consumed) = blocks_by_id.get(cid) else {
            continue;
        };
        for inc in &consumed.included_block_ids {
            if seen.insert(*inc) {
                included.push(*inc);
            }
        }
    }
    included
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn truncate_utf8_with_marker(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + 16);
    out.push_str(&s[..end]);
    out.push_str("\n[truncated]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StaticCompressConfig;
    use dcp_protected::ToolProtection;
    use dcp_types::{
        BlockId, CompressionBlock, CompressionMode, Message, Part, Role, RunId, SessionState,
        ToolParameterEntry, ToolStatus,
    };
    use serde_json::json;

    #[test]
    fn wrap_with_show_compression_emits_outer_tag() {
        let cfg = StaticCompressConfig::defaults();
        let s = wrap_compressed_summary(BlockId::new(7), "auth", "body", &cfg);
        assert!(s.contains("<dcp-block id=\"b7\""));
        assert!(s.contains("topic=\"auth\""));
        assert!(s.contains("body"));
        assert!(s.contains("</dcp-block>"));
    }

    #[test]
    fn wrap_without_show_compression_omits_outer_tag() {
        let cfg = StaticCompressConfig {
            show_compression: false,
            ..StaticCompressConfig::defaults()
        };
        let s = wrap_compressed_summary(BlockId::new(7), "auth", "body", &cfg);
        assert!(!s.contains("<dcp-block"));
        assert!(s.contains("body"));
    }

    #[test]
    fn protected_user_messages_disabled_by_default() {
        let cfg = StaticCompressConfig::defaults();
        let messages = vec![
            Message::user_text("u1", 0, "secret"),
            Message::assistant_text("a1", 0, "ack"),
        ];
        let plan = ResolvedRange {
            start_raw: "u1".into(),
            end_raw: "a1".into(),
            selection_indices: vec![0, 1],
            required_block_ids: vec![],
            anchor_message_id: "u1".into(),
            direct_message_ids: vec!["u1".into(), "a1".into()],
            direct_tool_ids: vec![],
        };
        let out = append_protected_user_messages("base", &plan, &messages, &cfg);
        assert_eq!(out, "base");
    }

    #[test]
    fn protected_user_messages_appends_when_enabled() {
        let cfg = StaticCompressConfig {
            protect_user_messages: true,
            ..StaticCompressConfig::defaults()
        };
        let messages = vec![Message::user_text("u1", 0, "secret content")];
        let plan = ResolvedRange {
            start_raw: "u1".into(),
            end_raw: "u1".into(),
            selection_indices: vec![0],
            required_block_ids: vec![],
            anchor_message_id: "u1".into(),
            direct_message_ids: vec!["u1".into()],
            direct_tool_ids: vec![],
        };
        let out = append_protected_user_messages("base", &plan, &messages, &cfg);
        assert!(out.contains("<dcp-protected-user>"));
        assert!(out.contains("secret content"));
    }

    #[test]
    fn protected_tool_outputs_appends_for_protected_tool_only() {
        let cfg = StaticCompressConfig {
            protected_tools: ToolProtection::new(["task"]),
            ..StaticCompressConfig::defaults()
        };
        let messages = vec![
            Message::new(
                "a1",
                Role::Assistant,
                vec![Part::tool_call("c1", "task", json!({}))],
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
            Message::new(
                "a2",
                Role::Assistant,
                vec![Part::tool_call("c2", "read", json!({}))],
                0,
            ),
            Message::new(
                "u2",
                Role::User,
                vec![Part::tool_result(
                    "c2",
                    ToolStatus::Completed,
                    Some("file contents".into()),
                    None,
                )],
                0,
            ),
        ];
        let mut state = SessionState::default();
        state.tool_parameters.insert(
            "c1".into(),
            ToolParameterEntry {
                tool: "task".into(),
                ..ToolParameterEntry::default()
            },
        );
        state.tool_parameters.insert(
            "c2".into(),
            ToolParameterEntry {
                tool: "read".into(),
                ..ToolParameterEntry::default()
            },
        );
        let plan = ResolvedRange {
            start_raw: "a1".into(),
            end_raw: "u2".into(),
            selection_indices: vec![0, 1, 2, 3],
            required_block_ids: vec![],
            anchor_message_id: "a1".into(),
            direct_message_ids: vec!["a1".into(), "u1".into(), "a2".into(), "u2".into()],
            direct_tool_ids: vec!["c1".into(), "c2".into()],
        };
        let out = append_protected_tool_outputs("base", &plan, &messages, &state, &cfg);
        assert!(out.contains("important task output"));
        assert!(!out.contains("file contents"));
    }

    #[test]
    fn compute_effective_unions_with_consumed() {
        let mut blocks = std::collections::HashMap::new();
        let mut child = CompressionBlock::new(
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
        child.effective_message_ids = vec!["m_child_1".into()];
        child.effective_tool_ids = vec!["c_child_1".into()];
        blocks.insert(child.block_id, child);

        let (msgs, tools) = compute_effective(
            &["m_direct_1".to_string()],
            &["c_direct_1".to_string()],
            &[BlockId::new(1)],
            &blocks,
        );
        assert_eq!(msgs, vec!["m_direct_1", "m_child_1"]);
        assert_eq!(tools, vec!["c_direct_1", "c_child_1"]);
    }

    #[test]
    fn compute_included_unions_transitive() {
        let mut blocks = std::collections::HashMap::new();
        let mut child = CompressionBlock::new(
            BlockId::new(1),
            RunId::new(1),
            CompressionMode::Range,
            "t",
            "s",
            "m",
            "m",
            "r",
            "c",
        );
        child.included_block_ids = vec![BlockId::new(99)];
        blocks.insert(child.block_id, child);

        let included = compute_included(&[BlockId::new(1)], &blocks);
        // Order: [1, 99] (1 from direct consumed list, 99 from transitive).
        assert_eq!(included, vec![BlockId::new(1), BlockId::new(99)]);
    }

    #[test]
    fn truncate_utf8_marker_preserves_codepoint_boundary() {
        let s = "🦀🦀🦀";
        let t = truncate_utf8_with_marker(s, 5);
        // 5 bytes lands mid-emoji; back up to 4 bytes (one emoji).
        assert!(t.starts_with("🦀"));
        assert!(t.contains("[truncated]"));
    }
}
