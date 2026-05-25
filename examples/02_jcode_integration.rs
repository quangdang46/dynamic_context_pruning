//! 02_jcode_integration — pseudocode for wiring DCP into the `jcode`
//! agent (PLAN.md §9.2).
//!
//! `jcode` owns its own native message types; this example does **not**
//! depend on `jcode` and does not compile against it. It demonstrates
//! the canonical wiring pattern using DCP's own IR
//! ([`Message`] / [`Part`]) so the call shapes are visible end-to-end.
//!
//! Real adapter touch-points are flagged with `// JCODE:` comments —
//! they map onto the call sites a `jcode` patch would update.

use dynamic_context_pruning::{Config, ContextPruner, Message, Part, Role, ToolStatus};

// ── 1. IR adapter ────────────────────────────────────────────────────
//
// JCODE: Replace the body of this function with a real lift from
// `jcode_compaction_core::Message` (the IR `jcode` already has) into
// the canonical [`dynamic_context_pruning::Message`] / [`Part`] types.
// The mapping is mostly mechanical:
//
//   jcode::Role::User      -> Role::User
//   jcode::Role::Assistant -> Role::Assistant
//   jcode::Part::Text(s)   -> Part::Text(s)
//   jcode::Part::ToolCall  -> Part::ToolCall { call_id, tool, input }
//   jcode::Part::ToolResult-> Part::ToolResult { call_id, status, .. }
//
// The example synthesises a tiny conversation with one tool call so the
// pipeline has something realistic to operate on.
fn host_messages_into_canonical() -> Vec<Message> {
    vec![
        Message::user_text("u1", 0, "Open src/main.rs and tell me what it does."),
        Message::new(
            "a1",
            Role::Assistant,
            vec![
                Part::text("Reading the file."),
                Part::tool_call(
                    "call-1",
                    "read_file",
                    serde_json::json!({ "path": "src/main.rs" }),
                ),
            ],
            0,
        ),
        Message::new(
            "u2",
            Role::User,
            vec![Part::tool_result(
                "call-1",
                ToolStatus::Completed,
                Some("fn main() { println!(\"hi\"); }".into()),
                None,
            )],
            0,
        ),
        Message::assistant_text("a2", 0, "It's a hello-world program."),
    ]
}

// ── 2. messages_for_provider hook ────────────────────────────────────
//
// JCODE: this is the only line `jcode` needs to add to the existing
// `messages_for_provider()` (or equivalently named) function:
//
//     let pruned = pruner.transform_messages(messages)?;
//
// Everything else — adapter, network, retries — is unchanged.
fn build_provider_payload(pruner: &mut ContextPruner) -> anyhow::Result<Vec<Message>> {
    let messages = host_messages_into_canonical();
    let pruned = pruner.transform_messages(messages)?;
    Ok(pruned)
}

// ── 3. compress tool dispatch ────────────────────────────────────────
//
// JCODE: register `pruner.compress_tool_schema()` with the host's tool
// registry once at session start, then route the model's tool call:
//
//     match tool_call.name.as_str() {
//         "compress" => {
//             let args = serde_json::from_value(tool_call.input)?;
//             let result = pruner.handle_compress(args, &raw_messages)?;
//             // forward `result` back to the model as the tool result
//         }
//         _ => /* dispatch as today */,
//     }
//
// (See dcp-core's commands.rs `handle_command` for how `/dcp compress`
// is wired through the slash-command surface.)
fn show_tool_schema(pruner: &ContextPruner) {
    let schema = pruner.compress_tool_schema();
    println!(
        "registered tool: {} ({} chars desc)",
        schema.name,
        schema.description.len()
    );
}

// ── 4. system prompt addendum ────────────────────────────────────────
//
// JCODE: in `jcode`'s `assemble_system_prompt()` (or equivalent), append
// the DCP system-prompt fragment so the model knows about the message
// references and the compress tool:
//
//     pruner.transform_system(&mut system);
fn build_system_prompt(pruner: &ContextPruner) -> String {
    let mut system = String::from("You are jcode, a helpful coding assistant.");
    pruner.transform_system(&mut system);
    system
}

fn main() -> anyhow::Result<()> {
    let mut pruner = ContextPruner::new(Config::default())?;

    // Steps the real `jcode` patch performs every turn:
    let payload = build_provider_payload(&mut pruner)?;
    println!("jcode -> provider payload: {} messages", payload.len());
    show_tool_schema(&pruner);
    let system = build_system_prompt(&pruner);
    println!("system prompt: {} chars (addendum included)", system.len());

    Ok(())
}
