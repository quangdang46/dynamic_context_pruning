//! End-to-end smoke test: drive a 30-message synthetic session through
//! [`ContextPruner`] and verify the public surface returns sane,
//! non-empty results at every step (PLAN.md §11, Phase 5 quality
//! gates).
//!
//! Concretely the test:
//!
//! 1. Builds a 30-message multi-turn session that exercises tool
//!    calls, tool results, and pure-text exchanges.
//! 2. Drives it through `transform_messages` in turn-sized chunks so
//!    the cache-stability gate has natural firing points.
//! 3. Asserts the public counters update as expected (turn counter,
//!    apply triggers, telemetry totals).
//! 4. Calls `transform_system`, the slash-command surface, and
//!    `compress_tool_schema` so the whole public API is touched
//!    once.

use dynamic_context_pruning::{
    CommandOutcome, Config, ContextPruner, Message, Part, Role, ToolStatus,
};

fn build_session(turns: usize) -> Vec<Message> {
    let mut msgs = Vec::with_capacity(turns * 4);
    let mut t = 0i64;
    for i in 0..turns {
        let uid = format!("u{i}");
        let aid = format!("a{i}");
        let aid2 = format!("a{i}-2");
        let call_id = format!("call-{i}");
        let result_id = format!("u{i}-r");

        // user prompt
        msgs.push(Message::user_text(
            uid,
            t,
            format!("Please read file_{i}.rs"),
        ));
        t += 1;

        // assistant tool call
        msgs.push(Message::new(
            aid,
            Role::Assistant,
            vec![
                Part::text("Reading."),
                Part::tool_call(
                    call_id.clone(),
                    "read_file",
                    serde_json::json!({ "path": format!("file_{i}.rs") }),
                ),
            ],
            t,
        ));
        t += 1;

        // tool result (alternate completed / errored to exercise
        // purge_errors)
        let status = if i % 5 == 4 {
            ToolStatus::Error
        } else {
            ToolStatus::Completed
        };
        let (output, error) = match status {
            ToolStatus::Completed => (Some(format!("// contents of file_{i}.rs")), None),
            _ => (None, Some(format!("file_{i}.rs not found"))),
        };
        msgs.push(Message::new(
            result_id,
            Role::User,
            vec![Part::tool_result(call_id, status, output, error)],
            t,
        ));
        t += 1;

        // assistant follow-up text
        msgs.push(Message::assistant_text(
            aid2,
            t,
            format!("Done with file_{i}.rs."),
        ));
        t += 1;
    }
    msgs.truncate(30);
    msgs
}

fn drive(pruner: &mut ContextPruner, all: Vec<Message>) -> usize {
    // Replay the conversation in growing prefixes so each
    // transform_messages call sees the full history known so far —
    // mirroring how a real agent feeds the same growing list to the
    // provider every turn.
    let mut last_len = 0;
    for n in (4..=all.len()).step_by(4) {
        let prefix: Vec<Message> = all[..n].to_vec();
        let out = pruner.transform_messages(prefix).expect("transform");
        last_len = out.len();
    }
    // Final pass with the full session.
    let out = pruner.transform_messages(all).expect("transform final");
    last_len = last_len.max(out.len());
    last_len
}

#[test]
fn end_to_end_30_messages() {
    let mut pruner = ContextPruner::new(Config::default()).expect("build pruner");
    pruner.set_session_id("smoke-30");

    let session = build_session(8); // 8 turns x 4 = 32 -> truncated to 30
    assert_eq!(session.len(), 30, "expected exactly 30 messages");

    let final_len = drive(&mut pruner, session.clone());

    // ── Output sanity ───────────────────────────────────────────────
    assert!(final_len > 0, "transform_messages produced an empty stream");
    assert!(
        final_len <= session.len(),
        "transform should not invent new messages"
    );

    // ── State counters ──────────────────────────────────────────────
    let state = pruner.state();
    assert!(
        state.current_turn > 0,
        "current_turn should advance over 30 messages"
    );
    assert!(
        state.last_apply_turn.is_some(),
        "at least one apply phase should have fired in agent-message mode"
    );
    assert!(
        !state.message_ids.by_raw_id.is_empty(),
        "message references should have been allocated"
    );

    // ── Telemetry ───────────────────────────────────────────────────
    let snap = pruner.telemetry();
    assert!(
        snap.total_events() > 0,
        "telemetry should record at least one event"
    );

    // ── transform_system ────────────────────────────────────────────
    let mut system = String::from("You are a helpful coding assistant.");
    pruner.transform_system(&mut system);
    assert!(
        system.contains("Context-pruning support"),
        "system addendum must be appended"
    );

    // ── compress_tool_schema ────────────────────────────────────────
    let schema = pruner.compress_tool_schema();
    assert_eq!(schema.name, "compress");
    let json = serde_json::to_string(&schema.parameters).unwrap();
    assert!(json.contains("startId"), "schema mentions startId");

    // ── slash command ──────────────────────────────────────────────
    let outcome = pruner.handle_command("context", &[], &[]);
    match outcome {
        CommandOutcome::Context {
            current_turn,
            cache_stability_mode,
            ..
        } => {
            assert!(current_turn > 0);
            assert_eq!(cache_stability_mode, "agent-message");
        }
        other => panic!("unexpected outcome: {other:?}"),
    }

    // ── Validation surface ──────────────────────────────────────────
    // Insert an invalid message and confirm it is dropped + counted.
    let mut next = pruner;
    let bad = vec![Message::new(
        "bad",
        Role::User,
        vec![Part::tool_call("oops", "read", serde_json::json!({}))],
        0,
    )];
    let dropped_before = next.state().stats.dropped_invalid;
    let _ = next.transform_messages(bad).expect("transform");
    assert!(
        next.state().stats.dropped_invalid > dropped_before,
        "invalid message must increment dropped_invalid"
    );
}
