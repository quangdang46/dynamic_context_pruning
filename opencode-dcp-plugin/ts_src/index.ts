import type { Plugin, PluginInput } from "@opencode-ai/plugin"
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
  const platformCandidates = [
    join(root, "npm", "darwin-arm64", "opencode-dcp-bridge.darwin-arm64.node"),
    join(root, "npm", "darwin-x64", "opencode-dcp-bridge.darwin-x64.node"),
    join(root, "npm", "linux-x64-gnu", "opencode-dcp-bridge.linux-x64-gnu.node"),
    join(root, "npm", "win32-x64-msvc", "opencode-dcp-bridge.win32-x64-msvc.node"),
  ]
  const rootCandidates = [
    join(root, "opencode-dcp-bridge.darwin-arm64.node"),
    join(root, "opencode-dcp-bridge.darwin-x64.node"),
    join(root, "opencode-dcp-bridge.linux-x64-gnu.node"),
    join(root, "opencode-dcp-bridge.win32-x64-msvc.node"),
  ]
  const candidates = [...platformCandidates, ...rootCandidates]
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

/** Send an ignored message via the OpenCode SDK session API. */
async function sendIgnoredMessage(client: any, sessionID: string, text: string): Promise<void> {
  try {
    await client.session.prompt({
      path: { id: sessionID },
      body: {
        noReply: true,
        parts: [{ type: "text", text, ignored: true }],
      },
    })
  } catch {
    // session.prompt may not be available; swallow.
  }
}

const createPlugin: Plugin = async (ctx: PluginInput) => {
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
        const cmd = input.command.replace(/^\//, "").split(/\s+/)[0]
        if (cmd !== "dcp" && cmd !== "dcp-compress") return

        const parts = input.command.split(/\s+/)
        const subcommand = parts.length > 1 ? parts[1] : "help"
        const args = parts.slice(2)

        let replyText: string
        if (subcommand === "help") {
          replyText = formatHelpText()
        } else {
          const actualCmd = cmd === "dcp-compress" ? "compress" : subcommand
          const resultJson = pruner.handleCommand(actualCmd, JSON.stringify(args), "[]")
          const result = JSON.parse(resultJson)
          replyText = result.status === "ok" ? result.text : `⚠️ ${result.text}`
        }

        // Send reply via client.session.prompt (reference plugin pattern).
        // Do NOT mutate output.parts — OpenCode 1.17.x crashes on
        // parts manipulation ("R.text.trim" TypeError).
        await sendIgnoredMessage(ctx.client, input.sessionID, replyText)
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
          opencodeConfig.command["dcp"] = {
            template: "",
            description: "DCP: context, stats, sweep, manual, decompress, recompress",
          }
          opencodeConfig.command["dcp-compress"] = {
            template: "",
            description: "Trigger DCP manual compression with: /dcp-compress [focus]",
          }
          const existingPrimaryTools = opencodeConfig.experimental?.primary_tools ?? []
          opencodeConfig.experimental = {
            ...opencodeConfig.experimental,
            primary_tools: [...existingPrimaryTools, "compress"],
          }
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

export default createPlugin satisfies Plugin
