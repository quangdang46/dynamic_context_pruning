import type { Plugin, PluginModule, PluginInput } from "@opencode-ai/plugin"
import { createTools } from "./tools.js"
import { createRequire } from "module"
import { fileURLToPath } from "url"
import { dirname, resolve, join } from "path"

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)
const _require = createRequire(import.meta.url)

interface DcpPruner {
  transformMessages(messagesJson: string): string
  transformSystem(system: string): string
  handleCompress(argsJson: string, messagesJson: string): string
  decompress(blockId: number): string
  recompress(blockId: number): string
  hasPendingWork(): boolean
  stats(): string
  setSessionId(sessionId: string): void
}

interface BridgeExports {
  DcpPruner: new (configJson: string) => DcpPruner
  loadDcpConfig(): string
}

function loadBridge(): BridgeExports {
  const root = resolve(__dirname, "..")
  const candidates = [
    join(root, "opencode-dcp-bridge.darwin-arm64.node"),
    join(root, "opencode-dcp-bridge.darwin-x64.node"),
    join(root, "opencode-dcp-bridge.linux-x64-gnu.node"),
    join(root, "opencode-dcp-bridge.win32-x64-msvc.node"),
    join(root, "opencode-dcp-bridge.node"),
  ]
  for (const name of candidates) {
    try {
      return _require(name) as BridgeExports
    } catch {
      /* try next */
    }
  }
  throw new Error(
    "Cannot load opencode-dcp-bridge native addon.\n" +
    "Build: cd ~/Projects/dynamic_context_pruning && cargo build -p opencode-dcp-bridge"
  )
}

const createPlugin: Plugin = async (_ctx: PluginInput) => {
  const nativeBridge = loadBridge()
  const configJson = nativeBridge.loadDcpConfig()
  const pruner: DcpPruner = new nativeBridge.DcpPruner(configJson)

  return {
    "experimental.chat.messages.transform": async (_input, output) => {
      if (!output.messages || output.messages.length === 0) return
      const json = JSON.stringify(output.messages)
      const transformed = pruner.transformMessages(json)
      const parsed = JSON.parse(transformed)
      output.messages.length = 0
      output.messages.push(...parsed)
    },

    "experimental.chat.system.transform": async (_input, output) => {
      const joined = output.system.join("\n")
      const result = pruner.transformSystem(joined)
      if (result !== joined) {
        output.system[output.system.length - 1] = result
      }
    },

    tool: createTools(pruner),
  }
}

export default { server: createPlugin } satisfies PluginModule
