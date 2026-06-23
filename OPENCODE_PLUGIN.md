# OPENCODE_PLUGIN.md — OpenCode DCP Plugin via Rust NAPI-RS Bridge

> **Goal**: Build `@quangdang46/opencode-dcp-plugin` — an OpenCode plugin that uses the existing Rust DCP library (`dynamic_context_pruning`) via a NAPI-RS native addon, instead of reimplementing the 16-step pipeline in TypeScript.

---

## 1. Overview

### What We Have

| Component | Location | Language |
|-----------|----------|----------|
| **Rust DCP library** | `dynamic_context_pruning` (19 crates) | Rust |
| **OpenCode** | `anomalyco/opencode` (TypeScript, 69.2%) | TypeScript |
| **OpenCode SDK** | `@opencode-ai/plugin` (npm, MIT) | TypeScript |
| **Original DCP plugin** | `@tarquinen/opencode-dcp` (AGPL-3.0) | TypeScript |

### Why NAPI-RS (Not WASM, Not Subprocess, Not Reimplement in TS)

| Approach | State mgmt | Latency | Complexity | Verdict |
|----------|:---:|:---:|:---:|:---:|
| **NAPI-RS** | ✅ Zero-copy Rust heap | <100μs | Medium | ✅ **Winner** |
| WASM | ❌ No `dirs`/`std::fs` | <100μs | High | ❌ File I/O blocked |
| Subprocess (CLI) | ❌ Serialize full state each call | 50-200ms | Low | ❌ Lost state |
| Reimplement TS | ✅ But 25K LOC dupe | Same as DCP | Very High | ❌ Wasteful |

**DCP is stateful**: `ContextPruner` carries `SessionState` (message IDs, compression blocks, nudge anchors, tool prune maps, stats) across every `transform_messages()` call. NAPI-RS lets the Rust object live on the native heap — direct function calls with zero serialization overhead.

### Architecture

```
OpenCode (TypeScript)
  └─ spawns plugin (npm package)
       └─ src/index.ts  (createPlugin export)
            ├─ @opencode-ai/plugin SDK hooks
            │    ├─ experimental.chat.messages.transform → calls pruner.transform_messages()
            │    ├─ experimental.chat.system.transform   → calls pruner.transform_system()
            │    ├─ tool: { compress, decompress, recompress }
            │    └─ command.execute.before → handles /dcp commands
            │
            └─ NAPI-RS .node addon (opencode_dcp_bridge.node)
                 └─ class DcpPruner
                      ├─ transform_messages(json: string) → string
                      ├─ transform_system(system: string) → string
                      ├─ handle_compress(args_json, messages_json) → string
                      ├─ decompress(block_id: number) → string
                      ├─ recompress(block_id: number) → string
                      ├─ stats() → string
                      ├─ has_pending_work() → bool
                      └─ session_id(sid: string) → void
```

---

## 2. OpenCode Plugin System

### Plugin Type

From `@opencode-ai/plugin` source (`packages/plugin/src/index.ts`):

```typescript
type Plugin = (input: PluginInput, options?: PluginOptions) => Promise<Hooks>

type PluginInput = {
  client: ReturnType<typeof createOpencodeClient>
  project: Project
  directory: string       // current project directory
  worktree: string        // project root
  experimental_workspace: { register(type, adapter) }
  serverUrl: URL
  $: BunShell
}
```

### Key Hooks for DCP

From the `Hooks` interface in `packages/plugin/src/index.ts`:

```typescript
interface Hooks {
  // ─── Message Pipeline ─────────────────────────────────────────────
  "experimental.chat.messages.transform"?: (
    input: {},
    output: {
      messages: { info: Message; parts: Part[] }[]
    },
  ) => Promise<void>
  // This is THE critical hook. Output.messages can be mutated in-place.
  // DCP uses this to: filter, deduplicate, prune, compress, inject nudges

  // ─── System Prompt ────────────────────────────────────────────────
  "experimental.chat.system.transform"?: (
    input: { sessionID?: string; model: Model },
    output: { system: string[] },
  ) => Promise<void>
  // Append DCP instructions to the system prompt

  // ─── Tools (exposed to the LLM) ───────────────────────────────────
  tool?: { [key: string]: ToolDefinition }
  // Register compress (range or message), decompress, recompress

  // ─── Slash Commands ──────────────────────────────────────────────
  "command.execute.before"?: (
    input: { command: string; sessionID: string; arguments: string },
    output: { parts: Part[] },
  ) => Promise<void>
  // Intercept /dcp and /dcp-compress commands

  // ─── Events (compaction tracking) ────────────────────────────────
  event?: (input: { event: Event }) => Promise<void>
  // Listen for message.part.updated to track compression start/completion

  // ─── Config injection ────────────────────────────────────────────
  config?: (input: Config) => Promise<void>
  // Set tool permissions, register slash commands

  // ─── Output transformation ───────────────────────────────────────
  "experimental.text.complete"?: (
    input: { sessionID: string; messageID: string; partID: string },
    output: { text: string },
  ) => Promise<void>
  // Strip hallucinations from LLM output
}
```

### Tool Registration

From `packages/plugin/src/tool.ts`:

```typescript
import { z } from "zod"

type ToolContext = {
  sessionID: string
  messageID: string
  agent: string
  directory: string
  worktree: string
  abort: AbortSignal
  metadata(input: { title?: string; metadata?: object }): void
  ask(input: { permission: string; patterns: string[]; always: string[]; metadata: object }): Promise<void>
}

type ToolResult = string | { title?: string; output: string; metadata?: object; attachments?: ToolAttachment[] }

// Helper function:
function tool<Args extends z.ZodRawShape>(input: {
  description: string
  args: Args
  execute(args: z.infer<ZodObject<Args>>, context: ToolContext): Promise<ToolResult>
})
tool.schema = z
```

### Plugin Installation

OpenCode plugins are npm packages. Installation:

```bash
# In OpenCode project or globally
opencode plugin add @quangdang46/opencode-dcp-plugin
# Or via npm
npm install @quangdang46/opencode-dcp-plugin
```

The plugin exports a `PluginModule` with a `server` property:

```typescript
export default { server: createPlugin } satisfies PluginModule
```

---

## 3. NAPI-RS Bridge

### How It Works

NAPI-RS compiles Rust to a `.node` native addon that can be `require()`'d directly from Node.js/TypeScript:

```typescript
const addon = require("./opencode-dcp-bridge.linux-x64-gnu.node")
const pruner = new addon.DcpPruner(config)  // Rust object on native heap
const result = pruner.transformMessages(messagesJson)
```

### #[napi] Class Pattern

```rust
use napi_derive::napi;
use napi::bindgen_prelude::*;

#[napi]
pub struct DcpPruner {
    inner: std::sync::Mutex<dcp_core::ContextPruner>,
}

#[napi]
impl DcpPruner {
    #[napi(constructor)]
    pub fn new(config_json: String) -> Result<Self> {
        let config: dcp_config::Config = serde_json::from_str(&config_json)
            .map_err(|e| napi::Error::from_reason(format!("Config parse error: {e}")))?;
        let pruner = dcp_core::ContextPruner::new(config)
            .map_err(|e| napi::Error::from_reason(format!("Pruner init error: {e}")))?;
        Ok(Self { inner: std::sync::Mutex::new(pruner) })
    }

    #[napi]
    pub fn transform_messages(&self, messages_json: String) -> Result<String> {
        let messages: Vec<dcp_types::Message> = serde_json::from_str(&messages_json)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        let mut pruner = self.inner.lock()
            .map_err(|_| napi::Error::from_reason("lock poisoned".to_string()))?;
        let result = pruner.transform_messages(messages)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        serde_json::to_string(&result)
            .map_err(|e| napi::Error::from_reason(e.to_string()))
    }

    #[napi]
    pub fn transform_system(&self, system: String) -> String {
        let mut pruner = self.inner.lock().unwrap();
        let mut s = system;
        pruner.transform_system(&mut s);
        s
    }
}
```

### JSON Serialization

All complex data crosses the boundary as JSON strings:

- **Input**: OpenCode messages serialized to `string` → parsed as `Vec<DcpMessage>` via `serde_json`
- **Output**: `TransformResult` serialized to `string` via `serde_json` → parsed in TypeScript

This is the simplest approach. For production, napi-rs supports struct types natively, but JSON strings avoid type mapping complexity at the boundary.

### Error Handling

```rust
fn parse_json<T: serde::de::DeserializeOwned>(json: &str) -> napi::Result<T> {
    serde_json::from_str(json)
        .map_err(|e| napi::Error::from_reason(format!("JSON parse: {e}")))
}

fn serialize<T: serde::Serialize>(value: &T) -> napi::Result<String> {
    serde_json::to_string(value)
        .map_err(|e| napi::Error::from_reason(format!("JSON serialize: {e}")))
}
```

### Drop for Cleanup

The Rust `Drop` trait fires automatically when JS garbage-collects the napi object:

```rust
impl Drop for DcpPruner {
    fn drop(&mut self) {
        // ContextPruner cleanup, state flush, etc.
    }
}
```

### Build Configuration

```toml
# Cargo.toml
[package]
name = "opencode-dcp-bridge"
edition = "2024"
[lib]
crate-type = ["cdylib"]
[dependencies]
dcp-core = { path = "../dynamic_context_pruning/crates/dcp-core" }
dcp-config = { path = "../dynamic_context_pruning/crates/dcp-config" }
dcp-types = { path = "../dynamic_context_pruning/crates/dcp-types" }
dcp-storage = { path = "../dynamic_context_pruning/crates/dcp-storage" }
serde_json = "1"
serde = { version = "1", features = ["derive"] }
napi = { version = "2", features = ["napi4"] }
napi-derive = "2"
```

```rust
// build.rs
extern crate napi_build;
fn main() { napi_build::setup(); }
```

### Platform Targets

```
aarch64-apple-darwin   (Apple Silicon)
x86_64-apple-darwin    (Intel Mac)
x86_64-unknown-linux-gnu (Linux x86_64)
x86_64-pc-windows-msvc (Windows x86_64)
```

---

## 4. Project Structure

```
opencode-dcp-plugin/
├── Cargo.toml                    # Rust crate: opencode-dcp-bridge
├── build.rs                      # napi_build::setup()
├── package.json                  # npm: @quangdang46/opencode-dcp-plugin
├── tsconfig.json                 # TypeScript config
├── .npmignore
│
├── src/                          # Rust source (NAPI-RS)
│   ├── lib.rs                    # Module declarations + re-exports
│   ├── pruner.rs                 # #[napi] class DcpPruner
│   ├── message.rs                # OpenCode <-> DCP message conversion
│   ├── config.rs                 # Config loading from DCP cascade
│   └── error.rs                  # Error handling utilities
│
├── npm/                          # Per-platform packages (generated)
│   ├── darwin-arm64/
│   ├── darwin-x64/
│   ├── linux-x64-gnu/
│   └── win32-x64-msvc/
│
├── ts_src/                       # TypeScript plugin
│   ├── index.ts                  # createPlugin() export
│   ├── tools.ts                  # compress/decompress tool defs
│   ├── config.ts                 # Config loading helpers
│   └── types.ts                  # Shared TS types
│
└── dist/                         # Built output
    ├── index.js
    └── index.d.ts
```

---

## 5. Rust Crate: opencode-dcp-bridge

### 5.1 Cargo.toml

```toml
[package]
name = "opencode-dcp-bridge"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
dcp-core = { path = "../dynamic_context_pruning/crates/dcp-core" }
dcp-config = { path = "../dynamic_context_pruning/crates/dcp-config" }
dcp-types = { path = "../dynamic_context_pruning/crates/dcp-types" }
dcp-storage = { path = "../dynamic_context_pruning/crates/dcp-storage" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
napi = { version = "2", features = ["napi4"] }
napi-derive = "2"
```

### 5.2 build.rs

```rust
extern crate napi_build;
fn main() {
    napi_build::setup();
}
```

### 5.3 src/lib.rs

```rust
mod pruner;
mod message;
mod config;
mod error;

pub use pruner::DcpPruner;
```

### 5.4 src/pruner.rs

```rust
use napi_derive::napi;

#[napi]
pub struct DcpPruner {
    inner: std::sync::Mutex<dcp_core::ContextPruner>,
}

#[napi]
impl DcpPruner {
    /// Create a new DCP pruner with the given config JSON.
    /// Config follows the standard DCP cascade format.
    #[napi(constructor)]
    pub fn new(config_json: String) -> napi::Result<Self> {
        let config: dcp_config::Config = serde_json::from_str(&config_json)
            .map_err(|e| napi::Error::from_reason(format!("Config parse: {e}")))?;
        let pruner = dcp_core::ContextPruner::new(config)
            .map_err(|e| napi::Error::from_reason(format!("Pruner init: {e}")))?;
        Ok(Self { inner: std::sync::Mutex::new(pruner) })
    }

    /// Transform messages before sending to the LLM.
    /// Input: JSON array of DCP Message objects
    /// Output: JSON array of transformed DCP Message objects
    #[napi]
    pub fn transform_messages(&self, messages_json: String) -> napi::Result<String> {
        let messages: Vec<dcp_types::Message> = serde_json::from_str(&messages_json)
            .map_err(|e| napi::Error::from_reason(format!("Input parse: {e}")))?;
        let mut pruner = self.inner.lock()
            .map_err(|_| napi::Error::from_reason("mutex poisoned"))?;
        let result = pruner.transform_messages(messages)
            .map_err(|e| napi::Error::from_reason(format!("Transform: {e}")))?;
        serde_json::to_string(&result)
            .map_err(|e| napi::Error::from_reason(format!("Serialize: {e}")))
    }

    /// Append DCP system prompt addendum.
    #[napi]
    pub fn transform_system(&self, system: String) -> String {
        let mut pruner = self.inner.lock().unwrap();
        let mut s = system;
        pruner.transform_system(&mut s);
        s
    }

    /// Handle compress tool call from LLM.
    #[napi]
    pub fn handle_compress(&self, args_json: String, messages_json: String) -> napi::Result<String> {
        let args: dcp_core::CompressArgs = serde_json::from_str(&args_json)
            .map_err(|e| napi::Error::from_reason(format!("Args parse: {e}")))?;
        let messages: Vec<dcp_types::Message> = serde_json::from_str(&messages_json)
            .map_err(|e| napi::Error::from_reason(format!("Messages parse: {e}")))?;
        let mut pruner = self.inner.lock()
            .map_err(|_| napi::Error::from_reason("mutex poisoned"))?;
        let result = pruner.handle_compress(args, &messages)
            .map_err(|e| napi::Error::from_reason(format!("Compress: {e}")))?;
        serde_json::to_string(&result)
            .map_err(|e| napi::Error::from_reason(format!("Serialize: {e}")))
    }

    #[napi]
    pub fn decompress(&self, block_id: u32) -> napi::Result<String> {
        let mut pruner = self.inner.lock()
            .map_err(|_| napi::Error::from_reason("mutex poisoned"))?;
        let result = pruner.decompress(block_id)
            .map_err(|e| napi::Error::from_reason(format!("Decompress: {e}")))?;
        serde_json::to_string(&result)
            .map_err(|e| napi::Error::from_reason(format!("Serialize: {e}")))
    }

    #[napi]
    pub fn recompress(&self, block_id: u32) -> napi::Result<String> {
        let mut pruner = self.inner.lock()
            .map_err(|_| napi::Error::from_reason("mutex poisoned"))?;
        let result = pruner.recompress(block_id)
            .map_err(|e| napi::Error::from_reason(format!("Recompress: {e}")))?;
        serde_json::to_string(&result)
            .map_err(|e| napi::Error::from_reason(format!("Serialize: {e}")))
    }

    #[napi]
    pub fn has_pending_work(&self) -> bool {
        self.inner.lock().map(|p| p.has_pending_work()).unwrap_or(false)
    }

    #[napi]
    pub fn stats(&self) -> String {
        let pruner = self.inner.lock().unwrap();
        let stats = pruner.stats();
        serde_json::to_string(&stats).unwrap_or_default()
    }

    #[napi]
    pub fn set_session_id(&self, session_id: String) {
        if let Ok(mut pruner) = self.inner.lock() {
            pruner.set_session_id(&session_id);
        }
    }
}

impl Drop for DcpPruner {
    fn drop(&mut self) {
        // Auto-save state on GC
        if let Ok(mut pruner) = self.inner.lock() {
            let _ = pruner.save();
        }
    }
}
```

### 5.5 src/message.rs — OpenCode ↔ DCP Conversion

This is the critical mapping layer. The original DCP plugin receives messages in OpenCode's `{ info: Message; parts: Part[] }[]` format and converts them to DCP's `Message` format.

```rust
use dcp_types::{Message, Part, Role, ToolStatus};

/// Convert an OpenCode part (from the transform hook) to DCP Part.
/// OpenCode Part types include: "text", "tool", "reasoning", "image", "file"
fn opencode_part_to_dcp(part: &serde_json::Value) -> Option<Part> {
    let part_type = part.get("type")?.as_str()?;
    match part_type {
        "text" => {
            let text = part.get("text")?.as_str()?;
            Some(Part::Text(text.to_string()))
        }
        "tool" => {
            let call_id = part.get("callID").or_else(|| part.get("call_id"))
                .and_then(|v| v.as_str()).unwrap_or("");
            let tool = part.get("tool").and_then(|v| v.as_str()).unwrap_or("");
            let input = part.get("state").and_then(|s| s.get("input"))
                .and_then(|v| serde_json::to_string(v).ok()).unwrap_or_default();
            let output = part.get("state").and_then(|s| s.get("output"))
                .and_then(|v| v.as_str()).map(|s| s.to_string());
            let error = part.get("state").and_then(|s| s.get("error"))
                .and_then(|v| v.as_str()).map(|s| s.to_string());
            let status = part.get("state").and_then(|s| s.get("status"))
                .and_then(|v| v.as_str()).unwrap_or("pending");
            let ts = match status {
                "completed" => ToolStatus::Completed,
                "error" => ToolStatus::Error,
                "running" => ToolStatus::Running,
                _ => ToolStatus::Pending,
            };
            Some(Part::ToolResult {
                call_id: call_id.to_string(),
                status: ts,
                output,
                error,
            })
        }
        "reasoning" | "thinking" => {
            let text = part.get("reasoning").or_else(|| part.get("text"))
                .and_then(|v| v.as_str()).unwrap_or("");
            Some(Part::Reasoning(text.to_string()))
        }
        "image" | "image_url" => {
            let media_type = part.get("media_type").or_else(|| part.get("mimeType"))
                .and_then(|v| v.as_str()).unwrap_or("image/png").to_string();
            let data = part.get("data").or_else(|| part.get("source"))
                .and_then(|v| v.as_str()).unwrap_or("").to_string();
            Some(Part::Image { media_type, data })
        }
        _ => None,
    }
}

/// Convert a DCP Part back to OpenCode format.
fn dcp_part_to_opencode(part: &Part) -> serde_json::Value {
    match part {
        Part::Text(t) => serde_json::json!({
            "type": "text", "text": t
        }),
        Part::Reasoning(r) => serde_json::json!({
            "type": "reasoning", "reasoning": r
        }),
        Part::ToolCall { call_id, tool, input } => serde_json::json!({
            "type": "tool",
            "callID": call_id,
            "tool": tool,
            "state": {
                "status": "running",
                "input": input
            }
        }),
        Part::ToolResult { call_id, status, output, error } => {
            let mut obj = serde_json::json!({
                "type": "tool",
                "callID": call_id,
                "state": {
                    "status": match status {
                        ToolStatus::Completed => "completed",
                        ToolStatus::Error => "error",
                        ToolStatus::Running => "running",
                        _ => "pending",
                    }
                }
            });
            if let Some(o) = output { obj["state"]["output"] = serde_json::Value::String(o.clone()); }
            if let Some(e) = error { obj["state"]["error"] = serde_json::Value::String(e.clone()); }
            obj
        }
        Part::Image { media_type, data } => serde_json::json!({
            "type": "image",
            "media_type": media_type,
            "data": data
        }),
        _ => serde_json::json!({"type": "unknown"}),
    }
}
```

### 5.6 src/config.rs

```rust
use napi_derive::napi;

/// Load DCP config from the standard cascade paths.
/// Returns JSON string of Config struct.
#[napi]
pub fn load_dcp_config() -> napi::Result<String> {
    let config = dcp_config::Config::load_default()
        .map_err(|e| napi::Error::from_reason(format!("Config load: {e}")))?;
    serde_json::to_string(&config)
        .map_err(|e| napi::Error::from_reason(format!("Serialize: {e}")))
}
```

### 5.7 src/error.rs

```rust
pub fn json_parse_error<T: std::fmt::Display>(msg: T) -> napi::Error {
    napi::Error::from_reason(format!("JSON parse error: {msg}"))
}

pub fn napi_error(msg: impl Into<String>) -> napi::Error {
    napi::Error::from_reason(msg.into())
}
```

---

## 6. TypeScript Plugin

### 6.1 package.json

```json
{
  "name": "@quangdang46/opencode-dcp-plugin",
  "version": "0.1.0",
  "type": "module",
  "main": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" },
    "./server": { "types": "./dist/index.d.ts", "import": "./dist/index.js" }
  },
  "napi": {
    "name": "opencode-dcp-bridge",
    "triples": {
      "defaults": true,
      "additional": ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]
    }
  },
  "scripts": {
    "build": "napi build --platform --release",
    "build:debug": "napi build --platform",
    "prepublishOnly": "napi prepublish -t npm/@quangdang46/opencode-dcp-plugin"
  },
  "peerDependencies": {
    "@opencode-ai/plugin": ">=1.4.3"
  },
  "optionalDependencies": {
    "@quangdang46/opencode-dcp-plugin-darwin-arm64": "0.1.0",
    "@quangdang46/opencode-dcp-plugin-darwin-x64": "0.1.0",
    "@quangdang46/opencode-dcp-plugin-linux-x64-gnu": "0.1.0",
    "@quangdang46/opencode-dcp-plugin-win32-x64-msvc": "0.1.0"
  },
  "devDependencies": {
    "@napi-rs/cli": "^2.18.0",
    "@opencode-ai/plugin": "^1.4.3",
    "typescript": "^5.0.0",
    "tsup": "^8.0.0"
  }
}
```

### 6.2 ts_src/index.ts — Plugin Entry Point

```typescript
import type { Plugin, PluginModule } from "@opencode-ai/plugin"
import { createTools } from "./tools"
import { createMessageTransformHandler } from "./message-transform"

// The bridge native addon. Loaded dynamically during plugin init.
import type { DcpPruner } from "../src/pruner" // .d.ts generated by napi-rs

let bridge: typeof import("../opencode-dcp-bridge") | null = null

async function loadBridge(): Promise<typeof import("../opencode-dcp-bridge")> {
  if (!bridge) {
    bridge = require("../opencode-dcp-bridge.node")
  }
  return bridge
}

const createPlugin: Plugin = async (ctx) => {
  const bridge = await loadBridge()
  const configJson = bridge.loadDcpConfig()

  const pruner: DcpPruner = new bridge.DcpPruner(configJson)

  // Hook into /dcp commands
  ctx.client.on("command.execute.before", (input, output) => {
    if (input.command === "dcp" || input.command === "dcp-compress") {
      // Handle slash commands
    }
  })

  return {
    "experimental.chat.messages.transform": createMessageTransformHandler(pruner),
    "experimental.chat.system.transform": async (_input, output) => {
      output.system.push("\n\nContext-pruning support is available. ...")
    },
    tool: createTools(pruner),
    event: async (input) => {
      // Track compression tool completion for state persistence
      if (input.event.type === "message.part.updated") {
        // ...
      }
    },
  }
}

export default { server: createPlugin } satisfies PluginModule
```

### 6.3 ts_src/tools.ts — Tool Definitions

```typescript
import { tool } from "@opencode-ai/plugin"
import type { DcpPruner } from "../opencode-dcp-bridge"

export function createTools(pruner: DcpPruner) {
  return {
    compress: tool({
      description: "Replace stale conversation content with technical summaries.",
      args: {
        topic: tool.schema.string().describe("Short label (3-5 words)"),
        content: tool.schema.array(
          tool.schema.object({
            startId: tool.schema.string().describe("Message or block ID"),
            endId: tool.schema.string().describe("Message or block ID"),
            summary: tool.schema.string().describe("Technical summary"),
          })
        ),
      },
      async execute(args, toolCtx) {
        const messagesJson = await toolCtx.ask({ /* fetch session messages */ })
        const resultJson = pruner.handleCompress(
          JSON.stringify(args),
          messagesJson
        )
        const result = JSON.parse(resultJson)
        return `Compressed ${result.compressed_count} messages.`
      },
    }),

    decompress: tool({
      description: "Restore a compressed block to its original messages.",
      args: {
        blockId: tool.schema.number().describe("Block ID to restore"),
      },
      async execute(args) {
        pruner.decompress(args.blockId)
        return `Decompressed block ${args.blockId}.`
      },
    }),

    recompress: tool({
      description: "Re-compress a user-decompressed block.",
      args: {
        blockId: tool.schema.number().describe("Block ID to recompress"),
      },
      async execute(args) {
        pruner.recompress(args.blockId)
        return `Recompressed block ${args.blockId}.`
      },
    }),
  }
}
```

### 6.4 ts_src/config.ts

```typescript
import { readFileSync, existsSync } from "fs"
import { join } from "path"
import { homedir } from "os"

// DCP config cascade paths (matching dcp-config crate)
const XDG_CONFIG = process.env.XDG_CONFIG_HOME || join(homedir(), ".config")
const GLOBAL_CONFIG = join(XDG_CONFIG, "dynamic_context_pruning", "config.jsonc")
const CUSTOM_CONFIG = process.env.DCP_CONFIG_DIR
  ? join(process.env.DCP_CONFIG_DIR, "config.jsonc")
  : null

function findProjectConfig(startDir: string): string | null {
  let current = startDir
  while (current !== "/") {
    const candidate = join(current, ".dynamic_context_pruning", "config.jsonc")
    if (existsSync(candidate)) return candidate
    current = join(current, "..")
  }
  return null
}

export function findConfigPath(directory: string): string {
  return CUSTOM_CONFIG || findProjectConfig(directory) || GLOBAL_CONFIG
}
```

---

## 7. Message Transform Pipeline

The original DCP plugin (`@tarquinen/opencode-dcp`) runs a 16-step pipeline in `lib/hooks.ts:createChatMessageTransformHandler()`. Our Rust bridge replaces steps 5-13 with a single `pruner.transform_messages()` call.

### Steps Implemented in Rust (via `ContextPruner::transform_messages`)

| # | Step | Original TS | Rust DCP |
|---|------|-------------|----------|
| 5 | Cache system prompt tokens | `cacheSystemPromptTokens()` | `dcp-core` |
| 6 | Assign message refs | `assignMessageRefs()` | `dcp-messages` |
| 7 | Sync compression blocks | `syncCompressionBlocks()` | `dcp-state` |
| 8 | Sync tool cache | `syncToolCache()` | `dcp-state` |
| 9 | Build tool ID list | `buildToolIdList()` | `dcp-prune` |
| 10 | Prune (dedup, purge, filter compressed) | `prune()` | `dcp-prune` |
| 11 | Inject subagent results | `injectExtendedSubAgentResults()` | `dcp-messages` |
| 12 | Build priority map | `buildPriorityMap()` | `dcp-nudges` |
| 13 | Inject nudges | `injectCompressNudges()` | `dcp-nudges` |

### Steps Remaining in TypeScript (OpenCode-specific)

| # | Step | Why TS |
|---|------|--------|
| 1 | `filterMessagesInPlace()` | OpenCode message shape validation |
| 2 | `checkSession()` | Detect session changes, compaction events |
| 3 | `syncCompressPermissionState()` | OpenCode permission system |
| 4 | `stripHallucinations()` | Simple text replacement |
| 14 | `injectMessageIds()` | Format block IDs for LLM targeting |
| 15 | `applyPendingManualTrigger()` | Manual mode state management |
| 16 | `stripStaleMetadata()` | Clean expired metadata |
| — | `logger.saveContext()` | OpenCode log persistence |

### Pipeline Implementation

```typescript
async function messageTransform(input: {}, output: { messages: WithParts[] }) {
  // 1-3: OpenCode-specific setup
  filterMessagesInPlace(output.messages)
  await checkSession(client, state, output.messages)
  syncCompressPermissionState(state, config, output.messages)

  if (state.isSubAgent && !config.experimental.allowSubAgents) return

  // 4: Strip hallucinations
  stripHallucinations(output.messages)

  // 5-13: Call Rust DCP (replaces ~500 lines of TS)
  const opencodeJson = JSON.stringify(output.messages)
  const dcpMessages = convertOpenCodeToDcp(opencodeJson)     // TS: map format
  const transformedJson = pruner.transformMessages(dcpMessages)
  const transformed = JSON.parse(transformedJson)
  convertDcpToOpenCode(transformed, output.messages)          // TS: apply in-place

  // 14-16: OpenCode-specific post-processing
  injectMessageIds(state, config, output.messages)
  applyPendingManualTrigger(state, output.messages)
  stripStaleMetadata(output.messages)

  // Save state
  await saveSessionState(state)
}
```

---

## 8. Message Format Mapping

| OpenCode Part type | DCP Part variant | Direction | Notes |
|---|---|---|---|
| `{ type: "text", text: string }` | `Part::Text(String)` | ↔ | Direct |
| `{ type: "tool", callID, tool, state: { input, output, error, status } }` | `Part::ToolCall` / `Part::ToolResult` | ↔ | Status mapping: completed→Completed, error→Error |
| `{ type: "reasoning", reasoning: string }` | `Part::Reasoning(String)` | ↔ | Direct |
| `{ type: "image", media_type, data }` | `Part::Image { media_type, data }` | ↔ | Direct |
| `{ type: "file", source: { type, media_type, data } }` | `Part::ToolResult { output: file_content }` | → | File source expanded to tool result |
| DCP `Part::Text` | `{ type: "text", text: string }` | ← | Direct |
| DCP `Part::ToolCall { call_id, tool, input }` | `{ type: "tool", callID, tool, state: { status: "running", input } }` | ← | Status hardcoded "running" |
| DCP Message with `Role::User` | `{ info: { role: "user" }, parts: [...] }` | ↔ | |
| DCP Message with `Role::Assistant` | `{ info: { role: "assistant" }, parts: [...] }` | ↔ | |
| DCP Message with `Role::System` | `{ info: { role: "system" }, parts: [...] }` | ↔ | |

---

## 9. Configuration

### Config Cascade (same as DCP CLI)

```
1. Built-in defaults  ── compiled into dcp-config crate
2. Global config      ── $XDG_CONFIG_HOME/dynamic_context_pruning/config.jsonc
                         (fallback: ~/.config/dynamic_context_pruning/config.jsonc)
3. Custom config      ── $DCP_CONFIG_DIR/config.jsonc (if env var set)
4. Project config     ── .dynamic_context_pruning/config.jsonc (cwd walk-up)
```

### Config File Format (JSON5)

```json5
{
  enabled: true,
  debug: false,
  cacheStabilityMode: "agent-message",  // "aggressive" | "agent-message" | "manual"
  compress: {
    mode: "range",                       // "range" | "message"
    permission: "allow",                 // "allow" | "ask" | "deny"
    maxContextLimit: 100000,
    minContextLimit: 50000,
    nudgeFrequency: 5,
    nudgeForce: "soft",
    protectedTools: ["task", "skill"],
    protectTags: false,
    protectUserMessages: false,
  },
  strategies: {
    deduplication: { enabled: true, protectedTools: [] },
    purgeErrors: { enabled: true, turns: 4 },
  },
  manualMode: { enabled: false, automaticStrategies: true },
}
```

### Config Loaded from Rust Side

The DCP config cascade is already implemented in `dcp-config` crate (`Config::load_default()`). The bridge exposes `loadDcpConfig()` which returns the resolved config as JSON. The TS plugin doesn't need to reimplement cascade — it just calls the Rust function.

---

## 10. Build and Deployment

### Local Build

```bash
# Build the NAPI-RS bridge
npx napi build --platform --release
# Output: opencode-dcp-bridge.{platform}.node
```

### GitHub Actions CI

```yaml
name: Build and Release
on:
  push:
    tags: ['v*']
jobs:
  build:
    strategy:
      matrix:
        target:
          - aarch64-apple-darwin
          - x86_64-apple-darwin
          - x86_64-unknown-linux-gnu
          - x86_64-pc-windows-msvc
    runs-on: ${{ matrix.target == 'aarch64-apple-darwin' && 'macos-latest' || ... }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { targets: ${{ matrix.target }} }
      - uses: actions/setup-node@v4
      - run: npx napi build --platform --release --target ${{ matrix.target }}
      - run: npx napi prepublish -t npm/@quangdang46/opencode-dcp-plugin
      - uses: actions/upload-artifact@v4
```

### npm Publish

```bash
npx napi prepublish -t "npm/@quangdang46/opencode-dcp-plugin"
npm publish
```

This generates platform-specific packages:
- `@quangdang46/opencode-dcp-plugin-darwin-arm64`
- `@quangdang46/opencode-dcp-plugin-darwin-x64`
- `@quangdang46/opencode-dcp-plugin-linux-x64-gnu`
- `@quangdang46/opencode-dcp-plugin-win32-x64-msvc`

---

## 11. Implementation Phases

### Phase 1: NAPI-RS Bridge Crate (2-3 days)

| Day | Task |
|:---|------|
| 1 | Set up Cargo.toml, build.rs, lib.rs. Implement DcpPruner class with `new()`, `transform_messages()`, `transform_system()` |
| 2 | Implement `handle_compress()`, `decompress()`, `recompress()`, `stats()`. Implement message.rs conversion layer |
| 3 | Implement config.rs for loading config cascade. Test with `node -e "require('./bridge.node')"` |

### Phase 2: TypeScript Plugin Shell (1 day)

| Task | Details |
|------|---------|
| Plugin entry | `index.ts` with `createPlugin()` returning Hooks |
| Tool registration | `compress`, `decompress`, `recompress` tool definitions |
| Config bridge | Load config from Rust, merge with OpenCode options |
| Types | Shared TypeScript types for Message, Part conversion |

### Phase 3: Message Pipeline Integration (1 day)

| Task | Details |
|------|---------|
| Messages transform hook | Wire `pruner.transform_messages()` into the pipeline |
| System prompt hook | Wire `pruner.transform_system()` into system prompt |
| Command handler | Handle `/dcp` slash commands |
| Event handler | Track compression completion, save state |

### Phase 4: Build + Publish (1 day)

| Task | Details |
|------|---------|
| CI/CD | GitHub Actions matrix build for 4 platforms |
| npm publish | Platform-specific packages + main package |
| Verify | `opencode plugin add @quangdang46/opencode-dcp-plugin` |

---

## 12. Risks and Mitigations

| Risk | Mitigation |
|------|:-----------|
| **Message format mismatch** between OpenCode SDK versions | Pin `@opencode-ai/plugin` version; test against multiple SDK versions |
| **Rust compilation for 4 platforms** (ARM/Intel macOS, Linux, Windows) | Use cross-compilation via `napi build`; GitHub Actions build matrix |
| **State persistence races** under concurrent sessions | Use session-specific filenames (`${sessionId}.json`); never global state files |
| **NAPI-RS binary size** (~5-10MB per platform) | Acceptable for native addon; comparable to other napi-rs packages (swc, oxc) |
| **Token counting fidelity** with `Char4Tokenizer` | Add `claude-tokenizer` feature for accurate token counting |
| **Plugin reload** — OpenCode may reload plugins mid-session | Implement `dispose()` hook in Hooks; Drop trait on DcpPruner saves state |
| **OpenCode API instability** — experimental hooks may change | Monitor OpenCode releases; abstract hook usage behind adapter |

---

> **Next steps**: Start Phase 1 — create the `opencode-dcp-bridge` crate and implement DcpPruner class. The Rust DCP library already has all the logic, we just need to expose it via NAPI-RS.
