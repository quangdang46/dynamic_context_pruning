# ADDING.md — DCP Rust Port: Missing Features Implementation Plan

> **5 items** need to be added to achieve feature parity with the TypeScript source.
> Each item includes: TS reference, target Rust files, exact signatures, and step-by-step instructions.

---

## Table of Contents

1. [Item 1: `getFilePathsFromParameters` — Protected File Path Extraction](#item-1-getfilepathfromparameters)
2. [Item 2: Glob Tool-Name Protection](#item-2-glob-tool-name-protection)
3. [Item 3: Compression Timing Module](#item-3-compression-timing-module)
4. [Item 4: Manual Mode MCP Tool](#item-4-manual-mode-mcp-tool)
5. [Item 5: Wire Up Visual Output in MCP Tool Results](#item-5-wire-up-visual-output)

---

## Item 1: `getFilePathsFromParameters`

**TS Reference**: `lib/protected-patterns.ts` → `getFilePathsFromParameters()`
**Rust Target**: `crates/dcp-protected/src/lib.rs`

### Why
Without this, the prune strategies cannot extract file paths from tool parameters. If a user edits `/src/auth.rs`, DCP doesn't know that file is related and may prune the edit output.

### Current State
```rust
// dcp-protected/src/lib.rs — only has:
pub struct ToolProtection { set: HashSet<String> }  // exact match only
pub struct PathProtection { glob: GlobSet }           // glob for file paths
// ❌ No function to extract file paths from tool call parameters
```

### What to Add

#### 1.1 Add `extract_file_paths()` function

```rust
/// Extract file paths from tool call parameters.
/// Port of TS `getFilePathsFromParameters()`.
///
/// Handles:
/// - `apply_patch` → parses `*** Add/Delete/Update File: <path>` from patchText
/// - `multiedit`   → top-level `filePath` + nested `edits[].filePath`
/// - `read`/`write`/`edit` → `filePath` field
/// - Returns unique, non-empty paths
pub fn extract_file_paths(tool: &str, parameters: &serde_json::Value) -> Vec<String> {
    if !parameters.is_object() {
        return vec![];
    }

    let mut paths = Vec::new();

    // 1. apply_patch — embedded paths in patchText
    if tool == "apply_patch" {
        if let Some(patch_text) = parameters.get("patchText").and_then(|v| v.as_str()) {
            let re = regex::Regex::new(r"\*\*\* (?:Add|Delete|Update) File: ([^\n\r]+)").unwrap();
            for cap in re.captures_iter(patch_text) {
                paths.push(cap[1].trim().to_string());
            }
        }
    }

    // 2. multiedit — top-level filePath + nested edits[].filePath
    if tool == "multiedit" {
        if let Some(fp) = parameters.get("filePath").and_then(|v| v.as_str()) {
            paths.push(fp.to_string());
        }
        if let Some(edits) = parameters.get("edits").and_then(|v| v.as_array()) {
            for edit in edits {
                if let Some(fp) = edit.get("filePath").and_then(|v| v.as_str()) {
                    paths.push(fp.to_string());
                }
            }
        }
    }

    // 3. Default — common filePath parameter (read, write, edit, etc.)
    if let Some(fp) = parameters.get("filePath").and_then(|v| v.as_str()) {
        paths.push(fp.to_string());
    }

    // Deduplicate and filter empty
    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| !p.is_empty() && seen.insert(p.clone()));
    paths
}
```

#### 1.2 Add dependency

In `crates/dcp-protected/Cargo.toml`:
```toml
[dependencies]
regex = { workspace = true }
serde_json = { workspace = true }
```

#### 1.3 Add `is_file_path_protected()` convenience function

```rust
/// Check if any of the given file paths match the protection patterns.
/// Port of TS `isFilePathProtected()`.
pub fn is_file_path_protected(file_paths: &[String], patterns: &PathProtection) -> bool {
    if file_paths.is_empty() {
        return false;
    }
    file_paths.iter().any(|p| patterns.is_protected(p))
}
```

#### 1.4 Update prune strategies to use it

In each strategy that checks file protection (`crates/dcp-prune/src/deduplicate.rs`, `stale_file_reads.rs`, `purge_errors.rs`), replace:

```rust
// Before: cannot check file paths from tool params
// After:
let file_paths = extract_file_paths(&tool_name, &params);
if is_file_path_protected(&file_paths, &config.protected_file_patterns()) {
    continue; // skip — file is protected
}
```

### Files to Modify
| File | Action |
|------|--------|
| `crates/dcp-protected/src/lib.rs` | Add `extract_file_paths()`, `is_file_path_protected()` |
| `crates/dcp-protected/Cargo.toml` | Add `regex`, `serde_json` deps |
| `crates/dcp-prune/src/deduplicate.rs` | Use `extract_file_paths()` in protection check |
| `crates/dcp-prune/src/stale_file_reads.rs` | Use `extract_file_paths()` in protection check |
| `crates/dcp-prune/src/purge_errors.rs` | Use `extract_file_paths()` in protection check |

---

## Item 2: Glob Tool-Name Protection

**TS Reference**: `lib/protected-patterns.ts` → `isToolNameProtected()`
**Rust Target**: `crates/dcp-protected/src/lib.rs`

### Why
TS supports `protectTools: ["mcp__*"]` (glob wildcard). Rust `ToolProtection` only does exact `HashSet` match. This means `mcp__filesystem__read` would NOT be protected when user configures `mcp__*`.

### Current State
```rust
pub struct ToolProtection {
    set: HashSet<String>,  // ❌ exact match only
}

impl ToolProtection {
    pub fn is_protected(&self, tool_name: &str) -> bool {
        self.set.contains(tool_name)  // ❌ no glob support
    }
}
```

### What to Add

#### 2.1 Extend `ToolProtection` with glob support

```rust
use globset::GlobSet;

pub struct ToolProtection {
    /// Exact-match tool names.
    exact: HashSet<String>,
    /// Glob-pattern tool names (e.g. "mcp__*", "todo*").
    glob: GlobSet,
}
```

#### 2.2 Update constructor

```rust
impl ToolProtection {
    /// Build a ToolProtection from a list of patterns.
    /// Patterns containing `*` or `?` are treated as globs;
    /// all others are exact matches.
    pub fn new(patterns: &[String]) -> Result<Self, ProtectionError> {
        let mut exact = HashSet::new();
        let mut glob_builder = GlobSetBuilder::new();
        let mut has_glob = false;

        for pattern in patterns {
            if pattern.contains('*') || pattern.contains('?') {
                let glob = Glob::new(pattern)
                    .map_err(|e| ProtectionError::InvalidGlob(e.to_string()))?;
                glob_builder.add(glob);
                has_glob = true;
            } else {
                exact.insert(pattern.clone());
            }
        }

        let glob = if has_glob {
            glob_builder
                .build()
                .map_err(|e| ProtectionError::InvalidGlob(e.to_string()))?
        } else {
            GlobSet::empty()
        };

        Ok(Self { exact, glob })
    }

    /// Check if a tool name is protected (exact match or glob match).
    pub fn is_protected(&self, tool_name: &str) -> bool {
        if self.exact.contains(tool_name) {
            return true;
        }
        self.glob.is_match(tool_name)
    }
}
```

#### 2.3 Update `Config::rebuild_cache()`

In `crates/dcp-config/src/config.rs`, update `rebuild_cache()` to call `ToolProtection::new()` instead of building a plain `HashSet`:

```rust
fn rebuild_cache(&mut self) {
    // Before:
    // let tool_set: HashSet<String> = self.protected_tools.iter().cloned().collect();
    // self.cached_protections.tool = ToolProtection { set: tool_set };

    // After:
    self.cached_protections.tool = ToolProtection::new(&self.protected_tools)
        .expect("invalid tool protection patterns");
}
```

### Files to Modify
| File | Action |
|------|--------|
| `crates/dcp-protected/src/lib.rs` | Rewrite `ToolProtection` with `exact` + `glob` fields |
| `crates/dcp-config/src/config.rs` | Update `rebuild_cache()` to use `ToolProtection::new()` |

---

## Item 3: Compression Timing Module

**TS Reference**: `lib/compress/timing.ts`
**Rust Target**: New file `crates/dcp-compress/src/timing.rs`

### Why
Tracks how long each compression operation takes. The duration is stored in `CompressionBlock.duration_ms` and displayed in block summaries. Without it, `duration_ms` is always `None`.

### What to Add

#### 3.1 Create `crates/dcp-compress/src/timing.rs`

```rust
//! Compression timing — port of lib/compress/timing.ts.
//!
//! Tracks wall-clock duration of compression operations
//! and attaches them to CompressionBlock entries.

use dcp_types::SessionState;

/// Build a timing map key from message ID and call ID.
pub fn build_compression_timing_key(message_id: &str, call_id: &str) -> String {
    format!("{message_id}:{call_id}")
}

/// Resolve the compression duration from available timestamps.
///
/// Prefers `pending_to_running_ms` (time from compress start to tool execution).
/// Falls back to `runtime_ms` (tool execution wall time from part metadata).
pub fn resolve_compression_duration(
    started_at: Option<u64>,
    event_time: Option<u64>,
    part_start: Option<u64>,
    part_end: Option<u64>,
) -> Option<u64> {
    // Time from compress intent to actual execution
    let running_at = part_start.or(event_time);
    let pending_to_running_ms = match (started_at, running_at) {
        (Some(start), Some(run)) if run >= start => Some(run - start),
        _ => None,
    };

    // Tool execution wall time
    let runtime_ms = match (part_start, part_end) {
        (Some(s), Some(e)) if e >= s => Some(e - s),
        _ => None,
    };

    pending_to_running_ms.or(runtime_ms)
}
```

#### 3.2 Register module

In `crates/dcp-compress/src/lib.rs`, add:
```rust
pub mod timing;
```

#### 3.3 Wire into `handle_compress()`

In `crates/dcp-compress/src/handler.rs`, after a successful `commit_block()`:

```rust
// Record compression duration in the block
let duration_ms = timing::resolve_compression_duration(
    started_at,   // from args or state
    Some(now_ms), // event time
    None,         // part_start (not available in MCP context)
    None,         // part_end
);
if let Some(dur) = duration_ms {
    block.duration_ms = Some(dur);
}
```

### Files to Create/Modify
| File | Action |
|------|--------|
| `crates/dcp-compress/src/timing.rs` | **Create** — `resolve_compression_duration()`, `build_compression_timing_key()` |
| `crates/dcp-compress/src/lib.rs` | Add `pub mod timing;` |

---

## Item 4: Manual Mode MCP Tool

**TS Reference**: `lib/commands/manual.ts`
**Rust Target**: `crates/dcp-mcp/src/main.rs`

### Why
TS has `/dcp manual on|off` to toggle manual mode and `/dcp compress [focus]` to trigger compress manually. The Rust MCP server has no equivalent.

### What to Add

#### 4.1 Add `manual_toggle` MCP tool

In `crates/dcp-mcp/src/main.rs`, register a new tool:

```rust
// In the tool registration section, add:

// Tool: manual_toggle
{
    let mut obj = serde_json::Map::new();
    obj.insert("name".into(), "manual_toggle".into());
    // ... schema ...
    tools.push(obj);
}
```

#### 4.2 Add handler function

```rust
/// Handle the `manual_toggle` MCP tool.
///
/// Args:
///   - `mode` (optional): "on" | "off" | null (toggle)
///
/// Returns: confirmation message.
fn run_manual_toggle(&self, args_json: &JsonValue) -> rmcp::model::CallToolResult {
    let inner = match self.inner.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return rmcp::model::CallToolResult::error(vec![Content::text(
                "failed to acquire lock".to_string(),
            )]);
        }
    };

    let mode = args_json.get("mode").and_then(|v| v.as_str());

    let new_state = match mode {
        Some("on") => true,
        Some("off") => false,
        _ => !inner.pruner.state().manual_mode, // toggle
    };

    inner.pruner.state_mut().manual_mode = new_state;

    let msg = if new_state {
        "Manual mode is now ON. Use the `compress` tool to trigger context pruning manually."
    } else {
        "Manual mode is now OFF."
    };

    rmcp::model::CallToolResult::success(vec![Content::text(msg.to_string())])
}
```

#### 4.3 Dispatch in `Service` impl

```rust
// In the match block for ClientRequest::CallToolRequest:
"manual_toggle" => self.run_manual_toggle(&request.params.arguments),
```

#### 4.4 Add to `SessionState`

Ensure `SessionState` has the `manual_mode` field (check `crates/dcp-types/src/lib.rs`):

```rust
pub struct SessionState {
    // ... existing fields ...
    /// Whether manual mode is active (user must trigger compress manually).
    pub manual_mode: bool,
    /// Pending manual trigger prompt.
    pub pending_manual_trigger: Option<String>,
}
```

### Files to Modify
| File | Action |
|------|--------|
| `crates/dcp-mcp/src/main.rs` | Add `manual_toggle` tool + handler + dispatch |
| `crates/dcp-types/src/lib.rs` | Ensure `manual_mode`, `pending_manual_trigger` fields exist |
| `crates/dcp-state/src/session.rs` | Initialize `manual_mode: false` in `create_session_state()` |

---

## Item 5: Wire Up Visual Output in MCP Tool Results

**TS Reference**: `lib/ui/notification.ts` → `sendCompressNotification()`, `sendUnifiedNotification()`
**Rust Target**: `crates/dcp-mcp/src/main.rs` + `crates/dcp-notification/src/format.rs`

### Why
The MCP server currently returns raw JSON. User sees:
```json
{"compressed_messages": 3, "blocks": [...], "tokens_saved": 15000}
```

User should see:
```
▣ DCP | ~15K removed, +2.1K summary
│████░████░████⣿⣿⣿│
▣ Compression #1 -12K removed, +800 summary
→ Topic: auth refactoring
→ Items: 3 messages and 5 tools compressed
```

### Current State
```rust
// dcp-mcp/src/main.rs line ~298:
let json_out = serde_json::to_string_pretty(&result).unwrap_or_default();
rmcp::model::CallToolResult::success(vec![Content::text(json_out)])
// ❌ Raw JSON — not human-readable
```

Formatting functions already exist but are NEVER called:
- `format_stats_header()` ✅ exists in `dcp-notification/src/format.rs`
- `format_token_count()` ✅ exists
- `format_progress_bar()` ✅ exists
- `shorten_path()` ✅ exists
- ❌ `extract_parameter_key()` — missing, needs to be added
- ❌ `format_pruned_items_list()` — missing, needs to be added
- ❌ `format_pruning_result_for_tool()` — missing, needs to be added
- ❌ `build_compress_notification()` — missing, needs to be added

### Step-by-Step Implementation

#### 5.1 Add `extract_parameter_key()` to `crates/dcp-notification/src/format.rs`

```rust
/// Extract a human-readable key from tool parameters for display.
/// Port of TS `extractParameterKey()`.
pub fn extract_parameter_key(tool: &str, parameters: &serde_json::Value) -> String {
    if !parameters.is_object() {
        return String::new();
    }

    // read tool — show filePath with optional line range
    if tool == "read" {
        if let Some(fp) = parameters.get("filePath").and_then(|v| v.as_str()) {
            let offset = parameters.get("offset").and_then(|v| v.as_u64());
            let limit = parameters.get("limit").and_then(|v| v.as_u64());
            return match (offset, limit) {
                (Some(o), Some(l)) => format!("{fp} (lines {o}-{})", o + l),
                (Some(o), None) => format!("{fp} (lines {o}+)"),
                (None, Some(l)) => format!("{fp} (lines 0-{l})"),
                _ => fp.to_string(),
            };
        }
    }

    // write, edit, multiedit — filePath
    if matches!(tool, "write" | "edit" | "multiedit") {
        if let Some(fp) = parameters.get("filePath").and_then(|v| v.as_str()) {
            return fp.to_string();
        }
    }

    // apply_patch — parse embedded paths from patchText
    if tool == "apply_patch" {
        if let Some(patch) = parameters.get("patchText").and_then(|v| v.as_str()) {
            let re = regex::Regex::new(r"\*\*\* (?:Add|Delete|Update) File: ([^\n\r]+)").unwrap();
            let paths: Vec<String> = re
                .captures_iter(patch)
                .map(|c| c[1].trim().to_string())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            return match paths.len() {
                0 => "patch".to_string(),
                1 => paths[0].clone(),
                2 => format!("{}, {}", paths[0], paths[1]),
                n => format!("{n} files: {}, {}...", paths[0], paths[1]),
            };
        }
    }

    // list — path
    if tool == "list" {
        return parameters
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("(current directory)")
            .to_string();
    }

    // glob — pattern
    if tool == "glob" {
        if let Some(pattern) = parameters.get("pattern").and_then(|v| v.as_str()) {
            let path_info = parameters
                .get("path")
                .and_then(|v| v.as_str())
                .map(|p| format!(" in {p}"))
                .unwrap_or_default();
            return format!("\"{pattern}\"{path_info}");
        }
        return "(unknown pattern)".to_string();
    }

    // grep — pattern
    if tool == "grep" {
        if let Some(pattern) = parameters.get("pattern").and_then(|v| v.as_str()) {
            let path_info = parameters
                .get("path")
                .and_then(|v| v.as_str())
                .map(|p| format!(" in {p}"))
                .unwrap_or_default();
            return format!("\"{pattern}\"{path_info}");
        }
        return "(unknown pattern)".to_string();
    }

    // bash — description or command
    if tool == "bash" {
        if let Some(desc) = parameters.get("description").and_then(|v| v.as_str()) {
            return desc.to_string();
        }
        if let Some(cmd) = parameters.get("command").and_then(|v| v.as_str()) {
            return if cmd.len() > 50 {
                format!("{}...", &cmd[..50])
            } else {
                cmd.to_string()
            };
        }
    }

    // webfetch/websearch/codesearch
    if tool == "webfetch" {
        return parameters
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
    }
    if matches!(tool, "websearch" | "codesearch") {
        return parameters
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| format!("\"{q}\""))
            .unwrap_or_default();
    }

    // Fallback: truncate JSON
    let param_str = serde_json::to_string(parameters).unwrap_or_default();
    if param_str == "{}" || param_str == "[]" || param_str == "null" {
        return String::new();
    }
    truncate(&param_str, 50)
}
```

Add dependency to `crates/dcp-notification/Cargo.toml`:
```toml
[dependencies]
regex = { workspace = true }
serde_json = { workspace = true }
```

#### 5.2 Add `format_pruned_items_list()` and `format_pruning_result_for_tool()`

```rust
/// Format a list of pruned tool IDs with their parameter summaries.
/// Port of TS `formatPrunedItemsList()`.
pub fn format_pruned_items_list(
    pruned_tool_ids: &[String],
    tool_metadata: &std::collections::HashMap<String, (String, serde_json::Value)>,
    working_directory: Option<&str>,
) -> Vec<String> {
    let mut lines = Vec::new();

    for id in pruned_tool_ids {
        if let Some((tool, params)) = tool_metadata.get(id) {
            let param_key = extract_parameter_key(tool, params);
            if !param_key.is_empty() {
                let display = truncate(&shorten_path(&param_key, working_directory), 60);
                lines.push(format!("→ {tool}: {display}"));
            } else {
                lines.push(format!("→ {tool}"));
            }
        }
    }

    let known_count = pruned_tool_ids
        .iter()
        .filter(|id| tool_metadata.contains_key(*id))
        .count();
    let unknown_count = pruned_tool_ids.len() - known_count;

    if unknown_count > 0 {
        let plural = if unknown_count > 1 { "s" } else { "" };
        lines.push(format!("→ ({unknown_count} tool{plural} with unknown metadata)"));
    }

    lines
}

/// Format a complete pruning result for display in MCP tool output.
/// Port of TS `formatPruningResultForTool()`.
pub fn format_pruning_result_for_tool(
    pruned_ids: &[String],
    tool_metadata: &std::collections::HashMap<String, (String, serde_json::Value)>,
    working_directory: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "Context pruning complete. Pruned {} tool outputs.",
        pruned_ids.len()
    ));
    lines.push(String::new());

    if !pruned_ids.is_empty() {
        lines.push(format!("Semantically pruned ({}):", pruned_ids.len()));
        lines.extend(format_pruned_items_list(pruned_ids, tool_metadata, working_directory));
    }

    lines.join("\n").trim_end().to_string()
}
```

Also add the missing `truncate()` utility (check if already exists — it may not):
```rust
/// Truncate a string to max_len characters, appending "..." if truncated.
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    if max_len > 3 {
        format!("{}...", &s[..max_len - 3])
    } else {
        s[..max_len].to_string()
    }
}
```

#### 5.3 Add `build_compress_visual_output()` to notification module

In `crates/dcp-notification/src/notification.rs`, add:

```rust
/// Build a human-readable visual output for compress tool results.
/// This replaces raw JSON in MCP tool responses.
pub fn build_compress_visual_output(
    state: &SessionState,
    blocks: &[NotificationEntry],
    session_message_ids: &[String],
) -> String {
    use crate::format::{format_progress_bar, format_stats_header, format_token_count};

    if blocks.is_empty() {
        return "No blocks compressed.".to_string();
    }

    let mut lines = Vec::new();

    // Header with total stats
    let total_gross = state.stats.total_prune_tokens + state.stats.prune_token_counter;
    lines.push(format_stats_header(total_gross, 0));

    // Progress bar
    let active_pruned: HashMap<String, u64> = state
        .prune
        .messages
        .by_message_id
        .iter()
        .filter(|(_, entry)| !entry.active_block_ids.is_empty())
        .map(|(id, entry)| (id.clone(), entry.token_count))
        .collect();

    let newly_compressed: Vec<String> = blocks
        .iter()
        .flat_map(|b| {
            state
                .prune
                .messages
                .blocks_by_id
                .get(&b.block_id.as_u32())
                .map(|blk| blk.direct_message_ids.clone())
                .unwrap_or_default()
        })
        .collect();

    let bar = format_progress_bar(session_message_ids, &active_pruned, &newly_compressed, 50);
    lines.push(String::new());
    lines.push(bar);

    // Per-block details
    for entry in blocks {
        let run_id = entry.run_id;
        let summary_tokens = entry.summary_tokens;
        lines.push(format!(
            "▣ Compression #{} — {} summary",
            run_id,
            format_token_count(summary_tokens, true)
        ));
        lines.push(format!("→ Topic: {}", entry.topic()));
    }

    lines.join("\n")
}
```

#### 5.4 Wire into MCP tool results

In `crates/dcp-mcp/src/main.rs`, replace the raw JSON returns:

**Before:**
```rust
match inner.pruner.handle_compress(args, &messages) {
    Ok(result) => {
        let json_out = serde_json::to_string_pretty(&result).unwrap_or_default();
        rmcp::model::CallToolResult::success(vec![Content::text(json_out)])
    }
    // ...
}
```

**After:**
```rust
match inner.pruner.handle_compress(args, &messages) {
    Ok(result) => {
        // Visual output for user
        let visual = dcp_notification::build_compress_visual_output(
            inner.pruner.state(),
            &result.blocks,
            &session_message_ids,
        );
        rmcp::model::CallToolResult::success(vec![Content::text(visual)])
    }
    // ...
}
```

Do the same for `dcp_stats`, `dcp_sweep`, and `dcp_context` tool results — replace raw JSON with formatted output using `format_stats_header()`, `format_progress_bar()`, etc.

### Files to Create/Modify
| File | Action |
|------|--------|
| `crates/dcp-notification/src/format.rs` | Add `truncate()`, `extract_parameter_key()`, `format_pruned_items_list()`, `format_pruning_result_for_tool()` |
| `crates/dcp-notification/src/notification.rs` | Add `build_compress_visual_output()` |
| `crates/dcp-notification/Cargo.toml` | Add `regex`, `serde_json` deps |
| `crates/dcp-mcp/src/main.rs` | Replace raw JSON returns with visual formatting calls |

---

## Implementation Order

Recommended order (dependencies):

```
Item 2 (Glob tool-name protection)
  ↓
Item 1 (getFilePathsFromParameters)  ← needs Item 2's updated ToolProtection
  ↓
Item 3 (Compression timing)          ← independent, can be parallel with 1
  ↓
Item 5 (Visual output)               ← needs Item 3's duration data
  ↓
Item 4 (Manual mode tool)            ← independent, can be parallel with 5
```

Items 1+2 and 3 can be done in parallel. Item 5 depends on Item 3 for duration display. Item 4 is fully independent.

---

## Testing Checklist

After implementation, verify:

- [ ] `extract_file_paths("apply_patch", {...})` returns correct paths from patchText
- [ ] `extract_file_paths("multiedit", {...})` returns top-level + nested filePaths
- [ ] `ToolProtection::new(["mcp__*"])` matches `mcp__filesystem__read`
- [ ] `ToolProtection::new(["todo*"])` matches `todowrite`, `todoread`
- [ ] `ToolProtection::new(["read"])` still matches exact `read`
- [ ] `resolve_compression_duration()` returns correct ms
- [ ] `manual_toggle` MCP tool toggles state correctly
- [ ] `compress` tool returns visual output instead of raw JSON
- [ ] `dcp_stats` returns formatted stats header
- [ ] `dcp_context` returns formatted context breakdown
- [ ] All existing tests still pass: `cargo test --workspace`
