# dcp — Dynamic Context Pruning

<div align="center">
  <img src="dcp_illustration.webp" alt="dcp — deterministic + LLM context pruning for coding agents">
</div>

<div align="center">

![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-blue.svg)
![Rust](https://img.shields.io/badge/Rust-1.85%2B-orange.svg)
![License](https://img.shields.io/badge/License-AGPL--3.0--or--later-blue.svg)
![Release](https://img.shields.io/github/v/release/quangdang46/dynamic_context_pruning?include_prereleases)

</div>

**Deterministic + LLM-driven context pruning for coding agents.**  
Shrink session token load with dedup, error purge, stale-read removal, and optional LLM compression — via library embed or OpenCode plugin.

---

## 🤖 Agent Quickstart (Robot Mode)

```rust
use dcp_config::Config;
use dcp_core::ContextPruner;

let pruner = ContextPruner::builder()
    .config(Config::load_default()?)
    .build()?;
let pruned = pruner.transform_messages(messages)?;
```

Or as **OpenCode plugin** — `/dcp` opens the prune panel:

| Slash | Action |
|-------|--------|
| `/dcp` | Open panel |
| `/dcp context` | Token breakdown |
| `/dcp stats` | Prune stats |
| `/dcp sweep` | Flush pending strategies |
| `/dcp decompress <id>` | Restore a block |
| `/dcp recompress <id>` | Re-activate a block |
| `/dcp-compress [focus]` | Manual compression |

---

## TL;DR

### The Problem

Long agent sessions fill context with noise:

| Noise | Cost |
|-------|------|
| Duplicate tool results | Tokens for the same file twice |
| Stale file reads | Old snapshots after later edits |
| Error dumps already fixed | Dead stack traces |
| Unstructured history | Cache thrash + higher latency |

### The Solution

**dcp** prunes first with **deterministic strategies**, then optionally **LLM-compresses** ranges while tracking blocks for cache stability.

| Surface | What you get |
|---------|--------------|
| Library (`dcp-core`) | Embed `ContextPruner` in your agent |
| OpenCode plugin | Real-time prune panel + slash commands |

### Why Use dcp?

| Feature | What it does |
|---------|--------------|
| **Deterministic first** | Dedup, purge errors, drop stale file reads — no model required |
| **Optional LLM compress** | Range / message modes with block bookkeeping |
| **Cache-aware modes** | `aggressive` · `agent-message` · `manual` stability modes |
| **Protected globs** | Never prune secrets (e.g. `**/.env`) |
| **Agent-native surfaces** | Library + OpenCode plugin |
| **JSON robot output** | Scriptable analytics for agent loops |

---

### Quick Example

```rust
use dcp_config::Config;
use dcp_core::ContextPruner;

let pruner = ContextPruner::builder()
    .config(Config::load_default()?)
    .build()?;

// Deterministic strategies run automatically on transform
let pruned = pruner
    .transform_messages(raw_messages)?;

// Optional: LLM compress a range
use dcp_compress::{CompressArgs, RangeEntry};
pruner.handle_compress(
    CompressArgs::Range {
        topic: "session recap".into(),
        content: vec![RangeEntry {
            start_id: first.id.clone(),
            end_id: last.id.clone(),
            summary: "Compress early messages".into(),
        }],
    },
    &pruned,
)?;
```

---

## Design Philosophy

1. **Deterministic before generative.**  
   Cheap, predictable strategies run first. LLM compression is opt-in and block-tracked.

2. **Cache stability is a first-class goal.**  
   Naive "summarize everything" thrashes provider caches. dcp tracks compressible blocks and modes.

3. **Never silently destroy protected content.**  
   Glob protection keeps secrets and critical paths out of prune paths.

4. **Embeddable, not only a CLI.**  
   `dcp-core`'s `ContextPruner` is the library facade; plugins are thin surfaces.

5. **Degrade cleanly.**  
   No LLM backend configured? Deterministic strategies still run.

---

## How dcp Compares

| Approach | Control | Cache-aware | Agent-native | Restorable blocks |
|----------|---------|-------------|--------------|-------------------|
| Manual `/clear` | Coarse | No | No | No |
| Summarize-everything | Lossy | Often breaks | Partial | Rarely |
| Provider compact only | Opaque | Varies | Built-in only | Varies |
| **dcp** | Strategy + blocks | Yes | Library + plugin | Yes |

**When to use dcp:**
- Long coding-agent sessions with repeated tool reads
- Embedding pruning inside a custom agent runtime
- OpenCode users who want a live prune panel

**When dcp might not be ideal:**
- Single-turn prompts with tiny context
- Environments where you cannot persist block payloads for decompress

---

## Installation

### OpenCode plugin

```bash
opencode plugin @qdang46/opencode-dcp-plugin@latest --global
```

Restart OpenCode → `/dcp` opens the panel.

### From source

```bash
git clone https://github.com/quangdang46/dynamic_context_pruning.git
cd dynamic_context_pruning
cargo build --workspace --release
```

Requires **Rust 1.85+** (edition 2024).

### As a library

```toml
[dependencies]
dcp-core = { git = "https://github.com/quangdang46/dynamic_context_pruning" }
dcp-config = { git = "https://github.com/quangdang46/dynamic_context_pruning" }
```

```rust
use dcp_config::Config;
use dcp_core::ContextPruner;

let cfg = Config::load_default()?;
cfg.validate()?;
let pruner = ContextPruner::builder().config(cfg).build()?;
```

The same interface is exported via **NAPI bindings** for Node.js / Python consumers through the OpenCode plugin bridge (`opencode-dcp-bridge`).

See `examples/01_minimal.rs` … `06_cache_stability.rs`.

---

## Configuration

Later tiers win per key (arrays replace wholesale):

1. Built-in defaults  
2. `$XDG_CONFIG_HOME/dynamic_context_pruning/config.jsonc`  
3. `$DCP_CONFIG_DIR/config.jsonc`  
4. `.dynamic_context_pruning/config.jsonc` (project / ancestor)

```jsonc
{
  "enabled": true,
  // "aggressive" | "agent-message" | "manual"
  "cacheStabilityMode": "agent-message",
  "protectedFilePatterns": ["**/.env"],
  "compress": {
    "mode": "range",
    "maxBlocks": 50,
    "minTokens": 2000
  },
  "strategies": {
    "dedup": { "enabled": true },
    "purgeErrors": { "enabled": true },
    "staleFileReads": { "enabled": true, "maxAge": 3600 }
  },
  "notification": { "level": "essential" }
}
```

```rust
use dcp_config::Config;
let cfg = Config::load_default()?;
cfg.validate()?;
```

Schema reference: [`dcp.schema.json`](opencode-dcp-plugin/dcp.schema.json).

---

## Architecture

Modular Rust workspace (~18 crates):

```text
dcp-types / dcp-traits
    ├── dcp-tokens          token backends
    ├── dcp-protected       glob protection
    ├── dcp-state           session transitions
    │     ├── dcp-storage
    │     ├── dcp-prune       dedup · purge_errors · stale_file_reads
    │     ├── dcp-compress    LLM range/message modes
    │     └── dcp-nudges
    ├── dcp-config          4-tier cascade
    ├── dcp-core            ContextPruner facade
    └── opencode-dcp-bridge
```

| Crate | Role |
|-------|------|
| `dcp-core` | Primary library entry (`ContextPruner`) |
| `dcp-prune` | Deterministic strategies |
| `dcp-compress` | LLM compression + block bookkeeping |
| `dcp-config` | JSONC cascade: builtin → global → custom → project |
| `opencode-dcp-bridge` | OpenCode plugin bridge (NAPI bindings) |

---

## Testing

```bash
cargo test --workspace
cargo test -p dcp-core
```

| Kind | Where |
|------|-------|
| Unit | Inline `#[cfg(test)]` |
| Property | `dcp-state` + proptest |
| Snapshot | insta |
| Examples | `examples/01_minimal.rs` … `06_cache_stability.rs` |

---

## Troubleshooting

### OpenCode panel missing

```bash
opencode plugin @qdang46/opencode-dcp-plugin@latest --global
# fully restart OpenCode, then:
/dcp
```

### Decompress cannot restore a block

Blocks are tracked; restore depends on the persistence backend still holding the payload. If the block was GC'd or storage was wiped, compress from source messages instead.

### Library usage questions

Check the examples under `examples/` or open an issue on GitHub.

---

## CLI predecessors

This project was originally distributed as a standalone CLI binary. That binary has been removed; the library and OpenCode plugin are the supported surfaces going forward. If you need the CLI, pin to an earlier release.

---

## Limitations

### What dcp Doesn't Do (Yet)

- **Not a full agent runtime** — prunes context; does not replace the agent loop
- **LLM compress quality** depends on the model you wire in
- **Wrong cache mode** can still thrash provider caches if misconfigured

### Known Limitations

| Capability | Current state | Notes |
|------------|---------------|-------|
| Deterministic-only mode | ✅ | Disable compress / leave LLM unconfigured |
| Pixel-perfect memory of every byte | ⚠️ Block-based | Decompress needs persisted payload |
| Multi-agent shared store | ⚠️ | Design for single-session storage first |

---

## FAQ

### Deterministic only?

Yes — disable compress strategies / leave LLM backends unconfigured; dedup + purge + stale-read still run.

### Is OpenCode required?

No. The library (`dcp-core`) works standalone.

### Does decompress always restore originals?

Blocks are tracked; restore depends on the persistence backend still holding the payload.

### How do I protect secrets?

Set `protectedFilePatterns` in config (e.g. `**/.env`, `**/credentials.json`).

### Can I embed this in my own agent?

Yes — use `dcp-core`'s `ContextPruner` and the examples under `examples/`.

### What is `cacheStabilityMode`?

Controls how aggressively compressed ranges are allowed to invalidate provider prompt caches. Prefer `agent-message` unless you know you need `aggressive` or full `manual` control.

### Are there Node.js / Python bindings?

Yes — the `opencode-dcp-bridge` crate exports NAPI bindings consumed by the OpenCode plugin, and the same interface can be used from Node.js or Python.

---

## About Contributions

Please don't take this the wrong way, but I do not accept outside contributions for any of my projects. I simply don't have the mental bandwidth to review anything, and it's my name on the thing, so I'm responsible for any problems it causes; thus, the risk-reward is highly asymmetric from my perspective. I'd also have to worry about other "stakeholders," which seems unwise for tools I mostly make for myself for free. Feel free to submit issues, and even PRs if you want to illustrate a proposed fix, but know I won't merge them directly. Instead, I'll have Claude or Codex review submissions via `gh` and independently decide whether and how to address them. Bug reports in particular are welcome. Sorry if this offends, but I want to avoid wasted time and hurt feelings. I understand this isn't in sync with the prevailing open-source ethos that seeks community contributions, but it's the only way I can move at this velocity and keep my sanity.

---

## License

[AGPL-3.0-or-later](LICENSE)

---

<div align="center">

**Less noise. More context for the work that matters.**

</div>
