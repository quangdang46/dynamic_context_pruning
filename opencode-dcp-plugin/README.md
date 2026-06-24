# @quangdang46/opencode-dcp-plugin

NAPI-RS native Dynamic Context Pruning plugin for [OpenCode](https://github.com/anomalyco/opencode).

Automatically reduces token usage in OpenCode sessions by pruning obsolete tool outputs, 
deduplicating repeated calls, purging errored tool inputs, and compressing stale conversation 
content — all powered by a Rust native addon for zero-serialization performance.

## Installation

```bash
opencode plugin add @quangdang46/opencode-dcp-plugin
```

Or globally:

```bash
opencode plugin @quangdang46/opencode-dcp-plugin@latest --global
```

## Features

| Feature | Description |
|---------|-------------|
| **Message transform** | Automatically prunes, deduplicates, and compresses messages before each LLM request |
| **System prompt injection** | Appends DCP instructions so the model can use the compress tool |
| **Compress tool** | LLM-driven compression of stale conversation ranges into technical summaries |
| **Decompress / Recompress** | Restore or re-activate compressed blocks on demand |
| **Slash commands** | `/dcp context`, `/dcp stats`, `/dcp sweep`, `/dcp manual`, `/dcp decompress`, `/dcp recompress` |
| **Config cascade** | JSONC config loaded from 4-tier cascade (builtin → global → custom → project) |
| **Cache stability** | `agent-message`, `aggressive`, or `manual` modes for prompt cache preservation |
| **Protected tools** | Glob patterns to keep specific tool outputs from being pruned |

## Configuration

DCP loads configuration from `.dynamic_context_pruning/config.jsonc` in your project directory.
See the [dcp.schema.json](./dcp.schema.json) for the full config schema.

## Development

```bash
# Build the Rust native addon
cargo build --release -p opencode-dcp-bridge

# Compile TypeScript
cd opencode-dcp-plugin && npx tsc

# Test
node -e "require('./opencode-dcp-bridge.darwin-arm64.node')"
```

## License

MIT
