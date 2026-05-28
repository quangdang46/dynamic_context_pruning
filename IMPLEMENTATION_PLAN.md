# Implementation Plan: Missing Features (TS → Rust Parity)

> **Status**: Draft
> **Created**: 2026-05-28
> **Scope**: All modules from TypeScript upstream not yet implemented in Rust

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Architecture Decisions](#2-architecture-decisions)
3. [Phase A: Auth & Permissions](#3-phase-a-auth--permissions)
4. [Phase B: Message-ID System](#4-phase-b-message-id-system)
5. [Phase C: Messages Module (query, shape, sync, inject)](#5-phase-c-messages-module)
6. [Phase D: Subagent Results](#6-phase-d-subagent-results)
7. [Phase E: Reasoning Strip](#7-phase-e-reasoning-strip)
8. [Phase F: Prompt Extensions & Store](#8-phase-f-prompt-extensions--store)
9. [Phase G: UI Notification System](#9-phase-g-ui-notification-system)
10. [Phase H: JSON Schema & Config Validation](#10-phase-h-json-schema)
11. [Phase I: Utility Scripts (Rust CLI subcommands)](#11-phase-i-utility-scripts)
12. [Phase J: Documentation & Assets](#12-phase-j-documentation-assets)
13. [Dependency & Build Order](#13-dependency-build-order)
14. [Testing Requirements](#14-testing-requirements)
15. [Estimated Effort](#15-estimated-effort)

---

## 1. Executive Summary

19 TypeScript modules/files have no Rust equivalent. This plan covers all of them in 10 phases (A-J), organized by dependency order.

**What's already in place**: 18 Rust crates scaffolded, core types, traits, config, prompts (basic), prune strategies, compress logic, state management, and CLI/MCP binaries.

**What's missing**: Auth/permissions, message-ID injection, message query/shape/sync/inject, subagent results, reasoning strip, prompt extensions/store, UI notifications, JSON schema, utility scripts, and documentation assets.

**Strategy**: Add to existing crates where possible; create 2 new crates only when the scope warrants it.

---

## 2. Architecture Decisions

### 2.1 New crates vs extending existing

| Module | Decision | Target location | Rationale |
|---|---|---|---|
| auth + host-permissions + compress-permission | **New crate** | `crates/dcp-permissions/` | Distinct concern (auth/permissions), clean dependency boundary |
| message-ids | Extend existing | `crates/dcp-state/src/message_ids.rs` | Already has `message_refs.rs`, tightly coupled to state |
| messages/* (query, shape, sync, inject) | **New crate** | `crates/dcp-messages/` | 8 files of message processing logic, too much for dcp-core |
| subagent-results | Extend existing | `crates/dcp-messages/src/subagents.rs` | Part of message processing |
| reasoning-strip | Extend existing | `crates/dcp-messages/src/reasoning_strip.rs` | Part of message processing |
| prompts/extensions/* + store | Extend existing | `crates/dcp-prompts/` | Already exists, just needs extensions |
| UI notifications | **New crate** | `crates/dcp-notification/` | Formatting + delivery, separate concern |
| JSON schema | Extend existing | `crates/dcp-config/` | Already has schemars dep |
| Utility scripts | Extend existing | `crates/dcp-cli/src/` | Already a CLI, add subcommands |
| update.ts | **Skip** | N/A | npm-specific; adapt to crates.io later if needed |

### 2.2 New crate structure

```
crates/
├── dcp-permissions/          # NEW: auth + host-permissions + compress-permission
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── auth.rs            # Basic auth, env var checks
│       ├── host_permissions.rs # Permission resolution engine
│       └── compress_permission.rs # Thin adapter for compress permission
│
├── dcp-messages/              # NEW: message processing pipeline
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── query.rs           # getLastUserMessage, isIgnored, messageHasCompress
│       ├── shape.rs           # Type guards, filterMessages
│       ├── sync.rs            # syncCompressionBlocks
│       ├── inject.rs          # injectCompressNudges, injectMessageIds
│       ├── inject_utils.rs    # Anchor management, limit checking
│       ├── subagents.rs       # Subagent result expansion
│       ├── reasoning_strip.rs # Strip stale provider metadata
│       ├── utils.rs           # Synthetic messages, hallucination strip
│       └── priority.rs        # buildPriorityMap, classifyMessagePriority
│
├── dcp-notification/          # NEW: UI notifications
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── notification.rs    # sendUnifiedNotification, sendCompressNotification
│       └── format.rs          # formatStatsHeader, formatTokenCount, formatProgressBar
│
└── (existing crates extended)
```

### 2.3 Updated dependency graph

```
                    ┌─────────────────────────────────┐
                    │  dynamic_context_pruning        │
                    └────────────────┬────────────────┘
                                     │
                              ┌──────▼──────┐
                              │  dcp-core   │
                              └──┬─┬─┬─┬─┬──┘
                                 │ │ │ │ │
        ┌────────────────────────┘ │ │ │ └────────────────┐
        │                          │ │ │                  │
   ┌────▼────┐  ┌─────────┐  ┌───▼─▼─▼───┐  ┌────────┐ ┌▼──────────────┐
   │dcp-prune│ │dcp-compress│ │dcp-nudges │ │dcp-state│ │dcp-messages   │ ← NEW
   └────┬────┘ └─────┬───┘  └─────┬──────┘ └────┬───┘ └──────┬────────┘
        │            │            │              │            │
        └────┬───────┴────────────┴──────────────┴────────────┘
             │
        ┌────▼────────┐  ┌──────────┐  ┌────────────────┐
        │dcp-protected│ │dcp-storage│ │dcp-permissions │ ← NEW
        └────┬────────┘ └─────┬────┘ └──────┬─────────┘
             │                │             │
        ┌────▼────────────────▼─────────────▼──┐
        │        dcp-traits                    │
        └────┬─────────┬───────────────────────┘
             │         │
        ┌────▼───┐  ┌──▼────────┐  ┌───────────────┐
        │dcp-tokens│ │dcp-config │ │dcp-notification│ ← NEW
        └────┬───┘  └─────┬─────┘ └───────┬────────┘
             │            │               │
        ┌────▼────────────▼───────────────▼──┐
        │          dcp-types                 │
        └────────────────────────────────────┘
```

### 2.4 Workspace Cargo.toml additions

```toml
[workspace]
members = [
    # ... existing 18 members ...
    "crates/dcp-permissions",
    "crates/dcp-messages",
    "crates/dcp-notification",
]

[workspace.dependencies]
# New external deps needed
base64 = "0.22"          # for auth Basic encoding
regex = "1"              # for message ID parsing, hallucination strip
rusqlite = { version = "0.31", optional = true }  # for scripts/opencode_api equivalent
sha2 = "0.10"            # for deterministic ID generation

# New internal deps
dcp-permissions = { path = "crates/dcp-permissions", version = "=0.1.0" }
dcp-messages = { path = "crates/dcp-messages", version = "=0.1.0" }
dcp-notification = { path = "crates/dcp-notification", version = "=0.1.0" }
```

---

## 3. Phase A: Auth & Permissions

**Target**: `crates/dcp-permissions/`
**Depends on**: `dcp-types`, `dcp-traits`
**Estimated files**: 4 new files

### A1: `dcp-permissions/src/auth.rs`

Port of `lib/auth.ts`.

```rust
// Public API:
pub fn is_secure_mode() -> bool;
pub fn get_authorization_header() -> Option<String>;
pub fn configure_client_auth(client: &reqwest::Client) -> reqwest::Client;
```

Implementation notes:
- `is_secure_mode()` → `std::env::var("OPENCODE_SERVER_PASSWORD").is_ok_and(|v| !v.is_empty())`
- `get_authorization_header()` → `format!("Basic {}", base64::encode(format!("{}:{}", username, password)))`
- Username defaults to `"opencode"` via `OPENCODE_SERVER_USERNAME` env var
- `configure_client_auth` adds a default `Authorization` header to reqwest client builder

### A2: `dcp-permissions/src/host_permissions.rs`

Port of `lib/host-permissions.ts`.

```rust
// Types:
pub enum PermissionAction { Ask, Allow, Deny }
pub enum PermissionValue { Simple(PermissionAction), Patterned(HashMap<String, PermissionAction>) }
pub struct HostPermissionSnapshot {
    pub global: Option<HashMap<String, PermissionValue>>,
    pub agents: HashMap<String, Option<HashMap<String, PermissionValue>>>,
}

// Public API:
pub fn compress_disabled_by_opencode(configs: &[Option<&HashMap<String, PermissionValue>>]) -> bool;
pub fn resolve_effective_compress_permission(
    base: PermissionAction,
    host: &HostPermissionSnapshot,
    agent_name: Option<&str>,
) -> PermissionAction;
pub fn has_explicit_tool_permission(
    config: &Option<HashMap<String, PermissionValue>>,
    tool: &str,
) -> bool;

// Internal:
fn wildcard_match(value: &str, pattern: &str) -> bool;
fn find_last_matching_rule<F>(rules: &[PermissionRule], predicate: F) -> Option<&PermissionRule>;
fn get_permission_rules(configs: &[Option<&HashMap<String, PermissionValue>>]) -> Vec<PermissionRule>;
```

Key logic:
- Wildcard matching: normalize `\` to `/`, escape regex metachars, convert `*` → `.*`, `?` → `.`
- Last-match-wins: iterate rules in reverse, return first match
- Platform-aware case sensitivity via `cfg!(target_os = "windows")`

### A3: `dcp-permissions/src/compress_permission.rs`

Port of `lib/compress-permission.ts`.

```rust
// Public API:
pub fn compress_permission(state: &SessionState, config: &Config) -> PermissionAction;
pub fn sync_compress_permission_state(
    state: &mut SessionState,
    config: &Config,
    host_permissions: &HostPermissionSnapshot,
    messages: &[Message],
);
```

Key logic:
- `compress_permission()` → returns `state.compress_permission` if set, else falls back to `config.compress.permission`
- `sync_compress_permission_state()` → extracts agent from last user message, resolves permission, stores in state

### A4: `dcp-permissions/src/lib.rs`

Re-exports all public items from submodules.

---

## 4. Phase B: Message-ID System

**Target**: `crates/dcp-state/src/message_ids.rs` (extend existing `message_refs.rs`)
**Depends on**: `dcp-types`, `dcp-messages` (for `isIgnoredUserMessage`)
**Estimated files**: 1 file (extend from ~50 to ~200 lines)

### B1: Extend `message_refs.rs` with full TS parity

Port of `lib/message-ids.ts`.

```rust
// Constants:
pub const MESSAGE_REF_REGEX: &str = r"^m(\d{4})$";
pub const BLOCK_REF_REGEX: &str = r"^b([1-9]\d*)$";
pub const MESSAGE_REF_WIDTH: usize = 4;
pub const MESSAGE_REF_MIN_INDEX: u16 = 1;
pub const MESSAGE_REF_MAX_INDEX: u16 = 9999;

// Types:
pub enum ParsedBoundaryId {
    Message { ref_id: String, index: u16 },
    CompressedBlock { ref_id: String, block_id: u32 },
}

// Public API (additions to existing):
pub fn format_message_ref(index: u16) -> Result<String, Error>;
pub fn format_block_ref(block_id: u32) -> Result<String, Error>;
pub fn parse_message_ref(s: &str) -> Option<u16>;
pub fn parse_block_ref(s: &str) -> Option<u32>;
pub fn parse_boundary_id(id: &str) -> Option<ParsedBoundaryId>;
pub fn format_message_id_tag(
    ref_id: &str,
    attributes: Option<&HashMap<String, String>>,
) -> String;
pub fn assign_message_refs(
    state: &mut SessionState,
    messages: &[Message],
) -> usize;

// Internal:
fn escape_xml_attribute(value: &str) -> String;
fn allocate_next_message_ref(state: &mut SessionState) -> Result<String, Error>;
```

Key logic:
- `format_message_id_tag`: XML generation with sorted attributes, XML entity escaping
- `assign_message_refs`: walk messages, skip ignored users, skip first subagent prompt, allocate sequentially
- `allocate_next_message_ref`: linear scan from `next_ref`, throw at 9999

---

## 5. Phase C: Messages Module

**Target**: New crate `crates/dcp-messages/`
**Depends on**: `dcp-types`, `dcp-traits`, `dcp-config`, `dcp-state`, `dcp-tokens`, `dcp-prompts`, `dcp-permissions`
**Estimated files**: 10 new files

### C1: `query.rs` — Port of `lib/messages/query.ts`

```rust
pub fn get_last_user_message(messages: &[Message], start_index: Option<usize>) -> Option<&Message>;
pub fn message_has_compress(message: &Message) -> bool;
pub fn is_ignored_user_message(message: &Message) -> bool;
pub fn is_protected_user_message(config: &Config, message: &Message) -> bool;
```

### C2: `shape.rs` — Port of `lib/messages/shape.ts`

```rust
pub fn is_valid_message(msg: &Message) -> bool;
pub fn filter_messages(messages: Vec<Message>) -> Vec<Message>;
pub fn filter_messages_in_place(messages: &mut Vec<Message>);
```

Validation: non-empty `id`, non-empty `session_id`, role is User/Assistant, `time.created` is numeric, `parts` is a vec.

### C3: `sync.rs` — Port of `lib/messages/sync.ts`

```rust
pub fn sync_compression_blocks(
    state: &mut SessionState,
    messages: &[Message],
);
```

Algorithm:
1. Build `Set<message_id>` from current messages
2. Clear `active_block_ids` and `active_by_anchor_message_id`
3. Sort blocks by `created_at` then `block_id`
4. For each block: if origin missing → deactivate; if `deactivated_by_user` → deactivate; for each consumed → deactivate consumed; else → activate
5. Rebuild per-message `active_block_ids` from global active set

### C4: `inject.rs` — Port of `lib/messages/inject/inject.ts`

```rust
pub fn inject_compress_nudges(
    state: &mut SessionState,
    config: &Config,
    messages: &mut Vec<Message>,
    prompts: &RuntimePrompts,
    priorities: Option<&CompressionPriorityMap>,
);

pub fn inject_message_ids(
    state: &mut SessionState,
    config: &Config,
    messages: &mut Vec<Message>,
    priorities: Option<&CompressionPriorityMap>,
);
```

Nudge injection (3-tier):
- Tier 1 (overMaxLimit): contextLimitAnchors
- Tier 2 (overMinLimit): turnNudgeAnchors + iterationNudgeAnchors
- Resets lower tiers when context drops below minLimit
- Clears all if last assistant has completed compress

Message ID injection:
- Append `<dcp-message-id ref="mNNNN">` tag to message parts
- User messages: append to last text part or create synthetic
- Assistant messages: try tool parts first, then text, then synthetic

### C5: `inject_utils.rs` — Port of `lib/messages/inject/utils.ts`

```rust
pub struct LastUserModelContext {
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

pub fn get_nudge_frequency(config: &Config) -> usize;
pub fn get_iteration_nudge_threshold(config: &Config) -> usize;
pub fn find_last_non_ignored_message(messages: &[Message]) -> Option<(usize, &Message)>;
pub fn count_messages_after_index(messages: &[Message], index: usize) -> usize;
pub fn get_model_info(messages: &[Message]) -> LastUserModelContext;
pub fn is_context_over_limits(
    config: &Config, state: &SessionState,
    provider_id: Option<&str>, model_id: Option<&str>,
    messages: &[Message],
) -> (bool, bool); // (over_max, over_min)

pub fn add_anchor(
    anchor_set: &mut HashSet<String>,
    message_id: &str,
    message_index: usize,
    messages: &[Message],
    interval: usize,
) -> bool;

pub fn apply_anchored_nudges(
    state: &mut SessionState,
    config: &Config,
    messages: &mut Vec<Message>,
    prompts: &RuntimePrompts,
    priorities: Option<&CompressionPriorityMap>,
);
```

Context limit resolution:
- Per-model limit (keyed by `provider/model`) > global limit
- Supports both absolute token counts and percentage strings (`"80%"`)
- Percentage clamped to [0, 100], computed against `state.model_context_limit`
- Summary buffer added to max limit when `config.compress.summary_buffer` is true

### C6: `priority.rs` — Port of `lib/messages/priority.ts`

```rust
pub type MessagePriority = &'static str; // "low" | "medium" | "high"

pub struct CompressionPriorityEntry {
    pub ref_id: String,
    pub token_count: usize,
    pub priority: MessagePriority,
}

pub type CompressionPriorityMap = HashMap<String, CompressionPriorityEntry>;

pub fn build_priority_map(
    config: &Config, state: &SessionState, messages: &[Message],
) -> CompressionPriorityMap;
pub fn classify_message_priority(token_count: usize) -> MessagePriority;
pub fn list_priority_refs_before_index(
    messages: &[Message], priorities: &CompressionPriorityMap,
    anchor_index: usize, priority: MessagePriority,
) -> Vec<String>;
```

Thresholds: 5000+ = high, 500+ = medium, else low.

### C7: `utils.rs` — Port of `lib/messages/utils.ts`

```rust
pub fn create_synthetic_user_message(
    base: &Message, content: &str, stable_seed: Option<&str>,
) -> Message;
pub fn create_synthetic_text_part(
    base: &Message, content: &str, stable_seed: Option<&str>,
) -> Part;
pub fn append_to_last_text_part(message: &mut Message, injection: &str) -> bool;
pub fn append_to_all_tool_parts(message: &mut Message, tag: &str) -> bool;
pub fn has_content(message: &Message) -> bool;
pub fn build_tool_id_list(state: &SessionState, messages: &[Message]) -> Vec<String>;
pub fn replace_block_ids_with_blocked(text: &str) -> String;
pub fn strip_hallucinations_from_string(text: &str) -> String;
pub fn strip_hallucinations(messages: &mut Vec<Message>);
```

ID generation uses SHA256 truncated to 16 hex chars with prefix:
- `msg_dcp_summary_` for synthetic messages
- `prt_dcp_summary_` for synthetic parts
- `prt_dcp_text_` for synthetic text parts

Regex constants for DCP tag matching:
- `DCP_BLOCK_ID_TAG_REGEX`
- `DCP_PAIRED_TAG_REGEX`
- `DCP_UNPAIRED_TAG_REGEX`

---

## 6. Phase D: Subagent Results

**Target**: `crates/dcp-messages/src/subagents.rs`
**Depends on**: `dcp-types`, `dcp-state`

### D1: Port of `lib/subagents/subagent-results.ts` + `lib/messages/inject/subagent-results.ts`

```rust
pub fn get_sub_agent_id(part: &Part) -> Option<String>;
pub fn build_subagent_result_text(messages: &[Message]) -> String;
pub fn merge_subagent_result(output: &str, sub_agent_result_text: &str) -> String;
pub async fn inject_extended_sub_agent_results(
    client: &dyn SessionClient,
    state: &mut SessionState,
    messages: &mut Vec<Message>,
    allow_sub_agents: bool,
);
```

Key logic:
- `build_subagent_result_text`: filter to assistant messages, get last text; if 2+ messages and second-to-last has compress tool, concatenate both
- `merge_subagent_result`: regex replace `<task_result>...</task_result>` content (case-insensitive, non-greedy)
- `inject_extended_sub_agent_results`: iterate messages for completed `task` tool calls, skip pruned, check cache, fetch from client, build result, cache and merge

---

## 7. Phase E: Reasoning Strip

**Target**: `crates/dcp-messages/src/reasoning_strip.rs`
**Depends on**: `dcp-types`

### E1: Port of `lib/messages/reasoning-strip.ts`

```rust
pub fn strip_stale_metadata(messages: &mut Vec<Message>);
```

Algorithm:
1. Find last user message → extract provider/model
2. For each assistant message with different model/provider → strip `metadata` from text, tool, and reasoning parts
3. In Rust: remove the `metadata` field from matching `Part` variants

---

## 8. Phase F: Prompt Extensions & Store

**Target**: `crates/dcp-prompts/` (extend existing)
**Depends on**: `dcp-types`, `dcp-state`
**Estimated files**: 4 new files, 1 modified

### F1: `src/extensions/nudge.rs` — Port of `lib/prompts/extensions/nudge.ts`

```rust
pub fn build_compressed_block_guidance(state: &SessionState) -> String;
pub fn render_message_priority_guidance(priority_label: &str, refs: &[String]) -> String;
pub fn append_guidance_to_dcp_tag(nudge_text: &str, guidance: &str) -> String;
```

### F2: `src/extensions/system.rs` — Port of `lib/prompts/extensions/system.ts`

```rust
pub const MANUAL_MODE_SYSTEM_EXTENSION: &str = "..."; // XML block
pub const SUBAGENT_SYSTEM_EXTENSION: &str = "...";    // XML block
pub fn build_protected_tools_extension(protected_tools: &[String]) -> String;
```

### F3: `src/extensions/tool.rs` — Port of `lib/prompts/extensions/tool.ts`

```rust
pub const RANGE_FORMAT_EXTENSION: &str = "...";
pub const MESSAGE_FORMAT_EXTENSION: &str = "...";
```

### F4: `src/store.rs` — Port of `lib/prompts/store.ts`

```rust
pub type PromptKey = &'static str;
// "system" | "compress-range" | "compress-message" | "context-limit-nudge" | "turn-nudge" | "iteration-nudge"

pub struct RuntimePrompts {
    pub system: String,
    pub compress_range: String,
    pub compress_message: String,
    pub context_limit_nudge: String,
    pub turn_nudge: String,
    pub iteration_nudge: String,
    pub manual_extension: String,
    pub subagent_extension: String,
}

pub const PROMPT_KEYS: &[PromptKey] = &[
    "system", "compress-range", "compress-message",
    "context-limit-nudge", "turn-nudge", "iteration-nudge",
];

pub struct PromptStore {
    // internal
}

impl PromptStore {
    pub fn new(working_directory: &Path, custom_prompts_enabled: bool) -> Self;
    pub fn get_runtime_prompts(&self) -> RuntimePrompts;
    pub fn reload(&mut self);
}
```

Override cascade (highest priority first):
1. Project: `.opencode/dcp-prompts/overrides/<filename>`
2. Config dir: `$OPENCODE_CONFIG_DIR/dcp-prompts/overrides/<filename>`
3. Global: `$XDG_CONFIG_HOME/opencode/dcp-prompts/overrides/<filename>`

Normalization pipeline:
- Strip BOM, normalize line endings
- Remove HTML comments and `// ... //` inline comments
- Strip `<manual>...</manual>` and `<subagent>...</subagent>` blocks (handled as extensions)
- Unwrap outer `<dcp-system-reminder>...</dcp-system-reminder>` for editing
- Re-wrap at runtime for non-compress prompts

---

## 9. Phase G: UI Notification System

**Target**: New crate `crates/dcp-notification/`
**Depends on**: `dcp-types`, `dcp-state`, `dcp-config`, `dcp-messages`, `dcp-tokens`
**Estimated files**: 3 new files

### G1: `src/format.rs` — Port of `lib/ui/utils.ts`

```rust
pub fn format_stats_header(total_tokens_saved: u64, prune_token_counter: u64) -> String;
pub fn format_token_count(tokens: u64, compact: bool) -> String;
pub fn truncate(s: &str, max_len: usize) -> String;
pub fn format_progress_bar(
    message_ids: &[String], pruned_messages: &HashSet<String>,
    recent_message_ids: &[String], width: usize,
) -> String;
pub fn cache_system_prompt_tokens(state: &mut SessionState, messages: &[Message]);
pub fn shorten_path(input: &str, working_directory: Option<&str>) -> String;
pub fn format_pruned_items_list(
    prune_tool_ids: &[String], tool_metadata: &HashMap<String, ToolParameterEntry>,
    working_directory: Option<&str>,
) -> Vec<String>;
pub fn format_pruning_result_for_tool(
    pruned_ids: &[String], tool_metadata: &HashMap<String, ToolParameterEntry>,
    working_directory: Option<&str>,
) -> String;
```

`extract_parameter_key` — large match on tool names:
- `read`/`write`/`edit`: file path
- `bash`: description or truncated command
- `glob`/`grep`: pattern with optional path
- `webfetch`/`websearch`: URL or query
- `task`/`skill`: description or name
- Fallback: first 50 chars of JSON

### G2: `src/notification.rs` — Port of `lib/ui/notification.ts`

```rust
pub type PruneReason = &'static str; // "completion" | "noise" | "extraction"

pub fn send_unified_notification(
    state: &SessionState,
    config: &Config,
    prune_tool_ids: &[String],
    tool_metadata: &HashMap<String, ToolParameterEntry>,
    reason: PruneReason,
    working_directory: Option<&str>,
) -> String; // Returns formatted notification string

pub fn send_compress_notification(
    state: &SessionState,
    config: &Config,
    entries: &[CompressionNotificationEntry],
    batch_topic: Option<&str>,
    session_message_ids: &[String],
) -> String; // Returns formatted notification string

pub struct CompressionNotificationEntry {
    pub block_id: u32,
    pub run_id: u32,
    pub summary: String,
    pub summary_tokens: u64,
}
```

Note: The TS version sends via `client.tui.showToast()` or `sendIgnoredMessage()`. In Rust, we produce formatted strings and let the host decide how to deliver (toast, chat, stderr, etc.).

### G3: `src/lib.rs`

Re-exports + PruneReason labels.

---

## 10. Phase H: JSON Schema & Config Validation

**Target**: `crates/dcp-config/` (extend existing)
**Depends on**: `schemars`

### H1: Generate `dcp.schema.json`

- Add `#[derive(JsonSchema)]` to all config types
- Add a build.rs or a test that writes the schema to `$OUT_DIR/dcp.schema.json`
- Include the generated schema in the published crate

### H2: ContextLimit enum

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ContextLimit {
    Tokens(usize),
    Percent(String), // validated via regex: ^\d+(?:\.\d+)?%$
}
```

---

## 11. Phase I: Utility Scripts (Rust CLI subcommands)

**Target**: `crates/dcp-cli/src/` (extend existing CLI)
**Depends on**: `rusqlite` (optional), `dcp-types`

### I1: Data access layer — Port of `scripts/opencode_api.py`

```rust
// crates/dcp-cli/src/db.rs
pub struct OpencodeDb {
    conn: rusqlite::Connection,
}

impl OpencodeDb {
    pub fn open(path: Option<&str>) -> Result<Self>;
    pub fn list_projects(&self) -> Result<Vec<ProjectRow>>;
    pub fn list_sessions(&self, directory: Option<&str>, limit: usize) -> Result<Vec<SessionRow>>;
    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionRow>>;
    pub fn get_session_messages(&self, session_id: &str) -> Result<Vec<WithParts>>;
    pub fn get_session_message(&self, session_id: &str, message_id: &str) -> Result<Option<WithParts>>;
}
```

Opens `~/.local/share/opencode/opencode.db` in read-only mode.

### I2: CLI subcommands

| TS Script | Rust Subcommand | Description |
|---|---|---|
| `opencode-dcp-stats` | `dcp stats` | DCP cache impact analysis |
| `opencode-find-session` | `dcp find-session` | Session search by title |
| `opencode-get-message` | `dcp get-message` | Message retrieval by ID |
| `opencode-token-stats` | `dcp token-stats` | Cross-session token aggregation |
| `opencode-session-timeline` | `dcp timeline` | Per-session step-by-step timeline |
| `opencode-message-token-counts` | `dcp message-tokens` | Per-message token counting |

Each subcommand:
- Parses args via `clap`
- Opens `OpencodeDb`
- Queries data
- Renders formatted table output

**Skip**: `scripts/print.ts` (TS-specific DX tool), `scripts/verify-package.mjs` (npm-specific)

---

## 12. Phase J: Documentation & Assets

### J1: `assets/images/`

Copy 10 demo screenshots from upstream to `assets/images/`:
- `dcp-demo.png` through `dcp-demo9.png`
- `3.0 release.png`

### J2: `CONTRIBUTING.md`

Write a contributing guide covering:
- Build instructions (`cargo build --workspace`)
- Test instructions (`cargo test --workspace`)
- Code style (`cargo fmt`, `cargo clippy`)
- PR process

### J3: Update `README.md`

Add sections for:
- New crates (dcp-permissions, dcp-messages, dcp-notification)
- CLI subcommands (stats, timeline, find-session, etc.)
- JSON schema link

---

## 13. Dependency & Build Order

Phases must be implemented in this order due to dependencies:

```
Phase A (Permissions)  ← no deps on other new crates
    │
Phase B (Message IDs)  ← depends on Phase C1 (query.rs is_ignored_user_message)
    │                      OR: move is_ignored_user_message to dcp-state
Phase C (Messages)     ← depends on Phase A (permissions check), Phase F (prompts)
    │
Phase D (Subagents)    ← depends on Phase C
    │
Phase E (Reasoning)    ← depends on Phase C (for model info)
    │
Phase F (Prompts)      ← no deps on other new crates
    │
Phase G (Notification) ← depends on Phase C
    │
Phase H (Schema)       ← no deps on other new crates
    │
Phase I (Scripts)      ← depends on all above for data types
    │
Phase J (Docs)         ← last
```

**Recommended parallel tracks:**

Track 1 (independent): Phase A → Phase F → Phase H
Track 2 (core): Phase C → Phase B → Phase D → Phase E → Phase G
Track 3 (end): Phase I → Phase J

---

## 14. Testing Requirements

### Per-phase tests:

| Phase | Test type | Count | Description |
|---|---|---|---|
| A | Unit | 15 | Permission resolution, wildcard matching, auth env vars |
| B | Unit | 10 | Ref formatting, parsing, tag generation, allocation |
| C | Unit | 30 | Query, shape validation, sync algorithm, inject logic |
| C | Property | 5 | idempotent sync, anchor monotonicity |
| D | Unit | 5 | Subagent ID extraction, result building, merging |
| E | Unit | 3 | Metadata stripping |
| F | Unit | 10 | Prompt override cascade, normalization |
| G | Unit | 10 | Formatting functions, notification building |
| H | Unit | 3 | Schema generation, config parsing |
| I | Integration | 6 | Each CLI subcommand with test DB |
| I | Snapshot | 6 | Golden output for each command |

### Total: ~103 new test cases

---

## 15. Estimated Effort

| Phase | Files | Lines (est.) | Effort |
|---|---|---|---|
| A: Permissions | 4 | ~400 | 1 day |
| B: Message IDs | 1 | ~200 | 0.5 day |
| C: Messages | 10 | ~1500 | 3 days |
| D: Subagents | 1 | ~200 | 0.5 day |
| E: Reasoning Strip | 1 | ~50 | 0.25 day |
| F: Prompts | 5 | ~500 | 1.5 days |
| G: Notifications | 3 | ~400 | 1 day |
| H: JSON Schema | 1 | ~100 | 0.5 day |
| I: Scripts/CLI | 7 | ~800 | 2 days |
| J: Docs/Assets | 3 | ~100 | 0.5 day |
| **Total** | **36** | **~4250** | **~11 days** |

---

## Appendix: File-level mapping reference

| TS File | Rust File | Phase |
|---|---|---|
| `lib/auth.ts` | `crates/dcp-permissions/src/auth.rs` | A |
| `lib/host-permissions.ts` | `crates/dcp-permissions/src/host_permissions.rs` | A |
| `lib/compress-permission.ts` | `crates/dcp-permissions/src/compress_permission.rs` | A |
| `lib/message-ids.ts` | `crates/dcp-state/src/message_ids.rs` | B |
| `lib/messages/query.ts` | `crates/dcp-messages/src/query.rs` | C |
| `lib/messages/shape.ts` | `crates/dcp-messages/src/shape.rs` | C |
| `lib/messages/sync.ts` | `crates/dcp-messages/src/sync.rs` | C |
| `lib/messages/inject/inject.ts` | `crates/dcp-messages/src/inject.rs` | C |
| `lib/messages/inject/utils.ts` | `crates/dcp-messages/src/inject_utils.rs` | C |
| `lib/messages/inject/subagent-results.ts` | `crates/dcp-messages/src/subagents.rs` | D |
| `lib/messages/reasoning-strip.ts` | `crates/dcp-messages/src/reasoning_strip.rs` | E |
| `lib/messages/priority.ts` | `crates/dcp-messages/src/priority.rs` | C |
| `lib/messages/utils.ts` | `crates/dcp-messages/src/utils.rs` | C |
| `lib/subagents/subagent-results.ts` | `crates/dcp-messages/src/subagents.rs` | D |
| `lib/prompts/extensions/nudge.ts` | `crates/dcp-prompts/src/extensions/nudge.rs` | F |
| `lib/prompts/extensions/system.ts` | `crates/dcp-prompts/src/extensions/system.rs` | F |
| `lib/prompts/extensions/tool.ts` | `crates/dcp-prompts/src/extensions/tool.rs` | F |
| `lib/prompts/store.ts` | `crates/dcp-prompts/src/store.rs` | F |
| `lib/ui/notification.ts` | `crates/dcp-notification/src/notification.rs` | G |
| `lib/ui/utils.ts` | `crates/dcp-notification/src/format.rs` | G |
| `lib/update.ts` | **SKIP** (npm-specific) | — |
| `dcp.schema.json` | `crates/dcp-config/schema/` (generated) | H |
| `scripts/opencode_api.py` | `crates/dcp-cli/src/db.rs` | I |
| `scripts/opencode-dcp-stats` | `crates/dcp-cli/src/cmd/stats.rs` | I |
| `scripts/opencode-find-session` | `crates/dcp-cli/src/cmd/find_session.rs` | I |
| `scripts/opencode-get-message` | `crates/dcp-cli/src/cmd/get_message.rs` | I |
| `scripts/opencode-token-stats` | `crates/dcp-cli/src/cmd/token_stats.rs` | I |
| `scripts/opencode-session-timeline` | `crates/dcp-cli/src/cmd/timeline.rs` | I |
| `scripts/opencode-message-token-counts` | `crates/dcp-cli/src/cmd/message_tokens.rs` | I |
| `scripts/print.ts` | **SKIP** (TS DX tool) | — |
| `scripts/verify-package.mjs` | **SKIP** (npm-specific) | — |
| `assets/images/*` | `assets/images/` (copy) | J |
| `CONTRIBUTING.md` | `CONTRIBUTING.md` (write) | J |
