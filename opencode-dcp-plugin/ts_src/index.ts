import type { Plugin, PluginInput } from "@opencode-ai/plugin"
import { createTools } from "./tools.js"
import { createPruner, type DcpPruner } from "../lib/bridge.js"

/** Upstream-style manual compress prompt (injected on next transform). */
function buildCompressTriggerPrompt(focus?: string): string {
  const sections = [
    "<compress triggered manually>",
    "Manual mode trigger received. You must now use the compress tool.",
    "Find the most significant completed conversation content that can be compressed into a high-fidelity technical summary.",
    "Follow the active compress mode, preserve all critical implementation details, and choose safe targets.",
    "Return after compress with a brief explanation of what content was compressed.",
  ]
  if (focus && focus.trim()) {
    sections.push(`Additional user focus:\n${focus.trim()}`)
  }
  return sections.join("\n\n")
}

/** Send an ignored (no-reply) chat notification. */
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
    // session.prompt may not be available
  }
}

async function fetchSessionMessagesJson(
  client: any,
  sessionID: string | undefined,
): Promise<string> {
  if (!sessionID || !client?.session?.messages) return "[]"
  try {
    const res = await client.session.messages({ path: { id: sessionID } })
    const list = res?.data ?? res ?? []
    const normalised = (Array.isArray(list) ? list : []).map((item: any) => {
      if (item?.info && Array.isArray(item.parts)) return item
      if (item?.id && item?.role) {
        return { info: item, parts: item.parts ?? [] }
      }
      return item
    })
    return JSON.stringify(normalised)
  } catch {
    return "[]"
  }
}

/**
 * Apply pending manual-compress prompt onto the last real user text part
 * that still shows `/dcp-compress…` (mirrors upstream applyPendingManualTrigger).
 */
function applyPendingCompressPrompt(messages: any[], prompt: string): boolean {
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i]
    if (msg?.info?.role !== "user") continue
    const parts = Array.isArray(msg.parts) ? msg.parts : []
    for (const part of parts) {
      if (part?.type !== "text") continue
      if (part.ignored || part.synthetic) continue
      const t = String(part.text || "")
      if (t.startsWith("/dcp-compress") || t.includes("<compress triggered manually>")) {
        part.text = prompt
        return true
      }
    }
  }
  return false
}

const createPlugin: Plugin = async (ctx: PluginInput) => {
  let pruner: DcpPruner
  let config: any

  try {
    const created = createPruner()
    pruner = created.pruner
    config = created.config
  } catch (err) {
    console.error("[DCP] Failed to load native bridge:", err)
    return {}
  }

  if (config?.enabled === false) {
    return {}
  }

  let currentSessionID: string | undefined
  /** Pending compress prompt to inject on next messages.transform (upstream pattern). */
  let pendingCompressPrompt: string | null = null
  let pendingCompressSession: string | null = null

  const getMessagesJson = async () => fetchSessionMessagesJson(ctx.client, currentSessionID)

  return {
    // ─── Message pipeline ─────────────────────────────────────────
    "experimental.chat.messages.transform": async (input: any, output: any) => {
      try {
        if (!output?.messages || output.messages.length === 0) return

        const sid =
          input?.sessionID || output.messages[0]?.info?.sessionID || currentSessionID
        if (sid && sid !== currentSessionID) {
          currentSessionID = sid
          pruner.setSessionId(sid)
        }

        // Apply queued /dcp-compress prompt before DCP transform (upstream).
        if (
          pendingCompressPrompt &&
          pendingCompressSession &&
          sid &&
          pendingCompressSession === sid
        ) {
          if (applyPendingCompressPrompt(output.messages, pendingCompressPrompt)) {
            pendingCompressPrompt = null
            pendingCompressSession = null
          }
        }

        const json = JSON.stringify(output.messages)
        const transformed = pruner.transformMessages(json)
        const parsed = JSON.parse(transformed)
        if (!Array.isArray(parsed)) {
          console.error("[DCP] transform_messages returned non-array")
          return
        }

        output.messages.length = 0
        output.messages.push(...parsed)
      } catch (err) {
        console.error("[DCP] transform_messages error:", err)
      }
    },

    // ─── System prompt ────────────────────────────────────────────
    "experimental.chat.system.transform": async (input: any, output: any) => {
      try {
        if (!Array.isArray(output?.system)) return
        if (input?.sessionID) {
          currentSessionID = input.sessionID
          pruner.setSessionId(input.sessionID)
        }
        const joined = output.system.join("\n")
        const result = pruner.transformSystem(joined)
        if (result === joined) return

        const addendum = result.startsWith(joined)
          ? result.slice(joined.length).replace(/^\n+/, "")
          : result
        if (!addendum) return

        if (output.system.length > 0) {
          const last = output.system[output.system.length - 1]
          const sep = last.endsWith("\n") ? "\n" : "\n\n"
          output.system[output.system.length - 1] = last + sep + addendum
        } else {
          output.system.push(addendum)
        }
      } catch (err) {
        console.error("[DCP] transform_system error:", err)
      }
    },

    // ─── Text completion (strip hallucinated DCP tags) ────────────
    "experimental.text.complete": async (_input: any, output: any) => {
      if (output?.text) {
        output.text = output.text
          .replace(/<dcp-message-id>[\s\S]*?<\/dcp-message-id>/g, "")
          .replace(/<dcp-system-reminder>[\s\S]*?<\/dcp-system-reminder>/g, "")
          .replace(/<dcp-manual-compress>[\s\S]*?<\/dcp-manual-compress>/g, "")
          .replace(/<dcp-nudge>[\s\S]*?<\/dcp-nudge>/g, "")
          .replace(/<compress triggered manually>[\s\S]*$/g, "")
      }
    },

    // ─── Slash commands (chat surface only) ───────────────────────
    // Upstream: /dcp is TUI-only (panel dialog). Only /dcp-compress is a
    // chat slash command. Subcommands like /dcp context are TUI or
    // ignored-message fallbacks — never dump the old ASCII help box.
    "command.execute.before": async (input: any, output: any) => {
      try {
        if (config?.commands?.enabled === false) return

        // OpenCode may pass "dcp" or "dcp-compress" (no leading slash).
        // Arguments often arrive separately in input.arguments.
        const rawCmd = String(input.command || "").replace(/^\//, "").trim()
        const rawArgs = String(input.arguments || "").trim()
        const cmdParts = rawCmd.split(/\s+/).filter(Boolean)
        const cmd = cmdParts[0]
        if (!cmd) return

        if (input.sessionID) {
          currentSessionID = input.sessionID
          pruner.setSessionId(input.sessionID)
        }

        // ── /dcp-compress [focus] ─────────────────────────────────
        // Upstream: queue prompt, leave user message as the command line.
        // The real prompt is swapped in on the next messages.transform.
        if (cmd === "dcp-compress") {
          const focus =
            rawArgs ||
            (cmdParts.length > 1 ? cmdParts.slice(1).join(" ") : "") ||
            ""
          pendingCompressPrompt = buildCompressTriggerPrompt(focus)
          pendingCompressSession = input.sessionID || currentSessionID || null

          const display = focus ? `/dcp-compress ${focus}` : "/dcp-compress"
          if (Array.isArray(output.parts)) {
            output.parts.length = 0
            output.parts.push({ type: "text", text: display })
          }
          return
        }

        // ── /dcp … ────────────────────────────────────────────────
        // Bare /dcp must open the TUI panel (registered in tui.tsx).
        // If it still hits the chat hook (e.g. older OpenCode, or we
        // accidentally registered it as a command), do NOT print help.
        // Only handle explicit subcommands as ignored chat messages.
        if (cmd === "dcp") {
          // Prefer arguments field; also accept "dcp context" in command.
          const argTokens = (
            rawArgs || (cmdParts.length > 1 ? cmdParts.slice(1).join(" ") : "")
          )
            .split(/\s+/)
            .filter(Boolean)

          // Bare `/dcp` → leave alone so TUI slash handler can open panel.
          // If OpenCode already routed here, cancel the chat reply.
          if (argTokens.length === 0) {
            if (Array.isArray(output.parts)) {
              output.parts.length = 0
              // Empty parts: prevent a bogus user message; TUI should own /dcp.
            }
            return
          }

          const sub = argTokens[0].toLowerCase()
          const subArgs = argTokens.slice(1)

          // /dcp compress → same as /dcp-compress
          if (sub === "compress") {
            const focus = subArgs.join(" ")
            pendingCompressPrompt = buildCompressTriggerPrompt(focus)
            pendingCompressSession = input.sessionID || currentSessionID || null
            if (Array.isArray(output.parts)) {
              output.parts.length = 0
              output.parts.push({
                type: "text",
                text: focus ? `/dcp-compress ${focus}` : "/dcp-compress",
              })
            }
            return
          }

          // Other subcommands: run via bridge, post as ignored message.
          const msgs = await getMessagesJson()
          const resultJson = pruner.handleCommand(sub, JSON.stringify(subArgs), msgs)
          let text = resultJson
          try {
            const parsed = JSON.parse(resultJson)
            text = parsed.text || resultJson
          } catch {
            /* keep raw */
          }

          if (input.sessionID) {
            await sendIgnoredMessage(ctx.client, input.sessionID, text)
          }

          // Prevent the slash command from becoming a normal user turn.
          if (Array.isArray(output.parts)) {
            output.parts.length = 0
          }
        }
      } catch (err) {
        console.error("[DCP] command.execute.before error:", err)
      }
    },

    // ─── Event tracking ──────────────────────────────────────────
    event: async (input: any) => {
      try {
        if (input?.event) {
          pruner.notifyEvent(JSON.stringify(input.event))
          const sid =
            input.event.properties?.sessionID ||
            input.event.properties?.info?.sessionID ||
            input.event.sessionID
          if (sid) {
            currentSessionID = sid
            if (String(input.event.type || "").includes("session")) {
              pruner.setSessionId(sid)
            }
          }
        }
      } catch (err) {
        console.error("[DCP] event error:", err)
      }
    },

    // ─── Config: only register chat slash commands OpenCode needs ─
    // Upstream registers ONLY dcp-compress here. `/dcp` is TUI-only
    // (palette + slash via tui.tsx). Registering `dcp` as a chat command
    // steals it from the TUI and shows help/text instead of the dialog.
    config: async (opencodeConfig: any) => {
      try {
        if (config.compress?.permission === "deny") return

        opencodeConfig.command ??= {}
        opencodeConfig.command["dcp-compress"] = {
          template: "",
          description: "Trigger DCP manual compression with: /dcp-compress [focus]",
        }

        const existingPrimaryTools = opencodeConfig.experimental?.primary_tools ?? []
        opencodeConfig.experimental = {
          ...opencodeConfig.experimental,
          primary_tools: [...new Set([...existingPrimaryTools, "compress"])],
        }

        const permission = opencodeConfig.permission ?? {}
        if (permission.compress === undefined) {
          opencodeConfig.permission = {
            ...permission,
            compress: config.compress?.permission ?? "allow",
          }
        }
      } catch (err) {
        console.error("[DCP] config error:", err)
      }
    },

    dispose: async () => {
      try {
        pruner.notifyEvent(JSON.stringify({ type: "plugin.dispose" }))
      } catch (err) {
        console.error("[DCP] dispose error:", err)
      }
    },

    tool: createTools({ pruner, getMessagesJson }),
  }
}

export default createPlugin satisfies Plugin
