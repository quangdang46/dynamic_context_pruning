//! Message validation and filtering — port of lib/messages/shape.ts.
//!
//! Provides: is_valid_message, filter_messages, filter_messages_in_place.

use dcp_types::{Message, Role};

/// Validates that a [`Message`] is well-formed for use in the library.
///
/// A valid message must have:
/// - A non-empty `id` string
/// - A `role` of [`Role::User`] or [`Role::Assistant`] (not [`Role::System`])
/// - At least one part in `parts`
///
/// # Example
///
/// ```
/// use dcp_types::{Message, Role, Part};
/// use dcp_messages::shape::is_valid_message;
///
/// let valid = Message::new("m1", Role::User, vec![Part::text("hello")], 0);
/// assert!(is_valid_message(&valid));
///
/// let system_msg = Message::new("m2", Role::System, vec![Part::text("system")], 0);
/// assert!(!is_valid_message(&system_msg));
///
/// let empty_parts = Message::new("m3", Role::User, vec![], 0);
/// assert!(!is_valid_message(&empty_parts));
/// ```
pub fn is_valid_message(message: &Message) -> bool {
    // id must be non-empty
    if message.id.is_empty() {
        return false;
    }
    // role must be User or Assistant, not System
    match message.role {
        Role::User | Role::Assistant => {}
        Role::System => return false,
        _ => return false,
    }
    // parts must be non-empty
    if message.parts.is_empty() {
        return false;
    }
    true
}

/// Returns a vector of references to only the valid messages in the input slice.
///
/// # Example
///
/// ```
/// use dcp_types::{Message, Role, Part};
/// use dcp_messages::shape::filter_messages;
///
/// let msgs = vec![
///     Message::new("m1", Role::User, vec![Part::text("hello")], 0),
///     Message::new("m2", Role::System, vec![Part::text("system")], 0), // invalid
///     Message::new("m3", Role::Assistant, vec![Part::text("ack")], 0),
///     Message::new("", Role::User, vec![Part::text("bad")], 0), // invalid - empty id
/// ];
///
/// let valid = filter_messages(&msgs);
/// assert_eq!(valid.len(), 2);
/// assert_eq!(valid[0].id, "m1");
/// assert_eq!(valid[1].id, "m3");
/// ```
pub fn filter_messages(messages: &[Message]) -> Vec<&Message> {
    messages.iter().filter(|m| is_valid_message(m)).collect()
}

/// Retains only the valid messages in-place in the given vector.
///
/// This modifies the vector directly using `retain`.
///
/// # Example
///
/// ```
/// use dcp_types::{Message, Role, Part};
/// use dcp_messages::shape::filter_messages_in_place;
///
/// let mut msgs = vec![
///     Message::new("m1", Role::User, vec![Part::text("hello")], 0),
///     Message::new("m2", Role::System, vec![Part::text("system")], 0), // invalid
///     Message::new("m3", Role::Assistant, vec![Part::text("ack")], 0),
///     Message::new("", Role::User, vec![Part::text("bad")], 0), // invalid - empty id
/// ];
///
/// filter_messages_in_place(&mut msgs);
/// assert_eq!(msgs.len(), 2);
/// assert_eq!(msgs[0].id, "m1");
/// assert_eq!(msgs[1].id, "m3");
/// ```
pub fn filter_messages_in_place(messages: &mut Vec<Message>) {
    messages.retain(is_valid_message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{Part, Role};

    // =========================================================================
    // is_valid_message tests
    // =========================================================================

    #[test]
    fn is_valid_message_valid_user_message() {
        let msg = Message::user_text("u1", 0, "hello");
        assert!(is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_valid_assistant_message() {
        let msg = Message::assistant_text("a1", 0, "hi there");
        assert!(is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_valid_multiple_parts() {
        let msg = Message::new(
            "m1",
            Role::User,
            vec![Part::text("hello"), Part::text("world")],
            0,
        );
        assert!(is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_valid_with_tool_call() {
        use serde_json::json;
        let msg = Message::new(
            "tc1",
            Role::Assistant,
            vec![Part::tool_call("c1", "read_file", json!({"path": "x"}))],
            0,
        );
        assert!(is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_rejects_system_role() {
        let msg = Message::system_text("s1", 0, "system prompt");
        assert!(!is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_rejects_empty_id() {
        let msg = Message::new("", Role::User, vec![Part::text("hello")], 0);
        assert!(!is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_rejects_empty_parts() {
        let msg = Message::new("m1", Role::User, vec![], 0);
        assert!(!is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_rejects_empty_id_with_system_role() {
        // Both invalid, should return false
        let msg = Message::new("", Role::System, vec![Part::text("x")], 0);
        assert!(!is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_rejects_empty_id_with_empty_parts() {
        let msg = Message::new("", Role::Assistant, vec![], 0);
        assert!(!is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_rejects_empty_id_and_system_role_and_empty_parts() {
        // All three invalid
        let msg = Message::new("", Role::System, vec![], 0);
        assert!(!is_valid_message(&msg));
    }

    // =========================================================================
    // filter_messages tests
    // =========================================================================

    #[test]
    fn filter_messages_all_valid() {
        let msgs = vec![
            Message::user_text("u1", 0, "hello"),
            Message::assistant_text("a1", 0, "hi"),
        ];
        let valid = filter_messages(&msgs);
        assert_eq!(valid.len(), 2);
    }

    #[test]
    fn filter_messages_all_invalid() {
        let msgs = vec![
            Message::system_text("s1", 0, "system"),
            Message::new("", Role::User, vec![Part::text("x")], 0),
            Message::new("m2", Role::Assistant, vec![], 0),
        ];
        let valid = filter_messages(&msgs);
        assert!(valid.is_empty());
    }

    #[test]
    fn filter_messages_mixed() {
        let msgs = vec![
            Message::user_text("u1", 0, "hello"),                   // valid
            Message::system_text("s1", 0, "system"),                // invalid
            Message::assistant_text("a1", 0, "hi"),                 // valid
            Message::new("", Role::User, vec![Part::text("x")], 0), // invalid
            Message::new(
                "m4",
                Role::Assistant,
                vec![Part::tool_call("c1", "tool", serde_json::json!({}))],
                0,
            ), // valid
        ];
        let valid = filter_messages(&msgs);
        assert_eq!(valid.len(), 3);
        assert_eq!(valid[0].id, "u1");
        assert_eq!(valid[1].id, "a1");
        assert_eq!(valid[2].id, "m4");
    }

    #[test]
    fn filter_messages_empty_input() {
        let msgs: Vec<Message> = vec![];
        let valid = filter_messages(&msgs);
        assert!(valid.is_empty());
    }

    #[test]
    fn filter_messages_returns_references() {
        // Ensure we return references, not copies
        let msgs = vec![Message::user_text("u1", 0, "hello")];
        let valid = filter_messages(&msgs);
        assert!(matches!(valid[0], &Message { .. }));
        // The reference should point to the same message
        assert_eq!(valid[0].id, "u1");
        assert_eq!(valid[0].parts[0], Part::Text("hello".to_string()));
    }

    // =========================================================================
    // filter_messages_in_place tests
    // =========================================================================

    #[test]
    fn filter_messages_in_place_all_valid() {
        let mut msgs = vec![
            Message::user_text("u1", 0, "hello"),
            Message::assistant_text("a1", 0, "hi"),
        ];
        filter_messages_in_place(&mut msgs);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, "u1");
        assert_eq!(msgs[1].id, "a1");
    }

    #[test]
    fn filter_messages_in_place_all_invalid() {
        let mut msgs = vec![
            Message::system_text("s1", 0, "system"),
            Message::new("", Role::User, vec![Part::text("x")], 0),
            Message::new("m2", Role::Assistant, vec![], 0),
        ];
        filter_messages_in_place(&mut msgs);
        assert!(msgs.is_empty());
    }

    #[test]
    fn filter_messages_in_place_mixed() {
        let mut msgs = vec![
            Message::user_text("u1", 0, "hello"),
            Message::system_text("s1", 0, "system"),
            Message::assistant_text("a1", 0, "hi"),
            Message::new("bad", Role::User, vec![], 0), // invalid - empty parts
        ];
        filter_messages_in_place(&mut msgs);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, "u1");
        assert_eq!(msgs[1].id, "a1");
    }

    #[test]
    fn filter_messages_in_place_empty_input() {
        let mut msgs: Vec<Message> = vec![];
        filter_messages_in_place(&mut msgs);
        assert!(msgs.is_empty());
    }

    #[test]
    fn filter_messages_in_place_preserves_order() {
        // Valid messages should remain in their original relative order
        let mut msgs = vec![
            Message::new("first", Role::User, vec![Part::text("1")], 0),
            Message::system_text("invalid", 0, "x"),
            Message::new("second", Role::User, vec![Part::text("2")], 0),
            Message::new("", Role::Assistant, vec![Part::text("x")], 0), // invalid
            Message::new("third", Role::Assistant, vec![Part::text("3")], 0),
        ];
        filter_messages_in_place(&mut msgs);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].id, "first");
        assert_eq!(msgs[1].id, "second");
        assert_eq!(msgs[2].id, "third");
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn is_valid_message_whitespace_id() {
        // Whitespace-only id is non-empty but still valid from id perspective
        // (the validation only checks non-empty, not whitespace-only)
        let msg = Message::new("   ", Role::User, vec![Part::text("hello")], 0);
        // id is non-empty (has spaces), so this passes our check
        assert!(is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_single_empty_string_parts() {
        // A message with a single empty text part is still valid (has parts)
        let msg = Message::new("m1", Role::User, vec![Part::text("")], 0);
        assert!(is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_reasoning_only() {
        // A message with only reasoning part
        let msg = Message::new(
            "r1",
            Role::Assistant,
            vec![Part::reasoning("thinking...")],
            0,
        );
        assert!(is_valid_message(&msg));
    }

    #[test]
    fn is_valid_message_image_part() {
        let msg = Message::new(
            "img1",
            Role::User,
            vec![Part::image("image/png", "AAAA")],
            0,
        );
        assert!(is_valid_message(&msg));
    }

    #[test]
    fn filter_messages_with_reasoning_and_text() {
        let msgs = vec![
            Message::new("r1", Role::Assistant, vec![Part::reasoning("think")], 0),
            Message::new("t1", Role::User, vec![Part::text("hello")], 0),
            Message::system_text("s1", 0, "system"),
        ];
        let valid = filter_messages(&msgs);
        assert_eq!(valid.len(), 2);
    }

    #[test]
    fn filter_messages_in_place_with_reasoning_and_text() {
        let mut msgs = vec![
            Message::new("r1", Role::Assistant, vec![Part::reasoning("think")], 0),
            Message::new("t1", Role::User, vec![Part::text("hello")], 0),
            Message::system_text("s1", 0, "system"),
        ];
        filter_messages_in_place(&mut msgs);
        assert_eq!(msgs.len(), 2);
    }
}
