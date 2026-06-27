// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────────
 *   lib/tui/dialogs.tsx — dialog content components
 *
 *   PanelDialog     — main DCP overview panel
 *   ContextDialog   — context / token breakdown
 *   StatsDialog     — session stats and all-time stats
 * ─────────────────────────────────────────── */

import { TextAttributes } from "@opentui/core"
import type { TuiApi } from "./types.js"
import type { StatsReport } from "../types.js"
import {
  DcpFrame,
  Card,
  Metric,
  Progress,
  PromptRow,
  StatusPill,
  ActionRow,
  FooterButton,
} from "./ui.jsx"
import { openContextModal, openStatsModal, openCommandDialog } from "./modals.js"

/* ─── PanelDialog ───────────────────────────
 *
 *   Main DCP overview panel.  Shows the DCP header
 *   with a ▣ section, a COMMANDS card with available
 *   DCP actions, and a close button.
 */

export function PanelDialog(api: TuiApi) {
  const fg = (k: string): string =>
    ((api.theme?.current ?? {}) as Record<string, string>)[k] ?? "#ffffff"

  return (
    <DcpFrame api={api}>
      {/* ── Header ──────────────────────────── */}
      <text fg={fg("text")} attributes={TextAttributes.BOLD} size="large">
        {"▣"} DCP — Dynamic Context Pruning
      </text>
      <text fg={fg("textMuted")} size="small">
        Manage conversation context, pruning and compression strategies
      </text>

      {/* ── Command Palette ────────────────── */}
      <Card api={api} header="COMMANDS">
        <ActionRow
          api={api}
          command="/dcp help"
          description="Show available DCP commands"
          onClick={async () => {
            await openCommandDialog(api, "/dcp help", "Show available DCP commands")
          }}
        />
        <ActionRow
          api={api}
          command="/dcp context"
          description="Show token usage breakdown and context analysis"
          onClick={async () => {
            await openContextModal(api)
          }}
        />
        <ActionRow
          api={api}
          command="/dcp stats"
          description="Show pruning and compression statistics"
          onClick={async () => {
            await openStatsModal(api)
          }}
        />
        <ActionRow
          api={api}
          command="/dcp sweep"
          description="Flush pending prune strategies"
          onClick={async () => {
            await openCommandDialog(api, "/dcp sweep", "Flush pending prune strategies")
          }}
        />
        <ActionRow
          api={api}
          command="/dcp manual on"
          description="Toggle manual compression mode"
          onClick={async () => {
            await openCommandDialog(api, "/dcp manual on", "Toggle manual compression mode")
          }}
        />
        <ActionRow
          api={api}
          command="/dcp-compress [focus]"
          description="Trigger manual compression with optional focus topic"
          onClick={async () => {
            await openCommandDialog(
              api,
              "/dcp-compress [focus]",
              "Trigger manual compression with an optional focus topic",
            )
          }}
        />
      </Card>

      {/* ── Quick Status ─────────────────────── */}
      <Card api={api} header="QUICK STATUS">
        <box justifyContent="spaceBetween" width="100%">
          <StatusPill api={api} status="ok" label="DCP Active" />
          <StatusPill api={api} status="info" label="Compress: Allow" />
        </box>
      </Card>
    </DcpFrame>
  )
}

/* ─── ContextDialog ─────────────────────────
 *
 *   Context analysis panel showing a breakdown
 *   of tokens and messages across the four
 *   conversation segments:
 *     • System prompts
 *     • User messages
 *     • Assistant messages / responses
 *     • Tool call messages
 *
 *   Each segment gets a real Progress bar.
 *   Below the breakdown, aggregate metrics are
 *   shown in a Card.
 */

interface ContextMetrics {
  system: { current: number; max: number }
  user: { current: number; max: number }
  assistant: { current: number; max: number }
  tools: { current: number; max: number }
  totalTurns: number
  totalTokens: number
  tokensSaved: number
  compressionRatio: number
}

function defaultContextMetrics(): ContextMetrics {
  return {
    system: { current: 0, max: 0 },
    user: { current: 0, max: 0 },
    assistant: { current: 0, max: 0 },
    tools: { current: 0, max: 0 },
    totalTurns: 0,
    totalTokens: 0,
    tokensSaved: 0,
    compressionRatio: 0,
  }
}

function readContextMetrics(api: TuiApi): ContextMetrics {
  try {
    const theme = (api.theme?.current ?? {}) as Record<string, unknown>
    const raw = theme["dcpContext"]
    const data: Record<string, unknown> =
      typeof raw === "string" ? JSON.parse(raw) : (raw as Record<string, unknown>)

    if (data && typeof data === "object") {
      return {
        system: {
          current: Number((data.system as Record<string, unknown>)?.current ?? 0),
          max: Number((data.system as Record<string, unknown>)?.max ?? 0),
        },
        user: {
          current: Number((data.user as Record<string, unknown>)?.current ?? 0),
          max: Number((data.user as Record<string, unknown>)?.max ?? 0),
        },
        assistant: {
          current: Number((data.assistant as Record<string, unknown>)?.current ?? 0),
          max: Number((data.assistant as Record<string, unknown>)?.max ?? 0),
        },
        tools: {
          current: Number((data.tools as Record<string, unknown>)?.current ?? 0),
          max: Number((data.tools as Record<string, unknown>)?.max ?? 0),
        },
        totalTurns: Number(data.totalTurns ?? 0),
        totalTokens: Number(data.totalTokens ?? 0),
        tokensSaved: Number(data.tokensSaved ?? 0),
        compressionRatio: Number(data.compressionRatio ?? 0),
      }
    }
  } catch {
    /* fall through */
  }
  return defaultContextMetrics()
}

export function ContextDialog(api: TuiApi) {
  const fg = (k: string): string =>
    ((api.theme?.current ?? {}) as Record<string, string>)[k] ?? "#ffffff"

  const metrics = readContextMetrics(api)

  return (
    <DcpFrame api={api}>
      {/* ── Header ──────────────────────────── */}
      <text fg={fg("text")} attributes={TextAttributes.BOLD} size="large">
        {"▣"} Context Analysis
      </text>
      <text fg={fg("textMuted")} size="small">
        Token usage and conversation breakdown
      </text>

      {/* ── Token Breakdown ──────────────────── */}
      <Card api={api} header="TOKEN BREAKDOWN BY SEGMENT">
        <Progress
          api={api}
          label="System"
          current={metrics.system.current}
          max={metrics.system.max}
          color={fg("accent")}
          width={24}
        />
        <Progress
          api={api}
          label="User"
          current={metrics.user.current}
          max={metrics.user.max}
          color={fg("primary")}
          width={24}
        />
        <Progress
          api={api}
          label="Assistant"
          current={metrics.assistant.current}
          max={metrics.assistant.max}
          color={fg("success")}
          width={24}
        />
        <Progress
          api={api}
          label="Tools"
          current={metrics.tools.current}
          max={metrics.tools.max}
          color={fg("warning")}
          width={24}
        />
      </Card>

      {/* ── Aggregate Metrics ────────────────── */}
      <Card api={api} header="AGGREGATE METRICS">
        <Metric api={api} label="Total Turns" value={metrics.totalTurns} />
        <Metric api={api} label="Total Tokens" value={metrics.totalTokens} />
        <Metric api={api} label="Tokens Saved" value={metrics.tokensSaved} />
        <Metric
          api={api}
          label="Compression Ratio"
          value={metrics.compressionRatio > 0 ? `${metrics.compressionRatio.toFixed(1)}x` : "—"}
        />
      </Card>

      {/* ── Recent Messages Preview ──────────── */}
      <Card api={api} header="RECENT MESSAGES">
        <PromptRow api={api} role="system" preview="System prompt / instructions" />
        <PromptRow api={api} role="user" preview="User message…" />
        <PromptRow api={api} role="assistant" preview="Assistant response…" />
        <PromptRow api={api} role="tool" preview="Tool call results…" />
      </Card>
    </DcpFrame>
  )
}

/* ─── StatsDialog ───────────────────────────
 *
 *   Pruning and compression statistics shown in
 *   two Card groups — "SESSION" for the current
 *   session data and "ALL TIME" for cumulative
 *   numbers.
 *
 *   Reads stats from a "dcpStats" theme slot, or
 *   shows placeholder values when unavailable.
 */

export function StatsDialog(api: TuiApi) {
  const fg = (k: string): string =>
    ((api.theme?.current ?? {}) as Record<string, string>)[k] ?? "#ffffff"

  /* ── Read stats from theme ────────────── */
  let stats: StatsReport | null = null
  try {
    const theme = (api.theme?.current ?? {}) as Record<string, unknown>
    const raw = theme["dcpStats"]
    if (typeof raw === "string") {
      stats = JSON.parse(raw) as StatsReport
    } else if (raw && typeof raw === "object") {
      stats = raw as StatsReport
    }
  } catch {
    /* null stay null */
  }

  const s = stats ?? {
    session: {
      totalTurns: 0,
      compressedBlocks: 0,
      decompressedBlocks: 0,
      tokensSaved: 0,
      compressionRatio: 0,
    },
    allTime: {
      totalCompressed: 0,
      totalDecompressed: 0,
      totalRecompressed: 0,
      tokensSaved: 0,
      compressionRatio: 0,
      activeBlocks: 0,
      pendingWork: 0,
    },
  }

  return (
    <DcpFrame api={api}>
      {/* ── Header ──────────────────────────── */}
      <text fg={fg("text")} attributes={TextAttributes.BOLD} size="large">
        {"▣"} Pruning Statistics
      </text>
      <text fg={fg("textMuted")} size="small">
        Compression and pruning performance data
      </text>

      {/* ── Session Stats ───────────────────── */}
      <Card api={api} header="SESSION">
        <Metric api={api} label="Total Turns" value={s.session.totalTurns} />
        <Metric
          api={api}
          label="Compressed Blocks"
          value={s.session.compressedBlocks}
        />
        <Metric
          api={api}
          label="Decompressed Blocks"
          value={s.session.decompressedBlocks}
        />
        <Metric api={api} label="Tokens Saved" value={s.session.tokensSaved} />
        <Metric
          api={api}
          label="Compression Ratio"
          value={
            s.session.compressionRatio > 0
              ? `${s.session.compressionRatio.toFixed(1)}x`
              : "—"
          }
        />
      </Card>

      {/* ── All-Time Stats ──────────────────── */}
      <Card api={api} header="ALL TIME">
        <Metric
          api={api}
          label="Total Compressed"
          value={s.allTime.totalCompressed}
        />
        <Metric
          api={api}
          label="Total Decompressed"
          value={s.allTime.totalDecompressed}
        />
        <Metric
          api={api}
          label="Total Recompressed"
          value={s.allTime.totalRecompressed}
        />
        <Metric api={api} label="Tokens Saved" value={s.allTime.tokensSaved} />
        <Metric
          api={api}
          label="Compression Ratio"
          value={
            s.allTime.compressionRatio > 0
              ? `${s.allTime.compressionRatio.toFixed(1)}x`
              : "—"
          }
        />
        <Metric
          api={api}
          label="Active Blocks"
          value={s.allTime.activeBlocks}
        />
        <Metric
          api={api}
          label="Pending Work"
          value={s.allTime.pendingWork}
        />
      </Card>
    </DcpFrame>
  )
}
