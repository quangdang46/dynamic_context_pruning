// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────────
 *   tui.tsx — OpenCode plugin TUI entry point
 *
 *   This file is the TUI sub-entry declared in
 *   package.json under `exports["./tui"]`.
 *
 *   It registers the DCP command palette entries
 *   (panel, context, stats) and optionally loads
 *   the config from the bridge.
 *
 *   OpenCode loads this file when the session
 *   starts and calls the exported `tui` function
 *   with the TuiApi instance.
 * ─────────────────────────────────────────── */

import { registerCommands } from "./lib/tui/commands.js"
import type { TuiApi } from "./lib/tui/types.js"

/**
 * Plugin TUI descriptor.
 *
 * `id` must match the `exports` key in package.json
 * so OpenCode can route the TuiApi to this handler.
 *
 * The `tui` function:
 *   1. Register DCP commands in the command palette.
 *   2. Additional initialisation can be added here.
 */
const descriptor = {
  id: "opencode-dcp",
  tui: async (api: TuiApi): Promise<void> => {
    /* ── Register palette commands ────────────── */
    await registerCommands(api, [])

    /* ── Optional: attach to lifecycle events ── */
    /* The bridge will push stats through theme
       slots.  Polling is not needed — OpenCode
       re-renders dialogs on each open. */
  },
}

export default descriptor
