//! Nudge extension helpers — guidance text injected into nudge templates.

use dcp_types::SessionState;

/// Builds guidance text listing compressed block IDs from state.
///
/// Format: "Active compressed blocks: b1, b2, b3. Include placeholders in your summary."
///
/// # Example
///
/// ```rust
/// use dcp_state::create_session_state;
/// use dcp_prompts::build_compressed_block_guidance;
///
/// let state = create_session_state();
/// let guidance = build_compressed_block_guidance(&state);
/// assert!(guidance.contains("No active compressed blocks"));
/// ```
pub fn build_compressed_block_guidance(state: &SessionState) -> String {
    let active_ids: Vec<_> = state
        .prune
        .messages
        .active_block_ids
        .iter()
        .map(|id| id.reference())
        .collect();

    if active_ids.is_empty() {
        return "No active compressed blocks.".to_string();
    }

    let list = active_ids.join(", ");
    format!(
        "Active compressed blocks: {list}. Include placeholders in your summary."
    )
}

/// Renders priority guidance for messages before an anchor point.
///
/// Format: "High priority messages: m0001, m0002. Consider compressing these first."
///
/// # Example
///
/// ```rust
/// use dcp_prompts::render_message_priority_guidance;
/// let refs = vec!["m0001".to_string(), "m0002".to_string()];
/// let guidance = render_message_priority_guidance("high", &refs);
/// assert!(guidance.contains("m0001"));
/// assert!(guidance.contains("m0002"));
/// ```
pub fn render_message_priority_guidance(priority_label: &str, refs: &[String]) -> String {
    if refs.is_empty() {
        return String::new();
    }

    let list = refs.join(", ");
    format!(
        "{priority_label} priority messages: {list}. Consider compressing these first."
    )
}

/// Inserts guidance text before the closing `</dcp>` tag in nudge text.
///
/// If `</dcp>` is not found, returns `nudge_text` unchanged.
///
/// # Example
///
/// ```rust
/// use dcp_prompts::append_guidance_to_dcp_tag;
/// let nudge = "<dcp>Some content</dcp>";
/// let guidance = "Extra guidance.";
/// let result = append_guidance_to_dcp_tag(nudge, guidance);
/// assert!(result.contains("Extra guidance"));
/// assert!(result.contains("</dcp>"));
/// ```
pub fn append_guidance_to_dcp_tag(nudge_text: &str, guidance: &str) -> String {
    match nudge_text.find("</dcp>") {
        Some(pos) => {
            let (before, after) = nudge_text.split_at(pos);
            format!("{before}{guidance}{after}")
        }
        None => nudge_text.to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_state::session::rebuild_from_messages;
    use dcp_types::{BlockId, CompressionBlock, CompressionMode, Message, Part, Role, RunId};
    use dcp_state::config_like::StaticConfigLike;

    fn cfg() -> StaticConfigLike {
        StaticConfigLike::default()
    }

    fn make_block(id: u32, active: bool) -> CompressionBlock {
        let mut b = CompressionBlock::new(
            BlockId::new(id),
            RunId::new(id),
            CompressionMode::Range,
            "topic",
            "summary",
            "m0001",
            "m0002",
            "raw-anchor",
            "raw-compress",
        );
        b.active = active;
        b
    }

    #[test]
    fn test_build_guidance_with_active_blocks() {
        let mut state = SessionState::default();
        let b1 = make_block(1, true);
        let b2 = make_block(2, true);
        let b3 = make_block(3, true);

        state
            .prune
            .messages
            .blocks_by_id
            .insert(BlockId::new(1), b1.clone());
        state
            .prune
            .messages
            .blocks_by_id
            .insert(BlockId::new(2), b2.clone());
        state
            .prune
            .messages
            .blocks_by_id
            .insert(BlockId::new(3), b3.clone());
        state.prune.messages.active_block_ids.insert(BlockId::new(1));
        state.prune.messages.active_block_ids.insert(BlockId::new(2));
        state.prune.messages.active_block_ids.insert(BlockId::new(3));

        let guidance = build_compressed_block_guidance(&state);
        assert!(guidance.contains("b1"));
        assert!(guidance.contains("b2"));
        assert!(guidance.contains("b3"));
        assert!(guidance.contains("Active compressed blocks:"));
        assert!(guidance.contains("Include placeholders"));
    }

    #[test]
    fn test_build_guidance_with_no_blocks() {
        let state = SessionState::default();
        let guidance = build_compressed_block_guidance(&state);
        assert!(guidance.contains("No active compressed blocks"));
    }

    #[test]
    fn test_render_priority_guidance_high() {
        let refs = vec![
            "m0001".to_string(),
            "m0002".to_string(),
            "m0003".to_string(),
        ];
        let guidance = render_message_priority_guidance("High", &refs);
        assert!(guidance.contains("High"));
        assert!(guidance.contains("m0001"));
        assert!(guidance.contains("m0002"));
        assert!(guidance.contains("m0003"));
        assert!(guidance.contains("Consider compressing these first"));
    }

    #[test]
    fn test_render_priority_guidance_empty() {
        let refs: Vec<String> = vec![];
        let guidance = render_message_priority_guidance("low", &refs);
        assert!(guidance.is_empty());
    }

    #[test]
    fn test_append_guidance_to_dcp_tag() {
        let nudge = "<dcp>Some nudge content</dcp>";
        let guidance = " Consider this extra guidance.";
        let result = append_guidance_to_dcp_tag(nudge, guidance);
        assert!(result.contains("Some nudge content"));
        assert!(result.contains("Consider this extra guidance"));
        assert!(result.contains("</dcp>"));
        // Guidance should appear before </dcp>
        let pos_dcp = result.find("</dcp>").unwrap();
        let pos_guidance = result.find("Consider this extra guidance").unwrap();
        assert!(pos_guidance < pos_dcp);
    }

    #[test]
    fn test_append_guidance_no_tag() {
        let nudge = "No dcp tag here";
        let guidance = "Some guidance";
        let result = append_guidance_to_dcp_tag(nudge, guidance);
        assert_eq!(result, nudge);
    }

    #[test]
    fn test_append_guidance_multiple_dcp_tags() {
        let nudge = "<dcp>First</dcp> middle <dcp>Second</dcp>";
        let guidance = " INSERTED";
        let result = append_guidance_to_dcp_tag(nudge, guidance);
        // Should insert before the first </dcp>
        assert!(result.starts_with("<dcp>First INSERTED</dcp>"));
    }
}