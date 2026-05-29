
> **Repo**: `dynamic_context_pruning`
> **License**: MIT
> **Status**: Implementation complete (Phase 0–8 + A–J) · all 30 beads closed
> **Plan author**: kiro_default + 4 sub-agent research streams (memory frameworks, prompt compression, provider-native, Rust ecosystem)
> **Document language**: English (prose) + English (code/identifiers)
> **Methodology**: Clean-room reimplementation from public specs, with no reference to AGPL upstream source code

---

## Table of Contents

1. [Goals & Scope](#1-goals--scope)
2. [Research Summary](#2-research-summary)
3. [Design Principles](#3-design-principles)
4. [Public API](#4-public-api)
5. [Workspace Architecture](#5-workspace-architecture)
6. [Algorithm specifications](#6-algorithm-specifications)
7. [State & persistence](#7-state--persistence)
8. [Configuration schema](#8-configuration-schema)
9. [Integration patterns](#9-integration-patterns)
10. [Phased delivery](#10-phased-delivery)
11. [Testing strategy](#11-testing-strategy)
12. [Risks & mitigation](#12-risks--mitigation)
13. [License & clean-room methodology](#13-license--clean-room-methodology)
14. [Locked decisions](#14-locked-decisions)
15. [Open questions](#15-open-questions)
16. [Appendix A — Cargo.toml skeletons](#appendix-a--cargotoml-skeletons)
17. [Appendix B — SPEC.md outline](#appendix-b--specmd-outline)

---

## 1. Goals & Scope

### 1.1 Problem

When a coding agent runs a long session, the conversation context grows due to:

- Repeated tool calls (same tool, same arguments) → redundant information.
- Older versions of the same file (each `read`/`write`/`edit` adds new output).
- Tool errors with large inputs (pasted logs, long code) that occupy space permanently.
- Conversation segments whose tasks are complete and no longer need to be kept verbatim.

Consequences:

- Token cost grows multiplicatively (every turn re-sends the whole history).
- The model becomes "diluted" — attention is split between stale information and the current task.
- The context window limit is hit → failure or emergency compaction.

### 1.2 Solution

`dynamic_context_pruning` is a **Rust library** that performs three layers of processing on the message stream **before** it is sent to the LLM provider:

1. **Automatic strategies** (every turn, deterministic):
   - Deduplicate identical tool calls
   - Purge inputs of errored tools after N turns
   - Remove stale file reads (older versions of same path)

2. **LLM-driven compression** (the model triggers it itself):
   - A `compress` tool the model can call with a range or message summary
   - Replaces verbatim content with a compact summary block
   - Supports nesting (compressing a region that already contains compressed blocks)

3. **Cache-aware nudging**:
   - Detects when context is near the limit → nudges the model to compress
   - Default mode "agent-message" to avoid busting the prompt cache between tool turns

### 1.3 Higher-level goal (per user requirement)

> "the higher-level goal beyond CLI/MCP is to expose an API for use, instead of relying on MCP or CLI"

`dynamic_context_pruning` is **library-first**. The CLI and MCP server are only thin binary wrappers that demonstrate usage; they are not the primary product.

Any Rust agent (jcode, codex-rs, pi_agent_rust, custom) can `cargo add dynamic_context_pruning` and call three lines:

```rust
let mut pruner = ContextPruner::new(Config::load_default()?)?;
let pruned = pruner.transform_messages(messages)?;
let result = pruner.handle_compress(args, &raw_messages)?;
```

### 1.4 Non-goals

- **Not building a new agent**. This is only a middleware layer for an existing agent.
- **No concern for model selection / routing / streaming**. That is the host's responsibility.
- **No concern for tool execution sandbox / permissions**. The host decides.
- **No cross-session memory store** (this is not MemGPT). The focus is intra-session pruning.
- **No RAG / knowledge base fetching**. A `MemoryRetriever` hook is provided for the host to inject.
- **No guarantee of an exact token count** across all providers — the host picks the appropriate tokenizer.

### 1.5 v1.0 scope

| In scope | Out of scope |
|---|---|
| 3 deterministic strategies (dedup, purge_errors, stale_file_reads) | Token-level compression (LLMLingua) — feature gated for v1.1 |
| LLM-driven `compress` tool (range + message mode) | Embedding-based semantic dedup — feature gated for v1.2 |
| Cache-stability modes (Aggressive / AgentMessage / Manual) | Cross-session memory persistence |
| Stable canonical IR (`Message`, `Part`) | UI components (TUI overlay) — host responsibility |
| File-backed + in-memory state stores | Real-time sync between agents |
| 6 default prompts + override | Fine-tuning / model training |
| Nudges (context-limit / turn / iteration) | Native provider compaction integration (e.g. Anthropic auto) |
| `ContextPruner` facade with 8 main methods | Distributed/multi-process state |
| jcode adapter + Claude Code hook + MCP server | ACP middleware (v1.1+) |

---

## 2. Research Summary

> This section consolidates four research streams covering memory frameworks, prompt compression, provider-native context handling, and the Rust ecosystem. Each insight is tagged with a source for traceability.

### 2.1 Memory frameworks (Python)

| Framework | Approach | Key insight |
|---|---|---|
| **MemGPT / Letta** | OS-style hierarchical memory: main context + external context + recall storage. Recursive summarization when main fills up. | **3-segment model** (active / cooling / cold) — only prune cold; keep active+cooling verbatim. Applicable to us. |
| **mem0** | Extract atomic memories from messages, store in a vector store, retrieve by query. | Different scope — cross-session memory, not intra-session. Could be integrated via a `MemoryRetriever` trait. |
| **langmem** | Automatically extracts semantic/episodic/procedural memories. | Let the host choose — we only expose a hook. |
| **LangChain ConversationSummaryBufferMemory** | Buffer of N recent tokens + summary chunk for the older portion | Simple pattern, easy to implement. But it **busts the prompt cache** every time it re-summarizes → we avoid this. |
| **LlamaIndex ChatSummaryMemoryBuffer** | Similar but with a token budget instead of message count | Reference for token counting approach and tool-message handling |

**Conclusion from stream 1**:
- The **3-segment model** from MemGPT is a strong design pattern and should be adopted (active is never touched, cooling is protected, cold is prunable).
- **Recursive summarization** easily breaks the cache → only run when explicitly triggered.
- **Memory retrieval** is a separate concern; expose it as a trait, do not build it into the core.

### 2.2 Prompt compression algorithms

| Name | Type | Compression | Overhead | Suitable for real-time? |
|---|---|---|---|---|
| **LLMLingua / LLMLingua-2** | Token-level removal using a small LM | 2-20× | High (model inference per turn) | ⚠️ Conditional — feature flag |
| **Selective Context** | Per-token self-information score | 2-3× | Medium | ⚠️ Conditional |
| **CompAct** | Active learning compression | 5-10× | High | ❌ Too slow |
| **RECOMP** | Retrieve-then-compress | 3-5× | High | ❌ |
| **xRAG** | 1-token compressed RAG | 100× | Medium | ❌ Requires fine-tuning the host model |
| **AutoCompressors** | Soft prompt embedding | High | High | ❌ Requires model training |
| **Activation Beacon** | Sliding window beacons | High | Very high | ❌ |
| **Gist tokens / ICAE** | Soft prompt compression | 26× | Medium-high | ❌ Requires model support |

**Conclusion from stream 2**:

- Most techniques require inference with an auxiliary model → not suitable for the **real-time hot path** of a coding agent (transform latency must be < 100ms).
- Top 3 candidates worth feature-gating:
  1. **LLMLingua-2** (`feature = "lingua"`) — runs on a DistilBERT-tier model, may be acceptable.
  2. **Selective Context** (`feature = "self-info"`) — simple, can be implemented in pure Rust.
  3. **Smart truncation** (head + tail + middle ellipsis) — not really "compression" but very cheap and effective.

- The **LLM-driven compression** approach is actually wise: the model is already online and already has the context, so the marginal cost is low; no extra auxiliary model is needed. We retain this pattern.

### 2.3 Provider-native context management

| Provider | Feature | Implication for us |
|---|---|---|
| **Anthropic prompt caching** | `cache_control: ephemeral`, breakpoints, ~90% cost reduction with cached prefix | **Busting the cache = increased cost.** This is why `CacheStabilityMode` is a MUST-ship for v1. |
| **Anthropic context editing** (recent) | Auto trim tool results | This means Anthropic officially endorses the pattern. We should do better (configurable, deterministic). |
| **OpenAI Responses API** | Native `previous_response_id`, encrypted_content, `store=true` | Agents that use the Responses API may not need us. We are still useful for the Chat Completions API and non-OpenAI providers. |
| **Gemini context caching** | `cachedContent` API, TTL-based | Similar to Anthropic. |
| **Bedrock prompt caching** | `cachePoint` blocks | Similar. |
| **Claude 1M context tier** | Sonnet 4.5/Opus with 1M context | Having 1M context does not mean it is cheap — you still pay per input token. Pruning is still relevant. |

**Conclusion from stream 3**:

- **Prompt cache awareness is mandatory, not optional.** When upstream context-pruning literature mentions "cache hit rate dropping from 90% to 85%", that indicates incomplete optimization. We will do better with:
  - Default mode = batch-prune (only prune after an assistant message ends, not between tool turns).
  - Track cache-bust events and expose them via telemetry.
- **CacheAccountant trait** so the host can inject a token-cost-of-cache-miss callback.

### 2.4 OSS coding agents

| Agent | Compaction trigger | Strategy | Tool call handling | Cache awareness |
|---|---|---|---|---|
| **Cline** | Token budget threshold | Remove stale file reads + sliding window | Preserve pairing | Concerned, partial |
| **Roo Code** | Manual `/condense` + auto threshold | LLM summarizes older messages | Preserve pairing | Better than Cline |
| **OpenCode (Go)** | Token threshold | LLM summarize | Preserve pairing | Limited |
| **Codex (Rust)** | Configurable threshold | Recursive summary + recent cap | Preserve pairing | Cache-aware |
| **Claude Code** | Auto context limit | Provider-side compaction | Preserve | Provider handles |
| **aider** | Token budget | Repo-map dynamic + truncate | N/A — different model | Different scope |

**Conclusion from stream 4**:

- Common pattern: **summarize older messages while keeping the most recent N turns verbatim**. Simple, works.
- Our differentiator: **3 deterministic strategies + LLM-driven block compression** = a layered approach. Each strategy addresses a different kind of waste.
- **Stale file read removal (Cline)** is a good feature, cheap to implement, does not bust the cache (because the file was updated in some turn, the header is identical across versions, and only the body changes). We add it.
- **Prune frontier (pi-context-prune)** — when the summary is larger than the raw content, advance the frontier without applying the summary. Avoids retrying the same range. We add it.

### 2.5 Rust ecosystem

| Crate | Role | Suitability |
|---|---|---|
| **rig** (0xPlaygrounds/rig) | Agent framework with tools and prompts | ✅ Could be one of three ship targets (alongside jcode + Claude Code hook). We provide a `dcp-rig` adapter. |
| **kalosm** (floneum/floneum) | Local LLM inference | ❌ Not directly relevant |
| **async-openai** | OpenAI client | ✅ Standard. We convert message types via an adapter. |
| **llm-chain** | Lower-level chain | ⚠️ Project rarely updated. Skip. |
| **swiftide** | RAG framework | ❌ Different scope |
| **codex-rs core::compact** | Compaction module | ✅ Reference for the Rust agent pattern. Read the spec, do not copy code. |
| **pi_agent_rust src/compaction.rs** | Compaction module | ✅ Similar. |
| **jcode-compaction-core** | IR reference for us | ✅ Integration target. |

**Tokenizer crates**:

| Crate | Speed | Coverage | Pick |
|---|---|---|---|
| `tiktoken-rs` | Slow (1×) | OpenAI BPE | Skip |
| **`tiktoken`** (lib.rs, not tiktoken-rs) | 7-10× faster | OpenAI + Llama + DeepSeek + Qwen + Mistral | ✅ Default for `feature = "tiktoken-fast"` |
| **`tokenizers`** (HuggingFace official) | Universal | Any tokenizer.json | ✅ Default — most general purpose |
| `claude-tokenizer` | OK | Embedded HF JSON | ✅ Feature `"claude-tokens"` |
| `kitoken` | Fast | Multi-format | Alternative |
| `splintr` | Fastest | OpenAI-compat | Alternative |
| Char/4 heuristic | Instant | All | ✅ No-feature default |

**Pick stack**:
- Default tokenizer = char/4 heuristic (zero deps, "good enough" for budget estimation)
- `feature = "tokenizers"` → HuggingFace `tokenizers` crate (universal, accurate)
- `feature = "tiktoken-fast"` → `tiktoken` crate (when speed matters)
- `feature = "claude-tokens"` → `claude-tokenizer` crate (Claude-specific accuracy)

**JSONC parser**: `serde_json` + a custom strip-comments wrapper, or the `json5` crate. Pick: **`json5`** (well-maintained, JSON5 is a superset of JSONC).

**Glob matcher**: **`globset`** (used by the `ignore` crate, battle-tested).

**MCP server**: `rmcp` (official) > `fastmcp_rust`. Pick **`rmcp`** because it is the official crate from the ModelContextProtocol org.

**ACP**: the `agent-client-protocol` crate is official from zed-industries. Integrated in v1.1.

### 2.6 Synthesis: 6 core principles after research

1. **Cache stability is critical** — pruning must avoid busting the prompt cache or it will increase cost.
2. **3-segment workspace** (active / cooling / cold) — prune only cold; keep active+cooling.
3. **Layered strategies** — deterministic (cheap) runs first; LLM-driven (expensive) runs when needed.
4. **Library-first, not service** — sync API, in-process, no I/O except for persistence.
5. **Pluggable traits** — Tokenizer, Storage, MemoryRetriever, CacheAccountant are all traits.
6. **Idempotent state** — state can be rebuilt from messages + persisted blocks. Crash recovery is free.

---

> **Continued in part 2 (Sections 3-9)**: Design principles, Public API, Architecture, Algorithms, State, Config, Integration.

---

## 3. Design Principles

### 3.1 Cache-stability first

**Problem**: Anthropic / Bedrock / Gemini all use prefix-based prompt caching. A cache hit reduces cost by ~90%. But caching requires the prefix to remain byte-for-byte identical.

If DCP rewrites messages every turn → every request is a cache miss → cost increases 10×.

**Rules**:

- Default `CacheStabilityMode::AgentMessage`: only apply pruning after the assistant finishes a text turn (not after every tool call).
- `CacheStabilityMode::Aggressive`: prune on every fetch (debug only).
- `CacheStabilityMode::Manual`: only prune when the host calls `force_apply()`.

**Measurement**:

- Track `cache_bust_events` in `Stats`.
- Optional `CacheAccountant` trait so the host can inject cost calculation.

### 3.2 Library-first, not service

**Rules**:

- The public API is Rust traits/structs/functions, not HTTP/IPC.
- Do not spawn a thread/process inside the core.
- Do not bind to an async runtime (sync core).
- I/O is limited to the persistence layer — abstracted behind a trait, host-injectable.
- The CLI and MCP server are separate binary wrappers (`bin/cli.rs`, `bin/mcp.rs`).

### 3.3 Sync core, optional async facade

```rust
// Core: sync, no async runtime
impl ContextPruner {
    pub fn transform_messages(&mut self, msgs: Vec<Message>) -> Result<Vec<Message>>;
}

// Async facade: thin wrapper, behind feature flag
#[cfg(feature = "async")]
impl ContextPrunerAsync {
    pub async fn transform_messages(&mut self, msgs: Vec<Message>) -> Result<Vec<Message>>;
}
```

**Reasoning**: The main logic is purely CPU/memory. `transform_messages` performs no I/O. Async is only required for `handle_compress` (which calls an LLM tool) — the host wraps it in their own async block.

### 3.4 Pluggable traits

```rust
pub trait Tokenizer: Send + Sync {
    fn count(&self, text: &str) -> usize;
}

pub trait StatePersistence: Send + Sync {
    fn load(&self, session_id: &str) -> Result<Option<PersistedState>>;
    fn save(&self, session_id: &str, state: &PersistedState) -> Result<()>;
}

pub trait MemoryRetriever: Send + Sync {
    fn retrieve(&self, query: &str, k: usize) -> Result<Vec<RetrievedMemory>>;
}

pub trait CacheAccountant: Send + Sync {
    fn cost_per_cache_miss(&self, tokens: usize) -> f64;
    fn record_event(&mut self, event: CacheEvent);
}

pub trait PruneStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn apply(&self, state: &mut SessionState, messages: &[Message]) -> Result<PruneOutcome>;
}
```

The host injects implementations or uses the defaults shipped with the library.

### 3.5 Idempotent state

**Rule**: `SessionState` must be rebuildable from:
- `messages: &[Message]` (raw history)
- `persisted_blocks: Vec<CompressionBlock>` (loaded from disk)

This means:
- Crash mid-run → restart → state fully recovered.
- Switching host (jcode → codex-rs) → import messages + blocks → state recovered.
- Test: `assert_eq!(rebuild_state(msgs, blocks), original_state)`.

### 3.6 Format-agnostic via canonical IR

```rust
// Canonical Internal Representation
pub struct Message {
    pub id: String,
    pub role: Role,
    pub parts: Vec<Part>,
    pub time: i64,
}

pub enum Part {
    Text(String),
    Reasoning(String),
    ToolCall { call_id: String, tool: String, input: serde_json::Value },
    ToolResult { call_id: String, status: ToolStatus, output: Option<String>, error: Option<String> },
    Image { media_type: String, data: String },
}
```

**Rules**:
- The core works only with `Message` / `Part`.
- The host writes adapters to convert in/out.
- Provider format breaking changes (Anthropic/OpenAI message v2) → fix the adapter, the core does not change.
- Adapters are ~50 lines per host.

---

## 4. Public API

### 4.1 Quick start

```toml
[dependencies]
dynamic_context_pruning = "0.1"
# Optional features:
# dynamic_context_pruning = { version = "0.1", features = ["tokenizers", "async"] }
```

```rust
use dynamic_context_pruning::{ContextPruner, Config, Message};

fn main() -> anyhow::Result<()> {
    // 1. Initialize
    let mut pruner = ContextPruner::new(Config::load_default()?)?;

    // 2. Transform messages before sending to the LLM
    let messages = vec![/* ... */];
    let pruned = pruner.transform_messages(messages)?;

    // 3. Append system prompt
    let mut system = String::from("You are a helpful assistant.");
    pruner.transform_system(&mut system);

    // 4. When the LLM calls the compress tool
    let args: CompressArgs = serde_json::from_value(/* tool args */)?;
    let result = pruner.handle_compress(args, &raw_messages)?;

    // 5. Slash command
    let outcome = pruner.handle_command("context", &[], &raw_messages);

    Ok(())
}
```

### 4.2 ContextPruner facade

```rust
pub struct ContextPruner { /* opaque */ }

impl ContextPruner {
    // === CONSTRUCTORS ===

    pub fn new(config: Config) -> Result<Self, Error>;

    pub fn builder() -> ContextPrunerBuilder;

    // === CONFIG ===

    pub fn config(&self) -> &Config;
    pub fn update_config(&mut self, config: Config) -> Result<(), Error>;

    // === HOT PATH (sync) ===

    /// Transform messages before sending them to the LLM.
    /// Applies: dedup, purge_errors, stale_file_reads, replacement of compressed ranges.
    /// Injects: nudges, message refs.
    pub fn transform_messages(&mut self, messages: Vec<Message>) -> Result<Vec<Message>, Error>;

    /// Append the DCP system prompt to the host's system message.
    pub fn transform_system(&self, system: &mut String);

    // === COMPRESS TOOL ===

    /// Schema for registering the `compress` tool with the LLM.
    pub fn compress_tool_schema(&self) -> ToolSchema;

    /// Execute when the LLM calls the compress tool.
    pub fn handle_compress(
        &mut self,
        args: CompressArgs,
        raw_messages: &[Message],
    ) -> Result<CompressResult, Error>;

    /// Decompress one block (revert).
    pub fn decompress(&mut self, block_id: BlockId) -> Result<DecompressResult, Error>;

    /// Re-apply a previously decompressed block.
    pub fn recompress(&mut self, block_id: BlockId) -> Result<RecompressResult, Error>;

    // === SLASH COMMANDS ===

    pub fn handle_command(
        &mut self,
        cmd: &str,
        args: &[&str],
        raw_messages: &[Message],
    ) -> CommandOutcome;

    // === SUB-AGENT ===

    /// Fold subagent results into the parent context (DCP has subAgentResultCache).
    pub fn fold_subagent(&mut self, subagent_messages: Vec<Message>) -> Result<Message, Error>;

    // === INTROSPECTION ===

    pub fn stats(&self) -> &Stats;
    pub fn state(&self) -> &SessionState;
    pub fn telemetry(&self) -> Telemetry;

    // === LIFECYCLE ===

    pub fn reset(&mut self);
    pub fn save(&self) -> Result<(), Error>;

    // === MANUAL CONTROL ===

    /// Force apply a pending prune (overrides CacheStabilityMode).
    pub fn force_apply(&mut self) -> Result<(), Error>;

    /// Set manual mode (no auto strategies).
    pub fn set_manual_mode(&mut self, enabled: bool);
}
```

### 4.3 Builder

```rust
pub struct ContextPrunerBuilder { /* ... */ }

impl ContextPrunerBuilder {
    pub fn config(self, config: Config) -> Self;
    pub fn tokenizer(self, tokenizer: Arc<dyn Tokenizer>) -> Self;
    pub fn storage(self, storage: Arc<dyn StatePersistence>) -> Self;
    pub fn memory(self, retriever: Arc<dyn MemoryRetriever>) -> Self;
    pub fn cache_accountant(self, accountant: Arc<dyn CacheAccountant>) -> Self;
    pub fn add_strategy(self, strategy: Box<dyn PruneStrategy>) -> Self;
    pub fn prompts(self, prompts: Prompts) -> Self;
    pub fn build(self) -> Result<ContextPruner, Error>;
}
```

### 4.4 Types (re-export)

```rust
// From the dcp-types crate
pub use dynamic_context_pruning_types::{
    Message, Part, Role, ToolStatus,
    BlockId, RunId, MessageRef,
    CompressionBlock, CompressionMode,
    SessionState, Stats, Telemetry,
};

// From the dcp-config crate
pub use dynamic_context_pruning_config::{
    Config, CompressConfig, StrategiesConfig,
    CacheStabilityMode, NudgeForce, InjectionMode,
};

// From the dcp-traits crate
pub use dynamic_context_pruning_traits::{
    Tokenizer, StatePersistence, MemoryRetriever,
    CacheAccountant, PruneStrategy,
};
```

### 4.5 Error type

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid message: {0}")]
    InvalidMessage(String),

    #[error("invalid compress args: {0}")]
    InvalidCompressArgs(String),

    #[error("block {0} not found")]
    BlockNotFound(BlockId),

    #[error("range overlap: {0}")]
    RangeOverlap(String),

    #[error("placeholder mismatch: {0}")]
    PlaceholderMismatch(String),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("config error: {0}")]
    Config(#[from] ConfigError),

    #[error("tokenizer error: {0}")]
    Tokenizer(String),

    #[error("manual mode blocks operation")]
    ManualModeBlocked,

    #[error("permission denied")]
    PermissionDenied,
}
```

### 4.6 Versioning policy

- **Public API stability**: standard SemVer. Breaking changes bump the major version.
- **Persistence schema**: separately versioned (`PersistedStateV1`, `V2`...). Automatic migration path.
- **Wire format** (canonical `Message`): field-additive only. Adding a field = minor bump.
- `#[non_exhaustive]` for every enum that may be extended (`Part`, `CommandOutcome`, `CacheEvent`).

---

## 5. Workspace Architecture

### 5.1 Crate layout

```
dynamic_context_pruning/                # repo root
├── Cargo.toml                          # workspace
├── README.md
├── PLAN.md                             # this file
├── SPEC.md                             # behavior spec (clean-room source of truth)
├── LICENSE                             # MIT
├── CHANGELOG.md
├── crates/
│   ├── dcp-types/                      # canonical IR (Message, Part, BlockId, ...)
│   ├── dcp-traits/                     # Tokenizer, StatePersistence, MemoryRetriever, ...
│   ├── dcp-tokens/                     # tokenizer impls (char/4, tiktoken, hf, claude)
│   ├── dcp-protected/                  # glob matcher, protected patterns
│   ├── dcp-state/                      # SessionState transitions, idempotent rebuild
│   ├── dcp-storage/                    # FileStateStore, InMemoryStore impls
│   ├── dcp-prune/                      # dedup, purge_errors, stale_file_reads
│   ├── dcp-compress/                   # range/message modes, block bookkeeping, frontier
│   ├── dcp-prompts/                    # 6 default prompts + override loader
│   ├── dcp-nudges/                     # context-limit / turn / iteration nudges
│   ├── dcp-config/                     # JSONC parse + cascade
│   ├── dcp-telemetry/                  # metrics, observers
│   ├── dcp-core/                       # facade ContextPruner
│   └── dynamic_context_pruning/        # umbrella crate (pub use everything)
├── adapters/                           # split out from crates/ because targets differ from the workspace
│   ├── dcp-rig/                        # adapter for the rig framework
│   └── dcp-claude-hook/                # binary for the Claude Code SessionStart hook
├── bins/
│   ├── dcp-cli/                        # interactive REPL / one-shot debugging
│   └── dcp-mcp/                        # MCP server
├── examples/
│   ├── 01_minimal.rs                   # 20-line hello world
│   ├── 02_jcode_integration.rs         # hook into messages_for_provider
│   ├── 03_codex_integration.rs         # hook into codex-rs
│   ├── 04_custom_agent.rs              # 100-line full custom
│   ├── 05_with_memory.rs               # MemoryRetriever pattern
│   └── 06_cache_stability.rs           # CacheAccountant + modes
├── tests/                              # integration tests
│   ├── e2e_jcode.rs
│   ├── property_tool_pairing.rs
│   ├── snapshot_compression.rs
│   └── fixtures/
├── benches/                            # criterion benchmarks
└── docs/
    ├── architecture.md
    ├── algorithms.md
    └── integration_guide.md
```

### 5.2 Dependency graph

```
                    ┌─────────────────────────────────┐
                    │  dynamic_context_pruning        │  (umbrella)
                    │  pub use dcp_core::*;            │
                    └────────────────┬────────────────┘
                                     │
                              ┌──────▼──────┐
                              │  dcp-core   │  (facade ContextPruner)
                              └──┬─┬─┬─┬─┬──┘
                                 │ │ │ │ │
        ┌────────────────────────┘ │ │ │ └────────────────────┐
        │                          │ │ │                      │
   ┌────▼────┐  ┌─────────┐  ┌────▼─▼─▼────┐  ┌────────┐  ┌──▼─────────┐
   │ dcp-prune│ │dcp-compress│ │dcp-nudges │ │dcp-state│ │dcp-prompts │
   └────┬────┘  └─────┬───┘  └─────┬──────┘  └────┬───┘  └──────┬─────┘
        │             │              │             │             │
        └────┬────────┴──────────────┴─────────────┴─────────────┘
             │
        ┌────▼────────┐  ┌────────────┐
        │ dcp-protected│ │dcp-storage │
        └────┬────────┘  └─────┬──────┘
             │                 │
        ┌────▼─────────────────▼───┐
        │   dcp-traits             │  (no internal deps)
        └────┬─────────┬───────────┘
             │         │
        ┌────▼───┐  ┌──▼────────┐
        │dcp-tokens│ │dcp-config │
        └────┬───┘  └─────┬─────┘
             │            │
        ┌────▼────────────▼─────┐
        │     dcp-types         │  (leaf, no deps except serde, chrono)
        └───────────────────────┘
```

**Rules**:
- `dcp-types` sits at the bottom and depends only on `serde`, `chrono`, `serde_json`.
- `dcp-traits` depends only on `dcp-types`.
- Every implementation crate depends on `dcp-traits` (and not on each other).
- `dcp-core` glues everything together into the facade.
- `dynamic_context_pruning` is the umbrella; the host depends only on this.

### 5.3 Feature flags

```toml
# dynamic_context_pruning umbrella crate
[features]
default = []

# Tokenizer backends
tokenizers   = ["dcp-tokens/tokenizers"]    # HuggingFace
tiktoken     = ["dcp-tokens/tiktoken"]      # OpenAI BPE
claude       = ["dcp-tokens/claude"]        # Claude tokenizer

# Storage backends
sled         = ["dcp-storage/sled"]
sqlite       = ["dcp-storage/sqlite"]

# Async facade
async        = ["dcp-core/async"]

# Optional features
mcp          = ["dep:rmcp"]                 # MCP server
claude-hook  = []                            # Claude Code hook binary
rig          = ["dep:rig"]                   # rig adapter
lingua       = ["dcp-prune/lingua"]         # LLMLingua-2 (experimental)
semantic     = ["dcp-prune/embedding"]      # Semantic dedup (experimental)
quality      = ["dcp-telemetry/quality"]    # Quality regression detector
```

---

## 6. Algorithm specifications

### 6.1 Strategy: deduplicate

**Purpose**: Remove older tool calls that have the same `(tool_name, normalized_input)` as a newer call.

**Algorithm**:

```rust
fn deduplicate(state: &mut SessionState, messages: &[Message]) -> PruneOutcome {
    // 1. Skip if manual mode + automatic_strategies disabled
    if state.manual_mode && !config.manual_mode.automatic_strategies {
        return PruneOutcome::skipped("manual_mode");
    }

    // 2. Collect all not-yet-pruned tool call IDs
    let unpruned_ids: Vec<&str> = state.tool_id_list.iter()
        .filter(|id| !state.prune.tools.contains_key(*id))
        .collect();

    // 3. Group by signature (tool + sorted normalized params)
    let mut groups: HashMap<String, Vec<&str>> = HashMap::new();
    for id in &unpruned_ids {
        let meta = state.tool_parameters.get(*id).unwrap();
        if is_protected_tool(&meta.tool, &config.deduplication.protected_tools) { continue; }
        let paths = extract_file_paths(&meta.tool, &meta.parameters);
        if any_path_protected(&paths, &config.protected_file_patterns) { continue; }
        let sig = signature(&meta.tool, &meta.parameters);
        groups.entry(sig).or_default().push(*id);
    }

    // 4. Within each group, mark all-except-last for pruning
    let mut to_prune = Vec::new();
    for ids in groups.values() {
        if ids.len() > 1 {
            to_prune.extend(&ids[..ids.len() - 1]);  // all but last
        }
    }

    // 5. Update state
    for id in &to_prune {
        let entry = state.tool_parameters.get(*id).unwrap();
        state.prune.tools.insert(id.to_string(), entry.token_count.unwrap_or(0));
    }
    state.stats.total_prune_tokens += to_prune.iter()
        .map(|id| state.tool_parameters.get(*id).unwrap().token_count.unwrap_or(0))
        .sum::<usize>();

    PruneOutcome::pruned(to_prune.len())
}

fn signature(tool: &str, params: &Value) -> String {
    let normalized = normalize(params);  // remove undefined, sort keys recursively
    format!("{}::{}", tool, serde_json::to_string(&normalized).unwrap())
}
```

**Edge cases**:
- Tool errors are not deduplicated (status != Completed → skip).
- File-path-based protection: `read("Cargo.toml")` is protected if the pattern `Cargo.toml` is in `protectedFilePatterns`.

### 6.2 Strategy: purge_errors

**Purpose**: Tool calls that errored N turns ago → drop their input (keep the error message).

**Algorithm**:

```rust
fn purge_errors(state: &mut SessionState, messages: &[Message]) -> PruneOutcome {
    if state.manual_mode && !config.manual_mode.automatic_strategies {
        return PruneOutcome::skipped("manual_mode");
    }
    if !config.strategies.purge_errors.enabled {
        return PruneOutcome::skipped("disabled");
    }

    let threshold = config.strategies.purge_errors.turns.max(1);
    let mut to_prune = Vec::new();

    for id in &state.tool_id_list {
        if state.prune.tools.contains_key(id) { continue; }
        let meta = state.tool_parameters.get(id).unwrap();
        if is_protected_tool(&meta.tool, &config.purge_errors.protected_tools) { continue; }
        if meta.status != Some(ToolStatus::Error) { continue; }
        if state.current_turn - meta.turn >= threshold {
            to_prune.push(id.clone());
        }
    }

    for id in &to_prune {
        let entry = state.tool_parameters.get(id).unwrap();
        state.prune.tools.insert(id.clone(), entry.token_count.unwrap_or(0));
    }
    PruneOutcome::pruned(to_prune.len())
}
```

**When pruning is applied** (in `apply_prune_to_messages`):
- Tool result content → keep the error message untouched
- Tool input fields → replace with `[input removed due to failed tool call]`

### 6.3 Strategy: stale_file_reads (NEW — distinctive feature)

**Purpose**: The same file is `read`/`write`/`edit`-ed multiple times. Keep the most recent version, prune the older outputs.

**Algorithm**:

```rust
fn stale_file_reads(state: &mut SessionState, messages: &[Message]) -> PruneOutcome {
    if !config.strategies.stale_file_reads.enabled {
        return PruneOutcome::skipped("disabled");
    }

    // Group tool calls by file path
    let mut by_path: HashMap<String, Vec<&str>> = HashMap::new();
    for id in &state.tool_id_list {
        if state.prune.tools.contains_key(id) { continue; }
        let meta = state.tool_parameters.get(id).unwrap();
        if !["read", "write", "edit", "multiedit"].contains(&meta.tool.as_str()) { continue; }
        let paths = extract_file_paths(&meta.tool, &meta.parameters);
        for path in paths {
            if any_path_protected(&[path.clone()], &config.protected_file_patterns) { continue; }
            by_path.entry(path).or_default().push(id);
        }
    }

    // Within each path, prune all except the latest
    let mut to_prune = Vec::new();
    for ids in by_path.values() {
        if ids.len() > 1 {
            to_prune.extend(&ids[..ids.len() - 1]);
        }
    }

    for id in &to_prune {
        state.prune.tools.insert(id.to_string(), 
            state.tool_parameters.get(*id).unwrap().token_count.unwrap_or(0));
    }
    PruneOutcome::pruned(to_prune.len())
}
```

**Why it is distinct from dedup**: Dedup matches the exact tool+params signature. Stale file reads match by **path only** — `read("foo.rs", offset=0)` and `read("foo.rs", offset=100)` share a path → dedup does not match (parameters differ), but stale_file_reads does match.

### 6.4 Pruning pipeline

```rust
pub fn transform_messages(&mut self, mut messages: Vec<Message>) -> Result<Vec<Message>> {
    // Phase 0: Validate, filter malformed
    messages = filter_valid_messages(messages);

    // Phase 1: Sync state with current messages
    self.check_session(&messages)?;
    sync_compress_permission(&self.state, &self.config, &messages);

    if self.state.is_subagent && !self.config.experimental.allow_subagents {
        return Ok(messages);
    }

    // Phase 2: Strip hallucinations, cache system tokens
    strip_hallucinations(&mut messages);
    cache_system_prompt_tokens(&mut self.state, &messages);

    // Phase 3: Assign refs, sync compression blocks
    assign_message_refs(&mut self.state, &messages);
    sync_compression_blocks(&mut self.state, &messages);
    sync_tool_cache(&mut self.state, &self.config, &messages);
    build_tool_id_list(&mut self.state, &messages);

    // Phase 4: Run strategies (cache-stability gated)
    if self.should_apply_now() {
        self.run_strategy::<Deduplicate>(&messages)?;
        self.run_strategy::<PurgeErrors>(&messages)?;
        self.run_strategy::<StaleFileReads>(&messages)?;
    }

    // Phase 5: Apply prune to message content
    let pruned = apply_prune_to_messages(messages, &self.state, &self.config);

    // Phase 6: Filter compressed ranges (replace with summary blocks)
    let pruned = filter_compressed_ranges(pruned, &self.state, &self.config);

    // Phase 7: Inject extended subagent results
    let pruned = inject_extended_subagent_results(pruned, &self.state)?;

    // Phase 8: Compute priorities, inject nudges
    let priorities = build_priority_map(&self.config, &self.state, &pruned);
    self.inject_nudges(&mut pruned, &priorities);
    self.inject_message_ids(&mut pruned, &priorities);

    // Phase 9: Apply pending manual triggers
    self.apply_pending_manual_trigger(&mut pruned);

    // Phase 10: Strip stale metadata, persist
    strip_stale_metadata(&mut pruned);
    self.persistence.save(&self.state.session_id.unwrap(), &self.state.persisted())?;

    Ok(pruned)
}

fn should_apply_now(&self) -> bool {
    use CacheStabilityMode::*;
    match self.config.cache_stability_mode {
        Aggressive => true,
        AgentMessage => self.state.last_message_was_assistant_text,
        Manual => false,  // only apply when force_apply()
    }
}
```

### 6.5 Compress: range mode

**Tool schema** (for the LLM):

```json
{
  "name": "compress",
  "description": "Compress contiguous ranges of conversation into block summaries...",
  "parameters": {
    "type": "object",
    "properties": {
      "topic": { "type": "string" },
      "content": {
        "type": "array",
        "items": {
          "type": "object",
          "properties": {
            "startId": { "type": "string", "description": "m0001 or b2" },
            "endId": { "type": "string" },
            "summary": { "type": "string" }
          },
          "required": ["startId", "endId", "summary"]
        }
      }
    },
    "required": ["topic", "content"]
  }
}
```

**Execution**:

```rust
fn handle_compress_range(&mut self, args: CompressRangeArgs, raw: &[Message]) -> Result<CompressResult> {
    // 1. Validate args
    validate_args(&args)?;
    let call_id = self.current_tool_call_id();

    // 2. Prepare session
    let (raw_messages, search_ctx) = self.prepare_session(raw, &args.topic)?;
    let resolved = resolve_ranges(&args, &search_ctx, &self.state)?;
    validate_non_overlapping(&resolved)?;

    // 3. For each range, build final summary with protected content appended
    let mut prepared = Vec::with_capacity(resolved.len());
    for plan in &resolved {
        let placeholders = parse_block_placeholders(&plan.entry.summary);
        let missing = validate_summary_placeholders(
            &placeholders, &plan.required_block_ids,
            &plan.start_ref, &plan.end_ref, &search_ctx.summary_by_block_id,
        )?;
        let injected = inject_block_placeholders(/* ... */);
        let with_users = append_protected_user_messages(/* ... */);
        let with_prompts = append_protected_prompt_info(/* ... */);
        let with_tools = self.append_protected_tools(/* ... */).await?;
        let completed = append_missing_block_summaries(/* ... */);
        prepared.push(PreparedRange {
            entry: plan.entry.clone(),
            selection: plan.selection.clone(),
            anchor_message_id: plan.anchor_message_id.clone(),
            final_summary: completed.expanded_summary,
            consumed_block_ids: completed.consumed_block_ids,
        });
    }

    // 4. Allocate run_id, then for each plan allocate block_id and apply state
    let run_id = self.state.allocate_run_id();
    let mut notifications = Vec::new();
    let mut total_compressed = 0;
    for plan in &prepared {
        let block_id = self.state.allocate_block_id();
        let stored_summary = wrap_compressed_summary(block_id, &plan.final_summary);
        let summary_tokens = self.tokenizer.count(&stored_summary);

        let applied = apply_compression_state(
            &mut self.state,
            CompressionStateInput {
                topic: args.topic.clone(),
                batch_topic: args.topic.clone(),
                start_id: plan.entry.start_id.clone(),
                end_id: plan.entry.end_id.clone(),
                mode: CompressionMode::Range,
                run_id, compress_message_id: self.current_message_id().to_string(),
                compress_call_id: call_id.clone(),
                summary_tokens,
            },
            &plan.selection, &plan.anchor_message_id, block_id,
            &stored_summary, &plan.consumed_block_ids,
        );
        total_compressed += applied.message_ids.len();
        notifications.push(NotificationEntry {
            block_id, run_id, summary: plan.final_summary.clone(), summary_tokens,
        });
    }

    // 5. Finalize
    self.finalize_session(raw_messages, &notifications, Some(args.topic.clone()))?;
    Ok(CompressResult { compressed_messages: total_compressed, blocks: notifications })
}
```

### 6.6 Compress: message mode

**Difference from range**: the model picks individual messages (not a span). No natural nesting.

```json
{
  "topic": "Closed research notes",
  "content": [
    { "messageId": "m0007", "topic": "API exploration", "summary": "..." },
    { "messageId": "m0012", "topic": "Bug fix attempt 1", "summary": "..." }
  ]
}
```

The implementation is similar to range mode but with `start_id == end_id == messageId`, and there is no nested block expansion.

### 6.7 Block bookkeeping & nesting

**CompressionBlock**:

```rust
pub struct CompressionBlock {
    pub block_id: BlockId,
    pub run_id: RunId,
    pub active: bool,
    pub deactivated_by_user: bool,
    pub compressed_tokens: u64,
    pub summary_tokens: u64,
    pub duration_ms: u64,
    pub mode: CompressionMode,
    pub topic: String,
    pub batch_topic: Option<String>,
    pub start_id: String,
    pub end_id: String,
    pub anchor_message_id: String,
    pub compress_message_id: String,
    pub compress_call_id: Option<String>,
    pub included_block_ids: Vec<BlockId>,
    pub consumed_block_ids: Vec<BlockId>,
    pub parent_block_ids: Vec<BlockId>,
    pub direct_message_ids: Vec<String>,
    pub direct_tool_ids: Vec<String>,
    pub effective_message_ids: Vec<String>,
    pub effective_tool_ids: Vec<String>,
    pub created_at: i64,
    pub deactivated_at: Option<i64>,
    pub deactivated_by_block_id: Option<BlockId>,
    pub summary: String,
}
```

**Nesting rules**:
- When a new compress range covers an old block → the old block is consumed → deactivated.
- The new block has `consumed_block_ids = [old_id]`, `included_block_ids = [old_id, ...]`.
- The old block has `parent_block_ids = [new_id]`, `deactivated_by_block_id = Some(new_id)`.
- The new block's `effective_message_ids` = union(direct_message_ids of new, effective ids of consumed blocks).

### 6.8 Cache-stability mode

```rust
pub enum CacheStabilityMode {
    /// Apply pruning every turn (debug, may bust the cache).
    Aggressive,
    /// Default: apply after the assistant has finished a text turn (not between tool calls).
    AgentMessage,
    /// Only apply when the host calls force_apply().
    Manual,
}
```

**Logic**:

```rust
impl ContextPruner {
    fn should_apply_now(&self, messages: &[Message]) -> bool {
        use CacheStabilityMode::*;
        match self.config.cache_stability_mode {
            Aggressive => true,
            AgentMessage => self.is_assistant_text_turn_end(messages),
            Manual => false,
        }
    }

    fn is_assistant_text_turn_end(&self, messages: &[Message]) -> bool {
        // Last message: assistant role, has Text part, no pending ToolCall
        if let Some(last) = messages.last() {
            if last.role == Role::Assistant {
                let has_text = last.parts.iter().any(|p| matches!(p, Part::Text(_)));
                let has_pending_tool = last.parts.iter().any(|p| matches!(p, Part::ToolCall { .. }));
                return has_text && !has_pending_tool;
            }
        }
        false
    }
}
```

**Pending state**: When `should_apply_now() == false`, strategies still run (writing to pending state), but they do NOT modify outgoing messages. When the turn boundary is reached → flush pending.

```rust
struct PendingPrune {
    tool_ids: Vec<String>,
    cumulative_tokens: usize,
    accumulated_at_turn: u32,
}
```

### 6.9 Message references (m0001 / b1)

**Format**:
- Messages: `m0001`, `m0002`, ... `m9999` (zero-padded 4 digits)
- Blocks: `b1`, `b2`, ... `b<N>` (no padding)

**Allocation**:

```rust
pub struct MessageIdState {
    pub by_raw_id: HashMap<String, String>,   // raw → ref
    pub by_ref: HashMap<String, String>,      // ref → raw
    pub next_ref: u32,
}

fn assign_message_refs(state: &mut SessionState, messages: &[Message]) -> usize {
    let mut assigned = 0;
    let mut skipped_subagent_prompt = false;
    for msg in messages {
        if is_ignored_user_message(msg) { continue; }
        if state.is_subagent && !skipped_subagent_prompt && msg.role == Role::User {
            skipped_subagent_prompt = true;
            continue;
        }
        let raw_id = &msg.id;
        if state.message_ids.by_raw_id.contains_key(raw_id) { continue; }
        let r = allocate_next_message_ref(state)?;
        state.message_ids.by_raw_id.insert(raw_id.clone(), r.clone());
        state.message_ids.by_ref.insert(r, raw_id.clone());
        assigned += 1;
    }
    assigned
}
```

**Capacity**: 9999 message refs per session. Sufficient for any practical case.

**Inject**: After `transform_messages` finishes, each message has a single XML tag appended to its content:

```
<dcp-message-id>m0042</dcp-message-id>
```

The LLM uses this tag to pick start_id/end_id when calling compress.

### 6.10 Nudges

3 kinds:

1. **Context-limit nudge**: when token usage > maxContextLimit, inject after every N fetches.
2. **Turn nudge**: each (user, assistant) pair without compression → inject a mild nudge.
3. **Iteration nudge**: when the message count since the last user message > iterationNudgeThreshold → strong nudge.

**Render**:

```rust
fn render_nudge(kind: NudgeKind, prompts: &Prompts, state: &SessionState, config: &Config) -> String {
    match kind {
        NudgeKind::ContextLimit { tokens, limit } => prompts.context_limit_nudge.render(&[
            ("tokens", &tokens.to_string()),
            ("limit", &limit.to_string()),
        ]),
        NudgeKind::Turn => prompts.turn_nudge.clone(),
        NudgeKind::Iteration { count } => prompts.iteration_nudge.replace("{count}", &count.to_string()),
    }
}

fn inject_nudges(state: &mut SessionState, config: &Config, messages: &mut Vec<Message>, ...) {
    let priorities = build_priority_map(config, state, messages);
    for (idx, msg) in messages.iter_mut().enumerate() {
        if let Some(kind) = priorities.get(&msg.id) {
            let nudge = render_nudge(*kind, &state.prompts, state, config);
            inject_nudge_into_message(msg, &nudge, config.injection_mode);
        }
    }
}
```

---

## 7. State & persistence

### 7.1 SessionState (in-memory)

```rust
pub struct SessionState {
    pub session_id: Option<String>,
    pub is_subagent: bool,
    pub manual_mode: ManualMode,
    pub compress_permission: CompressPermission,
    pub pending_manual_trigger: Option<PendingManualTrigger>,
    pub prune: Prune,
    pub nudges: Nudges,
    pub stats: Stats,
    pub compression_timing: CompressionTimingState,
    pub tool_parameters: HashMap<String, ToolParameterEntry>,
    pub subagent_result_cache: HashMap<String, String>,
    pub tool_id_list: Vec<String>,
    pub message_ids: MessageIdState,
    pub last_compaction: i64,
    pub current_turn: u32,
    pub model_context_limit: Option<u64>,
    pub system_prompt_tokens: Option<u64>,
    pub last_message_was_assistant_text: bool,  // for AgentMessage cache mode
    pub pending_prune: Option<PendingPrune>,
}

pub struct Prune {
    pub tools: HashMap<String, u64>,                      // call_id → tokens saved
    pub messages: PruneMessagesState,
}

pub struct PruneMessagesState {
    pub by_message_id: HashMap<String, PrunedMessageEntry>,
    pub blocks_by_id: HashMap<BlockId, CompressionBlock>,
    pub active_block_ids: HashSet<BlockId>,
    pub active_by_anchor_message_id: HashMap<String, BlockId>,
    pub next_block_id: BlockId,
    pub next_run_id: RunId,
}
```

### 7.2 PersistedState (on-disk)

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "schema_version")]
pub enum PersistedState {
    #[serde(rename = "1")]
    V1(PersistedStateV1),
    // V2(PersistedStateV2), ...
}

#[derive(Serialize, Deserialize)]
pub struct PersistedStateV1 {
    pub session_name: Option<String>,
    pub prune: PersistedPrune,
    pub nudges: PersistedNudges,
    pub stats: Stats,
    pub last_updated: String,  // RFC3339
}
```

**Migration**: On load, if the version is older → migrate up. `migrate_v1_to_v2`, etc.

### 7.3 Storage path

The default file store follows XDG:

```
$XDG_DATA_HOME/dynamic_context_pruning/sessions/{session_id}.json
```

Fallback: `~/.local/share/dynamic_context_pruning/sessions/{session_id}.json`.

**Atomic writes**: write to a `.tmp` file, then atomically rename. A backup `.bak` copy is kept before each save.

### 7.4 Idempotent rebuild

```rust
pub fn rebuild_from_messages(
    messages: &[Message],
    persisted_blocks: Vec<CompressionBlock>,
    config: &Config,
) -> SessionState {
    let mut state = SessionState::default();
    state.session_id = messages.last().map(|m| m.id.clone());

    // Replay all blocks into state
    for block in persisted_blocks {
        state.prune.messages.blocks_by_id.insert(block.block_id, block.clone());
        if block.active {
            state.prune.messages.active_block_ids.insert(block.block_id);
            state.prune.messages.active_by_anchor_message_id
                .insert(block.anchor_message_id.clone(), block.block_id);
        }
        state.prune.messages.next_block_id = state.prune.messages.next_block_id.max(block.block_id + 1);
        state.prune.messages.next_run_id = state.prune.messages.next_run_id.max(block.run_id + 1);
    }

    // Sync tool cache, message refs, current turn from messages
    sync_tool_cache(&mut state, config, messages);
    assign_message_refs(&mut state, messages);
    state.current_turn = count_turns(&state, messages);

    state
}
```

**Test**:

```rust
#[test]
fn rebuild_idempotent() {
    let original = run_full_session(&messages, &config);
    let rebuilt = rebuild_from_messages(&messages, original.persisted_blocks(), &config);
    assert_eq!(rebuilt.compute_pruning_decisions(&messages),
               original.compute_pruning_decisions(&messages));
}
```

---

## 8. Configuration schema

### 8.1 JSONC config file

```jsonc
{
    "$schema": "https://raw.githubusercontent.com/<user>/dynamic_context_pruning/master/schema.json",

    "enabled": true,
    "debug": false,

    // Cache stability — KEY DECISION
    "cacheStabilityMode": "agent-message",  // "aggressive" | "agent-message" | "manual"

    // Notifications
    "notification": {
        "level": "detailed",   // "off" | "minimal" | "detailed"
        "kind": "chat"          // "chat" | "toast"
    },

    // Manual mode
    "manualMode": {
        "enabled": false,
        "automaticStrategies": true
    },

    // Turn protection
    "turnProtection": {
        "enabled": false,
        "turns": 4
    },

    // Protected file globs
    "protectedFilePatterns": [
        "**/*.config.ts",
        "Cargo.toml"
    ],

    // Compress tool config
    "compress": {
        "mode": "range",                  // "range" | "message"
        "permission": "allow",            // "ask" | "allow" | "deny"
        "showCompression": false,
        "summaryBuffer": true,
        "maxContextLimit": 100000,        // number or "X%"
        "minContextLimit": 50000,
        "modelMaxLimits": {
            "anthropic/claude-sonnet-4.5": "80%",
            "openai/gpt-5-codex": 120000
        },
        "modelMinLimits": {},
        "nudgeFrequency": 5,
        "iterationNudgeThreshold": 15,
        "nudgeForce": "soft",             // "strong" | "soft"
        "protectedTools": ["task", "skill"],
        "protectTags": false,
        "protectUserMessages": false
    },

    // Strategies
    "strategies": {
        "deduplication": {
            "enabled": true,
            "protectedTools": []
        },
        "purgeErrors": {
            "enabled": true,
            "turns": 4,
            "protectedTools": []
        },
        "staleFileReads": {
            "enabled": true,
            "protectedTools": [],
            "trackedTools": ["read", "write", "edit", "multiedit"]
        }
    },

    // Slash commands
    "commands": {
        "enabled": true,
        "protectedTools": []
    },

    // Experimental
    "experimental": {
        "allowSubagents": false,
        "customPrompts": false
    }
}
```

### 8.2 Cascade

1. Built-in defaults (compiled in)
2. Global: `$XDG_CONFIG_HOME/dynamic_context_pruning/config.jsonc`
3. Custom directory: `$DCP_CONFIG_DIR/config.jsonc`
4. Project: `.dynamic_context_pruning/config.jsonc` in the project root
5. Runtime override (via `Config::with_overrides`)

Each level overrides the previous one. Project config wins over global.

### 8.3 JSON schema validation

Generate `schema.json` from the Rust types via the `schemars` crate. Publish it in the repo. Enables IDE autocomplete.

---

## 9. Integration patterns

### 9.1 jcode (in-process)

**File**: `/data/projects/jcode/src/dcp_bridge.rs` (~80 lines)

```rust
use dynamic_context_pruning as dcp;
use jcode_message_types::{Message as JMsg, ContentBlock, Role as JRole};

pub fn jcode_to_dcp(msgs: &[JMsg]) -> Vec<dcp::Message> {
    msgs.iter().map(|m| dcp::Message {
        id: format!("{:x}", jcode_message_types::stable_message_hash(m)),
        role: match m.role {
            JRole::User => dcp::Role::User,
            JRole::Assistant => dcp::Role::Assistant,
        },
        time: m.timestamp.map(|t| t.timestamp_millis()).unwrap_or(0),
        parts: m.content.iter().filter_map(content_to_part).collect(),
    }).collect()
}

fn content_to_part(b: &ContentBlock) -> Option<dcp::Part> {
    Some(match b {
        ContentBlock::Text { text, .. } => dcp::Part::Text(text.clone()),
        ContentBlock::Reasoning { text } => dcp::Part::Reasoning(text.clone()),
        ContentBlock::ToolUse { id, name, input } => dcp::Part::ToolCall {
            call_id: id.clone(), tool: name.clone(), input: input.clone(),
        },
        ContentBlock::ToolResult { tool_use_id, content, is_error } => dcp::Part::ToolResult {
            call_id: tool_use_id.clone(),
            status: if is_error.unwrap_or(false) { dcp::ToolStatus::Error } else { dcp::ToolStatus::Completed },
            output: Some(content.clone()),
            error: None,
        },
        ContentBlock::Image { media_type, data } => dcp::Part::Image {
            media_type: media_type.clone(), data: data.clone(),
        },
        ContentBlock::OpenAICompaction { .. } => return None,  // skip
    })
}

pub fn dcp_to_jcode(msgs: Vec<dcp::Message>) -> Vec<JMsg> {
    msgs.into_iter().map(|m| JMsg {
        role: match m.role {
            dcp::Role::User => JRole::User,
            dcp::Role::Assistant => JRole::Assistant,
            dcp::Role::System => JRole::User,  // shouldn't happen
        },
        content: m.parts.into_iter().filter_map(part_to_content).collect(),
        timestamp: Some(chrono::DateTime::<chrono::Utc>::from_timestamp_millis(m.time)
            .unwrap_or_else(|| chrono::Utc::now())),
        tool_duration_ms: None,
    }).collect()
}

fn part_to_content(p: dcp::Part) -> Option<ContentBlock> {
    Some(match p {
        dcp::Part::Text(text) => ContentBlock::Text { text, cache_control: None },
        dcp::Part::Reasoning(text) => ContentBlock::Reasoning { text },
        dcp::Part::ToolCall { call_id, tool, input } => ContentBlock::ToolUse {
            id: call_id, name: tool, input,
        },
        dcp::Part::ToolResult { call_id, status, output, error } => ContentBlock::ToolResult {
            tool_use_id: call_id,
            content: output.or(error).unwrap_or_default(),
            is_error: matches!(status, dcp::ToolStatus::Error).then_some(true),
        },
        dcp::Part::Image { media_type, data } => ContentBlock::Image { media_type, data },
    })
}
```

**Hook**: Modify `/data/projects/jcode/src/agent.rs:581` `messages_for_provider`:

```rust
fn messages_for_provider(&mut self) -> (Vec<Message>, Option<CompactionEvent>) {
    let mut raw = self.session.provider_messages().to_vec();

    // NEW: DCP transform
    if let Some(pruner) = self.dcp.as_mut() {
        let dcp_msgs = dcp_bridge::jcode_to_dcp(&raw);
        match pruner.transform_messages(dcp_msgs) {
            Ok(transformed) => raw = dcp_bridge::dcp_to_jcode(transformed),
            Err(e) => logging::warn(&format!("DCP transform failed, using raw: {}", e)),
        }
    }

    // ... rest of existing logic (compaction-core fallback)
    (raw, None)
}
```

**Tool registration**: Add `compress` to the `crate::tool` registry; the callback invokes `dcp.handle_compress(...)`.

**Slash command**: Add `/dcp` to the `src/cli/commands.rs` dispatcher.

### 9.2 Claude Code hook (binary)

**Binary**: `dcp-claude-hook` from the `claude-hook` feature.

**Install**:
```bash
cargo install --path bins/dcp-claude-hook
# or:
cargo install dynamic_context_pruning --features claude-hook
```

**Register hook** in `~/.claude/settings.json`:
```json
{
  "hooks": {
    "PreToolUse": [{
      "matcher": "*",
      "hooks": [{
        "type": "command",
        "command": "$HOME/.cargo/bin/dcp-claude-hook"
      }]
    }],
    "SessionStart": [{
      "matcher": "compact",
      "hooks": [{
        "type": "command",
        "command": "$HOME/.cargo/bin/dcp-claude-hook --on-compact"
      }]
    }]
  }
}
```

**Behavior**: The hook receives stdin JSON with session info, transforms the messages, and outputs the transformed JSON. Claude Code uses the result instead of the raw messages.

### 9.3 MCP service (optional bin)

**Binary**: `dcp-mcp` from the `mcp` feature.

**Tools exposed**:
- `compress` (range/message)
- `decompress(block_id)`
- `recompress(block_id)`
- `dcp_context` — show breakdown
- `dcp_stats` — cumulative stats
- `dcp_sweep [count]` — manual sweep

**Resources**:
- `dcp://session/{id}/state`
- `dcp://session/{id}/blocks`

**Run**:
```bash
dcp-mcp --transport stdio
dcp-mcp --transport http --port 7820
```

**Client config** (e.g. for Codex):
```toml
[mcp.dcp]
command = "dcp-mcp"
args = ["--transport", "stdio"]
```

### 9.4 rig adapter (v1.1)

```rust
use rig::completion::CompletionModel;
use dynamic_context_pruning::ContextPruner;

pub struct PrunedAgent<M: CompletionModel> {
    inner: rig::agent::Agent<M>,
    pruner: ContextPruner,
}

impl<M: CompletionModel> PrunedAgent<M> {
    pub async fn complete(&mut self, msgs: Vec<rig::Message>) -> Result<String> {
        let dcp_msgs = rig_to_dcp(msgs);
        let pruned = self.pruner.transform_messages(dcp_msgs)?;
        let rig_msgs = dcp_to_rig(pruned);
        self.inner.complete(rig_msgs).await
    }
}
```

---

## 10. Phased delivery

### Phase 0 — Spec & scaffold (week 0)

- ✅ Lock 5 decisions
- ✅ Write `PLAN.md`
- 🔲 Write `SPEC.md` — clean-room behavior spec from research, with no upstream code references
- 🔲 Init git repo, set MIT license, push initial commit
- 🔲 Scaffold the workspace with 14 crate stubs (`cargo new --lib` for each crate)
- 🔲 CI: GitHub Actions running cargo test + cargo clippy + cargo fmt --check

**Acceptance**: `cargo build --workspace` passes (stubs only).

### Phase 1 — Foundation (weeks 1-2)

- 🔲 `dcp-types`: Message, Part, Role, BlockId, RunId, MessageRef, core types
- 🔲 `dcp-traits`: Tokenizer, StatePersistence, MemoryRetriever, CacheAccountant, PruneStrategy
- 🔲 `dcp-tokens`: char/4 default, tokenizers feature, tiktoken-fast feature
- 🔲 `dcp-protected`: glob matcher (via `globset`), is_tool_protected, is_path_protected

**Acceptance**:
- 100% test parity with the public spec for token counting (<1% tolerance vs the Anthropic API)
- Glob matcher passes 50+ test cases (including `**`, `*`, `?`)

### Phase 2 — State + Storage (week 3)

- 🔲 `dcp-state`: SessionState, transitions, idempotent rebuild
- 🔲 `dcp-storage`: FileStateStore, InMemoryStore implementations

**Acceptance**:
- Property test `prop_rebuild_idempotent`: 1000 random sessions → state(rebuild) == state(original)
- Round-trip serialize/deserialize works for the V1 schema
- Atomic writes do not corrupt data on crash (verified with fault injection)

### Phase 3 — Prune strategies + Cache stability (weeks 4-5)

- 🔲 `dcp-prune`: deduplicate, purge_errors, **stale_file_reads**, prune-to-messages applier
- 🔲 `dcp-core`: CacheStabilityMode logic, pending prune, force_apply

**Acceptance**:
- Property test: tool call/result pairing preserved after each strategy
- Snapshot test: same input → same output across runs
- AgentMessage mode: does not apply mid-tool-turn

### Phase 4 — Compress + Frontier (weeks 6-7)

- 🔲 `dcp-compress`: range mode, message mode, block bookkeeping
- 🔲 Prune frontier — do not retry oversized summaries
- 🔲 `dcp-prompts`: 6 default prompts embedded
- 🔲 `dcp-nudges`: context-limit, turn, iteration

**Acceptance**:
- Compress tool tests: range overlap detected, placeholder validation works
- Nesting test: compress over a compressed range → properly consume + deactivate
- Frontier test: oversized summary → frontier advances, raw is kept

### Phase 5 — Facade + API freeze (week 8)

- 🔲 `dcp-core`: ContextPruner facade, builder, complete API
- 🔲 `dynamic_context_pruning`: umbrella crate
- 🔲 4 examples build and run
- 🔲 `cargo doc` clean, no warnings

**Acceptance**:
- Public API surface review (no leaked `pub` items)
- All API methods have doc comments with examples
- Examples run: minimal, jcode_integration, custom_agent, with_memory

### Phase 6 — jcode integration (week 9)

- 🔲 Adapter `dcp_bridge.rs` in jcode
- 🔲 Hook `Agent::messages_for_provider`
- 🔲 Register `compress` tool
- 🔲 Slash `/dcp` command
- 🔲 Integration test with a mocked provider

**Acceptance**:
- jcode builds with the `dynamic_context_pruning` dep — no errors
- E2E test: 100-message session → DCP transforms → tokens reduced ≥30%
- Cache hit rate measured: not worse than baseline

### Phase 7 — Test parity (week 10)

- 🔲 Port all relevant external tests into Rust integration tests
- 🔲 Snapshot tests for compression outputs
- 🔲 Behavior parity verified

**Acceptance**: All tests pass.

### Phase 8 — Optional bins (bonus)

- 🔲 `dcp-cli` interactive REPL
- 🔲 `dcp-mcp` MCP server using rmcp
- 🔲 `dcp-claude-hook` Claude Code hook

**Acceptance**: Each binary works standalone with an example session.

---

## 11. Testing strategy

### 11.1 Test pyramid

```
                ┌──────────────┐
                │   Property   │  100+ random inputs
                │     tests    │  proptest crate
                └──────┬───────┘
                       │
              ┌────────▼────────┐
              │   Integration   │  jcode hook, E2E
              │     tests       │  ~30 scenarios
              └────────┬────────┘
                       │
            ┌──────────▼──────────┐
            │      Unit tests     │  per-module
            │                     │  ~200 cases
            └─────────────────────┘
```

### 11.2 Property tests (`proptest`)

```rust
proptest! {
    #[test]
    fn prop_tool_pairing_preserved(messages in arb_messages()) {
        let pruned = pruner.transform_messages(messages.clone()).unwrap();
        for msg in &pruned {
            for part in &msg.parts {
                if let Part::ToolResult { call_id, .. } = part {
                    let has_call = pruned.iter().any(|m| m.parts.iter().any(|p|
                        matches!(p, Part::ToolCall { call_id: c, .. } if c == call_id)
                    ));
                    prop_assert!(has_call, "ToolResult {} without ToolCall", call_id);
                }
            }
        }
    }

    #[test]
    fn prop_idempotent_rebuild(messages in arb_messages()) {
        let original = full_run(&messages);
        let rebuilt = rebuild_from_messages(&messages, original.blocks(), &config);
        prop_assert_eq!(rebuilt.decisions(&messages), original.decisions(&messages));
    }

    #[test]
    fn prop_block_ids_monotonic(events in arb_compress_events()) {
        let mut state = SessionState::default();
        let mut last = 0;
        for ev in events {
            let id = state.allocate_block_id();
            prop_assert!(id > last);
            last = id;
        }
    }
}
```

### 11.3 Snapshot tests (`insta`)

```rust
#[test]
fn snapshot_compress_range_basic() {
    let messages = load_fixture("conversation_50_turns.json");
    let mut pruner = ContextPruner::new(Config::default()).unwrap();
    let args = CompressArgs::range("Auth exploration", "m0001", "m0012", "Summary...");
    let result = pruner.handle_compress(args, &messages).unwrap();
    insta::assert_yaml_snapshot!(result);
}
```

### 11.4 Cache-stability tests

```rust
#[test]
fn cache_agent_message_mode_no_mid_turn_apply() {
    let mut pruner = ContextPruner::new(Config {
        cache_stability_mode: CacheStabilityMode::AgentMessage,
        ..Default::default()
    }).unwrap();

    // Simulate: user → assistant tool call → tool result → assistant tool call → ...
    let mid_turn_messages = build_mid_tool_turn_messages();
    let r1 = pruner.transform_messages(mid_turn_messages.clone()).unwrap();

    // Mid-turn: no prune applied
    assert_eq!(r1, mid_turn_messages);

    // After assistant text turn end: prune applied
    let end_turn_messages = build_assistant_text_turn_end();
    let r2 = pruner.transform_messages(end_turn_messages).unwrap();
    assert_ne!(r2, mid_turn_messages);
}
```

### 11.5 Benchmarks (`criterion`)

```rust
fn bench_transform_messages(c: &mut Criterion) {
    let messages = load_fixture("100_turn_session.json");
    let mut pruner = ContextPruner::new(Config::default()).unwrap();
    c.bench_function("transform_100_msgs", |b| {
        b.iter(|| pruner.transform_messages(black_box(messages.clone())))
    });
}
```

**Target**: `transform_messages` < 5ms for a 100-message session.

---

## 12. Risks & mitigation

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Tokenizer drift (HF tokenizer.json does not exactly match Claude) | Medium | Medium | Property test against fixtures from the Anthropic token-counting endpoint; expose a tolerance %. Document the approximation in the README. |
| R2 | Persistence corruption (atomic write fails) | Low | High | Schema version + migration path + `--repair` command + a single `.bak` backup copy retained. |
| R3 | Provider format breaking changes | Medium | Medium | Canonical IR `dcp-types` does not change. Adapters convert in/out — breakage occurs in the adapter, not the core. |
| R4 | AGPL upstream license (clean-room contamination) | Medium | Critical | DO NOT READ upstream source code while writing Rust. The spec is written in prose from public docs only. PRs are reviewed for derived-work language. |
| R5 | Cache miss costs > token savings | Medium | High | Default CacheStabilityMode = AgentMessage. CacheAccountant trait for the host to monitor. Telemetry exposes `cache_bust_events`. |
| R6 | Public API churn | Medium | Medium | `#[non_exhaustive]` enums. Strict SemVer. RFC process for breaking changes after v1.0. |
| R7 | Performance regression on large sessions | Low | Medium | Bench gate in CI: transform_messages < 5ms for 100 msgs. Fail if it regresses by 20%. |
| R8 | Subagent context bleeding | Low | Medium | `is_subagent` flag, `experimental.allow_subagents` defaults to false. Test nested agent scenarios. |
| R9 | Prompt injection via summary content | Medium | Medium | Sanitize summary content; escape `<dcp-*>` tags inside user-controllable input. |
| R10 | UTF-8 truncation panic | Low | Medium | `truncate_str_boundary` preserves char boundaries. Property test with multi-byte UTF-8. |

---

## 13. License & clean-room methodology

### 13.1 License choice

- **MIT** — single license, clear, no riders.
- Standard LICENSE file:

```
MIT License

Copyright (c) 2026 <Your Name>

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, ...
```

### 13.2 Clean-room rules

| Rule | Description |
|---|---|
| **R1**: Do not read upstream source code while writing Rust | Once an upstream file has been read, the corresponding Rust file may not be authored. |
| **R2**: Spec from public docs only | README, JSON schema, blog posts, behavior tests. Not source code. |
| **R3**: SPEC.md is the source of truth | All decisions reference SPEC.md, not upstream source. |
| **R4**: No mention of upstream | Do not name upstream projects in code, comments, commit messages, docs, or README. |
| **R5**: Naming differs | Struct/method names differ from upstream (ContextPruner ≠ Plugin/PluginConfig). |
| **R6**: Architecture differs | Crate split and trait boundaries are our design choices, not a clone of an upstream folder structure. |
| **R7**: Distinctive features | stale_file_reads, CacheStabilityMode, prune_frontier, 3-segment workspace — original contributions. |
| **R8**: Reviewed at each PR | Review checklist: derived-work language? upstream references? code similarity? |

### 13.3 Workflow

```
┌──────────────────────────┐
│  Read upstream specs     │ ← Done during the research phase
│  (public docs only)      │
└──────┬───────────────────┘
       │
┌──────▼──────────────────────────────────┐
│  Write SPEC.md (behavior spec, prose)    │ ← Phase 0
│  Do not reference upstream source        │
└──────┬──────────────────────────────────┘
       │
┌──────▼──────────────────────┐
│  Stop reading upstream code  │ ← After Phase 0
│  Implement Rust from SPEC.md │
└──────────────────────────────┘
```

### 13.4 SPEC.md outline (Appendix B)

Will contain behavior specifications in the form:

> **Strategy: Deduplicate**
>
> Goal: Among multiple tool calls with identical effective parameters, retain only the most recent.
>
> Inputs: Session state with `tool_id_list`, `tool_parameters`. Config `protected_tools`.
>
> Algorithm:
> 1. Filter out already-pruned IDs and protected-tool IDs.
> 2. Group remaining IDs by signature. Signature = `tool_name :: canonical_json(sorted(non_null_params))`.
> 3. In each group of size ≥ 2, mark all-except-last for pruning.
> 4. ...

Not code. Specs only.

---

## 14. Locked decisions

| ID | Decision | Value | Rationale |
|---|---|---|---|
| **D1** | Repo location | Standalone `github.com/<user>/dynamic_context_pruning` | Reusable across agents, not bound to jcode |
| **D2** | Async vs sync | Sync core + `async` feature flag for an async facade | Main logic is CPU-bound, sync is cheap; async only wraps |
| **D3** | Default tokenizer | Char/4 (no deps) + `tokenizers` feature default + `tiktoken-fast` feature | Universal accuracy via HF, fast for OpenAI, zero-dep fallback |
| **D4** | License | MIT | Clear, permissive, no AGPL contamination (clean-room requirement) |
| **D5** | Cache stability | Mandatory in v1, default `AgentMessage` mode | Without it → net-negative cost on Anthropic/Bedrock/Gemini |
| **D6** | Repo name | `dynamic_context_pruning` | User-confirmed |
| **D7** | Facade struct name | `ContextPruner` | Descriptive, does not collide with upstream |
| **D8** | Methodology | Clean-room | Required to license MIT given AGPL upstream |
| **D9** | Branding | Standalone Rust library, not "port of X" | User-confirmed |

---

## 15. Open questions

| # | Question | Decision needed before |
|---|---|---|
| Q1 | GitHub username/org for the repo? | Phase 0 (init repo) |
| Q2 | Publish to crates.io at v0.1 or wait for v0.5? | Phase 5 (API freeze) |
| Q3 | MSRV (Minimum Supported Rust Version)? | Phase 1 (CI setup) — proposed 1.75 |
| Q4 | CI provider: GitHub Actions or GitLab CI? | Phase 0 |
| Q5 | Doc hosting: docs.rs (auto) or custom site? | Phase 5 |
| Q6 | Logo / branding? | Optional, after v0.1 |
| Q7 | Discord/Slack community channel? | After public release |

---

## Appendix A — Cargo.toml skeletons

### A.1 Workspace root

```toml
[workspace]
resolver = "2"
members = [
    "crates/dcp-types",
    "crates/dcp-traits",
    "crates/dcp-tokens",
    "crates/dcp-protected",
    "crates/dcp-state",
    "crates/dcp-storage",
    "crates/dcp-prune",
    "crates/dcp-compress",
    "crates/dcp-prompts",
    "crates/dcp-nudges",
    "crates/dcp-config",
    "crates/dcp-telemetry",
    "crates/dcp-core",
    "crates/dynamic_context_pruning",
]
default-members = ["crates/dynamic_context_pruning"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.75"
license = "MIT"
repository = "https://github.com/<user>/dynamic_context_pruning"
keywords = ["llm", "agent", "context", "pruning", "tokens"]
categories = ["development-tools"]

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
thiserror = "1"
anyhow = "1"
tokio = { version = "1", optional = true }
proptest = { version = "1", optional = true }
insta = { version = "1", optional = true }
criterion = { version = "0.5", optional = true }

# Internal
dcp-types = { path = "crates/dcp-types", version = "=0.1.0" }
dcp-traits = { path = "crates/dcp-traits", version = "=0.1.0" }
# ... (other internal)

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

### A.2 Umbrella crate `crates/dynamic_context_pruning/Cargo.toml`

```toml
[package]
name = "dynamic_context_pruning"
description = "Dynamic context pruning library for LLM agents"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
keywords.workspace = true
categories.workspace = true

[dependencies]
dcp-core = { path = "../dcp-core", version = "=0.1.0" }
dcp-types = { workspace = true }
dcp-traits = { workspace = true }
dcp-config = { workspace = true }

[features]
default = []
async = ["dcp-core/async"]
tokenizers = ["dcp-core/tokenizers"]
tiktoken-fast = ["dcp-core/tiktoken-fast"]
claude-tokens = ["dcp-core/claude-tokens"]
mcp = ["dep:rmcp"]
claude-hook = []
rig = ["dep:rig"]

[dependencies.rmcp]
version = "0.1"
optional = true

[dependencies.rig]
version = "0.5"
optional = true

[lib]
path = "src/lib.rs"
```

### A.3 Leaf crate `crates/dcp-types/Cargo.toml`

```toml
[package]
name = "dcp-types"
description = "Canonical message types for dynamic_context_pruning"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
```

---

## Appendix B — SPEC.md outline

`SPEC.md` will be written separately and is the sole source of truth for implementation. Outline:

1. **Glossary** — definitions for: Message, Block, Run, Anchor, Compaction, Prune, Frontier
2. **Canonical IR** — Message/Part shape, role semantics, ID rules
3. **Session lifecycle** — start, turn, compaction, end
4. **Tool tracking** — call/result pairing, status transitions
5. **Strategies**:
   - 5.1 Deduplicate (full algorithm pseudocode)
   - 5.2 Purge errors
   - 5.3 Stale file reads
6. **Compression**:
   - 6.1 Range mode protocol
   - 6.2 Message mode protocol
   - 6.3 Block bookkeeping rules
   - 6.4 Nesting & consumption
   - 6.5 Frontier mechanism
7. **Cache stability**:
   - 7.1 Mode definitions
   - 7.2 Pending state semantics
   - 7.3 Apply triggers
8. **Nudges**:
   - 8.1 Context-limit
   - 8.2 Turn
   - 8.3 Iteration
9. **Persistence**:
   - 9.1 Schema V1
   - 9.2 Migration rules
   - 9.3 Atomic write protocol
10. **Configuration**:
    - 10.1 Cascade order
    - 10.2 Field semantics
    - 10.3 Validation rules
11. **Edge cases & invariants**:
    - 11.1 Tool call/result pairing must be preserved
    - 11.2 Block IDs monotonic
    - 11.3 UTF-8 boundary safety
    - 11.4 Idempotent rebuild guarantee
12. **Test fixtures** — minimum coverage matrix

---

## Final notes

**Plan version**: v6 (locked 2026-05-25)
**Next action**: User confirms Q1-Q5 in [Open questions](#15-open-questions) → begin Phase 0.

When ready:
1. Confirm Q1 (GitHub user/org).
2. Run scaffold: `cargo init --lib` for each crate, push initial commit.
3. Begin SPEC.md from research notes (no upstream source references).
