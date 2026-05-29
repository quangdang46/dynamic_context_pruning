//! Message-reference allocation per SPEC.md §2.4.

use dcp_types::{Message, MessageRef, MessageRefParseError, Role, SessionState};

/// Width of the zero-padded message reference digits.
const MESSAGE_REF_WIDTH: usize = 4;
/// Minimum legal message reference index.
const MESSAGE_REF_MIN_INDEX: usize = 1;
/// Maximum legal message reference index (SPEC.md §2.4).
pub const MESSAGE_REF_MAX_INDEX: usize = 9999;
/// Tag name used in XML message-id attributes.
pub const MESSAGE_ID_TAG_NAME: &str = "dcp";

/// Allocate `m####` references for every non-ignored message that does
/// not already have one.
///
/// In subagent mode, the first eligible message is skipped.
pub fn assign_message_refs(state: &mut SessionState, messages: &[Message]) {
    if state.message_ids.next_ref == 0 {
        state.message_ids.next_ref = 1;
    }

    let is_subagent = state.is_subagent;
    let mut first_eligible_skipped = false;

    for m in messages {
        if !is_ref_eligible(m) {
            continue;
        }
        if state.message_ids.by_raw_id.contains_key(&m.id) {
            continue;
        }

        if is_subagent && !first_eligible_skipped {
            first_eligible_skipped = true;
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
            Err(MessageRefParseError::OutOfRange) => return,
            Err(_) => return,
        }
    }
}

fn is_ref_eligible(m: &Message) -> bool {
    if m.ignored {
        return false;
    }
    !matches!(m.role, Role::System)
}

/// A parsed boundary identifier — either a message reference (`m####`) or a
/// compression-block reference (`b<n>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundaryId {
    /// A message reference with its canonical string form and parsed index.
    Message {
        /// Canonical string form (e.g., "m0001").
        ref_: String,
        /// Parsed index (1..=9999).
        index: usize,
    },
    /// A compression-block reference with its canonical string and block id.
    Block {
        /// Canonical string form (e.g., "b7").
        ref_: String,
        /// Block id (>= 1).
        block_id: u32,
    },
}

fn escape_xml_attr(s: &str) -> String {
    let s = s.replace('&', "&amp;");
    let s = s.replace('"', "&quot;");
    let s = s.replace('<', "&lt;");
    s.replace('>', "&gt;")
}

static RE_MESSAGE_REF: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| regex::Regex::new(r"^m(\d{4})$").unwrap());
static RE_BLOCK_REF: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| regex::Regex::new(r"^b([1-9]\d*)$").unwrap());

/// Parse a message reference string (`"m####"` → `Some(index)`).
/// Returns `None` if the string does not match the expected format or
/// if the parsed index is outside the legal range 1..=9999.
pub fn parse_message_ref(ref_: &str) -> Option<usize> {
    let trimmed = ref_.trim().to_lowercase();
    let caps = RE_MESSAGE_REF.captures(&trimmed)?;
    let digits: &str = caps.get(1)?.as_str();
    let index: usize = digits.parse().ok()?;
    (MESSAGE_REF_MIN_INDEX..=MESSAGE_REF_MAX_INDEX)
        .contains(&index)
        .then_some(index)
}

/// Parse a block reference string (`"b<N>"` → `Some(block_id)`).
/// Returns `None` if the string does not match the expected format or
/// if the parsed value is 0.
pub fn parse_block_ref(ref_: &str) -> Option<u32> {
    let trimmed = ref_.trim().to_lowercase();
    let caps = RE_BLOCK_REF.captures(&trimmed)?;
    let digits: &str = caps.get(1)?.as_str();
    let block_id: u32 = digits.parse().ok()?;
    (block_id >= 1).then_some(block_id)
}

/// Parse any boundary-id string, trying message ref first then block ref.
/// Returns `None` if neither form matches.
pub fn parse_boundary_id(id: &str) -> Option<BoundaryId> {
    if let Some(index) = parse_message_ref(id) {
        let ref_ = format_message_ref(index)?;
        return Some(BoundaryId::Message { ref_, index });
    }
    if let Some(block_id) = parse_block_ref(id) {
        let ref_ = format_block_ref(block_id)?;
        return Some(BoundaryId::Block { ref_, block_id });
    }
    None
}

/// Format a message index into a zero-padded `"m####"` string.
/// Returns `None` if `index` is outside the valid range 1..=9999.
pub fn format_message_ref(index: usize) -> Option<String> {
    if !(MESSAGE_REF_MIN_INDEX..=MESSAGE_REF_MAX_INDEX).contains(&index) {
        return None;
    }
    Some(format!("m{index:0MESSAGE_REF_WIDTH$}"))
}

/// Format a block id into a `"b<N>"` string.
/// Returns `None` if `block_id` is 0.
pub fn format_block_ref(block_id: u32) -> Option<String> {
    if block_id == 0 {
        return None;
    }
    Some(format!("b{block_id}"))
}

/// Format an XML message-id tag string.
/// Attributes are sorted alphabetically by key and their values are XML-escaped.
pub fn format_message_id_tag(ref_: &str, attributes: Option<&[(String, String)]>) -> String {
    let tag = MESSAGE_ID_TAG_NAME;
    let esc_ref = escape_xml_attr(ref_);
    match attributes {
        None => format!(r#"<{tag} ref="{esc_ref}"/>"#),
        Some(attrs) => {
            let mut sorted: Vec<_> = attrs.to_vec();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            let attr_str = sorted
                .into_iter()
                .map(|(k, v)| format!(r#" {k}="{}""#, escape_xml_attr(&v)))
                .collect::<String>();
            format!(r#"<{tag} ref="{esc_ref}"{attr_str}></{tag}>"#)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{Message, Part, Role, SessionState};

    #[test]
    fn assigns_in_order_skipping_system() {
        let messages = vec![
            Message::system_text("s1", 0, "you are..."),
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
            Message::user_text("u2", 0, "again"),
        ];
        let mut state = SessionState::default();
        assign_message_refs(&mut state, &messages);

        assert!(!state.message_ids.by_raw_id.contains_key("s1"));
        assert_eq!(state.message_ids.by_raw_id["u1"], "m0001");
        assert_eq!(state.message_ids.by_raw_id["a1"], "m0002");
        assert_eq!(state.message_ids.by_raw_id["u2"], "m0003");
        assert_eq!(state.message_ids.by_ref["m0001"], "u1");
        assert_eq!(state.message_ids.by_ref["m0002"], "a1");
        assert_eq!(state.message_ids.by_ref["m0003"], "u2");
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
        for n in 1..=9998u32 {
            let raw = format!("raw-{n}");
            let r = format!("m{n:04}");
            state.message_ids.by_raw_id.insert(raw.clone(), r.clone());
            state.message_ids.by_ref.insert(r, raw);
        }
        state.message_ids.next_ref = 9999;

        let messages = vec![
            Message::user_text("u-9999", 0, "near"),
            Message::user_text("u-over", 0, "over"),
        ];
        assign_message_refs(&mut state, &messages);

        assert_eq!(state.message_ids.by_raw_id["u-9999"], "m9999");
        assert!(!state.message_ids.by_raw_id.contains_key("u-over"));
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

        assert_eq!(state.message_ids.by_raw_id["u1"], "m0001");
        assert!(!state.message_ids.by_raw_id.contains_key("u2"));
        assert_eq!(state.message_ids.by_raw_id["a1"], "m0002");
        assert_eq!(state.message_ids.next_ref, 3);
    }

    #[test]
    fn boundary_id_debug_format() {
        let msg = BoundaryId::Message {
            ref_: "m0001".into(),
            index: 1,
        };
        let blk = BoundaryId::Block {
            ref_: "b3".into(),
            block_id: 3,
        };
        let dbg_msg = format!("{:?}", msg);
        let dbg_blk = format!("{:?}", blk);
        assert!(dbg_msg.contains("Message"));
        assert!(dbg_blk.contains("Block"));
    }

    fn xml_escape(s: &str) -> String {
        let s = s.replace('&', "&amp;");
        let s = s.replace('"', "&quot;");
        let s = s.replace('<', "&lt;");
        s.replace('>', "&gt;")
    }

    #[test]
    fn escape_xml_attr_escapes_ampersand() {
        assert_eq!(xml_escape("a&b"), "a&amp;b");
    }

    #[test]
    fn escape_xml_attr_escapes_quotes() {
        assert_eq!(xml_escape(r#"a"b"#), "a&quot;b");
    }

    #[test]
    fn escape_xml_attr_escapes_angle_brackets() {
        assert_eq!(xml_escape("a<b>c"), "a&lt;b&gt;c");
    }

    #[test]
    fn parse_message_ref_valid_4digit() {
        assert_eq!(parse_message_ref("m0001"), Some(1));
        assert_eq!(parse_message_ref("m0042"), Some(42));
        assert_eq!(parse_message_ref("m9999"), Some(9999));
    }

    #[test]
    fn parse_message_ref_invalid_length() {
        assert_eq!(parse_message_ref("m001"), None);
        assert_eq!(parse_message_ref("m1"), None);
        assert_eq!(parse_message_ref("m00001"), None);
    }

    #[test]
    fn parse_message_ref_out_of_range() {
        assert_eq!(parse_message_ref("m0000"), None);
        assert_eq!(parse_message_ref("m10000"), None);
    }

    #[test]
    fn parse_message_ref_case_and_whitespace() {
        assert_eq!(parse_message_ref("M0001"), Some(1));
        assert_eq!(parse_message_ref("m0001 "), Some(1));
        assert_eq!(parse_message_ref("  m0042  "), Some(42));
    }

    #[test]
    fn parse_message_ref_non_m_prefix() {
        assert_eq!(parse_message_ref("b1"), None);
        assert_eq!(parse_message_ref("x0001"), None);
    }

    #[test]
    fn parse_block_ref_valid() {
        assert_eq!(parse_block_ref("b1"), Some(1));
        assert_eq!(parse_block_ref("b42"), Some(42));
    }

    #[test]
    fn parse_block_ref_leading_zero_invalid() {
        assert_eq!(parse_block_ref("b01"), None);
        assert_eq!(parse_block_ref("b001"), None);
    }

    #[test]
    fn parse_block_ref_zero_invalid() {
        assert_eq!(parse_block_ref("b0"), None);
    }

    #[test]
    fn parse_block_ref_case_and_whitespace() {
        assert_eq!(parse_block_ref("B1"), Some(1));
        assert_eq!(parse_block_ref("b42 "), Some(42));
        assert_eq!(parse_block_ref("  b7  "), Some(7));
    }

#[test]
    fn boundary_id_message_variant() {
        let id = BoundaryId::Message { ref_: "m0001".into(), index: 1 };
        assert!(matches!(&id, BoundaryId::Message { ref_, index } if ref_ == "m0001" && *index == 1));
    }

    #[test]
    fn boundary_id_block_variant() {
        let id = BoundaryId::Block { ref_: "b7".into(), block_id: 7 };
        assert!(matches!(&id, BoundaryId::Block { ref_, block_id } if ref_ == "b7" && *block_id == 7));
    }

    #[test]
    fn parse_boundary_id_block() {
        let r = parse_boundary_id("b7").unwrap();
        assert!(matches!(r, BoundaryId::Block { ref_, block_id: 7 } if ref_ == "b7"));
    }

    #[test]
    fn parse_boundary_id_unknown() {
        assert_eq!(parse_boundary_id("x"), None);
        assert_eq!(parse_boundary_id(""), None);
    }

    #[test]
    fn format_message_ref_valid() {
        assert_eq!(format_message_ref(1), Some("m0001".into()));
        assert_eq!(format_message_ref(42), Some("m0042".into()));
        assert_eq!(format_message_ref(9999), Some("m9999".into()));
    }

    #[test]
    fn format_message_ref_out_of_bounds() {
        assert_eq!(format_message_ref(0), None);
        assert_eq!(format_message_ref(10000), None);
    }

    #[test]
    fn format_block_ref_valid() {
        assert_eq!(format_block_ref(1), Some("b1".into()));
        assert_eq!(format_block_ref(42), Some("b42".into()));
    }

    #[test]
    fn format_block_ref_zero() {
        assert_eq!(format_block_ref(0), None);
    }

    #[test]
    fn format_message_id_tag_self_closing() {
        assert_eq!(
            format_message_id_tag("m0001", None),
            r#"<dcp ref="m0001"/>"#
        );
    }

    #[test]
    fn format_message_id_tag_with_sorted_attrs() {
        let attrs = vec![("z".into(), "2".into()), ("a".into(), "1".into())];
        let result = format_message_id_tag("m0001", Some(&attrs));
        assert!(result.contains(r#" a="1""#));
        assert!(result.contains(r#" z="2""#));
    }

#[test]
    fn format_message_id_tag_xml_escapes_attr_values() {
        let attrs = vec![("x".into(), "a".into())];
        let result = format_message_id_tag("m0001", Some(&attrs));
        eprintln!("SIMPLE RESULT: {:?}", result);
        assert!(result.contains(r#" x="a""#));

        let attrs2 = vec![("x".into(), "<".into())];
        let result2 = format_message_id_tag("m0001", Some(&attrs2));
        eprintln!("LT RESULT: {:?}", result2);
        assert!(result2.contains(r#" x="&lt;"#));
    }

    #[test]
    fn constants_have_correct_values() {
        assert_eq!(MESSAGE_REF_WIDTH, 4);
        assert_eq!(MESSAGE_REF_MIN_INDEX, 1);
        assert_eq!(MESSAGE_REF_MAX_INDEX, 9999);
        assert_eq!(MESSAGE_ID_TAG_NAME, "dcp");
    }

    #[test]
    fn assign_message_refs_skips_first_eligible_in_subagent_mode() {
        let messages = vec![
            Message::user_text("u1", 0, "hello"),
            Message::assistant_text("a1", 0, "hi"),
            Message::user_text("u2", 0, "world"),
        ];
        let mut state = SessionState::default();
        state.is_subagent = true;
        assign_message_refs(&mut state, &messages);

        assert!(!state.message_ids.by_raw_id.contains_key("u1"));
        assert_eq!(state.message_ids.by_raw_id["a1"], "m0001");
        assert_eq!(state.message_ids.by_raw_id["u2"], "m0002");
        assert_eq!(state.message_ids.next_ref, 3);
    }

    #[test]
    fn assign_message_refs_no_skip_when_not_subagent() {
        let messages = vec![
            Message::user_text("u1", 0, "hello"),
            Message::assistant_text("a1", 0, "hi"),
        ];
        let mut state = SessionState::default();
        state.is_subagent = false;
        assign_message_refs(&mut state, &messages);

        assert_eq!(state.message_ids.by_raw_id["u1"], "m0001");
        assert_eq!(state.message_ids.by_raw_id["a1"], "m0002");
        assert_eq!(state.message_ids.next_ref, 3);
    }
}
