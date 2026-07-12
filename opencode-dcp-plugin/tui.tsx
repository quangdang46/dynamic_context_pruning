/** @jsxImportSource @opentui/solid */

import type { TuiPluginModule } from "@opencode-ai/plugin/tui"
import { registerCommands } from "./lib/tui/commands"
import { loadConfig } from "./lib/tui/data"
import { openPanelModal } from "./lib/tui/modals"

const tui: TuiPluginModule["tui"] = async (api) => {
    const config = loadConfig(api)
    if (!config.enabled || !config.commands.enabled) return

    registerCommands(api, [
        {
            title: "DCP rust",
            name: "dcp.panel",
            description: "Open DCP rust panel",
            slashName: "dcp",
            run: () => openPanelModal(api, config),
        },
    ])
}

export default {
    id: "dcp-rust",
    tui,
} satisfies TuiPluginModule
