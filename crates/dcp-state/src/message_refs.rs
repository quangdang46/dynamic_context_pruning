//! Message-reference allocation per SPEC.md §2.4.
//!
//! The library exposes `m####` references to the model. Each reference is
//! permanent for the lifetime of a session once allocated, so on rebuild
//! we replay the message list in order to reconstruct the same mapping
//! the original session would have produced.
//!
//! Allocation rules (SPEC.md §2.4):
//!
//! * `state.message_ids.next_ref` starts at `0` (uninitialised). The first
//!   allocation promotes it to `1`.
//! * Values above `9999` are illegal; once the allocator would emit
//!   `m10000` it must refuse further allocations
//!   (`MessageRefExhausted`). Per SPEC.md §11.7, that case is recorded in
//!   `stats.persisted_corruption`-like telemetry; the in-memory state
//!   simply stops minting refs.
//! * The library does not allocate refs for `system` messages — SPEC.md
//!   §2.3 states the library "never emits or rewrites system messages
//!   directly", so they are out of the model-facing reference space.

use dcp_types::{Message, MessageRef, MessageRefParseError, Role, SessionState};

/// Allocate `m####` references for every non-ignored message that does
/// not already have one.
///
/// Behavior:
///
/// 1. Idempotent — re-running over the same input does not re-allocate.
/// 2. Order-preserving — references are minted in the order messages
///    appear, matching the spec's "in order of first appearance" rule.
/// 3. Stops cleanly at `m9999`; later messages simply do not receive a
///    reference (caller may inspect [`SessionState::message_ids`] to
///    detect exhaustion).
/// 4. Skips `Role::System` messages (see module docs).
///
/// The function never returns an error — exhaustion is silent so
/// `transform_messages` does not abort mid-conversation.
///
/// # Example
///
/// ```rust
/// use dcp_state::message_refs::assign_message_refs;
/// use dcp_types::{Message, SessionState};
///
/// let messages = vec![
///     Message::user_text("u1", 0, "hi"),
///     Message::assistant_text("a1", 0, "hello"),
/// ];
/// let mut state = SessionState::default();
/// assign_message_refs(&mut state, &messages);
///
/// assert_eq!(state.message_ids.by_raw_id.get("u1").map(|s| s.as_str()), Some("m0001"));
/// assert_eq!(state.message_ids.by_raw_id.get("a1").map(|s| s.as_str()), Some("m0002"));
/// assert_eq!(state.message_ids.next_ref, 3);
/// ```
pub fn assign_message_refs(state: &mut SessionState, messages: &[Message]) {
    if state.message_ids.next_ref == 0 {
        state.message_ids.next_ref = 1;
    }

    for m in messages {
        if !is_ref_eligible(m) {
            continue;
        }
        if state.message_ids.by_raw_id.contains_key(&m.id) {
            continue;
        }
        match MessageRef::message(state.message_ids.next_ref) {
            Ok(reference) => {
                let raw = reference.raw().to_string();
                state
                    .message_ids
                    .by_raw_id
                    .insert(m.id.clone(), raw.clone());
                state.message_ids.by_ref.insert(raw, m.id.clone());
                state.message_ids.next_ref += 1;
            }
            // `MessageRef::message` only returns Zero (impossible — we just
            // promoted from 0) or OutOfRange. Treat the latter as exhaustion.
            Err(MessageRefParseError::OutOfRange) => return,
            Err(_) => return,
        }
    }
}

/// `true` when a message should receive a library reference.
fn is_ref_eligible(m: &Message) -> bool {
    // Skip messages the host has elided from the LLM's view.
    if m.ignored {
        return false;
    }
    !matches!(m.role, Role::System)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{Message, Part, Role, SessionState};

    #[test]
    fn assigns_in_order_skipping_system() {
        let messages = vec![
            Message::system_text("s1", 0, "you are…"),
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
            Message::user_text("u2", 0, "again"),
        ];
        let mut state = SessionState::default();
        assign_message_refs(&mut state, &messages);

        // System message has no reference.
        assert!(!state.message_ids.by_raw_id.contains_key("s1"));
        // Others are numbered in order.
        assert_eq!(state.message_ids.by_raw_id["u1"], "m0001");
        assert_eq!(state.message_ids.by_raw_id["a1"], "m0002");
        assert_eq!(state.message_ids.by_raw_id["u2"], "m0003");
        // by_ref is the inverse.
        assert_eq!(state.message_ids.by_ref["m0001"], "u1");
        assert_eq!(state.message_ids.by_ref["m0002"], "a1");
        assert_eq!(state.message_ids.by_ref["m0003"], "u2");
        // next_ref advances past the last allocation.
        assert_eq!(state.message_ids.next_ref, 4);
    }

    #[test]
    fn idempotent_on_repeat() {
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let mut state = SessionState::default();
        assign_message_refs(&mut state, &messages);
        let snapshot = state.message_ids.clone();

        assign_message_refs(&mut state, &messages);
        assert_eq!(state.message_ids, snapshot);
    }

    #[test]
    fn appends_to_existing_allocation() {
        let original = vec![Message::user_text("u1", 0, "hi")];
        let mut state = SessionState::default();
        assign_message_refs(&mut state, &original);
        assert_eq!(state.message_ids.next_ref, 2);

        let mut extended = original.clone();
        extended.push(Message::assistant_text("a1", 0, "hello"));
        assign_message_refs(&mut state, &extended);
        assert_eq!(state.message_ids.by_raw_id["a1"], "m0002");
        assert_eq!(state.message_ids.next_ref, 3);
    }

    #[test]
    fn promotes_zero_next_ref_to_one() {
        let messages = vec![Message::user_text("u1", 0, "hi")];
        let mut state = SessionState::default();
        assert_eq!(state.message_ids.next_ref, 0);
        assign_message_refs(&mut state, &messages);
        assert_eq!(state.message_ids.by_raw_id["u1"], "m0001");
    }

    #[test]
    fn stops_silently_at_m9999() {
        let mut state = SessionState::default();
        // Pre-position the allocator near exhaustion. We pre-fill 9998
        // entries with synthetic ids so the next allocation lands at m9999
        // and the one after would overflow.
        for n in 1..=9998u32 {
            let raw = format!("raw-{n}");
            let r = format!("m{n:04}");
            state.message_ids.by_raw_id.insert(raw.clone(), r.clone());
            state.message_ids.by_ref.insert(r, raw);
        }
        state.message_ids.next_ref = 9999;

        let messages = vec![
            Message::user_text("u-9999", 0, "near"),
            Message::user_text("u-overflow", 0, "over"),
        ];
        assign_message_refs(&mut state, &messages);

        assert_eq!(state.message_ids.by_raw_id["u-9999"], "m9999");
        assert!(!state.message_ids.by_raw_id.contains_key("u-overflow"));
        // After m9999 is allocated, next_ref points at 10000 — out of range.
        assert_eq!(state.message_ids.next_ref, 10_000);
    }

    #[test]
    fn ignores_role_system_eligible_check() {
        let s = Message::system_text("s1", 0, "x");
        let u = Message::user_text("u1", 0, "x");
        assert!(!is_ref_eligible(&s));
        assert!(is_ref_eligible(&u));
        assert_eq!(u.role, Role::User);
    }
    #[test]
    fn skipped_ignored_user_message() {
        let messages = vec![
            Message::user_text("u1", 0, "visible"),
            Message {
                id: "u2".into(),
                role: Role::User,
                parts: vec![Part::text("ignored")],
                time: 0,
                ignored: true,
            },
            Message::assistant_text("a1", 0, "ack"),
        ];
        let mut state = SessionState::default();
        assign_message_refs(&mut state, &messages);

        // u1 visible -> gets m0001
        assert_eq!(state.message_ids.by_raw_id["u1"], "m0001");
        // u2 ignored -> no ref
        assert!(!state.message_ids.by_raw_id.contains_key("u2"));
        // a1 visible -> gets m0002
        assert_eq!(state.message_ids.by_raw_id["a1"], "m0002");
        // next_ref = 3
        assert_eq!(state.message_ids.next_ref, 3);
    }
}

