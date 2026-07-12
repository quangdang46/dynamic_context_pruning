# @qdang46/opencode-dcp-plugin

[![npm version](https://img.shields.io/npm/v/@qdang46/opencode-dcp-plugin?style=for-the-badge&logo=npm)](https://www.npmjs.com/package/@qdang46/opencode-dcp-plugin)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/License-AGPL--3.0--or--later-blue.svg?style=for-the-badge)](https://github.com/quangdang46/dynamic_context_pruning/blob/main/LICENSE)

**Dynamic Context Pruning plugin for [OpenCode](https://github.com/anomalyco/opencode)**

Full-parity TypeScript port of [@tarquinen/opencode-dcp](https://github.com/Opencode-DCP/opencode-dynamic-context-pruning) (v3.1.x). Same hooks, TUI panel, compress tools, notifications, persistence, and config as upstream.

## Installation

### Local (dev / pin)

In both `~/.config/opencode/opencode.jsonc` **and** `~/.config/opencode/tui.json`:

```jsonc
{
  "plugin": [
    "file:///Users/you/Projects/dynamic_context_pruning/opencode-dcp-plugin"
  ]
}
```

> OpenCode 1.17 loads TUI plugins from `tui.json` separately from server plugins in `opencode.jsonc`. Use an absolute `file://` path (outside `node_modules`) so OpenTUI JSX transforms correctly ([opencode#33884](https://github.com/anomalyco/opencode/issues/33884)).

### npm

```bash
opencode plugin @qdang46/opencode-dcp-plugin@latest --global
```

Restart OpenCode completely after install.

## Usage

| Command | What happens |
|---------|----------------|
| `/dcp` | **TUI dialog panel** — Context / Stats / Manual mode / compress prompt hint |
| `/dcp-compress [focus]` | Chat slash: queues a manual compress prompt; model must call `compress` |
| `/dcp context` | Ignored chat message with token breakdown |
| `/dcp stats` | Ignored chat message with session + all-time stats |
| `/dcp sweep` | Flush pending prune strategies |
| `/dcp manual on\|off` | Toggle manual mode (persisted) |
| `/dcp decompress <id>` | Restore a compressed block |
| `/dcp recompress <id>` | Re-activate a decompressed block |

LLM tools (when compress permission allows):

- `compress` — range mode (`startId`/`endId`) or message mode (`messageId`), per config

## Configuration

`~/.config/opencode/dcp.jsonc` (or project `.opencode/dcp.jsonc`):

```jsonc
{
  "enabled": true,
  "strategies": {
    "deduplication": { "enabled": true },
    "purgeErrors": { "enabled": true }
  },
  "compress": {
    "mode": "range",           // or "message"
    "permission": "allow",
    "showCompression": true
  },
  "pruneNotification": "detailed", // off | minimal | detailed
  "pruneNotificationType": "chat", // chat | toast
  "manualMode": { "enabled": false },
  "commands": { "enabled": true }
}
```

Full schema: [`dcp.schema.json`](./dcp.schema.json).

## Architecture

```
OpenCode server process
  index.ts (Plugin)
    → hooks (messages.transform, system, commands, events)
    → strategies (dedup, purge-errors)
    → compress tools (range | message)
    → state persistence (~/.local/share/opencode/storage/plugin/dcp/)

OpenCode TUI process
  tui.tsx
    → /dcp → PanelDialog (Context / Stats / Manual)
```

Same structure as upstream `@tarquinen/opencode-dcp`. Prune mutates OpenCode tool parts in place (placeholder outputs) — no IR round-trip.

## Development

```bash
cd opencode-dcp-plugin
npm install --legacy-peer-deps
npm run build          # tsup → dist/index.js + tsc declarations
npm run typecheck
```

After changing TUI sources, restart OpenCode (TUI loads `tui.tsx` live via `file://`).

> Do **not** leave nested `node_modules/@opentui` or `solid-js` in the plugin package when loading via `file://` — OpenCode must use its host renderer.

## Credits

Based on [@tarquinen/opencode-dcp](https://github.com/Opencode-DCP/opencode-dynamic-context-pruning) (AGPL-3.0) by [tarquinen](https://github.com/tarquinen).

## License

AGPL-3.0-or-later — see [LICENSE](./LICENSE).
