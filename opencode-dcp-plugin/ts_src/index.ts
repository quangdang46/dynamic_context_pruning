import type { Plugin, PluginModule, PluginInput } from "@opencode-ai/plugin"
import { createTools } from "./tools.js"

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
  const candidates = [
    "opencode-dcp-bridge.darwin-arm64.node",
    "opencode-dcp-bridge.darwin-x64.node",
    "opencode-dcp-bridge.linux-x64-gnu.node",
    "opencode-dcp-bridge.win32-x64-msvc.node",
    "opencode-dcp-bridge.node",
  ]
  for (const name of candidates) {
    try {
      return require(name) as BridgeExports
    } catch {
      /* try next */
    }
  }
  throw new Error(
    "Cannot load opencode-dcp-bridge native addon. " +
    "Build it first: cd crates/opencode-dcp-bridge && cargo build"
  )
}

const createPlugin: Plugin = async (_ctx: PluginInput) => {
  const bridge = loadBridge()
  const configJson = bridge.loadDcpConfig()
  const pruner: DcpPruner = new bridge.DcpPruner(configJson)

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
