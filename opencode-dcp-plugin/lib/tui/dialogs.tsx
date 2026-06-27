// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────
 *   lib/tui/dialogs.tsx — dialog content components
 *
 *   PanelDialog  — main DCP overview panel
 *   ContextDialog — context / token breakdown
 *   StatsDialog  — pruning statistics
 * ─────────────────────────────────────── */

import { TuiApi } from "./types.js"
import { DcpFrame, Card, Metric, ActionRow, FooterButton } from "./ui.jsx"
import { openCommandDialog } from "./modals.js"

/* ─── PanelDialog ────────────────────── */

export function PanelDialog(api: TuiApi) {
  return (
    <DcpFrame api={api} title={undefined}>
      <text fg={api.theme?.current?.text ?? "#fff"} attributes={{ bold: true }} size="large">
        ▣ DCP — Dynamic Context Pruning
      </text>
      <text
        fg={api.theme?.current?.textMuted ?? "#888"}
        size="small"
      >
        Manage conversation context, pruning and compression
      </text>

      <Card api={api} header="COMMANDS">
        <ActionRow
          api={api}
          command="/dcp help"
          description="Show available DCP commands"
          onClick={() => openCommandDialog(api, "/dcp help", "Show available DCP commands")}
        />
        <ActionRow
          api={api}
          command="/dcp context"
          description="Show token usage breakdown"
          onClick={() => openCommandDialog(api, "/dcp context", "Show token usage breakdown")}
        />
        <ActionRow
          api={api}
          command="/dcp stats"
          description="Show pruning statistics"
          onClick={() => openCommandDialog(api, "/dcp stats", "Show pruning statistics")}
        />
        <ActionRow
          api={api}
          command="/dcp sweep"
          description="Flush pending prune strategies"
          onClick={() => openCommandDialog(api, "/dcp sweep", "Flush pending prune strategies")}
        />
        <ActionRow
          api={api}
          command="/dcp manual on"
          description="Toggle manual mode"
          onClick={() => openCommandDialog(api, "/dcp manual on", "Toggle manual mode")}
        />
        <ActionRow
          api={api}
          command="/dcp-compress [focus]"
          description="Trigger manual compression"
          onClick={() => openCommandDialog(api, "/dcp-compress [focus]", "Trigger manual compression")}
        />
      </Card>

      <box justifyContent="flexEnd" gap={1}>
        <FooterButton api={api} label="Close" variant="muted" />
      </box>
    </DcpFrame>
  )
}

/* ─── ContextDialog ──────────────────── */

export function ContextDialog(api: TuiApi) {
  const fg = (k: any) => api.theme?.current?.[k] ?? "#fff"
  // Context values shown as placeholders — real values come from the bridge.
  const metrics = [
    { label: "Total Turns", value: "—" },
    { label: "Compressed Blocks", value: "—" },
    { label: "Active Messages", value: "—" },
    { label: "Estimated Tokens", value: "—" },
    { label: "Tokens Saved", value: "—" },
    { label: "Compression Ratio", value: "—" },
  ]

  return (
    <DcpFrame api={api} title={"▣ Context Analysis"}>
      <text
        fg={fg("textMuted")}
        size="small"
      >
        Token usage and context breakdown
      </text>

      <Card api={api}>
        {metrics.map((m) => (
          <Metric api={api} label={m.label} value={m.value} />
        ))}
      </Card>

      <box justifyContent="flexEnd" gap={1}>
        <FooterButton api={api} label="Back" variant="muted" />
      </box>
    </DcpFrame>
  )
}

/* ─── StatsDialog ─────────────────────── */

export function StatsDialog(api: TuiApi) {
  const fg = (k: any) => api.theme?.current?.[k] ?? "#fff"
  const metrics = [
    { label: "Total Compressed", value: "—" },
    { label: "Total Decompressed", value: "—" },
    { label: "Total Recompressed", value: "—" },
    { label: "Tokens Saved", value: "—" },
    { label: "Compression Ratio", value: "—" },
    { label: "Active Blocks", value: "—" },
    { label: "Pending Work", value: "—" },
  ]

  return (
    <DcpFrame api={api} title={"▣ Pruning Statistics"}>
      <text
        fg={fg("textMuted")}
        size="small"
      >
        Compression and pruning performance data
      </text>

      <Card api={api}>
        {metrics.map((m) => (
          <Metric api={api} label={m.label} value={m.value} />
        ))}
      </Card>

      <box justifyContent="flexEnd" gap={1}>
        <FooterButton api={api} label="Back" variant="muted" />
      </box>
    </DcpFrame>
  )
}
