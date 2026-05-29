//! Compress permission resolution and state synchronization.
//!
//! Port of `lib/compress-permission.ts`.

use dcp_config::Config;
use dcp_state::SessionState;
use dcp_types::{CompressPermission, Message};

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

/// Synchronize the compress permission into `state` based on the current
/// message stream and host permissions.
///
/// This function:
/// 1. Finds the last non-ignored user message in `messages`.
/// 2. Uses `active_agent` (provided by the host) as the agent name.
/// 3. Resolves the effective compress permission via host permissions.
/// 4. Stores the resolved permission in `state.compress_permission`.
///
/// The TS upstream reads `msg.info.agent` from the message object.
/// Since the Rust `Message` type (dcp-types) does not carry an `agent` field,
/// the host must pass the active agent name explicitly.
#[allow(clippy::cognitive_complexity)]
pub fn sync_compress_permission_state(
    state: &mut SessionState,
    config: &Config,
    host_permissions: &crate::host_permissions::HostPermissionSnapshot,
    _messages: &[Message],
    active_agent: Option<&str>,
) {
    // Resolve effective permission
    let base = compress_permission(state, config);
    let effective = crate::host_permissions::resolve_effective_permission(
        convert_permission(base),
        host_permissions,
        active_agent,
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

    // ── sync_compress_permission_state ───────────────────────────────────────

    #[test]
    fn test_sync_with_agent_name() {
        let mut state = make_state(CompressPermission::Ask);
        let config = make_config(dcp_config::Permission::Allow);
        let host = HostPermissionSnapshot::default();

        let messages = vec![Message::user_text("u1", 0, "hello")];

        sync_compress_permission_state(&mut state, &config, &host, &messages, Some("test-agent"));

        // Agent-specific resolution would happen here
        // State should have been updated
        assert_eq!(state.compress_permission, CompressPermission::Allow);
    }

    #[test]
    fn test_sync_stores_resolved_permission_in_state() {
        let mut state = make_state(CompressPermission::Ask);
        let config = make_config(dcp_config::Permission::Allow);
        let host = HostPermissionSnapshot::default();

        let messages = vec![Message::user_text("u1", 0, "hello")];

        sync_compress_permission_state(&mut state, &config, &host, &messages, None);

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

        sync_compress_permission_state(&mut state, &config, &host, &messages, None);

        // Host global deny should override config allow
        assert_eq!(state.compress_permission, CompressPermission::Deny);
    }

    #[test]
    fn test_sync_no_agent_uses_config_default() {
        let mut state = make_state(CompressPermission::Ask);
        let config = make_config(dcp_config::Permission::Allow);
        let host = HostPermissionSnapshot::default();

        let messages = vec![Message::assistant_text("a1", 0, "response")];

        sync_compress_permission_state(&mut state, &config, &host, &messages, None);

        assert_eq!(state.compress_permission, CompressPermission::Allow);
    }

    #[test]
    fn test_sync_base_deny_always_wins() {
        let mut state = make_state(CompressPermission::Deny);
        let config = make_config(dcp_config::Permission::Allow);
        let host = HostPermissionSnapshot::default();

        let messages = vec![Message::user_text("u1", 0, "hello")];

        sync_compress_permission_state(&mut state, &config, &host, &messages, None);

        // Base deny always wins regardless of host permissions
        assert_eq!(state.compress_permission, CompressPermission::Deny);
    }
}
