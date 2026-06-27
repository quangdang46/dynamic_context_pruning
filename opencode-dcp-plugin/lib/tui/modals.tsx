// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────────
 *   lib/tui/modals.tsx — modal open functions
 *
 *   openPanelModal     — show the main DCP overview
 *   openContextModal   — show context / token analysis
 *   openStatsModal     — show pruning statistics
 *   openCommandDialog  — show a single command overview
 * ─────────────────────────────────────────── */

import { TextAttributes } from "@opentui/core"
import type { TuiApi } from "./types.js"
import { PanelDialog, ContextDialog, StatsDialog } from "./dialogs.jsx"
import { DcpFrame, FooterButton } from "./ui.jsx"

/* ─── Top-level dialog openers ────────────── */

/** Open the main DCP panel overview. */
export async function openPanelModal(api: TuiApi): Promise<void> {
  await api.ui.dialog.replace(PanelDialog(api))
}

/** Open the context analysis panel with token breakdown. */
export async function openContextModal(api: TuiApi): Promise<void> {
  await api.ui.dialog.replace(ContextDialog(api))
}

/** Open the pruning statistics panel. */
export async function openStatsModal(api: TuiApi): Promise<void> {
  await api.ui.dialog.replace(StatsDialog(api))
}

/* ─── Single-command info dialog ──────────── */

/**
 * Open a small dialog that shows a single command
 * and its description, with a "Back" button.
 */
export async function openCommandDialog(
  api: TuiApi,
  command: string,
  description: string,
): Promise<void> {
  const fg = (k: string): string =>
    ((api.theme?.current ?? {}) as Record<string, string>)[k] ?? "#ffffff"

  await api.ui.dialog.replace(
    <DcpFrame api={api} title={command}>
      <text fg={fg("textMuted")} size="small">
        {description}
      </text>
      <text fg={fg("text")} size="small" attributes={TextAttributes.BOLD}>
        Run this command in the chat: {command}
      </text>
    </DcpFrame>,
  )
}
