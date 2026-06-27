// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────────
 *   lib/tui/commands.ts — DCP palette commands
 *
 *   registerCommands  — adds DCP commands to OpenCode's
 *                       command palette via either:
 *     a) keymap.registerLayer (newer API)
 *     b) command.register     (legacy fallback)
 *
 *   Supports three modes:
 *     "panel"   — show the main DCP overview
 *     "context" — show context / token breakdown
 *     "stats"   — show pruning statistics
 * ─────────────────────────────────────────── */

import type {
  TuiApi,
  DcpCommand,
  DcpCommandPaletteItem,
  KeymapLayer,
  KeymapCommand,
} from "./types.js"

/**
 * Register DCP commands in the OpenCode command palette.
 * Tries the newer keymap.registerLayer API first, then falls
 * back to the legacy command.register API.
 *
 * @param api    — the TuiApi object provided by OpenCode
 * @param _cmds  — reserved for future use; currently unused
 */
export async function registerCommands(
  api: TuiApi,
  _cmds: DcpCommand[],
): Promise<void> {
  const api2 = api as Record<string, unknown>

  /* ── keymap.registerLayer (newer API) ────────────── */
  if (typeof (api2.keymap as Record<string, unknown>)?.registerLayer === "function") {
    const km = api2.keymap as Record<string, unknown>
    ;(km.registerLayer as Function)({
      namespace: "dcp",
      commands: [
        {
          name: "dcp.panel",
          title: "DCP Panel",
          description: "Open DCP overview panel with context, stats and commands",
          category: "DCP",
          slashName: "dcp",
          run: async () => {
            const { openPanelModal } = await import("./modals.js")
            await openPanelModal(api)
          },
        },
        {
          name: "dcp.context",
          title: "DCP Context",
          description: "Show context analysis — turns, blocks, token breakdown",
          category: "DCP",
          slashName: "dcp",
          run: async () => {
            const { openContextModal } = await import("./modals.js")
            await openContextModal(api)
          },
        },
        {
          name: "dcp.stats",
          title: "DCP Stats",
          description: "Show pruning and compression statistics",
          category: "DCP",
          slashName: "dcp",
          run: async () => {
            const { openStatsModal } = await import("./modals.js")
            await openStatsModal(api)
          },
        },
      ] satisfies KeymapCommand[],
    } as KeymapLayer)

    return
  }

  /* ── command.register (legacy fallback) ──────────── */
  if (typeof (api2.command as Record<string, unknown>)?.register === "function") {
    const cmd = api2.command as Record<string, unknown>
    ;(cmd.register as Function)({
      title: "DCP Panel",
      value: "dcp.panel",
      description: "Open DCP overview panel with context, stats and commands",
      category: "DCP",
      slash: { name: "dcp" },
      onSelect: async () => {
        const { openPanelModal } = await import("./modals.js")
        await openPanelModal(api)
      },
    } satisfies DcpCommandPaletteItem)

    ;(cmd.register as Function)({
      title: "DCP Context",
      value: "dcp.context",
      description: "Show context analysis — turns, blocks, token breakdown",
      category: "DCP",
      slash: { name: "dcp" },
      onSelect: async () => {
        const { openContextModal } = await import("./modals.js")
        await openContextModal(api)
      },
    } satisfies DcpCommandPaletteItem)

    ;(cmd.register as Function)({
      title: "DCP Stats",
      value: "dcp.stats",
      description: "Show pruning and compression statistics",
      category: "DCP",
      slash: { name: "dcp" },
      onSelect: async () => {
        const { openStatsModal } = await import("./modals.js")
        await openStatsModal(api)
      },
    } satisfies DcpCommandPaletteItem)

    return
  }

  console.warn("[DCP TUI] No command registration API found (keymap.registerLayer / command.register)")
}
