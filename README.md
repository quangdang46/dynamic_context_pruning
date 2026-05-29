# dynamic_context_pruning

Dynamic context pruning for LLM coding agents. Reduces token usage through deterministic strategies (deduplication, error purging, stale file read removal) and an LLM-driven compression tool, while keeping prompt caches stable.

## Architecture

```
dcp-types          Core types: Message, Part, Role, BlockId, SessionState, Stats
       ↑
dcp-traits         Pluggable traits: Tokenizer, StatePersistence, PruneStrategy, CacheAccountant
       ↑
       ├── dcp-tokens        Token counting (char/HuggingFace/tiktoken/Claude backends)
       ├── dcp-protected     Glob-based tool/file path protection
       ├── dcp-state         SessionState transitions, idempotent rebuild
       │         ↑
       │         ├── dcp-storage     Persistence backends (file/in-memory; optional sled/sqlite)
       │         ├── dcp-prune      Deterministic prune strategies (dedup, purge_errors, stale_file_reads)
       │         ├── dcp-compress   LLM-driven compression (range/message modes, block bookkeeping)
       │         └── dcp-nudges     Context-limit/turn/iteration nudge injection
       │
dcp-config         Config schema, JSONC parser, cascade resolution (4-tier: builtin → global → custom → project)
       ↑
dcp-prompts        Default system prompts + 3-tier override cascade (extension, nudge, tool)
       ↑
dcp-core           ContextPruner facade + orchestration (top-level entry point)
       ↑
       ├── dcp-mcp           MCP server binary (Model Context Protocol)
       ├── dcp-cli           CLI binary: stats, timeline, find-session, sweep, compress, decompress
       ├── dcp-claude-hook   Claude Code SessionStart hook binary
       └── dcp-rig           Rig framework adapter (test fixtures)

dcp-permissions    Auth, host permissions, compress permission resolution
dcp-messages       Message query, shape, sync, priority, injection, subagents, reasoning strip
dcp-notification   User-facing notification formatting and sending
dcp-telemetry      Telemetry, metrics, observer hooks
```

## Quick Start

```bash
# Build all crates
cargo build --workspace

# Run tests
cargo test --workspace

# Run the CLI
cargo run -p dcp-cli -- --help

# Run the MCP server
cargo run -p dcp-mcp
```

## Crate Guide

| Crate | Path | Purpose |
|---|---|---|
| `dcp-types` | `crates/dcp-types` | Canonical types: `Message`, `Part`, `Role`, `BlockId`, `SessionState`, `Stats` |
| `dcp-traits` | `crates/dcp-traits` | Pluggable interfaces: `Tokenizer`, `StatePersistence`, `PruneStrategy`, `CacheAccountant`, `MemoryRetriever` |
| `dcp-tokens` | `crates/dcp-tokens` | Token counting (char/4 default; optional HuggingFace, tiktoken, Claude backends) |
| `dcp-protected` | `crates/dcp-protected` | Glob-based tool and file-path protection helpers |
| `dcp-state` | `crates/dcp-state` | SessionState transitions and idempotent rebuild |
| `dcp-storage` | `crates/dcp-storage` | Persistence backends: file, in-memory; optional sled/sqlite |
| `dcp-prune` | `crates/dcp-prune` | Deterministic strategies: deduplicate, purge_errors, stale_file_reads (optional: lingua, embedding) |
| `dcp-compress` | `crates/dcp-compress` | LLM-driven compression: range/message modes, block bookkeeping, frontier tracking |
| `dcp-prompts` | `crates/dcp-prompts` | Default system prompts with 3-tier override cascade (extension → nudge → tool) |
| `dcp-nudges` | `crates/dcp-nudges` | Context-limit, turn, and iteration nudge injection |
| `dcp-config` | `crates/dcp-config` | Configuration schema, JSONC parser, 4-tier cascade resolution |
| `dcp-telemetry` | `crates/dcp-telemetry` | Telemetry, metrics, and observer hooks (optional: logging, quality regression) |
| `dcp-permissions` | `crates/dcp-permissions` | Auth, host permissions, compress permission resolution |
| `dcp-messages` | `crates/dcp-messages` | Message query, shape, sync, priority, injection, subagents, reasoning strip |
| `dcp-notification` | `crates/dcp-notification` | User-facing notification formatting and delivery |
| `dcp-core` | `crates/dcp-core` | `ContextPruner` facade and orchestration (primary entry point) |
| `dcp-mcp` | `crates/dcp-mcp` | MCP server binary exposing DCP via Model Context Protocol |
| `dcp-cli` | `crates/dcp-cli` | CLI binary with stats, timeline, find-session, sweep, compress, decompress commands |
| `dcp-claude-hook` | `crates/dcp-claude-hook` | Claude Code SessionStart hook binary |
| `dcp-rig` | `crates/dcp-rig` | Rig framework adapter (test fixtures) |
| `dynamic_context_pruning` | `crates/dynamic_context_pruning` | Umbrella crate re-exporting all public types |

## Configuration

Config cascades through 4 levels (later wins per key, arrays replace wholesale):

1. Built-in defaults (compiled in)
2. Global: `$XDG_CONFIG_HOME/dynamic_context_pruning/config.jsonc` (`~/.config/...` fallback)
3. Custom: `$DCP_CONFIG_DIR/config.jsonc` if env var set
4. Project: `.dynamic_context_pruning/config.jsonc` in working directory or ancestor

### Key Config Options

```jsonc
{
  "enabled": true,                    // Master switch
  "debug": false,                     // Verbose telemetry output
  "cacheStabilityMode": "agent-message", // "aggressive" | "agent-message" | "manual"
  "protectedFilePatterns": ["**/.env"],  // Glob patterns for protected files
  "compress": {
    "mode": "range",                 // "range" | "message"
    "maxBlocks": 50,
    "minTokens": 2000
  },
  "strategies": {
    "dedup": { "enabled": true },
    "purgeErrors": { "enabled": true },
    "staleFileReads": { "enabled": true, "maxAge": 3600 }
  },
  "manualMode": {
    "enabled": false,
    "requireExplicitConsent": false
  },
  "notification": {
    "level": "essential"             // "silent" | "essential" | "verbose"
  }
}
```

```rust
// Programmatic usage
use dcp_config::Config;

let cfg = Config::load_default()?;
cfg.validate()?;
```

## CLI Usage

```bash
# Show token usage breakdown and pruning stats
dcp context

# Show session statistics
dcp stats --session-id <SESSION_ID>
dcp stats -s <SESSION_ID>

# Show compression events over time
dcp timeline --session-id <SESSION_ID>
dcp timeline -s <SESSION_ID>

# Find sessions by pattern or date range
dcp find-session --pattern "session-*"
dcp find-session --after 2024-01-01 --before 2024-12-31
dcp find-session -p "test-*" -a 2024-01-01

# Flush pending prune tools
dcp sweep
dcp sweep 5  # flush 5 pending

# Run compress tool (reads messages from stdin or file)
dcp compress
dcp compress messages.json

# Restore a compressed block
dcp decompress b1

# Re-activate a user-decompressed block
dcp recompress b1

# Toggle manual mode
dcp manual on
dcp manual off
```

## Testing

```bash
# Run all workspace tests
cargo test --workspace

# Run tests for a specific crate
cargo test -p dcp-core
cargo test -p dcp-config

# Run with coverage
cargo test --workspace -- --nocapture

# Run specific test
cargo test -p dcp-cli -- stats_args_parsing
```

### Test Categories

- **Unit tests**: each crate has inline `#[cfg(test)]` modules
- **Property tests**: `dcp-state` uses `proptest` for state machine property testing
- **Snapshot tests**: `insta` for serialized output regression testing
- **Integration tests**: `tests/smoke.rs` at workspace root
- **Examples**: `examples/01_minimal.rs` through `examples/06_cache_stability.rs`

## License

MIT — see [LICENSE](./LICENSE).