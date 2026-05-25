//! 03_codex_integration — pseudocode for wiring DCP into the
//! `codex-rs` agent (PLAN.md §9.2 + research note 2.4).
//!
//! `codex-rs` already owns a `core::compact` module; the goal of this
//! example is to show the call-shape that replaces (or wraps) it with
//! DCP. The example does **not** depend on `codex-rs` and does not
//! compile against it. Touch-points in a real adapter are flagged
//! with `// CODEX:` comments.

use dynamic_context_pruning::{
    CompressArgs, Config, ContextPruner, Message, Part, RangeEntry, Role,
};

// ── 1. IR adapter ────────────────────────────────────────────────────
//
// CODEX: replace this stub with the real `codex_core::types::Message →
// dcp_types::Message` mapping. Codex models text + reasoning + tool
// calls separately so the mapping is one-to-one onto our [`Part`]
// variants:
//
//   codex::Part::Text       -> Part::Text
//   codex::Part::Reasoning  -> Part::Reasoning
//   codex::Part::ToolCall   -> Part::ToolCall
//   codex::Part::ToolResult -> Part::ToolResult
//
// Codex's reasoning parts MUST round-trip — DCP never strips them.
fn codex_to_canonical() -> Vec<Message> {
    vec![
        Message::user_text("u1", 0, "Refactor `Foo::bar` to be `async`."),
        Message::new(
            "a1",
            Role::Assistant,
            vec![
                Part::reasoning("The host wants the function to be awaitable."),
                Part::text("Working on it..."),
            ],
            0,
        ),
        Message::user_text("u2", 0, "Continue please."),
        Message::assistant_text("a2", 0, "Done — `Foo::bar` is now async."),
    ]
}

// ── 2. compaction hook ───────────────────────────────────────────────
//
// CODEX: replace `codex_core::compact::compact()` call sites with a
// `pruner.transform_messages(messages)` call. The behaviour is a
// strict superset:
//
//   - Codex's threshold-based summary remains available through
//     `pruner.handle_compress(...)` (LLM-driven).
//   - The three deterministic strategies (dedup, purge_errors,
//     stale_file_reads) run on every transform without any model call.
//   - Cache-stability gating (PLAN.md §3.1) means `transform_messages`
//     is safe to call on every turn without busting the cache.
fn run_one_turn(pruner: &mut ContextPruner) -> anyhow::Result<Vec<Message>> {
    let messages = codex_to_canonical();
    let pruned = pruner.transform_messages(messages)?;
    Ok(pruned)
}

// ── 3. /compact slash command ────────────────────────────────────────
//
// CODEX: wire codex's `/compact` slash command to a manual compress
// call. The example below uses an explicit range to keep the call
// shape concrete; the real adapter would take the range from the
// user's argument or pick a sensible default.
fn manual_compact(pruner: &mut ContextPruner, raw_messages: &[Message]) -> anyhow::Result<()> {
    let args = CompressArgs::Range {
        topic: "exploration".into(),
        content: vec![RangeEntry {
            start_id: "m0001".into(),
            end_id: "m0002".into(),
            summary: "Initial exploration of Foo::bar.".into(),
        }],
    };
    // In a real adapter, `result.compressed_messages` and
    // `result.blocks` are fed back to the user via the host's notice
    // surface ("compressed N messages into block bX").
    match pruner.handle_compress(args, raw_messages) {
        Ok(result) => println!(
            "codex /compact -> {} messages, {} block(s)",
            result.compressed_messages,
            result.blocks.len()
        ),
        // Compress can fail if the range hasn't been allocated yet
        // (e.g. running this example without prior transforms). The
        // real adapter should surface the error to the user.
        Err(e) => println!("codex /compact -> {e} (likely empty session)"),
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let mut pruner = ContextPruner::new(Config::default())?;

    // Each model turn (codex's per-step loop):
    let pruned = run_one_turn(&mut pruner)?;
    println!("codex -> pruned: {} messages", pruned.len());

    // /compact request (manual):
    manual_compact(&mut pruner, &pruned)?;

    // codex telemetry already records duration_ms; DCP records its own
    // counters via `pruner.telemetry()`.
    let snap = pruner.telemetry();
    println!("dcp telemetry total events: {}", snap.total_events());
    Ok(())
}
