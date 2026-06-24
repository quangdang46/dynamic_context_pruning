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
  handleCommand(cmd: string, argsJson: string, messagesJson: string): string
  notifyEvent(eventJson: string): void
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

function formatHelpText(): string {
  return [
    "╭──────────────────────────────────────────────────────────────╮",
    "│                      DCP Commands                           │",
    "╰──────────────────────────────────────────────────────────────╯",
    "",
    "  /dcp                    Show this help message",
    "  /dcp context            Show context analysis (turns, blocks, tokens)",
    "  /dcp stats              Show pruning statistics",
    "  /dcp sweep              Flush pending prune strategies",
    "  /dcp manual <on|off>    Toggle manual mode",
    "  /dcp decompress <id>    Restore a compressed block",
    "  /dcp recompress <id>    Re-activate a decompressed block",
    "  /dcp-compress [focus]   Trigger manual compression",
    "",
    "  Tools (available to the LLM):",
    "    compress    Replace stale content with summaries",
    "    decompress  Restore compressed blocks",
    "    recompress  Re-activate decompressed blocks",
    "",
  ].join("\n")
}

const createPlugin: Plugin = async (_ctx: PluginInput) => {
  const nativeBridge = loadBridge()
  const configJson = nativeBridge.loadDcpConfig()
  const config = JSON.parse(configJson)
  const pruner: DcpPruner = new nativeBridge.DcpPruner(configJson)

  return {
    // ─── Message pipeline ─────────────────────────────────────────
    "experimental.chat.messages.transform": async (_input, output) => {
      try {
        if (!output.messages || output.messages.length === 0) return
        const json = JSON.stringify(output.messages)
        const transformed = pruner.transformMessages(json)
        const parsed = JSON.parse(transformed)
        output.messages.length = 0
        output.messages.push(...parsed)
      } catch (err) {
        console.error("[DCP] transform_messages error:", err)
      }
    },

    // ─── System prompt ────────────────────────────────────────────
    "experimental.chat.system.transform": async (_input, output) => {
      try {
        const joined = output.system.join("\n")
        const result = pruner.transformSystem(joined)
        if (result !== joined) {
          output.system[output.system.length - 1] = result
        }
      } catch (err) {
        console.error("[DCP] transform_system error:", err)
      }
    },

    // ─── Slash commands ──────────────────────────────────────────
    "command.execute.before": async (input, output) => {
      try {
        const parts = input.command.split(/\s+/)
        let cmd = parts[0]

        // Strip leading slash if present (OpenCode passes "/dcp")
        if (cmd.startsWith("/")) {
          cmd = cmd.slice(1)
        }

        if (cmd === "dcp" || cmd === "dcp-compress") {
          const subcommand = parts.length > 1 ? parts[1] : "help"
          const args = parts.slice(2)

          // Handle help in TypeScript (purely presentational)
          if (subcommand === "help") {
            output.parts.length = 0
            output.parts.push({
              type: "text",
              text: formatHelpText(),
              id: `dcp-${Date.now()}`,
              sessionID: input.sessionID,
              messageID: `cmd-${Date.now()}`,
              synthetic: true,
            } as any)
            return
          }

          const actualCmd = cmd === "dcp-compress" ? "compress" : subcommand
          const resultJson = pruner.handleCommand(actualCmd, JSON.stringify(args), "[]")
          const result = JSON.parse(resultJson)

          output.parts.length = 0
          output.parts.push({
            type: "text",
            text: result.status === "ok"
              ? result.text
              : `⚠️ ${result.text}`,
            id: `dcp-${Date.now()}`,
            sessionID: input.sessionID,
            messageID: `cmd-${Date.now()}`,
            synthetic: true,
          } as any)
        }
      } catch (err) {
        console.error("[DCP] command.execute.before error:", err)
      }
    },

    // ─── Event tracking ──────────────────────────────────────────
    event: async (input) => {
      try {
        if (input.event?.type) {
          pruner.notifyEvent(JSON.stringify(input.event))
        }
      } catch (err) {
        console.error("[DCP] event error:", err)
      }
    },

    // ─── Config hook: register slash commands + negotiate permissions ─
    config: async (opencodeConfig) => {
      try {
        if (config.compress?.permission !== "deny") {
          opencodeConfig.command ??= {}

          // Register /dcp as a slash command (shows in palette)
          opencodeConfig.command["dcp"] = {
            template: "",
            description: "DCP: context, stats, sweep, manual, decompress, recompress",
          }

          // Register /dcp-compress as a slash command
          opencodeConfig.command["dcp-compress"] = {
            template: "",
            description: "Trigger DCP manual compression with: /dcp-compress [focus]",
          }

          // Add compress as a primary tool so it's always available
          const existingPrimaryTools = opencodeConfig.experimental?.primary_tools ?? []
          opencodeConfig.experimental = {
            ...opencodeConfig.experimental,
            primary_tools: [...existingPrimaryTools, "compress"],
          }

          // Set compress tool permission to match config
          const permission = opencodeConfig.permission ?? {}
          opencodeConfig.permission = {
            ...permission,
            compress: config.compress?.permission ?? "allow",
          } as typeof permission
        }
      } catch (err) {
        console.error("[DCP] config error:", err)
      }
    },

    // ─── Cleanup on plugin reload ────────────────────────────────
    dispose: async () => {
      try {
        pruner.setSessionId("__dispose__")
        pruner.notifyEvent(JSON.stringify({ type: "plugin.dispose" }))
      } catch (err) {
        console.error("[DCP] dispose error:", err)
      }
    },

    // ─── Tools exposed to the LLM ────────────────────────────────
    tool: createTools(pruner),
  }
}

export default { server: createPlugin } satisfies PluginModule
