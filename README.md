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
Shrink session token load with dedup, error purge, stale-read removal, and optional LLM compression — via CLI, library embed, or OpenCode plugin.

<div align="center">

```bash
curl -fsSL "https://raw.githubusercontent.com/quangdang46/dynamic_context_pruning/main/install.sh?$(date +%s)" \
  | bash -s -- --easy-mode --verify
```

</div>

---

## 🤖 Agent Quickstart (Robot Mode)

```bash
# Token usage breakdown
dcp token-stats --json
dcp message-tokens --session SESSION_ID --json --no-color

# Compression timeline
dcp timeline -s $SESSION_ID --format json

# Flush pending deterministic prunes
dcp sweep

# Compress messages (stdin or file)
dcp compress messages.json
dcp decompress b1
dcp recompress b1
```

Also available as **OpenCode plugin** — `/dcp` opens the prune panel.

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
| `dcp` CLI | stats, timeline, sweep, compress / decompress |
| Library (`dcp-core`) | Embed `ContextPruner` in your agent |
| OpenCode plugin | Real-time prune panel + slash commands |

### Why Use dcp?

| Feature | What it does |
|---------|--------------|
| **Deterministic first** | Dedup, purge errors, drop stale file reads — no model required |
| **Optional LLM compress** | Range / message modes with block bookkeeping |
| **Cache-aware modes** | `aggressive` · `agent-message` · `manual` stability modes |
| **Protected globs** | Never prune secrets (e.g. `**/.env`) |
| **Agent-native surfaces** | CLI + library + OpenCode plugin |
| **JSON robot output** | Scriptable analytics for agent loops |

---

### Quick Example

```bash
# Install
curl -fsSL "https://raw.githubusercontent.com/quangdang46/dynamic_context_pruning/main/install.sh?$(date +%s)" \
  | bash -s -- --easy-mode --verify

# Inspect context + stats
dcp context
dcp stats --session-id "$SESSION_ID"
dcp timeline -s "$SESSION_ID"

# Flush pending deterministic prunes
dcp sweep

# Compress a message batch (stdin or file)
dcp compress messages.json
dcp decompress b1
dcp recompress b1
```

---

## Design Philosophy

1. **Deterministic before generative.**  
   Cheap, predictable strategies run first. LLM compression is opt-in and block-tracked.

2. **Cache stability is a first-class goal.**  
   Naive “summarize everything” thrashs provider caches. dcp tracks compressible blocks and modes.

3. **Never silently destroy protected content.**  
   Glob protection keeps secrets and critical paths out of prune paths.

4. **Embeddable, not only a CLI.**  
   `dcp-core`’s `ContextPruner` is the library facade; CLI and plugins are thin surfaces.

5. **Degrade cleanly.**  
   No LLM backend configured? Deterministic strategies still run.

---

## How dcp Compares

| Approach | Control | Cache-aware | Agent-native | Restorable blocks |
|----------|---------|-------------|--------------|-------------------|
| Manual `/clear` | Coarse | No | No | No |
| Summarize-everything | Lossy | Often breaks | Partial | Rarely |
| Provider compact only | Opaque | Varies | Built-in only | Varies |
| **dcp** | Strategy + blocks | Yes | CLI + plugin + lib | Yes |

**When to use dcp:**
- Long coding-agent sessions with repeated tool reads
- Embedding pruning inside a custom agent runtime
- OpenCode users who want a live prune panel

**When dcp might not be ideal:**
- Single-turn prompts with tiny context
- Environments where you cannot persist block payloads for decompress

---

## Installation

### CLI binary

```bash
# macOS / Linux
curl -fsSL "https://raw.githubusercontent.com/quangdang46/dynamic_context_pruning/main/install.sh?$(date +%s)" \
  | bash -s -- --easy-mode --verify

# Windows PowerShell
irm "https://raw.githubusercontent.com/quangdang46/dynamic_context_pruning/main/install.ps1" | iex
```

### OpenCode plugin

```bash
opencode plugin @qdang46/opencode-dcp-plugin@latest --global
```

Restart OpenCode → `/dcp` opens the panel.

| Slash | Action |
|-------|--------|
| `/dcp` | Open panel |
| `/dcp context` | Token breakdown |
| `/dcp stats` | Prune stats |
| `/dcp sweep` | Flush pending strategies |
| `/dcp decompress <id>` | Restore a block |
| `/dcp recompress <id>` | Re-activate a block |
| `/dcp-compress [focus]` | Manual compression |

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

See `examples/01_minimal.rs` … `06_cache_stability.rs`.

---

## Quick Start

```bash
dcp context
dcp stats --session-id <SESSION_ID>
dcp timeline -s <SESSION_ID>
dcp sweep
dcp compress messages.json
dcp decompress b1
dcp recompress b1
dcp manual on
dcp manual off
```

### Robot / script surface

```bash
dcp token-stats --json
dcp message-tokens --session SESSION_ID --json --no-color
dcp find-session --pattern "session-*" --after 2024-01-01
```

> Analytics commands need the CLI built with `--features scripts` in some builds.

---

## Commands

| Command | Description |
|---------|-------------|
| `context` | Token usage breakdown + pruning stats |
| `stats -s <id>` | Session statistics (saved tokens, ratio) |
| `timeline -s <id>` | Compression events over time |
| `find-session` | Find sessions by pattern / date range |
| `get-message` | Full message payload(s) by ID |
| `token-stats` | Aggregate token stats (`--json`) |
| `message-tokens` | Per-message breakdown (`--json`) |
| `sweep [n]` | Flush pending prune tools (default: all) |
| `compress [file]` | One-shot compress (stdin / file) |
| `decompress <id>` | Restore compressed block |
| `recompress <id>` | Re-activate a user-decompressed block |
| `manual [on\|off]` | Toggle / show manual mode |

```bash
dcp stats --session-id abc123
dcp timeline -s abc123
dcp find-session --pattern "session-*" --after 2026-01-01
dcp compress -                 # stdin
dcp decompress b1
```

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

Modular Rust workspace (~19 crates):

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
| `opencode-dcp-bridge` | OpenCode plugin bridge |

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

### `dcp: command not found`

```bash
curl -fsSL "https://raw.githubusercontent.com/quangdang46/dynamic_context_pruning/main/install.sh?$(date +%s)" \
  | bash -s -- --easy-mode --verify
hash -r
dcp --help
```

### Stats / timeline empty for a session

Confirm the session ID and that the storage backend actually recorded the session:

```bash
dcp find-session --pattern "*"
dcp stats --session-id <exact-id>
```

### Decompress cannot restore a block

Blocks are tracked; restore depends on the persistence backend still holding the payload. If the block was GC’d or storage was wiped, recompress from source messages instead.

### OpenCode panel missing

```bash
opencode plugin @qdang46/opencode-dcp-plugin@latest --global
# fully restart OpenCode, then:
/dcp
```

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
| Scripts analytics | ⚠️ Feature-gated | `--features scripts` |
| Pixel-perfect memory of every byte | ⚠️ Block-based | Decompress needs persisted payload |
| Multi-agent shared store | ⚠️ | Design for single-session storage first |

---

## FAQ

### Deterministic only?

Yes — disable compress strategies / leave LLM backends unconfigured; dedup + purge + stale-read still run.

### Is OpenCode required?

No. CLI and library work standalone.

### Does decompress always restore originals?

Blocks are tracked; restore depends on the persistence backend still holding the payload.

### How do I protect secrets?

Set `protectedFilePatterns` in config (e.g. `**/.env`, `**/credentials.json`).

### Can I embed this in my own agent?

Yes — use `dcp-core`’s `ContextPruner` and the examples under `examples/`.

### What is `cacheStabilityMode`?

Controls how aggressively compressed ranges are allowed to invalidate provider prompt caches. Prefer `agent-message` unless you know you need `aggressive` or full `manual` control.

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
