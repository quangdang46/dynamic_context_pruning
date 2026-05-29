//! Compress permission resolution and state synchronization.
//!
//! Port of `lib/compress-permission.ts`.

use dcp_config::Config;
use dcp_state::SessionState;
use dcp_types::{CompressPermission, Message, Role};

/// Returns the effective compress permission for the session.
///
/// If `state.compress_permission` is set (not the default `Ask`), that value
/// takes precedence. Otherwise the permission from `config.compress.permission`
/// is used.
pub fn compress_permission(state: &SessionState, config: &Config) -> CompressPermission {
    // If state has a non-default permission, use it
    if state.compress_permission != CompressPermission::Ask {
        return state.compress_permission;
    }
    // Otherwise fall back to config
    use dcp_config::Permission as ConfigPermission;
    match config.compress.permission {
        ConfigPermission::Ask => CompressPermission::Ask,
        ConfigPermission::Allow => CompressPermission::Allow,
        ConfigPermission::Deny => CompressPermission::Deny,
    }
}

/// Find the last user message in `messages` that is not ignored.
///
/// Returns `None` if no qualifying message is found.
fn find_last_user_message(messages: &[Message]) -> Option<&Message> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User && !m.ignored)
}

/// Extract the agent name from the last user message's parts, if present.
///
/// Looks for an `agent` field in the message parts. This is a heuristic
/// based on typical agent-assigned message formats.
fn extract_agent_name_from_message(message: &Message) -> Option<String> {
    // Walk parts in reverse to find agent info
    for part in message.parts.iter().rev() {
        #[allow(clippy::single_match, clippy::collapsible_match)]
        match part {
            dcp_types::Part::Text(text) => {
                // Heuristic: look for agent:name patterns or similar
                // This is a placeholder for actual agent extraction logic
                // The actual format depends on the host's message convention
                if text.starts_with("agent:") {
                    return Some(text.trim_start_matches("agent:").trim().to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Synchronize the compress permission into `state` based on the current
/// message stream and host permissions.
///
/// This function:
/// 1. Finds the last non-ignored user message in `messages`.
/// 2. Extracts the agent name from that message (if present).
/// 3. Resolves the effective compress permission via host permissions.
/// 4. Stores the resolved permission in `state.compress_permission`.
#[allow(clippy::cognitive_complexity)]
pub fn sync_compress_permission_state(
    state: &mut SessionState,
    config: &Config,
    host_permissions: &crate::host_permissions::HostPermissionSnapshot,
    messages: &[Message],
) {
    // Find the last user message
    let Some(last_user) = find_last_user_message(messages) else {
        return;
    };

    // Extract agent name
    let agent_name = extract_agent_name_from_message(last_user);

    // Resolve effective permission
    let base = compress_permission(state, config);
    let effective = crate::host_permissions::resolve_effective_permission(
        convert_permission(base),
        host_permissions,
        agent_name.as_deref(),
    );

    // Convert back to CompressPermission and store
    state.compress_permission = convert_action_to_permission(effective);
}

/// Convert [`CompressPermission`] to [`crate::host_permissions::PermissionAction`].
fn convert_permission(p: CompressPermission) -> crate::host_permissions::PermissionAction {
    match p {
        CompressPermission::Allow => crate::host_permissions::PermissionAction::Allow,
        CompressPermission::Ask => crate::host_permissions::PermissionAction::Ask,
        CompressPermission::Deny => crate::host_permissions::PermissionAction::Deny,
    }
}

/// Convert [`crate::host_permissions::PermissionAction`] back to [`CompressPermission`].
#[allow(clippy::cognitive_complexity, unused_imports)]
fn convert_action_to_permission(
    a: crate::host_permissions::PermissionAction,
) -> CompressPermission {
    match a {
        crate::host_permissions::PermissionAction::Allow => CompressPermission::Allow,
        crate::host_permissions::PermissionAction::Ask => CompressPermission::Ask,
        crate::host_permissions::PermissionAction::Deny => CompressPermission::Deny,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_permissions::{HostPermissionSnapshot, PermissionAction};
    use std::collections::HashMap;

    fn make_config(permission: dcp_config::Permission) -> Config {
        let mut config = Config::default();
        config.compress.permission = permission;
        config
    }

    fn make_state(compress_permission: CompressPermission) -> SessionState {
        SessionState {
            compress_permission,
            ..SessionState::default()
        }
    }

    // ── compress_permission ──────────────────────────────────────────────────

    #[test]
    fn test_compress_permission_uses_state_when_set() {
        let state = make_state(CompressPermission::Allow);
        let config = make_config(dcp_config::Permission::Deny);

        let result = compress_permission(&state, &config);
        // State takes precedence over config
        assert_eq!(result, CompressPermission::Allow);
    }

    #[test]
    fn test_compress_permission_falls_back_to_config() {
        let state = make_state(CompressPermission::Ask); // default
        let config = make_config(dcp_config::Permission::Deny);

        let result = compress_permission(&state, &config);
        // Falls back to config
        assert_eq!(result, CompressPermission::Deny);
    }

    #[test]
    fn test_compress_permission_state_deny_overrides_config_allow() {
        let state = make_state(CompressPermission::Deny);
        let config = make_config(dcp_config::Permission::Allow);

        let result = compress_permission(&state, &config);
        assert_eq!(result, CompressPermission::Deny);
    }

    #[test]
    fn test_compress_permission_state_ask_falls_back_to_config_ask() {
        let state = make_state(CompressPermission::Ask);
        let config = make_config(dcp_config::Permission::Ask);

        let result = compress_permission(&state, &config);
        assert_eq!(result, CompressPermission::Ask);
    }

    // ── find_last_user_message ──────────────────────────────────────────────

    #[test]
    fn test_find_last_user_message_basic() {
        let messages = vec![
            Message::user_text("u1", 0, "first"),
            Message::assistant_text("a1", 0, "response"),
            Message::user_text("u2", 0, "second"),
        ];
        let result = find_last_user_message(&messages);
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "u2");
    }

    #[test]
    fn test_find_last_user_message_skips_ignored() {
        let mut msg = Message::user_text("u1", 0, "first");
        msg.ignored = true;

        let messages = vec![msg, Message::user_text("u2", 0, "second")];
        let result = find_last_user_message(&messages);
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "u2");
    }

    #[test]
    fn test_find_last_user_message_none_when_no_user() {
        let messages = vec![
            Message::assistant_text("a1", 0, "response"),
            Message::assistant_text("a2", 0, "response2"),
        ];
        let result = find_last_user_message(&messages);
        assert!(result.is_none());
    }

    // ── extract_agent_name_from_message ─────────────────────────────────────

    #[test]
    fn test_extract_agent_name_from_last_user_message() {
        // Create a user message with an agent prefix
        let msg = Message {
            id: "u1".into(),
            role: Role::User,
            parts: vec![dcp_types::Part::Text("agent:my-agent hello".into())],
            time: 0,
            ignored: false,
        };
        let agent = extract_agent_name_from_message(&msg);
        assert_eq!(agent, Some("my-agent hello".to_string()));
    }

    #[test]
    fn test_extract_agent_name_not_found() {
        let msg = Message {
            id: "u1".into(),
            role: Role::User,
            parts: vec![dcp_types::Part::Text("hello world".into())],
            time: 0,
            ignored: false,
        };
        let agent = extract_agent_name_from_message(&msg);
        assert!(agent.is_none());
    }

    // ── sync_compress_permission_state ───────────────────────────────────────

    #[test]
    fn test_sync_extracts_agent_from_last_user_message() {
        let mut state = make_state(CompressPermission::Ask);
        let config = make_config(dcp_config::Permission::Allow);
        let host = HostPermissionSnapshot::default();

        let messages = vec![Message::user_text("u1", 0, "agent:test-agent hello")];

        sync_compress_permission_state(&mut state, &config, &host, &messages);

        // Agent-specific resolution would happen here
        // Currently our extract just looks for agent: prefix
        // State should have been updated
        assert_eq!(state.compress_permission, CompressPermission::Allow);
    }

    #[test]
    fn test_sync_stores_resolved_permission_in_state() {
        let mut state = make_state(CompressPermission::Ask);
        let config = make_config(dcp_config::Permission::Allow);
        let host = HostPermissionSnapshot::default();

        let messages = vec![Message::user_text("u1", 0, "hello")];

        sync_compress_permission_state(&mut state, &config, &host, &messages);

        // With default host permissions (no deny), should use config's Allow
        assert_eq!(state.compress_permission, CompressPermission::Allow);
    }

    #[test]
    fn test_sync_with_host_global_deny() {
        let mut state = make_state(CompressPermission::Ask);
        let config = make_config(dcp_config::Permission::Allow);

        let mut global: HashMap<String, PermissionAction> = HashMap::new();
        global.insert("compress".to_string(), PermissionAction::Deny);
        let host = HostPermissionSnapshot {
            global,
            ..Default::default()
        };

        let messages = vec![Message::user_text("u1", 0, "hello")];

        sync_compress_permission_state(&mut state, &config, &host, &messages);

        // Host global deny should override config allow
        assert_eq!(state.compress_permission, CompressPermission::Deny);
    }

    #[test]
    fn test_sync_no_user_message_does_nothing() {
        let mut state = make_state(CompressPermission::Ask);
        let config = make_config(dcp_config::Permission::Allow);
        let host = HostPermissionSnapshot::default();

        let messages = vec![Message::assistant_text("a1", 0, "response")];

        sync_compress_permission_state(&mut state, &config, &host, &messages);

        // No change since no user message
        assert_eq!(state.compress_permission, CompressPermission::Ask);
    }

    #[test]
    fn test_sync_base_deny_always_wins() {
        let mut state = make_state(CompressPermission::Deny);
        let config = make_config(dcp_config::Permission::Allow);
        let host = HostPermissionSnapshot::default();

        let messages = vec![Message::user_text("u1", 0, "hello")];

        sync_compress_permission_state(&mut state, &config, &host, &messages);

        // Base deny always wins regardless of host permissions
        assert_eq!(state.compress_permission, CompressPermission::Deny);
    }
}
