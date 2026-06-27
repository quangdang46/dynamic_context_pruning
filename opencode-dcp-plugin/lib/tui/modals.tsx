// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────
 *   lib/tui/modals.tsx — modal open functions
 *
 *   openPanelModal     — show main DCP panel
 *   openContextModal   — show context analysis panel
 *   openStatsModal     — show pruning stats panel
 *   openCommandDialog  — show a single command description
 * ─────────────────────────────────────── */

import { TuiApi } from "./types.js"
import { PanelDialog, ContextDialog, StatsDialog } from "./dialogs.jsx"
import { DcpFrame, FooterButton } from "./ui.jsx"

/* ─── Top-level dialog openers ────────── */

export async function openPanelModal(api: TuiApi): Promise<void> {
  await api.ui.dialog.replace(PanelDialog(api))
}

export async function openContextModal(api: TuiApi): Promise<void> {
  await api.ui.dialog.replace(ContextDialog(api))
}

export async function openStatsModal(api: TuiApi): Promise<void> {
  await api.ui.dialog.replace(StatsDialog(api))
}

/* ─── Single-command info dialog ───────── */

export async function openCommandDialog(
  api: TuiApi,
  command: any,
  description: any,
): Promise<void> {
  const fg = (k: any) => api.theme?.current?.[k] ?? "#fff"

  await api.ui.dialog.replace(
    <DcpFrame api={api} title={command}>
      <text fg={fg("textMuted")} size="small">
        {description}
      </text>
      <text fg={fg("text")} size="small">
        Run this command directly in the chat by typing: {command}
      </text>
      <box justifyContent="flexEnd" gap={1}>
        <FooterButton api={api} label="Back" variant="muted" />
      </box>
    </DcpFrame>,
  )
}
