//! Tool output format extensions for compress modes.

/// Output format for range-mode compress.
///
/// # Example
///
/// ```rust
/// use dcp_prompts::RANGE_FORMAT_EXTENSION;
/// assert!(RANGE_FORMAT_EXTENSION.contains("startId"));
/// assert!(RANGE_FORMAT_EXTENSION.contains("endId"));
/// assert!(RANGE_FORMAT_EXTENSION.contains("summary"));
/// assert!(RANGE_FORMAT_EXTENSION.contains("topic"));
/// ```
pub const RANGE_FORMAT_EXTENSION: &str = r#"Output format for range mode:
```json
{
  "topic": "string",
  "content": [
    {
      "startId": "m0001 or b3",
      "endId": "m0005 or b3",
      "summary": "concise summary"
    }
  ]
}
```"#;

/// Output format for message-mode compress.
///
/// # Example
///
/// ```rust
/// use dcp_prompts::MESSAGE_FORMAT_EXTENSION;
/// assert!(MESSAGE_FORMAT_EXTENSION.contains("messageId"));
/// assert!(MESSAGE_FORMAT_EXTENSION.contains("topic"));
/// assert!(MESSAGE_FORMAT_EXTENSION.contains("summary"));
/// ```
pub const MESSAGE_FORMAT_EXTENSION: &str = r#"Output format for message mode:
```json
{
  "topic": "string",
  "content": [
    {
      "messageId": "m0001",
      "topic": "string",
      "summary": "concise summary"
    }
  ]
}
```"#;

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_format_contains_required_fields() {
        assert!(RANGE_FORMAT_EXTENSION.contains("startId"));
        assert!(RANGE_FORMAT_EXTENSION.contains("endId"));
        assert!(RANGE_FORMAT_EXTENSION.contains("summary"));
        assert!(RANGE_FORMAT_EXTENSION.contains("topic"));
        assert!(RANGE_FORMAT_EXTENSION.contains("content"));
        assert!(RANGE_FORMAT_EXTENSION.contains("m0001 or b3"));
        assert!(RANGE_FORMAT_EXTENSION.contains("m0005 or b3"));
    }

    #[test]
    fn test_message_format_contains_required_fields() {
        assert!(MESSAGE_FORMAT_EXTENSION.contains("messageId"));
        assert!(MESSAGE_FORMAT_EXTENSION.contains("topic"));
        assert!(MESSAGE_FORMAT_EXTENSION.contains("summary"));
        assert!(MESSAGE_FORMAT_EXTENSION.contains("content"));
        assert!(MESSAGE_FORMAT_EXTENSION.contains("m0001"));
    }

    #[test]
    fn test_both_formats_are_different() {
        assert_ne!(RANGE_FORMAT_EXTENSION, MESSAGE_FORMAT_EXTENSION);
    }

    #[test]
    fn test_range_format_is_valid_json_schema() {
        // Check structure - has the outer topic and content array
        assert!(RANGE_FORMAT_EXTENSION.contains("\"topic\": \"string\""));
        assert!(RANGE_FORMAT_EXTENSION.contains("\"content\": ["));
    }

    #[test]
    fn test_message_format_is_valid_json_schema() {
        // Check structure - has the outer topic and content array
        assert!(MESSAGE_FORMAT_EXTENSION.contains("\"topic\": \"string\""));
        assert!(MESSAGE_FORMAT_EXTENSION.contains("\"content\": ["));
    }
}