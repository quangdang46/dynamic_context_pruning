//! 04_custom_agent — full builder usage for a host that wires every
//! pluggable surface (PLAN.md §4.3).
//!
//! Demonstrates:
//!
//! * Programmatic [`Config`] construction with non-default fields.
//! * Custom [`Tokenizer`], [`StatePersistence`], and a swap-in
//!   [`PromptStore`].
//! * Driving the pruner across two turns and dispatching the
//!   `compress` tool through [`ContextPruner::handle_compress`].
//! * Observing telemetry and stats after the run.

use std::sync::Arc;

use dynamic_context_pruning::{
    CompressArgs, Config, ContextPruner, InMemoryStateStore, Message, Part, PromptStore, Prompts,
    RangeEntry, Role, StatePersistence, Tokenizer, ToolStatus,
};

/// Word-count tokenizer — simplistic but plenty accurate for local
/// budget estimation when the host already segments at whitespace.
#[derive(Debug, Default, Clone, Copy)]
struct WordTokenizer;

impl Tokenizer for WordTokenizer {
    fn count(&self, text: &str) -> usize {
        if text.trim().is_empty() {
            0
        } else {
            text.split_whitespace().count()
        }
    }
}

fn host_messages() -> Vec<Message> {
    vec![
        Message::user_text("u1", 1_700_000_000, "Hi! Read README.md"),
        Message::new(
            "a1",
            Role::Assistant,
            vec![
                Part::text("On it."),
                Part::tool_call(
                    "call-1",
                    "read_file",
                    serde_json::json!({ "path": "README.md" }),
                ),
            ],
            1_700_000_001,
        ),
        Message::new(
            "u2",
            Role::User,
            vec![Part::tool_result(
                "call-1",
                ToolStatus::Completed,
                Some("# dynamic_context_pruning\nMIT licensed.".into()),
                None,
            )],
            1_700_000_002,
        ),
        Message::assistant_text(
            "a2",
            1_700_000_003,
            "README is short and MIT-licensed — anything else?",
        ),
    ]
}

fn main() -> anyhow::Result<()> {
    // ── 1. Config tweaks ────────────────────────────────────────────
    let mut cfg = Config::default();
    cfg.compress.max_context_limit = dynamic_context_pruning::LimitValue::Number(120_000);
    cfg.compress.nudge_frequency = 3;
    cfg.protected_file_patterns.push("Cargo.toml".into());
    cfg.rebuild_cache()?;

    // ── 2. Pluggable surfaces ───────────────────────────────────────
    let tokenizer: Arc<dyn Tokenizer> = Arc::new(WordTokenizer);
    let storage: Arc<dyn StatePersistence> = Arc::new(InMemoryStateStore::new());
    let prompt_store = PromptStore::from_prompts(Prompts::default());

    // ── 3. Build the pruner ─────────────────────────────────────────
    let mut pruner = ContextPruner::builder()
        .config(cfg)
        .tokenizer(tokenizer)
        .storage(storage)
        .prompt_store(prompt_store)
        .build()?;
    pruner.set_session_id("custom-agent-demo");

    // ── 4. Turn 1 ──────────────────────────────────────────────────
    let turn1_in = host_messages();
    let turn1_out = pruner.transform_messages(turn1_in)?;
    println!("turn 1: {} messages out", turn1_out.len());

    // ── 5. Compress the first user/assistant pair into a block ──────
    let args = CompressArgs::Range {
        topic: "README exploration".into(),
        content: vec![RangeEntry {
            start_id: "m0001".into(),
            end_id: "m0002".into(),
            summary: "User asked to read README; assistant began the read.".into(),
        }],
    };
    match pruner.handle_compress(args, &turn1_out) {
        Ok(result) => println!(
            "compress -> {} compressed messages, {} block(s)",
            result.compressed_messages,
            result.blocks.len()
        ),
        Err(e) => println!("compress declined (small session): {e}"),
    }

    // ── 6. Turn 2 ──────────────────────────────────────────────────
    let mut turn2_in = host_messages();
    turn2_in.push(Message::user_text("u3", 1_700_000_010, "thanks!"));
    let turn2_out = pruner.transform_messages(turn2_in)?;
    println!("turn 2: {} messages out", turn2_out.len());

    // ── 7. Introspection ───────────────────────────────────────────
    let stats = pruner.stats();
    let telemetry = pruner.telemetry();
    println!(
        "stats: total_prune_tokens={} dedup_pruned={} compress_runs={}",
        stats.total_prune_tokens, stats.dedup_pruned, stats.compress_runs
    );
    println!(
        "telemetry: {} events ({} apply triggers)",
        telemetry.total_events(),
        telemetry.count_of(&dynamic_context_pruning::EventKind::ApplyTrigger {
            mode: "agent_message".into()
        })
    );

    // ── 8. Persist ──────────────────────────────────────────────────
    pruner.save()?;
    Ok(())
}
