// @ts-nocheck
/* ───────────────────────────────────────
 *   lib/tui/commands.ts — register DCP palette commands
 * ─────────────────────────────────────── */

import { TuiApi, DcpCommand } from "./types.js"

/**
 * Register DCP commands in the OpenCode command palette.
 * Tries the newer keymap.registerLayer API first, then falls
 * back to the legacy command.register API.
 */
export async function registerCommands(
  api: TuiApi,
  _commands: DcpCommand[],
): Promise<void> {
  const api2 = api as any

  if (typeof api2.keymap?.registerLayer === "function") {
    api2.keymap.registerLayer({
      namespace: "palette",
      commands: [
        {
          name: "dcp.panel",
          title: "DCP Panel",
          description: "Open DCP panel with context, stats and settings",
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
          description: "Show context analysis (turns, blocks, tokens)",
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
          description: "Show pruning statistics",
          category: "DCP",
          slashName: "dcp",
          run: async () => {
            const { openStatsModal } = await import("./modals.js")
            await openStatsModal(api)
          },
        },
      ],
    })
  } else if (typeof api2.command?.register === "function") {
    api2.command.register({
      title: "DCP Panel",
      value: "dcp.panel",
      description: "Open DCP panel with context, stats and settings",
      category: "DCP",
      slash: { name: "dcp" },
      onSelect: async () => {
        const { openPanelModal } = await import("./modals.js")
        await openPanelModal(api)
      },
    })
  }
}
