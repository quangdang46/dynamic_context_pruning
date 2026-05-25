//! 06_cache_stability — demonstrates the three [`CacheStabilityMode`]
//! values side-by-side (PLAN.md §3.1 / §6.8).
//!
//! Each mode runs the same synthetic message stream through a fresh
//! [`ContextPruner`] and prints the resulting telemetry. The relevant
//! signal is the `apply_trigger` count: how many times the apply phase
//! actually ran. That count is the proxy for "how often the prompt
//! cache would be busted" in production.

use dynamic_context_pruning::{
    CacheStabilityMode, Config, ContextPruner, EventKind, Message, Part, Role, ToolStatus,
};

/// Build a four-turn synthetic session. Each turn ends with an
/// assistant text message (the trigger for the `agent-message` mode).
fn synthetic_session(n: usize) -> Vec<Message> {
    let mut msgs = Vec::with_capacity(n * 4);
    let mut t = 0i64;
    for i in 0..n {
        let uid = format!("u{i}");
        let aid = format!("a{i}");
        msgs.push(Message::user_text(
            uid.clone(),
            t,
            format!("Read foo{i}.rs"),
        ));
        t += 1;
        let call_id = format!("call-{i}");
        msgs.push(Message::new(
            aid.clone(),
            Role::Assistant,
            vec![
                Part::text("Reading."),
                Part::tool_call(
                    call_id.clone(),
                    "read_file",
                    serde_json::json!({ "path": format!("foo{i}.rs") }),
                ),
            ],
            t,
        ));
        t += 1;
        msgs.push(Message::new(
            format!("u{i}-r"),
            Role::User,
            vec![Part::tool_result(
                call_id,
                ToolStatus::Completed,
                Some(format!("// content of foo{i}.rs")),
                None,
            )],
            t,
        ));
        t += 1;
        msgs.push(Message::assistant_text(
            format!("a{i}-done"),
            t,
            format!("Read foo{i}.rs successfully."),
        ));
        t += 1;
    }
    msgs
}

fn drive_in_chunks(pruner: &mut ContextPruner, all: Vec<Message>) -> anyhow::Result<usize> {
    // Process the session four messages at a time so the cache-stability
    // gate has a chance to trigger between turns.
    let mut total_out = 0;
    for chunk in all.chunks(4) {
        let out = pruner.transform_messages(chunk.to_vec())?;
        total_out += out.len();
    }
    Ok(total_out)
}

fn run_with_mode(label: &str, mode: CacheStabilityMode) -> anyhow::Result<()> {
    let mut cfg = Config::default();
    cfg.cache_stability_mode = mode;
    cfg.rebuild_cache()?;
    let mut pruner = ContextPruner::new(cfg)?;

    let total_out = drive_in_chunks(&mut pruner, synthetic_session(4))?;

    let snap = pruner.telemetry();
    let apply = snap.count_of(&EventKind::ApplyTrigger {
        mode: mode.as_str().to_string(),
    });
    let bust = snap
        .event_counts
        .iter()
        .filter(|(k, _)| matches!(k, EventKind::CacheBust { .. }))
        .map(|(_, v)| v)
        .sum::<u64>();

    println!(
        "mode={:<14} -> total_out={total_out:<3} apply_triggers={apply:<2} cache_busts={bust}",
        label,
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    println!("Comparing the three CacheStabilityMode values:\n");

    // Aggressive: apply on every turn — high apply count, high cache
    // turnover.
    run_with_mode("aggressive", CacheStabilityMode::Aggressive)?;

    // AgentMessage: apply only when the assistant just emitted text and
    // no tool calls are pending. The default; balanced.
    run_with_mode("agent-message", CacheStabilityMode::AgentMessage)?;

    // Manual: never apply automatically — the host calls `force_apply`
    // when ready. Lowest apply count.
    run_with_mode("manual", CacheStabilityMode::Manual)?;

    println!(
        "\nKey: `apply_triggers` ≈ how often the prompt cache would be \
         re-keyed. Lower is generally better for cost; the trade-off is \
         deferred token savings."
    );
    Ok(())
}
