//! Integration tests for the basic `transform_messages` flow
//! (PLAN.md §11.1 / SPEC.md §11).

use std::sync::Arc;

use dcp_core::{
    Config, ContextPruner, InMemoryStateStore, Message, Part, Role, StatePersistence,
};
use serde_json::json;

#[test]
fn minimal_session_transforms_messages() {
    let mut pruner = ContextPruner::new(Config::default()).unwrap();
    let messages = vec![
        Message::user_text("u1", 0, "hello"),
        Message::assistant_text("a1", 0, "hi there"),
    ];
    let pruned = pruner.transform_messages(messages.clone()).unwrap();

    assert_eq!(pruned.len(), 2);
    assert_eq!(pruned[0].id, "u1");
    assert_eq!(pruned[1].id, "a1");

    // The library has assigned m#### references.
    assert_eq!(
        pruner
            .state()
            .message_ids
            .by_raw_id
            .get("u1")
            .map(String::as_str),
        Some("m0001"),
    );
    assert_eq!(
        pruner
            .state()
            .message_ids
            .by_raw_id
            .get("a1")
            .map(String::as_str),
        Some("m0002"),
    );
}

#[test]
fn double_transform_is_idempotent_for_message_refs() {
    let mut pruner = ContextPruner::new(Config::default()).unwrap();
    let messages = vec![
        Message::user_text("u1", 0, "hi"),
        Message::assistant_text("a1", 0, "hello"),
    ];
    let _ = pruner.transform_messages(messages.clone()).unwrap();
    let snapshot = pruner.state().message_ids.clone();
    let _ = pruner.transform_messages(messages).unwrap();
    assert_eq!(pruner.state().message_ids, snapshot);
}

#[test]
fn transform_invalid_messages_filters_them_out() {
    let mut pruner = ContextPruner::new(Config::default()).unwrap();
    // Invalid: user-role with a tool_call.
    let messages = vec![
        Message::new(
            "u1",
            Role::User,
            vec![Part::tool_call("c1", "read", json!({}))],
            0,
        ),
        Message::user_text("u2", 0, "hi"),
        Message::assistant_text("a1", 0, "hello"),
    ];
    let pruned = pruner.transform_messages(messages).unwrap();
    let ids: Vec<&str> = pruned.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["u2", "a1"]);
    assert_eq!(pruner.state().stats.dropped_invalid, 1);
}

#[test]
fn transform_system_appends_addendum() {
    let pruner = ContextPruner::new(Config::default()).unwrap();
    let mut sys = String::from("You are a helpful assistant.");
    pruner.transform_system(&mut sys);
    assert!(sys.starts_with("You are a helpful assistant."));
    assert!(sys.contains("Context-pruning support"));
}

#[test]
fn transform_with_master_switch_off_passes_through() {
    let mut cfg = Config::default();
    cfg.enabled = false;
    cfg.rebuild_cache().unwrap();
    let mut pruner = ContextPruner::new(cfg).unwrap();
    let messages = vec![
        Message::user_text("u1", 0, "hi"),
        Message::assistant_text("a1", 0, "hello"),
    ];
    let out = pruner.transform_messages(messages.clone()).unwrap();
    assert_eq!(out, messages);
    assert_eq!(pruner.state().current_turn, 0);
    assert!(pruner.state().message_ids.by_raw_id.is_empty());
}

#[test]
fn save_persists_state_to_storage_after_transform() {
    let store = Arc::new(InMemoryStateStore::new());
    let mut pruner = ContextPruner::builder()
        .config(Config::default())
        .storage(store.clone())
        .build()
        .unwrap();
    pruner.set_session_id("test-session");
    let messages = vec![
        Message::user_text("u1", 0, "hi"),
        Message::assistant_text("a1", 0, "hello"),
    ];
    let _ = pruner.transform_messages(messages).unwrap();
    pruner.save().unwrap();
    let loaded = store.load("test-session").unwrap();
    assert!(loaded.is_some(), "session must be persisted");
}

#[test]
fn session_id_derived_from_last_message_when_unset() {
    let mut pruner = ContextPruner::new(Config::default()).unwrap();
    let messages = vec![
        Message::user_text("u1", 0, "hi"),
        Message::assistant_text("a1", 0, "hello"),
    ];
    let _ = pruner.transform_messages(messages).unwrap();
    assert_eq!(pruner.state().session_id.as_deref(), Some("a1"));
}

#[test]
fn turn_count_advances_at_assistant_text_turn_end() {
    let mut pruner = ContextPruner::new(Config::default()).unwrap();
    let messages = vec![
        Message::user_text("u1", 0, "hi"),
        Message::assistant_text("a1", 0, "hello"),
    ];
    let _ = pruner.transform_messages(messages).unwrap();
    assert_eq!(pruner.state().current_turn, 1);
    assert!(pruner.state().last_message_was_assistant_text);
}

#[test]
fn slash_command_context_returns_breakdown() {
    let mut pruner = ContextPruner::new(Config::default()).unwrap();
    let messages = vec![
        Message::user_text("u1", 0, "hi"),
        Message::assistant_text("a1", 0, "hello"),
    ];
    let _ = pruner.transform_messages(messages.clone()).unwrap();
    match pruner.handle_command("context", &[], &messages) {
        dcp_core::CommandOutcome::Context {
            current_turn,
            cache_stability_mode,
            ..
        } => {
            assert_eq!(current_turn, 1);
            assert_eq!(cache_stability_mode, "agent-message");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn telemetry_records_at_least_one_event_per_transform() {
    let mut pruner = ContextPruner::new(Config::default()).unwrap();
    let messages = vec![
        Message::user_text("u1", 0, "hi"),
        Message::assistant_text("a1", 0, "hello"),
    ];
    let before = pruner.telemetry().total_events();
    let _ = pruner.transform_messages(messages).unwrap();
    let after = pruner.telemetry().total_events();
    assert!(
        after > before,
        "telemetry should have recorded at least one event"
    );
}
