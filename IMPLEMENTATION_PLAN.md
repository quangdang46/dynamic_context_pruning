# Implementation Plan: Missing Features (TS → Rust Parity)

> **Status**: Updated — Phase A–H COMPLETE, Phase I partial, Phase J remaining
> **Created**: 2026-05-28
> **Updated**: 2026-05-29
> **Scope**: All modules from TypeScript upstream not yet implemented in Rust

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Phase Status Overview](#2-phase-status-overview)
3. [Phase I: Remaining CLI Subcommands](#3-phase-i-remaining-cli-subcommands)
4. [Phase J: Documentation & Assets](#4-phase-j-documentation-assets)
5. [Completed Phases Reference (A–H)](#5-completed-phases-reference-ah)

---

## 1. Executive Summary

### Original scope
19 TypeScript modules/files had no Rust equivalent across 10 phases (A–J).

### Current status
**Phase A–H: COMPLETE** (23 beads closed). All 3 new crates created, all missing modules implemented.

| Phase | Module | Status | Lines |
|---|---|---|---|
| A: Permissions | `dcp-permissions` (auth, host_permissions, compress_permission) | ✅ Done | 757 |
| B: Message IDs | `dcp-state/message_refs.rs` (full TS parity) | ✅ Done | 504 |
| C: Messages | `dcp-messages` (query, shape, sync, inject, inject_utils, priority, utils) | ✅ Done | 3,382 |
| D: Subagents | `dcp-messages/subagents.rs` | ✅ Done | 366 |
| E: Reasoning Strip | `dcp-messages/reasoning_strip.rs` | ✅ Done | 205 |
| F: Prompt Extensions | `dcp-prompts` (extensions/nudge, system, tool + store.rs) | ✅ Done | ~758 |
| G: Notifications | `dcp-notification` (format.rs, notification.rs) | ✅ Done | 475 |
| H: JSON Schema | `dcp-config` (LimitValue, generate-schema binary) | ✅ Done | ~340 |
| **I: CLI Scripts** | **3 of 6 subcommands done, db.rs + 3 commands remaining** | **🔲 Partial** | — |
| **J: Docs** | **Assets, CONTRIBUTING.md, README updates** | **🔲 Not started** | — |

### What's still missing
- **db.rs** — Shared SQLite data access layer for CLI analytics subcommands
- **get-message** CLI subcommand — Retrieve full message payloads by ID
- **token-stats** CLI subcommand — Cross-session token aggregation
- **message-tokens** CLI subcommand — Per-message token counting
- **CONTRIBUTING.md** — Contribution guide
- **assets/images/** — Demo screenshots from upstream
- **README.md** update — Reflect new crates and CLI subcommands

---

## 2. Phase Status Overview

### Beads status
- **23 closed** (Phase A–H scaffolds + implementations)
- **5 open** (Phase I1–I4 + Phase J)
- **0 dependency cycles**

### Current bead graph
```
I1 (db.rs) ← READY, no blockers
├── I2 (get-message)
├── I3 (token-stats)
└── I4 (message-tokens)
    └── J (docs/assets)
```

---

## 3. Phase I: Remaining CLI Subcommands

**Already done** (3 subcommands): `stats`, `find-session`, `timeline`

**Remaining** (1 shared module + 3 subcommands):

### I1: `crates/dcp-cli/src/db.rs` — SQLite Data Access Layer

Port of `scripts/opencode_api.py`. Shared data access layer for all CLI analytics subcommands.

```rust
pub struct OpencodeDb { conn: rusqlite::Connection }

impl OpencodeDb {
    pub fn open(path: Option<&str>) -> Result<Self>;  // ~/.local/share/opencode/opencode.db
    pub fn list_projects(&self) -> Result<Vec<ProjectRow>>;
    pub fn list_sessions(&self, directory: Option<&str>, limit: usize) -> Result<Vec<SessionRow>>;
    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionRow>>;
    pub fn get_session_messages(&self, session_id: &str) -> Result<Vec<MessageWithParts>>;
    pub fn get_session_message(&self, session_id: &str, message_id: &str) -> Result<Option<MessageWithParts>>;
}
```

New dependency: `rusqlite = { version = "0.31", features = ["bundled"], optional = true }` feature-gated as `scripts`.

Tasks:
1. Add rusqlite to dcp-cli/Cargo.toml
2. Create db.rs with OpencodeDb
3. Refactor existing commands (stats, find-session, timeline) to use db.rs
4. Add helper types: ProjectRow, SessionRow, MessageWithParts

Tests: open default/custom/missing path, list projects/sessions, get session/message found/not-found.

### I2: `crates/dcp-cli/src/commands/get_message.rs` — Get Message

```bash
dcp get-message <message-id> [message-id ...] [--session ID] [--scan-sessions N]
```

- With `--session`: direct lookup. Without: scan up to 200 recent sessions.
- Single result: print object. Multiple: print array. Not found: `{error: "message_not_found"}`.

### I3: `crates/dcp-cli/src/commands/token_stats.rs` — Token Stats

```bash
dcp token-stats [--sessions N] [--session ID] [--json]
```

- Aggregate token usage across N recent sessions (default 10)
- Per-session: input, output, reasoning, cache read/write, cost, finish reasons
- Grand totals with averages
- Formatted table or JSON output

### I4: `crates/dcp-cli/src/commands/message_tokens.rs` — Message Tokens

```bash
dcp message-tokens [--session ID] [--json] [--no-color]
```

- Per-message token counts for a session
- Tokenize via dcp-tokens or len/4 fallback
- Table with token counts, size bars, message previews
- Highlight 5 largest messages

---

## 4. Phase J: Documentation & Assets

### J1: Copy demo screenshots from upstream
Download to `assets/images/`:
- dcp-demo.png through dcp-demo9.png
- "3.0 release.png"

### J2: Write CONTRIBUTING.md
- Build: `cargo build --workspace`
- Test: `cargo test --workspace`
- Lint: `cargo fmt --check`, `cargo clippy -- -D warnings`
- PR process

### J3: Update README.md
- New crates (dcp-permissions, dcp-messages, dcp-notification)
- CLI subcommand reference
- JSON Schema link
- Updated architecture diagram (21 crates)

### J4: Update PLAN.md
Mark all phases complete.

---

## 5. Completed Phases Reference (A–H)

> These phases are DONE. The details below are kept for historical reference.

### Phase A: Auth & Permissions — `crates/dcp-permissions/`

| File | Lines | Description |
|---|---|---|
| `src/auth.rs` | ~100 | HTTP Basic Auth (env vars, base64 header) |
| `src/host_permissions.rs` | ~350 | Permission resolution engine (wildcard matching, last-match-wins) |
| `src/compress_permission.rs` | ~100 | Thin adapter for compress permission |
| `src/lib.rs` | ~207 | Re-exports + integration tests |

### Phase B: Message IDs — `crates/dcp-state/src/message_refs.rs`

504 lines, 25+ tests. Full TS parity: format_message_ref, parse_boundary_id, format_message_id_tag, assign_message_refs, BoundaryId enum.

### Phase C: Messages Module — `crates/dcp-messages/`

| File | Lines | Description |
|---|---|---|
| `src/query.rs` | ~150 | getLastUserMessage, isIgnored, messageHasCompress |
| `src/shape.rs` | ~80 | Message validation, filter_messages |
| `src/sync.rs` | ~200 | syncCompressionBlocks |
| `src/inject.rs` | ~350 | injectCompressNudges, injectMessageIds |
| `src/inject_utils.rs` | ~450 | Anchor management, limit checking, nudge application |
| `src/priority.rs` | ~120 | Message priority classification |
| `src/utils.rs` | ~350 | Synthetic messages, hallucination strip, tool ID list |
| `src/prune.rs` | ~200 | Prune pipeline (filterCompressedRanges, pruneToolOutputs) |
| `src/subagents.rs` | ~366 | Subagent result expansion |
| `src/reasoning_strip.rs` | ~205 | Strip stale provider metadata |

### Phase D: Subagents — `crates/dcp-messages/src/subagents.rs`

366 lines. getSubAgentId, buildSubagentResultText, mergeSubagentResult, injectExtendedSubAgentResults.

### Phase E: Reasoning Strip — `crates/dcp-messages/src/reasoning_strip.rs`

205 lines. stripStaleMetadata — removes provider metadata from mismatched-model parts.

### Phase F: Prompt Extensions & Store — `crates/dcp-prompts/`

| File | Lines | Description |
|---|---|---|
| `src/extensions/nudge.rs` | 210 | buildCompressedBlockGuidance, renderMessagePriorityGuidance |
| `src/extensions/system.rs` | 130 | MANUAL_MODE/SUBAGENT extensions, buildProtectedToolsExtension |
| `src/extensions/tool.rs` | 98 | RANGE_FORMAT/MESSAGE_FORMAT extensions |
| `src/store.rs` | 320 | PromptStore with 3-tier override cascade, RuntimePrompts |

### Phase G: Notifications — `crates/dcp-notification/`

| File | Lines | Description |
|---|---|---|
| `src/format.rs` | ~300 | formatStatsHeader, formatTokenCount, formatProgressBar, extractParameterKey |
| `src/notification.rs` | ~175 | sendUnifiedNotification, sendCompressNotification |

### Phase H: JSON Schema — `crates/dcp-config/`

- `LimitValue` enum (was `ContextLimit` in plan) — supports usize + percentage strings
- `json_schema()` function — generates schema from Rust types
- `src/bin/generate-schema.rs` — binary that writes dcp.schema.json
- Full schemars support with custom JsonSchema derives

---

## Workspace Status

### 21 crates total
```
crates/
├── dcp-types/            # Canonical IR
├── dcp-traits/           # Tokenizer, StatePersistence, etc.
├── dcp-tokens/           # Tokenizer implementations
├── dcp-protected/        # Glob matcher, protected patterns
├── dcp-state/            # SessionState, message refs, tool cache
├── dcp-storage/          # FileStateStore, InMemoryStore
├── dcp-prune/            # Dedup, purge_errors, stale_file_reads
├── dcp-compress/         # Range/message modes, block bookkeeping
├── dcp-prompts/          # 6 prompts + extensions + PromptStore ✅
├── dcp-nudges/           # Nudge system
├── dcp-config/           # JSONC parse + cascade + schema ✅
├── dcp-telemetry/        # Metrics, observers
├── dcp-core/             # ContextPruner facade
├── dcp-permissions/      # Auth + host permissions ✅ NEW
├── dcp-messages/         # Message processing pipeline ✅ NEW
├── dcp-notification/     # UI notification formatting ✅ NEW
├── dcp-mcp/              # MCP server binary
├── dcp-cli/              # CLI with subcommands (3/6 analytics done)
├── dcp-claude-hook/      # Claude Code hook binary
├── dcp-rig/              # Rig adapter
└── dynamic_context_pruning/  # Umbrella crate
```

### Estimated remaining work
| Task | Lines | Effort |
|---|---|---|
| I1: db.rs | ~200 | 0.5 day |
| I2: get-message | ~150 | 0.5 day |
| I3: token-stats | ~200 | 0.5 day |
| I4: message-tokens | ~250 | 0.5 day |
| J: Docs + assets | ~200 | 0.5 day |
| **Total** | **~1000** | **~2.5 days** |
