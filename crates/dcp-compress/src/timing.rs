//! Compression timing — port of lib/compress/timing.ts.
//!
//! Tracks wall-clock duration of compression operations
//! and attaches them to CompressionBlock entries.

/// Resolve compression duration from timing fields.
/// Returns the elapsed time in milliseconds.
///
/// # Arguments
/// * `started_at` - When the compression block was created (event_time in some contexts)
/// * `event_time` - When the compression was triggered (current time on commit)
/// * `part_time` - Partition time (0 if not yet partitioned)
pub fn resolve_compression_duration(started_at: i64, event_time: i64, part_time: i64) -> i64 {
    if started_at == 0 {
        return 0;
    }

    // If part_time is non-zero and different from event_time, we have partition info
    if part_time > 0 && part_time != event_time {
        // pending_to_running_ms: time from started_at to part_time
        return part_time.saturating_sub(started_at);
    }

    // Otherwise use event_time - started_at as runtime_ms
    event_time.saturating_sub(started_at)
}

/// Build a compression timing key for deduplication tracking.
/// Format: "messageId:callId" or "messageId:" if no callId.
pub fn build_compression_timing_key(message_id: &str, call_id: Option<&str>) -> String {
    match call_id {
        Some(cid) => format!("{}:{}", message_id, cid),
        None => format!("{}:", message_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_pending_to_running() {
        // started_at=1000, part_time=1050, event_time=1100 → 50ms
        let result = resolve_compression_duration(1000, 1100, 1050);
        assert_eq!(result, 50);
    }

    #[test]
    fn test_resolve_runtime() {
        // started_at=1000, event_time=1100, part_time=0 → 100ms
        let result = resolve_compression_duration(1000, 1100, 0);
        assert_eq!(result, 100);
    }

    #[test]
    fn test_resolve_zero_started_at() {
        let result = resolve_compression_duration(0, 1100, 1050);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_build_key_with_call_id() {
        let key = build_compression_timing_key("msg123", Some("call456"));
        assert_eq!(key, "msg123:call456");
    }

    #[test]
    fn test_build_key_without_call_id() {
        let key = build_compression_timing_key("msg123", None);
        assert_eq!(key, "msg123:");
    }
}
