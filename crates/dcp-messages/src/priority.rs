//! Message priority classification — port of lib/messages/priority.ts.
//!
//! Provides: build_priority_map, classify_message_priority, list_priority_refs_before_index.

use std::collections::HashMap;

use dcp_config::{CompressMode, Config};
use dcp_types::{Message, SessionState};

use crate::query::{is_ignored_user_message, is_protected_user_message};

/// Minimum token count for [`MessagePriority::Medium`].
const MEDIUM_PRIORITY_MIN_TOKENS: u64 = 500;

/// Minimum token count for [`MessagePriority::High`].
const HIGH_PRIORITY_MIN_TOKENS: u64 = 5000;

/// Message priority level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MessagePriority {
    /// Low priority — token count below [`MEDIUM_PRIORITY_MIN_TOKENS`].
    Low,
    /// Medium priority — token count >= [`MEDIUM_PRIORITY_MIN_TOKENS`] but < [`HIGH_PRIORITY_MIN_TOKENS`].
    Medium,
    /// High priority — token count >= [`HIGH_PRIORITY_MIN_TOKENS`].
    High,
}

/// Entry in the priority map — stores metadata for a message ref.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressionPriorityEntry {
    /// Canonical message reference (e.g., `"m0042"`).
    pub ref_: String,
    /// Token count of the message.
    pub token_count: u64,
    /// Computed priority level.
    pub priority: MessagePriority,
}

/// Map from raw message id → priority entry.
pub type CompressionPriorityMap = HashMap<String, CompressionPriorityEntry>;

/// Classify a message priority based on its token count.
///
/// Thresholds:
/// - `>= 5000` → [`MessagePriority::High`]
/// - `>= 500` → [`MessagePriority::Medium`]
/// - `< 500` → [`MessagePriority::Low`]
///
/// # Example
///
/// ```rust
/// use dcp_messages::priority::{classify_message_priority, MessagePriority};
///
/// assert_eq!(classify_message_priority(10_000), MessagePriority::High);
/// assert_eq!(classify_message_priority(500), MessagePriority::Medium);
/// assert_eq!(classify_message_priority(499), MessagePriority::Low);
/// ```
#[must_use]
pub fn classify_message_priority(token_count: u64) -> MessagePriority {
    if token_count >= HIGH_PRIORITY_MIN_TOKENS {
        MessagePriority::High
    } else if token_count >= MEDIUM_PRIORITY_MIN_TOKENS {
        MessagePriority::Medium
    } else {
        MessagePriority::Low
    }
}

/// Build the priority map from a message list.
///
/// Only operates in message mode (checked via `config.compress.mode`).
/// For each message:
/// - Skips if `is_ignored_user_message` returns true
/// - Skips if `is_protected_user_message` returns true
/// - Skips if the raw id has no entry in `state.message_ids.by_raw_id`
///
/// The token counter closure is used to determine priority classification.
pub fn build_priority_map<F>(
    config: &Config,
    state: &SessionState,
    messages: &[Message],
    token_counter: F,
) -> CompressionPriorityMap
where
    F: Fn(&Message) -> u64,
{
    let mut result = HashMap::new();

    // Only build priority map in message mode
    if config.compress.mode != CompressMode::Message {
        return result;
    }

    for msg in messages {
        // Skip ignored user messages
        if is_ignored_user_message(msg) {
            continue;
        }

        // Skip protected user messages
        if is_protected_user_message(config, msg) {
            continue;
        }

        // Look up the ref for this message's raw id
        let Some(ref_) = state.message_ids.by_raw_id.get(&msg.id) else {
            continue;
        };

        let token_count = token_counter(msg);
        let priority = classify_message_priority(token_count);

        let entry = CompressionPriorityEntry {
            ref_: ref_.clone(),
            token_count,
            priority,
        };

        result.insert(msg.id.clone(), entry);
    }

    result
}

/// List message refs with the given priority that appear before `anchor_index`.
///
/// Collects refs from all messages before `anchor_index` whose priority
/// matches the requested level. Results are deduplicated and returned
/// in the order they first appear.
#[must_use]
pub fn list_priority_refs_before_index(
    messages: &[Message],
    priorities: &CompressionPriorityMap,
    anchor_index: usize,
    priority: MessagePriority,
) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    for msg in messages.iter().take(anchor_index) {
        let Some(entry) = priorities.get(&msg.id) else {
            continue;
        };

        if entry.priority != priority {
            continue;
        }

        // Deduplicate by ref_
        if seen.insert(entry.ref_.clone()) {
            result.push(entry.ref_.clone());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_config::CompressMode;
    use dcp_types::{Message, Part, Role};

    fn make_config(mode: CompressMode) -> Config {
        let mut cfg = Config::default();
        cfg.compress.mode = mode;
        cfg
    }

    fn make_message(id: &str, role: Role, text: &str) -> Message {
        Message::new(id, role, vec![Part::text(text)], 0)
    }

    fn token_counter(_msg: &Message) -> u64 {
        // Default: simple count based on text parts
        100 // default medium priority
    }

    // ─────────────────────────────────────────────────────────────────
    // classify_message_priority tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn classify_high_at_exact_threshold() {
        assert_eq!(classify_message_priority(5000), MessagePriority::High);
    }

    #[test]
    fn classify_high_above_threshold() {
        assert_eq!(classify_message_priority(10_000), MessagePriority::High);
    }

    #[test]
    fn classify_medium_at_exact_threshold() {
        assert_eq!(classify_message_priority(500), MessagePriority::Medium);
    }

    #[test]
    fn classify_medium_below_high_threshold() {
        assert_eq!(classify_message_priority(4999), MessagePriority::Medium);
    }

    #[test]
    fn classify_low_at_zero() {
        assert_eq!(classify_message_priority(0), MessagePriority::Low);
    }

    #[test]
    fn classify_low_just_below_medium() {
        assert_eq!(classify_message_priority(499), MessagePriority::Low);
    }

    #[test]
    fn classify_low_just_above_zero() {
        assert_eq!(classify_message_priority(1), MessagePriority::Low);
    }

    // ─────────────────────────────────────────────────────────────────
    // build_priority_map tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn build_priority_map_empty_when_not_message_mode() {
        let cfg = make_config(CompressMode::Range);
        let state = SessionState::default();
        let messages = vec![make_message("m1", Role::User, "hello")];

        let result = build_priority_map(&cfg, &state, &messages, token_counter);
        assert!(result.is_empty());
    }

    #[test]
    fn build_priority_map_empty_when_no_messages() {
        let cfg = make_config(CompressMode::Message);
        let state = SessionState::default();
        let messages: Vec<Message> = vec![];

        let result = build_priority_map(&cfg, &state, &messages, token_counter);
        assert!(result.is_empty());
    }

    #[test]
    fn build_priority_map_includes_valid_message() {
        let mut cfg = make_config(CompressMode::Message);
        cfg.compress.protect_user_messages = false; // ensure not protected

        let mut state = SessionState::default();
        state
            .message_ids
            .by_raw_id
            .insert("m1".to_string(), "m0001".to_string());
        state.message_ids.next_ref = 2;

        let messages = vec![make_message("m1", Role::User, "hello")];

        let result = build_priority_map(&cfg, &state, &messages, token_counter);
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("m1").unwrap().ref_, "m0001");
    }

    #[test]
    fn build_priority_map_skips_when_no_ref_in_state() {
        let cfg = make_config(CompressMode::Message);
        let mut state = SessionState::default();
        // Don't add m1 to by_raw_id
        state.message_ids.next_ref = 2;

        let messages = vec![make_message("m1", Role::User, "hello")];

        let result = build_priority_map(&cfg, &state, &messages, token_counter);
        assert!(result.is_empty());
    }

    #[test]
    fn build_priority_map_multiple_messages() {
        let cfg = make_config(CompressMode::Message);
        let mut state = SessionState::default();
        state
            .message_ids
            .by_raw_id
            .insert("m1".to_string(), "m0001".to_string());
        state
            .message_ids
            .by_raw_id
            .insert("m2".to_string(), "m0002".to_string());
        state.message_ids.next_ref = 3;

        let messages = vec![
            make_message("m1", Role::User, "hello"),
            make_message("m2", Role::Assistant, "world"),
        ];

        let result = build_priority_map(&cfg, &state, &messages, token_counter);
        assert_eq!(result.len(), 2);
        assert!(result.contains_key("m1"));
        assert!(result.contains_key("m2"));
    }

    #[test]
    fn build_priority_map_uses_token_counter_for_priority() {
        let cfg = make_config(CompressMode::Message);
        let mut state = SessionState::default();
        state
            .message_ids
            .by_raw_id
            .insert("m1".to_string(), "m0001".to_string());
        state
            .message_ids
            .by_raw_id
            .insert("m2".to_string(), "m0002".to_string());
        state.message_ids.next_ref = 3;

        let messages = vec![
            make_message("m1", Role::User, "hello"),
            make_message("m2", Role::Assistant, "world"),
        ];

        // Custom token counter that gives m1 high priority and m2 low priority
        let custom_counter = |msg: &Message| {
            if msg.id == "m1" {
                6000 // High
            } else {
                100 // Low
            }
        };

        let result = build_priority_map(&cfg, &state, &messages, custom_counter);
        assert_eq!(result.get("m1").unwrap().priority, MessagePriority::High);
        assert_eq!(result.get("m2").unwrap().priority, MessagePriority::Low);
    }

    #[test]
    fn build_priority_map_stores_token_count() {
        let cfg = make_config(CompressMode::Message);
        let mut state = SessionState::default();
        state
            .message_ids
            .by_raw_id
            .insert("m1".to_string(), "m0001".to_string());
        state.message_ids.next_ref = 2;

        let messages = vec![make_message("m1", Role::User, "hello")];

        let counter = |_msg: &Message| 1234u64;
        let result = build_priority_map(&cfg, &state, &messages, counter);
        assert_eq!(result.get("m1").unwrap().token_count, 1234);
    }

    // ─────────────────────────────────────────────────────────────────
    // list_priority_refs_before_index tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn list_priority_refs_empty_map() {
        let messages = vec![
            make_message("m1", Role::User, "hello"),
            make_message("m2", Role::Assistant, "world"),
        ];
        let priorities = CompressionPriorityMap::new();

        let result =
            list_priority_refs_before_index(&messages, &priorities, 2, MessagePriority::Medium);
        assert!(result.is_empty());
    }

    #[test]
    fn list_priority_refs_empty_result_when_no_match() {
        let messages = vec![make_message("m1", Role::User, "hello")];
        let mut priorities = CompressionPriorityMap::new();
        priorities.insert(
            "m1".to_string(),
            CompressionPriorityEntry {
                ref_: "m0001".to_string(),
                token_count: 100,
                priority: MessagePriority::Low, // not Medium
            },
        );

        let result =
            list_priority_refs_before_index(&messages, &priorities, 1, MessagePriority::Medium);
        assert!(result.is_empty());
    }

    #[test]
    fn list_priority_refs_before_anchor_index() {
        let messages = vec![
            make_message("m1", Role::User, "hello"),
            make_message("m2", Role::Assistant, "world"),
        ];
        let mut priorities = CompressionPriorityMap::new();
        priorities.insert(
            "m1".to_string(),
            CompressionPriorityEntry {
                ref_: "m0001".to_string(),
                token_count: 100,
                priority: MessagePriority::Medium,
            },
        );
        priorities.insert(
            "m2".to_string(),
            CompressionPriorityEntry {
                ref_: "m0002".to_string(),
                token_count: 100,
                priority: MessagePriority::Medium,
            },
        );

        // anchor at index 1 should only look at m1
        let result =
            list_priority_refs_before_index(&messages, &priorities, 1, MessagePriority::Medium);
        assert_eq!(result, vec!["m0001"]);
    }

    #[test]
    fn list_priority_refs_deduplicates() {
        let messages = vec![
            make_message("m1", Role::User, "hello"),
            make_message("m2", Role::Assistant, "world"),
            make_message("m3", Role::User, "test"),
        ];
        let mut priorities = CompressionPriorityMap::new();
        // m1 and m3 have same ref (shouldn't happen in practice but test dedup)
        priorities.insert(
            "m1".to_string(),
            CompressionPriorityEntry {
                ref_: "m0001".to_string(),
                token_count: 100,
                priority: MessagePriority::Medium,
            },
        );
        priorities.insert(
            "m2".to_string(),
            CompressionPriorityEntry {
                ref_: "m0002".to_string(),
                token_count: 100,
                priority: MessagePriority::Medium,
            },
        );
        priorities.insert(
            "m3".to_string(),
            CompressionPriorityEntry {
                ref_: "m0001".to_string(), // same ref as m1
                token_count: 100,
                priority: MessagePriority::Medium,
            },
        );

        let result =
            list_priority_refs_before_index(&messages, &priorities, 3, MessagePriority::Medium);
        // Should deduplicate to only 2 unique refs
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"m0001".to_string()));
        assert!(result.contains(&"m0002".to_string()));
    }

    #[test]
    fn list_priority_refs_respects_priority_filter() {
        let messages = vec![
            make_message("m1", Role::User, "hello"),
            make_message("m2", Role::Assistant, "world"),
        ];
        let mut priorities = CompressionPriorityMap::new();
        priorities.insert(
            "m1".to_string(),
            CompressionPriorityEntry {
                ref_: "m0001".to_string(),
                token_count: 100,
                priority: MessagePriority::High,
            },
        );
        priorities.insert(
            "m2".to_string(),
            CompressionPriorityEntry {
                ref_: "m0002".to_string(),
                token_count: 100,
                priority: MessagePriority::Low,
            },
        );

        // Query for High priority
        let high_result =
            list_priority_refs_before_index(&messages, &priorities, 2, MessagePriority::High);
        assert_eq!(high_result, vec!["m0001"]);

        // Query for Low priority
        let low_result =
            list_priority_refs_before_index(&messages, &priorities, 2, MessagePriority::Low);
        assert_eq!(low_result, vec!["m0002"]);

        // Query for Medium priority (should be empty)
        let medium_result =
            list_priority_refs_before_index(&messages, &priorities, 2, MessagePriority::Medium);
        assert!(medium_result.is_empty());
    }

    #[test]
    fn list_priority_refs_empty_when_anchor_is_zero() {
        let messages = vec![make_message("m1", Role::User, "hello")];
        let mut priorities = CompressionPriorityMap::new();
        priorities.insert(
            "m1".to_string(),
            CompressionPriorityEntry {
                ref_: "m0001".to_string(),
                token_count: 100,
                priority: MessagePriority::Medium,
            },
        );

        let result =
            list_priority_refs_before_index(&messages, &priorities, 0, MessagePriority::Medium);
        assert!(result.is_empty());
    }

    #[test]
    fn list_priority_refs_all_before_anchor_included() {
        let messages = vec![
            make_message("m1", Role::User, "a"),
            make_message("m2", Role::User, "b"),
            make_message("m3", Role::User, "c"),
            make_message("m4", Role::User, "d"),
        ];
        let mut priorities = CompressionPriorityMap::new();
        for (i, msg_id) in ["m1", "m2", "m3", "m4"].iter().enumerate() {
            priorities.insert(
                msg_id.to_string(),
                CompressionPriorityEntry {
                    ref_: format!("m{:04}", i + 1),
                    token_count: 100,
                    priority: MessagePriority::High,
                },
            );
        }

        // anchor at 3 means take messages at indices 0,1,2 → m1, m2, m3
        let result =
            list_priority_refs_before_index(&messages, &priorities, 3, MessagePriority::High);
        assert_eq!(result, vec!["m0001", "m0002", "m0003"]);
    }
}
