<div align="center">

# @qdang46/opencode-dcp-plugin

**NAPI-RS native Dynamic Context Pruning plugin for [OpenCode](https://github.com/anomalyco/opencode)**

[![npm version](https://img.shields.io/npm/v/@qdang46/opencode-dcp-plugin?style=for-the-badge&logo=npm)](https://www.npmjs.com/package/@qdang46/opencode-dcp-plugin)
[![License: MIT](https://img.shields.io/github/license/quangdang46/dynamic_context_pruning?style=for-the-badge)](https://github.com/quangdang46/dynamic_context_pruning/blob/main/LICENSE)
[![npm downloads](https://img.shields.io/npm/dm/@qdang46/opencode-dcp-plugin?style=for-the-badge)](https://www.npmjs.com/package/@qdang46/opencode-dcp-plugin)

Automatically reduces token usage in OpenCode sessions by pruning obsolete tool outputs, 
deduplicating repeated calls, purging errored tool inputs, and compressing stale conversation 
content — all powered by a Rust native addon for zero-serialization performance.

</div>

---

## ✨ Features

| Feature | Description |
|---------|-------------|
| **Message transform** | Automatically prunes, deduplicates, and compresses messages before each LLM request |
| **System prompt injection** | Appends DCP instructions so the model can use the compress tool |
| **Compress tool** | LLM-driven compression of stale conversation ranges into technical summaries |
| **Decompress / Recompress** | Restore or re-activate compressed blocks on demand |
| **Slash commands** | `/dcp`, `/dcp-compress` for interactive context management |
| **Config cascade** | JSONC config loaded from 4-tier cascade (builtin → global → custom → project) |
| **Cache stability** | `agent-message`, `aggressive`, or `manual` modes for prompt cache preservation |
| **Protected tools** | Glob patterns to keep specific tool outputs from being pruned |
| **Turn protection** | Shield recent N turns from compression suggestions |
| **Purge errors** | Automatically purge errored tool outputs after N turns |
| **Stale file reads** | Remove stale file read outputs when files are modified |
| **Deduplication** | Merge duplicate tool calls and outputs |

---

## 🚀 Installation

### Global (Recommended)

```bash
opencode plugin @qdang46/opencode-dcp-plugin --global
```

### Local (Project-scoped)

```bash
cd your-project
opencode plugin @qdang46/opencode-dcp-plugin
```

### Manual

Add to your `~/.config/opencode/opencode.jsonc`:

```jsonc
{
  "plugin": [
    "@qdang46/opencode-dcp-plugin"
  ]
}
```

And install dependencies:

```bash
cd ~/.config/opencode && npm install @qdang46/opencode-dcp-plugin@latest
```

---

## ⚙️ Configuration

DCP loads configuration from multiple sources in cascade order (later overrides earlier):

1. **Built-in defaults** — compiled into the binary
2. **Global** — `~/.config/dynamic_context_pruning/config.jsonc` (or `$XDG_CONFIG_HOME/...`)
3. **Custom** — `$DCP_CONFIG_DIR/config.jsonc` (if env var is set)
4. **Project** — `.dynamic_context_pruning/config.jsonc` in working directory or ancestor

### Create Global Config

```bash
mkdir -p ~/.config/dynamic_context_pruning
cat > ~/.config/dynamic_context_pruning/config.jsonc << 'EOF'
{
  "enabled": true,
  "debug": false,
  
  "compress": {
    "mode": "range",
    "permission": "allow",
    "maxContextLimit": 100000,
    "minContextLimit": 50000,
    "protectedTools": ["task", "skill", "read", "write", "edit"],
    "protectTags": true,
    "maxSummaryChars": 32768
  },
  
  "strategies": {
    "deduplication": { "enabled": true },
    "purgeErrors": { "enabled": true, "turns": 4 },
    "staleFileReads": { "enabled": true }
  },
  
  "turnProtection": {
    "enabled": true,
    "turns": 4
  },
  
  "commands": { "enabled": true },
  "notification": { "level": "detailed", "kind": "chat" }
}
EOF
```

### Key Config Options

```jsonc
{
  // Master switch
  "enabled": true,
  
  // Compress tool configuration
  "compress": {
    "mode": "range",              // "range" | "message"
    "permission": "allow",        // "ask" | "allow" | "deny"
    "maxContextLimit": 100000,    // Max context tokens before compression
    "minContextLimit": 50000,     // Min context tokens to maintain
    "protectedTools": ["task"],   // Tools whose output is preserved
    "protectTags": true,          // Preserve <dcp-protected> tags
    "protectUserMessages": false, // Protect user messages from compression
    "maxSummaryChars": 32768      // Max summary length per block
  },
  
  // Pruning strategies
  "strategies": {
    "deduplication": { "enabled": true },
    "purgeErrors": { "enabled": true, "turns": 4 },
    "staleFileReads": { "enabled": true, "trackedTools": ["read", "write", "edit"] }
  },
  
  // Turn protection
  "turnProtection": {
    "enabled": true,
    "turns": 4  // Protect last 4 turns from compression
  },
  
  // Manual mode
  "manualMode": {
    "enabled": false,
    "automaticStrategies": true
  },
  
  // Cache stability
  "cacheStabilityMode": "agent-message",  // "aggressive" | "agent-message" | "manual"
  
  // Notification preferences
  "notification": {
    "level": "detailed",  // "off" | "minimal" | "detailed"
    "kind": "chat"        // "chat" | "toast"
  }
}
```

---

## 🎮 Usage

### Slash Commands

| Command | Description |
|---------|-------------|
| `/dcp` | Show help and available commands |
| `/dcp context` | Show context analysis (turns, blocks, tokens) |
| `/dcp stats` | Show pruning statistics |
| `/dcp sweep` | Flush pending prune strategies |
| `/dcp manual <on\|off>` | Toggle manual mode |
| `/dcp decompress <id>` | Restore a compressed block |
| `/dcp recompress <id>` | Re-activate a decompressed block |
| `/dcp-compress [focus]` | Trigger manual compression with optional focus topic |

### LLM Tools

The plugin exposes these tools to the LLM:

- **`compress`** — Replace stale conversation content with technical summaries
- **`decompress`** — Restore a compressed block to its original messages
- **`recompress`** — Re-activate a user-decompressed block for future compression

### Example Workflow

1. **Start OpenCode** — DCP loads automatically and monitors context
2. **Continue working** — DCP automatically deduplicates, purges errors, and removes stale reads
3. **Context grows** — DCP nudges the model to compress stale content
4. **Model compresses** — Stale content replaced with `<dcp-block>` summaries
5. **Inspect** — Use `/dcp context` to see current state
6. **Decompress if needed** — `/dcp decompress 1` to restore compressed block

---

## 🔧 Development

### Prerequisites

- Rust 1.75+ (with `cargo`)
- Node.js 18+
- macOS (ARM64) or Linux (x64)

### Build

```bash
# Clone the repo
git clone https://github.com/quangdang46/dynamic_context_pruning.git
cd dynamic_context_pruning/opencode-dcp-plugin

# Build Rust native addon (debug)
npm run build

# Build Rust native addon (release)
npm run build:release

# Compile TypeScript
npx tsc

# Typecheck only
npm run typecheck
```

### Test

```bash
# Test native addon loading
node -e "require('./opencode-dcp-bridge.darwin-arm64.node')"

# Test full plugin
cd ~/.config/opencode
node -e "
import('@qdang46/opencode-dcp-plugin').then(async (mod) => {
  const plugin = mod.default;
  const instance = await plugin.server({});
  console.log('✅ Plugin loaded');
  console.log('Hooks:', Object.keys(instance));
});
"
```

### Publish

```bash
# Update version
npm version patch  # or minor, major

# Build and publish
npm publish --access public

# Verify
npm view @qdang46/opencode-dcp-plugin
```

---

## 📖 How It Works

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    OpenCode TUI                              │
├─────────────────────────────────────────────────────────────┤
│  Plugin System                                              │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  @qdang46/opencode-dcp-plugin (TypeScript)         │   │
│  │  ┌─────────────────────────────────────────────┐   │   │
│  │  │  opencode-dcp-bridge (Rust NAPI-RS)        │   │   │
│  │  │  ┌───────────────────────────────────────┐  │   │   │
│  │  │  │  dcp-core / dcp-config / dcp-prune   │  │   │   │
│  │  │  └───────────────────────────────────────┘  │   │   │
│  │  └─────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
│  Hooks:                                                     │
│  • experimental.chat.messages.transform                     │
│  • experimental.chat.system.transform                       │
│  • command.execute.before                                   │
│  • event                                                    │
│  • config                                                   │
│  • dispose                                                  │
│  • tool (compress, decompress, recompress)                  │
└─────────────────────────────────────────────────────────────┘
```

### Message Pipeline

1. **Transform** — Each LLM request passes through `transformMessages()`
2. **Strategies** — Deterministic strategies run:
   - Deduplication — merge identical tool calls
   - Purge errors — remove errored tool outputs after N turns
   - Stale file reads — remove reads when files are modified
3. **Compression** — If context exceeds limits, model can use `compress` tool
4. **Injection** — DCP instructions appended to system prompt

### Config Cascade

```
Built-in defaults (compiled)
    ↓
Global config (~/.config/dynamic_context_pruning/config.jsonc)
    ↓
Custom config ($DCP_CONFIG_DIR/config.jsonc)
    ↓
Project config (.dynamic_context_pruning/config.jsonc)
```

Later layers override earlier ones per-key; arrays replace wholesale.

---

## 🐛 Troubleshooting

### Plugin not loading

```bash
# Check if plugin is in config
cat ~/.config/opencode/opencode.jsonc | grep plugin

# Check if package is installed
ls ~/.config/opencode/node_modules/@qdang46/opencode-dcp-plugin/

# Test native addon
cd ~/.config/opencode
node -e "require('./node_modules/@qdang46/opencode-dcp-plugin/opencode-dcp-bridge.darwin-arm64.node')"
```

### Config not loading

```bash
# Check global config exists
cat ~/.config/dynamic_context_pruning/config.jsonc

# Check XDG_CONFIG_HOME
echo $XDG_CONFIG_HOME

# Create config if missing
mkdir -p ~/.config/dynamic_context_pruning
echo '{}' > ~/.config/dynamic_context_pruning/config.jsonc
```

### Performance issues

- Set `"debug": false` in config
- Use `"cacheStabilityMode": "agent-message"` (default)
- Reduce `"maxContextLimit"` if context grows too fast

---

## 📚 Resources

- [DCP Documentation](https://github.com/quangdang46/dynamic_context_pruning)
- [Config Schema](./dcp.schema.json)
- [OpenCode Plugin Docs](https://opencode.ai/plugins)
- [npm Package](https://www.npmjs.com/package/@qdang46/opencode-dcp-plugin)

---

## 📄 License

MIT — see [LICENSE](./LICENSE).

---

<div align="center">

**Made with ❤️ by [quangdang46](https://github.com/quangdang46)**

</div>
