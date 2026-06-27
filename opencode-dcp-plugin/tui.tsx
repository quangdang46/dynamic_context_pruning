// @ts-nocheck
/* @jsxImportSource @opentui/solid */
import { registerCommands } from "./lib/tui/commands"

const tui = async (api: any) => {
  await registerCommands(api, [])
}

export default { id: "opencode-dcp", tui }
