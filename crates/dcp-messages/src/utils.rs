//! Utility functions — port of lib/messages/utils.ts.
//!
//! Provides: create_synthetic_message, strip_dcp_tags, extract_anchor_refs.

use regex::Regex;

use dcp_types::{Message, Part, Role};

/// DCP tag name used in XML-style tags.
pub const DCP_TAG_NAME: &str = "dcp";

/// Create a synthetic message with the given id, text content, and role.
pub fn create_synthetic_message(id: &str, text: &str, role: Role) -> Message {
    Message::new(id.to_owned(), role, vec![Part::text(text)], 0)
}

/// Strip all `<dcp>` tags from the text, extracting the inner content.
pub fn strip_dcp_tags(text: &str) -> String {
    let re = Regex::new(r"<dcp[^>]*>(.*?)</dcp>").expect("invalid DCP tag regex");
    let mut result = String::new();
    let mut last_end = 0;
    for cap in re.captures_iter(text) {
        let m = cap.get(0).unwrap();
        result.push_str(&text[last_end..m.start()]);
        result.push_str(&cap[1]);
        last_end = m.end();
    }
    result.push_str(&text[last_end..]);
    result
}

/// Extract all anchor references from `<dcp>` tags in the text.
/// Handles two forms:
/// - `<dcp ref="m####">content</dcp>` — extracts the `ref` attribute value
/// - `<dcp>m####</dcp>` — extracts the inner text content if it looks like a ref
pub fn extract_anchor_refs(text: &str) -> Vec<String> {
    let mut refs = Vec::new();

    // Pattern for <dcp ref="...">content</dcp>
    let ref_attr_re = Regex::new(r#"<dcp\s+ref="([^"]+)"[^>]*>"#).expect("invalid ref attr regex");

    // Pattern for <dcp>content</dcp> where content might be a ref
    let content_re = Regex::new(r"<dcp>([^<]+)</dcp>").expect("invalid content regex");

    // Extract ref attributes
    for cap in ref_attr_re.captures_iter(text) {
        if let Some(ref_match) = cap.get(1) {
            refs.push(ref_match.as_str().to_string());
        }
    }

    // Extract content refs (only if they look like message references)
    for cap in content_re.captures_iter(text) {
        if let Some(content_match) = cap.get(1) {
            let content = content_match.as_str().trim();
            // Only include if it looks like a message reference (m####)
            if content.starts_with('m') && content.len() >= 2 {
                refs.push(content.to_string());
            }
        }
    }

    refs
}

#[cfg(test)]
mod tests {
    use dcp_types::{Part, Role};

    use super::{DCP_TAG_NAME, create_synthetic_message, extract_anchor_refs, strip_dcp_tags};

    #[test]
    fn test_dcp_tag_name_constant() {
        assert_eq!(DCP_TAG_NAME, "dcp");
    }

    #[test]
    fn test_create_synthetic_message_user_role() {
        let msg = create_synthetic_message("synth1", "hello world", Role::User);
        assert_eq!(msg.id, "synth1");
        assert_eq!(msg.role, Role::User);
        assert!(!msg.ignored);
        assert_eq!(msg.time, 0);
        assert_eq!(msg.parts.len(), 1);
        assert!(matches!(&msg.parts[0], Part::Text(t) if t == "hello world"));
    }

    #[test]
    fn test_create_synthetic_message_assistant_role() {
        let msg = create_synthetic_message("synth2", "I am an assistant", Role::Assistant);
        assert_eq!(msg.id, "synth2");
        assert_eq!(msg.role, Role::Assistant);
        assert!(!msg.ignored);
        assert_eq!(msg.time, 0);
        assert_eq!(msg.parts.len(), 1);
        assert!(matches!(&msg.parts[0], Part::Text(t) if t == "I am an assistant"));
    }

    #[test]
    fn test_strip_dcp_tags_empty() {
        let result = strip_dcp_tags("no tags here");
        assert_eq!(result, "no tags here");
    }

    #[test]
    fn test_strip_dcp_tags_simple() {
        let result = strip_dcp_tags("hello <dcp>world</dcp>!");
        assert_eq!(result, "hello world!");
    }

    #[test]
    fn test_strip_dcp_tags_with_attributes() {
        let result = strip_dcp_tags("text <dcp ref=\"m0001\">inner</dcp> more");
        assert_eq!(result, "text inner more");
    }

    #[test]
    fn test_strip_dcp_tags_multiple() {
        let result = strip_dcp_tags("<dcp>one</dcp> and <dcp>two</dcp> and <dcp>three</dcp>");
        assert_eq!(result, "one and two and three");
    }

    #[test]
    fn test_strip_dcp_tags_non_greedy() {
        let result = strip_dcp_tags("<dcp>first</dcp><dcp>second</dcp>");
        assert_eq!(result, "firstsecond");
    }

    #[test]
    fn test_strip_dcp_tags_no_content() {
        let result = strip_dcp_tags("before <dcp></dcp> after");
        assert_eq!(result, "before  after");
    }

    #[test]
    fn test_strip_dcp_tags_self_closing_style() {
        // Self-closing style with attributes
        let result = strip_dcp_tags("text <dcp ref=\"m0001\">content</dcp> end");
        assert_eq!(result, "text content end");
    }

    #[test]
    fn test_extract_anchor_refs_empty() {
        let result = extract_anchor_refs("no refs here");
        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_anchor_refs_ref_attribute() {
        let result = extract_anchor_refs("see <dcp ref=\"m0001\">m0001</dcp> for details");
        assert_eq!(result, vec!["m0001"]);
    }

    #[test]
    fn test_extract_anchor_refs_ref_attribute_no_closing() {
        let result = extract_anchor_refs("<dcp ref=\"m0042\">");
        assert_eq!(result, vec!["m0042"]);
    }

    #[test]
    fn test_extract_anchor_refs_content_only() {
        // When content is the ref (no ref attribute)
        let result = extract_anchor_refs("<dcp>m0003</dcp>");
        assert_eq!(result, vec!["m0003"]);
    }

    #[test]
    fn test_extract_anchor_refs_multiple() {
        let result =
            extract_anchor_refs("<dcp ref=\"m0001\">a</dcp> and <dcp ref=\"m0042\">b</dcp>");
        assert_eq!(result, vec!["m0001", "m0042"]);
    }

    #[test]
    fn test_extract_anchor_refs_mixed() {
        let result = extract_anchor_refs("<dcp ref=\"m0001\">content</dcp> plain <dcp>m0005</dcp>");
        assert_eq!(result, vec!["m0001", "m0005"]);
    }

    #[test]
    fn test_extract_anchor_refs_content_is_inner_text() {
        // When content inside tag is a ref
        let result = extract_anchor_refs("<dcp>m0007</dcp> is the ref");
        assert_eq!(result, vec!["m0007"]);
    }

    #[test]
    fn test_extract_anchor_refs_duplicates() {
        let result =
            extract_anchor_refs("<dcp ref=\"m0001\">a</dcp> then <dcp ref=\"m0001\">b</dcp>");
        assert_eq!(result, vec!["m0001", "m0001"]);
    }
}
