# DCP API Plan — Additions for jcode Integration

> **Goal**: Expose 2 missing APIs so jcode can integrate DCP as a plugin layer.
> **Branch**: `main` — implement directly on current codebase.
> **Depends on**: Nothing — standalone additions to existing DCP crate.

---

## Overview

DCP currently has **10/12 APIs ready** for jcode. Two are missing:

| # | Missing API | Priority | Effort |
|---|-------------|----------|--------|
| **D1** | `has_pending_work()` | High | ~10 lines |
| **D2** | `TransformResult` return type | High | ~80 lines |
| D3 | `count_messages_tokens()` | Low | ~15 lines |
| D4 | `last_nudge()` | Low | ~5 lines |

Items D3-D4 are optional convenience methods. jcode can work without them.

---

## D1: Add `has_pending_work()` to ContextPruner

**File**: `crates/dcp-core/src/pruner.rs`
**Location**: After `set_session_id()` (~line 542)

### What to add

```rust
/// Check whether DCP has pending prune decisions or pending work.
///
/// Returns `true` when:
/// - There are pending tool-level prune decisions (`prune.tools` non-empty)
/// - There is a pending prune snapshot waiting to be applied
///
/// jcode can call this before `transform_messages()` to decide whether
/// the DCP transform is needed for the current turn.
pub fn has_pending_work(&self) -> bool {
    let state = &self.state;
    state.pending_prune.is_some()
        || !state.prune.tools.is_empty()
        || !state.prune.messages.by_message_id.is_empty()
}
```

### Export

In `crates/dynamic_context_pruning/src/lib.rs` — already re-exports `ContextPruner`, so no change needed. Method is on the struct, auto-available.

### Tests

```rust
#[test]
fn has_pending_work_false_on_fresh() {
    let pruner = ContextPruner::new(Config::default()).unwrap();
    assert!(!pruner.has_pending_work());
}

#[test]
fn has_pending_work_true_after_strategy() {
    // After running dedup on messages with duplicate tool outputs,
    // pending prune decisions should exist
    // (existing integration test already covers this indirectly)
}
```

---

## D2: Add `TransformResult` return type

**Files**:
- `crates/dcp-core/src/pipeline.rs` — new struct + updated return
- `crates/dcp-core/src/pruner.rs` — new method `transform_messages_with_diff()`
- `crates/dynamic_context_pruning/src/lib.rs` — re-export `TransformResult`

### Why not change `transform_messages()` signature?

Changing `transform_messages()` from `Result<Vec<Message>>` to `Result<TransformResult>` is a **breaking change** for existing consumers (MCP server, examples). Instead, add a sibling method:

- `transform_messages()` — unchanged, backward compatible
- `transform_messages_with_diff()` — new, returns `TransformResult`

### New struct

In `crates/dcp-core/src/pipeline.rs`:

```rust
/// Result of a DCP transform pass, including what changed.
///
/// jcode uses the diff fields to update its CompactionManager budget
/// and show notifications about what was pruned.
#[derive(Clone, Debug)]
pub struct TransformResult {
    /// The transformed message list (with pruned content replaced).
    pub messages: Vec<Message>,

    /// IDs of messages that were removed or replaced by the transform.
    pub removed_message_ids: Vec<String>,

    /// IDs of tool results that were pruned.
    pub pruned_tool_ids: Vec<String>,

    /// Estimated tokens saved by this transform pass.
    pub tokens_saved: u64,

    /// Block IDs of new compression blocks created during this pass.
    pub new_block_ids: Vec<BlockId>,

    /// Whether any changes were made.
    pub changed: bool,
}
```

### New method

In `crates/dcp-core/src/pruner.rs`:

```rust
/// Transform messages and return a diff of what changed.
///
/// This is the same as `transform_messages()` but returns a
/// `TransformResult` with details about what was pruned, allowing
/// the caller (jcode) to update its own state accordingly.
pub fn transform_messages_with_diff(
    &mut self,
    messages: Vec<Message>,
) -> Result<TransformResult, Error> {
    // Capture pre-transform state
    let input_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
    let stats_before = self.stats().clone();

    // Run the existing pipeline
    let output = self.transform_messages_inner(messages)?;

    // Compute diff
    let output_ids: std::collections::HashSet<String> =
        output.iter().map(|m| m.id.clone()).collect();
    let removed: Vec<String> = input_ids
        .into_iter()
        .filter(|id| !output_ids.contains(id))
        .collect();

    let stats_after = self.stats();
    let tokens_saved = stats_after.total_prune_tokens.saturating_sub(stats_before.total_prune_tokens);

    let changed = !removed.is_empty() || tokens_saved > 0;

    // Collect new blocks
    let new_block_ids: Vec<BlockId> = self
        .state
        .prune
        .messages
        .active_block_ids
        .iter()
        .cloned()
        .collect();

    Ok(TransformResult {
        messages: output,
        removed_message_ids: removed,
        pruned_tool_ids: Vec::new(), // filled from strategy results
        tokens_saved,
        new_block_ids,
        changed,
    })
}
```

### How to implement without breaking existing API

1. Rename current `transform_messages()` body to `transform_messages_inner()` (private).
2. `transform_messages()` calls `transform_messages_inner()` and returns just `Vec<Message>`.
3. `transform_messages_with_diff()` calls `transform_messages_inner()` and builds `TransformResult`.

### Re-export

In `crates/dcp-core/src/lib.rs`:
```rust
pub use pipeline::TransformResult;
```

In `crates/dynamic_context_pruning/src/lib.rs`:
```rust
pub use dcp_core::TransformResult;
```

### Tests

```rust
#[test]
fn transform_with_diff_no_changes() {
    let mut pruner = ContextPruner::new(Config::default()).unwrap();
    let msgs = vec![
        Message::user_text("hello", "m0001"),
        Message::assistant_text("hi", "m0002"),
    ];
    let result = pruner.transform_messages_with_diff(msgs).unwrap();
    assert!(!result.changed);
    assert!(result.removed_message_ids.is_empty());
    assert_eq!(result.messages.len(), 2);
}

#[test]
fn transform_with_diff_detects_pruned() {
    // Create messages with duplicate tool outputs
    // Run transform, verify changed=true and removed IDs present
}
```

---

## D3 (Optional): Add `count_messages_tokens()` to ContextPruner

**File**: `crates/dcp-core/src/pruner.rs`

```rust
/// Count total tokens across all message parts using the installed tokenizer.
///
/// Convenience method so jcode doesn't need to access the tokenizer directly.
pub fn count_messages_tokens(&self, messages: &[Message]) -> u64 {
    let mut total = 0u64;
    for msg in messages {
        for part in &msg.parts {
            match part {
                Part::Text(text) | Part::Reasoning(text) => {
                    total += self.tokenizer.count(text) as u64;
                }
                Part::ToolCall { input, .. } => {
                    total += self.tokenizer.count(&input.to_string()) as u64;
                }
                Part::ToolResult { output, .. } => {
                    total += self.tokenizer.count(output) as u64;
                }
                Part::Image { .. } => {
                    total += 85; // standard image token estimate
                }
            }
        }
    }
    total
}
```

---

## D4 (Optional): Add `last_nudge()` to ContextPruner

**File**: `crates/dcp-core/src/pruner.rs`

```rust
/// Return the kind of the most recently injected nudge, if any.
///
/// jcode can check this after `transform_messages()` to decide
/// whether to show a context-limit warning in the TUI.
pub fn last_nudge_kind(&self) -> Option<String> {
    self.state.nudges.last_nudge_kind.clone()
}
```

Requires adding `last_nudge_kind: Option<String>` to `Nudges` struct in `dcp-types`, updated during phase 7 of the pipeline.

---

## Implementation Order

```
D1 (has_pending_work)     ← 5 min, do first
  ↓
D2 (TransformResult)      ← 30 min, main work
  ↓
D3 (count_messages_tokens) ← 5 min, optional
  ↓
D4 (last_nudge)            ← 5 min, optional
```

## Verification

After implementation:
```bash
cargo test --workspace
cargo build --release -p dynamic_context_pruning
```

Then jcode can depend on the updated crate and use all APIs.
