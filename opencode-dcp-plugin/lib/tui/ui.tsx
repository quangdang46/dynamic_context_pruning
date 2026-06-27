// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────
 *   lib/tui/ui.tsx — reusable UI components
 *
 *   DcpFrame     — page-level wrapper with title bar
 *   Card         — bordered container with optional header
 *   Metric       — label / value row
 *   Progress     — horizontal progress bar
 *   StatusPill   — coloured status badge
 *   ActionRow    — clickable command entry
 *   FooterButton — dismiss / action button
 * ─────────────────────────────────────── */

import { TuiApi, Theme } from "./types.js"

/* ─── helpers ─────────────────────────── */
function useTheme(api: TuiApi): (key: any) => string {
  const t: Theme = api.theme?.current ?? {}
  return (k: any): any => t[k] ?? "#fff"
}

/* ─── DcpFrame ────────────────────────── */

interface FrameProps {
  api: TuiApi
  children: any
  title?: any
}

export function DcpFrame(props: FrameProps) {
  const fg = useTheme(props.api)
  return (
    <box
      width="100%"
      height="100%"
      flexDirection="column"
      padding={1}
      gap={1}
      backgroundColor={fg("backgroundElement")}
    >
      {props.title !== undefined && (
        <text fg={fg("text")} attributes={{ bold: true }} size="large">
          {props.title}
        </text>
      )}
      {props.children}
    </box>
  )
}

/* ─── Card ──────────────────────────── */

interface CardProps {
  api: TuiApi
  children: any
  header?: any
}

export function Card(props: CardProps) {
  const fg = useTheme(props.api)
  return (
    <box
      flexDirection="column"
      gap={1}
      padding={1}
      border={{ type: "round" }}
      backgroundColor={fg("backgroundElement")}
    >
      {props.header !== undefined && (
        <text fg={fg("textMuted")} size="xsmall" attributes={{ bold: true }}>
          {props.header}
        </text>
      )}
      {props.children}
    </box>
  )
}

/* ─── Metric ──────────────────────────── */

interface MetricProps {
  api: TuiApi
  label: any
  value: any | number
}

export function Metric(props: MetricProps) {
  const fg = useTheme(props.api)
  return (
    <box justifyContent="spaceBetween" width="100%">
      <text fg={fg("textMuted")} size="small">
        {props.label}
      </text>
      <text fg={fg("text")} attributes={{ bold: true }}>
        {String(props.value)}
      </text>
    </box>
  )
}

/* ─── Progress ────────────────────────── */

interface ProgressProps {
  api: TuiApi
  current: any
  max: any
  label?: any
  color?: any
}

export function Progress(props: ProgressProps) {
  const fg = useTheme(props.api)
  const pct = props.max > 0 ? Math.min(props.current / props.max, 1) : 0
  const barW = 20
  const filled = Math.round(pct * barW)
  const empty = barW - filled
  const barColor = props.color ?? fg("primary")

  return (
    <box gap={1} alignItems="center">
      {props.label !== undefined && (
        <text fg={fg("textMuted")} size="xsmall">
          {props.label}
        </text>
      )}
      <text fg={barColor}>
        {"█".repeat(filled)}
        {"░".repeat(empty)}
      </text>
      <text fg={fg("textMuted")} size="xsmall">
        {props.max > 0 ? `${(pct * 100).toFixed(0)}%` : "—"}
      </text>
    </box>
  )
}

/* ─── StatusPill ───────────────────────── */

interface StatusPillProps {
  api: TuiApi
  status: "ok" | "warn" | "error" | "info" | "disabled"
  label?: any
}

const STATUS_COLORS: Record<string, string> = {
  ok: "success",
  warn: "warning",
  error: "error",
  info: "info",
  disabled: "textMuted",
}

export function StatusPill(props: StatusPillProps) {
  const fg = useTheme(props.api)
  const key = STATUS_COLORS[props.status] ?? "textMuted"
  const color = fg(key)
  const display = props.label ?? props.status.toUpperCase()
  return (
    <box
      paddingX={1}
      border={{ type: "round" }}
      borderColor={color}
    >
      <text fg={color} size="xsmall" attributes={{ bold: true }}>
        {display}
      </text>
    </box>
  )
}

/* ─── ActionRow ────────────────────────── */

interface ActionRowProps {
  api: TuiApi
  command: any
  description: any
  onClick?: () => void | Promise<void>
}

export function ActionRow(props: ActionRowProps) {
  const fg = useTheme(props.api)
  return (
    <box
      flexDirection="column"
      padding={1}
      onMouseUp={props.onClick}
      backgroundColor={fg("backgroundElement")}
    >
      <text fg={fg("primary")} attributes={{ bold: true }}>
        {props.command}
      </text>
      <text fg={fg("textMuted")} size="small">
        {props.description}
      </text>
    </box>
  )
}

/* ─── FooterButton ─────────────────────── */

interface FooterButtonProps {
  api: TuiApi
  label: any
  variant?: "muted" | "primary"
  onClick?: () => void | Promise<void>
}

export function FooterButton(props: FooterButtonProps) {
  const fg = useTheme(props.api)
  const accent =
    props.variant === "primary" ? fg("primary") : fg("textMuted")
  const handle = props.onClick ?? (() => props.api.ui.dialog.close())

  return (
    <box
      paddingX={2}
      paddingY={1}
      border={{ type: "round" }}
      borderColor={accent}
      onMouseUp={handle}
    >
      <text fg={accent}>{props.label}</text>
    </box>
  )
}
