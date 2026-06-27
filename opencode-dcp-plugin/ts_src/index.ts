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
  const root = resolve(__dirname, "../..")
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

    // ─── Text completion (hallucination stripping) ────────────────
    "experimental.text.complete": async (_input, output) => {
      // Strip DCP XML markers from standalone text completions
      // that skip the chat message transform pipeline.
      if (output.text) {
        output.text = output.text
          .replace(/<dcp-message-id>.*?<\/dcp-message-id>/g, "")
          .replace(/<dcp-system-reminder>.*?<\/dcp-system-reminder>/g, "")
          .replace(/<dcp-manual-compress>.*?<\/dcp-manual-compress>/g, "")
      }
    },

    // ─── Slash commands ──────────────────────────────────────────
    "command.execute.before": async (input, output) => {
      try {
        const full = input.command.replace(/^\//, "")
        const parts = full.split(/\s+/)
        const cmd = parts[0]
        if (cmd !== "dcp" && cmd !== "dcp-compress") return

        const subcommand = parts.length > 1 ? parts[1] : "help"
        const args = parts.slice(2)

        // /dcp-compress [focus] — inject a manual-compress prompt into
        // output.parts so the model sees it and runs the compress tool.
        if (cmd === "dcp-compress" || subcommand === "compress") {
          const focus = args.join(" ") || ""
          const prompt =
            "<dcp-manual-compress>\nManual compression triggered" +
            (focus ? " (focus: " + focus + ")" : "") +
            ".\nPlease use the compress tool to compress stale conversation content.\n</dcp-manual-compress>"

          const textIdx = (output.parts as any).findIndex((p: any) => p.type === "text")
          if (textIdx >= 0) {
            ;(output.parts as any)[textIdx].text = prompt
          } else {
            ;(output.parts as any).unshift({ type: "text", text: prompt })
          }
          return
        }

        // Build reply for all other subcommands.
        let replyText: string
        if (subcommand === "help") {
          replyText = formatHelpText()
        } else {
          // Fetch session messages for accurate context/stats.
          let messagesJson = "[]"
          try {
            const c = ctx.client as any
            if (typeof c?.session?.messages === "function") {
              const resp = await c.session.messages({ path: { id: input.sessionID } })
              const msgData = resp?.data ?? resp ?? []
              messagesJson = JSON.stringify(Array.isArray(msgData) ? msgData : [])
            }
          } catch {
            // messages() may not be available; fall back to "[]".
          }

          const resultJson = pruner.handleCommand(subcommand, JSON.stringify(args), messagesJson)
          const result = JSON.parse(resultJson)
          replyText = result.status === "ok" ? result.text : `⚠️ ${result.text}`
        }

        await sendIgnoredMessage(ctx.client, input.sessionID, replyText)

        // Replace command text so the model does NOT see "/dcp" as
        // user input.  OpenCode 1.17.x snapshots the parts array
        // before the handler runs, so clearing the array is futile.
        // Instead we replace every text part with an empty placeholder.
        for (let i = 0; i < (output.parts as any).length; i++) {
          const p = (output.parts as any)[i]
          if (p.type === "text" && p.text && p.text.startsWith("/dcp")) {
            p.text = ""
          }
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
