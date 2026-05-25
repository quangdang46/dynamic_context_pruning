//! Placeholder parsing and expansion — SPEC.md §6.1 ("Summary placeholder
//! syntax").
//!
//! Placeholders take the literal form `{{block:b<N>}}` (whitespace-free
//! inside the braces). On render, each placeholder is replaced by:
//!
//! ```text
//! <dcp-block id="b<N>">
//! ... summary text of b<N> ...
//! </dcp-block>
//! ```
//!
//! Required-but-not-mentioned blocks are appended in ascending block-id
//! order so information is never silently lost.

use std::collections::{BTreeSet, HashMap};

use dcp_types::{BlockId, SessionState};

use crate::error::CompressError;

/// Parse the set of `{{block:b<N>}}` placeholders mentioned in
/// `summary`. Duplicate references collapse to one entry.
///
/// The function is permissive — it scans linearly without an AST and
/// only matches the canonical form. Anything else (e.g. whitespace
/// inside the braces, alternate prefixes) is treated as plain text and
/// caught by validation downstream.
pub fn parse_placeholders(summary: &str) -> BTreeSet<BlockId> {
    let mut out: BTreeSet<BlockId> = BTreeSet::new();
    let mut rest = summary;
    while let Some(pos) = rest.find("{{block:b") {
        let after = &rest[pos + "{{block:b".len()..];
        // Read digits until '}}'.
        let Some(end) = after.find("}}") else {
            break;
        };
        let digits = &after[..end];
        if !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && let Ok(n) = digits.parse::<u32>()
            && n > 0
        {
            out.insert(BlockId::new(n));
        }
        rest = &after[end + 2..];
    }
    out
}

/// Verify every parsed placeholder references a block in
/// `required_block_ids`. Returns `Err(PlaceholderMismatch)` on the first
/// mismatch.
pub fn validate_placeholders(
    placeholders: &BTreeSet<BlockId>,
    required_block_ids: &[BlockId],
) -> Result<(), CompressError> {
    let required: BTreeSet<BlockId> = required_block_ids.iter().copied().collect();
    for p in placeholders {
        if !required.contains(p) {
            return Err(CompressError::PlaceholderMismatch(format!(
                "placeholder {} not in required_block_ids",
                p.reference()
            )));
        }
    }
    Ok(())
}

/// Replace every `{{block:b<N>}}` in `summary` with the canonical
/// expansion, using `state.prune.messages.blocks_by_id` to look up the
/// summaries. If a placeholder does not resolve, the function returns
/// [`CompressError::PlaceholderMismatch`] — validation should have
/// already caught this, but the function is defensive.
pub fn inject_placeholder_expansions(
    summary: &str,
    state: &SessionState,
) -> Result<String, CompressError> {
    let mut out = String::with_capacity(summary.len());
    let mut rest = summary;
    let blocks: &HashMap<BlockId, dcp_types::CompressionBlock> = &state.prune.messages.blocks_by_id;

    while let Some(pos) = rest.find("{{block:b") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + "{{block:b".len()..];
        let Some(end) = after.find("}}") else {
            // Unterminated placeholder — keep verbatim so the caller can
            // diagnose.
            out.push_str(&rest[pos..]);
            return Ok(out);
        };
        let digits = &after[..end];
        let n: u32 = match digits.parse() {
            Ok(n) => n,
            Err(_) => {
                out.push_str(&rest[pos..pos + "{{block:b".len() + end + 2]);
                rest = &after[end + 2..];
                continue;
            }
        };
        let bid = BlockId::new(n);
        let Some(block) = blocks.get(&bid) else {
            return Err(CompressError::PlaceholderMismatch(format!(
                "placeholder {} does not resolve",
                bid.reference()
            )));
        };
        out.push_str(&format!(
            "<dcp-block id=\"{}\">\n{}\n</dcp-block>",
            bid.reference(),
            block.summary
        ));
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Append `<dcp-included-blocks>…</dcp-included-blocks>` to `summary`
/// for every block id in `required_block_ids` that is *not* mentioned by
/// `mentioned`. Block ids are appended in ascending order.
///
/// SPEC §6.1: "A required block id that is not mentioned by any
/// placeholder is appended automatically by the library at the end of
/// the new summary in deterministic order".
pub fn append_missing_block_summaries(
    summary: &str,
    required_block_ids: &[BlockId],
    mentioned: &BTreeSet<BlockId>,
    state: &SessionState,
) -> Result<String, CompressError> {
    let mut missing: Vec<BlockId> = required_block_ids
        .iter()
        .copied()
        .filter(|b| !mentioned.contains(b))
        .collect();
    missing.sort_by_key(|b| b.value());
    if missing.is_empty() {
        return Ok(summary.to_string());
    }
    let mut out = String::with_capacity(summary.len() + 256);
    out.push_str(summary);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("<dcp-included-blocks>\n");
    for bid in missing {
        let Some(block) = state.prune.messages.blocks_by_id.get(&bid) else {
            return Err(CompressError::PlaceholderMismatch(format!(
                "missing block {} not found in state",
                bid.reference()
            )));
        };
        out.push_str(&format!(
            "<dcp-block id=\"{}\">\n{}\n</dcp-block>\n",
            bid.reference(),
            block.summary
        ));
    }
    out.push_str("</dcp-included-blocks>");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{BlockId, CompressionBlock, CompressionMode, RunId, SessionState};

    fn state_with_block(id: u32, summary: &str) -> SessionState {
        let mut state = SessionState::default();
        let mut block = CompressionBlock::new(
            BlockId::new(id),
            RunId::new(id),
            CompressionMode::Range,
            "t",
            summary,
            "m0001",
            "m0001",
            "raw1",
            "comp",
        );
        block.active = true;
        state
            .prune
            .messages
            .blocks_by_id
            .insert(block.block_id, block);
        state
    }

    #[test]
    fn parses_placeholder() {
        let s = parse_placeholders("see {{block:b3}} and {{block:b7}} and {{block:b3}}");
        let v: Vec<u32> = s.iter().map(|b| b.value()).collect();
        assert_eq!(v, vec![3, 7]);
    }

    #[test]
    fn validates_against_required() {
        let mut p: BTreeSet<BlockId> = BTreeSet::new();
        p.insert(BlockId::new(3));
        let req = vec![BlockId::new(3), BlockId::new(7)];
        assert!(validate_placeholders(&p, &req).is_ok());

        let mut bad: BTreeSet<BlockId> = BTreeSet::new();
        bad.insert(BlockId::new(99));
        assert!(matches!(
            validate_placeholders(&bad, &req),
            Err(CompressError::PlaceholderMismatch(_))
        ));
    }

    #[test]
    fn inject_expands_placeholder_with_dcp_block_tag() {
        let state = state_with_block(3, "child summary");
        let out = inject_placeholder_expansions("see {{block:b3}} done", &state).unwrap();
        assert!(out.contains("<dcp-block id=\"b3\">"));
        assert!(out.contains("child summary"));
        assert!(out.contains("</dcp-block>"));
        assert!(out.starts_with("see "));
        assert!(out.ends_with(" done"));
    }

    #[test]
    fn append_missing_appends_unreferenced_required_blocks() {
        let mut state = state_with_block(3, "summary 3");
        // Add a second block.
        let mut block = CompressionBlock::new(
            BlockId::new(7),
            RunId::new(1),
            CompressionMode::Range,
            "t",
            "summary 7",
            "m0001",
            "m0001",
            "raw2",
            "comp",
        );
        block.active = true;
        state
            .prune
            .messages
            .blocks_by_id
            .insert(block.block_id, block);

        let required = vec![BlockId::new(3), BlockId::new(7)];
        let mentioned: BTreeSet<BlockId> = std::iter::once(BlockId::new(3)).collect();
        let out = append_missing_block_summaries("base", &required, &mentioned, &state).unwrap();
        assert!(out.contains("<dcp-included-blocks>"));
        assert!(out.contains("summary 7"));
        assert!(!out.contains("summary 3")); // already mentioned, not included
    }

    #[test]
    fn append_missing_is_noop_when_all_mentioned() {
        let state = state_with_block(3, "x");
        let mentioned: BTreeSet<BlockId> = std::iter::once(BlockId::new(3)).collect();
        let out =
            append_missing_block_summaries("base", &[BlockId::new(3)], &mentioned, &state).unwrap();
        assert_eq!(out, "base");
    }
}
