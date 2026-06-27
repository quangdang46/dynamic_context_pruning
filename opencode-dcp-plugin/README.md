# @qdang46/opencode-dcp-plugin

[![npm version](https://img.shields.io/npm/v/@qdang46/opencode-dcp-plugin?style=for-the-badge&logo=npm)](https://www.npmjs.com/package/@qdang46/opencode-dcp-plugin)
[![License: MIT](https://img.shields.io/github/license/quangdang46/dynamic_context_pruning?style=for-the-badge)](https://github.com/quangdang46/dynamic_context_pruning/blob/main/LICENSE)
[![npm downloads](https://img.shields.io/npm/dm/@qdang46/opencode-dcp-plugin?style=for-the-badge)](https://www.npmjs.com/package/@qdang46/opencode-dcp-plugin)

**NAPI-RS native Dynamic Context Pruning plugin for [OpenCode](https://github.com/anomalyco/opencode)**

Forked from [@tarquinen/opencode-dcp](https://github.com/Opencode-DCP/opencode-dynamic-context-pruning) (AGPL-3.0). Rewritten with a Rust native addon for the core DCP logic while keeping the TypeScript plugin architecture and TUI from the original.

## Installation

```bash
opencode plugin @qdang46/opencode-dcp-plugin@latest --global
```

Restart OpenCode and run `/dcp` to open the DCP panel.

## Usage

| Slash Command | Description |
|---|---|
| `/dcp` | Open the DCP panel |
| `/dcp context` | Show token usage breakdown |
| `/dcp stats` | Show pruning statistics |
| `/dcp sweep` | Flush pending prune strategies |
| `/dcp manual <on\|off>` | Toggle manual mode |
| `/dcp decompress <id>` | Restore a compressed block |
| `/dcp recompress <id>` | Re-activate a decompressed block |
| `/dcp-compress [focus]` | Trigger manual compression |

## Configuration

Config cascades through 4 tiers (later wins per key):

1. Built-in defaults (compiled)
2. Global: `~/.config/dynamic_context_pruning/config.jsonc`
3. Custom: `$DCP_CONFIG_DIR/config.jsonc`
4. Project: `.dynamic_context_pruning/config.jsonc`

See the [reference docs](https://github.com/Opencode-DCP/opencode-dynamic-context-pruning) for full config schema.

## Development

```bash
cd opencode-dcp-plugin
npm run build           # Build Rust native addon
npx tsc                 # Compile TypeScript
```

## Credits

Based on [@tarquinen/opencode-dcp](https://github.com/Opencode-DCP/opencode-dynamic-context-pruning) (AGPL-3.0) by [tarquinen](https://github.com/tarquinen). The core DCP logic (pruning, compression, token counting, config cascade) is implemented as a Rust NAPI-RS native addon for maximum performance.

## License

MIT — see [LICENSE](./LICENSE).
