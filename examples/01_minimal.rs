//! 01_minimal — the shortest path from zero to a pruned message stream.
//!
//! Constructs a [`ContextPruner`] with bundled defaults and runs three
//! synthetic messages through `transform_messages`. Mirrors PLAN.md
//! §1.3 ("three lines to wire").

use dynamic_context_pruning::{Config, ContextPruner, Message};

fn main() -> anyhow::Result<()> {
    let mut pruner = ContextPruner::new(Config::default())?;

    let messages = vec![
        Message::user_text("u1", 0, "Read Cargo.toml please."),
        Message::assistant_text("a1", 0, "Sure — here is the file."),
        Message::user_text("u2", 0, "Now run the tests."),
    ];

    let pruned = pruner.transform_messages(messages)?;

    println!("input    -> 3 messages");
    println!("pruned   -> {} messages", pruned.len());
    println!("turn     -> {}", pruner.state().current_turn);
    println!("telemetry-> {} events", pruner.telemetry().total_events());
    Ok(())
}
