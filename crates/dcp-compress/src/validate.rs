//! Validation of compress arguments — SPEC.md §6.1 ("Validation rules").

use std::collections::HashSet;

use crate::config::CompressConfig;
use crate::error::CompressError;
use crate::resolve::ResolvedRange;
use crate::types::{CompressArgs, MessageEntry, RangeEntry};

/// Run the input-shape validation that does not depend on session state.
///
/// Specifically: trims the topic and verifies it is non-empty; checks
/// that `content` is non-empty; verifies each entry's `summary` length
/// is within bounds. References are resolved separately by
/// [`crate::resolve::resolve_range`].
pub fn validate_topic_and_content<C: CompressConfig + ?Sized>(
    args: &CompressArgs,
    config: &C,
) -> Result<(), CompressError> {
    let max_chars = config.max_summary_chars();
    match args {
        CompressArgs::Range { topic, content } => {
            check_topic(topic)?;
            if content.is_empty() {
                return Err(CompressError::InvalidCompressArgs("empty content".into()));
            }
            for entry in content {
                check_range_entry(entry, max_chars)?;
            }
        }
        CompressArgs::Message { topic, content } => {
            check_topic(topic)?;
            if content.is_empty() {
                return Err(CompressError::InvalidCompressArgs("empty content".into()));
            }
            // Reject duplicate `messageId` across `content` (SPEC §6.2).
            let mut seen: HashSet<&str> = HashSet::new();
            for entry in content {
                check_message_entry(entry, max_chars)?;
                if !seen.insert(entry.message_id.as_str()) {
                    return Err(CompressError::InvalidCompressArgs(format!(
                        "duplicate messageId {}",
                        entry.message_id
                    )));
                }
            }
        }
    }
    Ok(())
}

fn check_topic(topic: &str) -> Result<(), CompressError> {
    if topic.trim().is_empty() {
        return Err(CompressError::InvalidCompressArgs("empty topic".into()));
    }
    Ok(())
}

fn check_range_entry(entry: &RangeEntry, max_chars: usize) -> Result<(), CompressError> {
    if entry.start_id.is_empty() || entry.end_id.is_empty() {
        return Err(CompressError::InvalidCompressArgs("malformed entry".into()));
    }
    let len = entry.summary.chars().count();
    if len == 0 {
        return Err(CompressError::InvalidCompressArgs(
            "summary too short".into(),
        ));
    }
    if len > max_chars {
        return Err(CompressError::InvalidCompressArgs(
            "summary too long".into(),
        ));
    }
    Ok(())
}

fn check_message_entry(entry: &MessageEntry, max_chars: usize) -> Result<(), CompressError> {
    if entry.message_id.is_empty() {
        return Err(CompressError::InvalidCompressArgs("malformed entry".into()));
    }
    if entry.topic.trim().is_empty() {
        return Err(CompressError::InvalidCompressArgs(
            "empty per-entry topic".into(),
        ));
    }
    let len = entry.summary.chars().count();
    if len == 0 {
        return Err(CompressError::InvalidCompressArgs(
            "summary too short".into(),
        ));
    }
    if len > max_chars {
        return Err(CompressError::InvalidCompressArgs(
            "summary too long".into(),
        ));
    }
    Ok(())
}

/// SPEC §11.5: ranges within a single call must have non-overlapping
/// resolved selections (excluding active blocks the ranges share, since
/// those are handled by consumption).
///
/// Implementation: count appearance of each `direct_message_id` across
/// plans. Any id that appears more than once means the ranges overlap.
pub fn validate_non_overlapping(plans: &[ResolvedRange]) -> Result<(), CompressError> {
    let mut seen: HashSet<&str> = HashSet::new();
    for plan in plans {
        for id in &plan.direct_message_ids {
            if !seen.insert(id.as_str()) {
                return Err(CompressError::RangeOverlap(format!(
                    "ranges share direct message id {id}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StaticCompressConfig;

    fn cfg() -> StaticCompressConfig {
        StaticCompressConfig::defaults()
    }

    #[test]
    fn valid_range_args_pass() {
        let args = CompressArgs::Range {
            topic: "auth".into(),
            content: vec![RangeEntry {
                start_id: "m0001".into(),
                end_id: "m0003".into(),
                summary: "summary text".into(),
            }],
        };
        assert!(validate_topic_and_content(&args, &cfg()).is_ok());
    }

    #[test]
    fn empty_topic_rejected() {
        let args = CompressArgs::Range {
            topic: "  ".into(),
            content: vec![RangeEntry {
                start_id: "m0001".into(),
                end_id: "m0001".into(),
                summary: "x".into(),
            }],
        };
        let e = validate_topic_and_content(&args, &cfg()).unwrap_err();
        assert!(matches!(e, CompressError::InvalidCompressArgs(s) if s == "empty topic"));
    }

    #[test]
    fn empty_content_rejected() {
        let args = CompressArgs::Range {
            topic: "ok".into(),
            content: vec![],
        };
        let e = validate_topic_and_content(&args, &cfg()).unwrap_err();
        assert!(matches!(e, CompressError::InvalidCompressArgs(s) if s == "empty content"));
    }

    #[test]
    fn summary_length_bounds() {
        let too_short = CompressArgs::Range {
            topic: "ok".into(),
            content: vec![RangeEntry {
                start_id: "m0001".into(),
                end_id: "m0001".into(),
                summary: "".into(),
            }],
        };
        assert!(validate_topic_and_content(&too_short, &cfg()).is_err());

        let too_long = CompressArgs::Range {
            topic: "ok".into(),
            content: vec![RangeEntry {
                start_id: "m0001".into(),
                end_id: "m0001".into(),
                summary: "x".repeat(33 * 1024),
            }],
        };
        assert!(validate_topic_and_content(&too_long, &cfg()).is_err());
    }

    #[test]
    fn message_mode_duplicate_message_id_rejected() {
        let args = CompressArgs::Message {
            topic: "x".into(),
            content: vec![
                MessageEntry {
                    message_id: "m0001".into(),
                    topic: "t".into(),
                    summary: "s".into(),
                },
                MessageEntry {
                    message_id: "m0001".into(),
                    topic: "t".into(),
                    summary: "s".into(),
                },
            ],
        };
        let e = validate_topic_and_content(&args, &cfg()).unwrap_err();
        assert!(matches!(e, CompressError::InvalidCompressArgs(s) if s.contains("duplicate")));
    }

    #[test]
    fn non_overlapping_passes_disjoint() {
        let p = vec![
            ResolvedRange {
                start_raw: "u1".into(),
                end_raw: "a1".into(),
                selection_indices: vec![0, 1],
                required_block_ids: vec![],
                anchor_message_id: "u1".into(),
                direct_message_ids: vec!["u1".into(), "a1".into()],
                direct_tool_ids: vec![],
            },
            ResolvedRange {
                start_raw: "u2".into(),
                end_raw: "a2".into(),
                selection_indices: vec![2, 3],
                required_block_ids: vec![],
                anchor_message_id: "u2".into(),
                direct_message_ids: vec!["u2".into(), "a2".into()],
                direct_tool_ids: vec![],
            },
        ];
        assert!(validate_non_overlapping(&p).is_ok());
    }

    #[test]
    fn non_overlapping_detects_overlap() {
        let p = vec![
            ResolvedRange {
                start_raw: "u1".into(),
                end_raw: "a1".into(),
                selection_indices: vec![0, 1],
                required_block_ids: vec![],
                anchor_message_id: "u1".into(),
                direct_message_ids: vec!["u1".into(), "a1".into()],
                direct_tool_ids: vec![],
            },
            ResolvedRange {
                start_raw: "a1".into(),
                end_raw: "u2".into(),
                selection_indices: vec![1, 2],
                required_block_ids: vec![],
                anchor_message_id: "a1".into(),
                direct_message_ids: vec!["a1".into(), "u2".into()],
                direct_tool_ids: vec![],
            },
        ];
        assert!(matches!(
            validate_non_overlapping(&p),
            Err(CompressError::RangeOverlap(_))
        ));
    }
}
