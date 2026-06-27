// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────────
 *   lib/tui/ui.tsx — reusable TUI components
 *
 *   DcpFrame     — full-screen wrapper with eyebrow
 *                  title bar, optional esc-to-close
 *                  and back-button footer
 *   Card         — bordered container w/ header
 *   Metric       — label / value row
 *   Progress     — ██░░░ bar
 *   PromptRow    — single-prompt display row
 *   StatusPill   — coloured status badge
 *   ActionRow    — clickable command entry
 *   FooterButton — dismiss / action button
 * ─────────────────────────────────────────── */

import { TextAttributes } from "@opentui/core"
import type { TuiApi, ThemeColor } from "./types.js"

/* ─── helpers ─────────────────────────────── */

function useTheme(api: TuiApi): ThemeColor {
  const t: Record<string, string> = (api.theme?.current ?? {}) as Record<string, string>
  return (k: string): string => t[k] ?? "#ffffff"
}

/* ─── DcpFrame ──────────────────────────────
 *
 *   Page-level wrapper.  Renders:
 *     • an eyebrow bar ("DCP · Dynamic Context Pruning")
 *     • a title line (optional, shown when `title` provided)
 *     • children
 *     • a footer row with optional "Close" / "Back" buttons
 *
 *   Props:
 *     api       — TuiApi
 *     title     — optional title shown below the eyebrow
 *     onClose   — click handler for the close button (defaults to dialog.close)
 *     children  — body content
 */

interface FrameProps {
  api: TuiApi
  children?: unknown
  title?: string
  onClose?: () => void | Promise<void>
}

export function DcpFrame(props: FrameProps) {
  const fg = useTheme(props.api)
  const handleClose =
    props.onClose ??
    (() => {
      try {
        props.api.ui.dialog.close()
      } catch {
        /* silent */
      }
    })

  return (
    <box
      width="100%"
      height="100%"
      flexDirection="column"
      padding={1}
      gap={1}
      backgroundColor={fg("backgroundElement")}
    >
      {/* ── Eyebrow ─────────────────────────────── */}
      <box border={["bottom"]} borderColor={fg("borderSubtle")} paddingY={1}>
        <text fg={fg("primary")} attributes={TextAttributes.BOLD} size="small">
          DCP · Dynamic Context Pruning
        </text>
      </box>

      {/* ── Optional Title ──────────────────────── */}
      {props.title !== undefined && props.title !== null && (
        <text
          fg={fg("text")}
          attributes={TextAttributes.BOLD}
          size="large"
        >
          {String(props.title)}
        </text>
      )}

      {/* ── Body ─────────────────────────────────── */}
      {props.children}

      {/* ── Spacer ───────────────────────────────── */}
      <box flexGrow={1} />

      {/* ── Footer ──────────────────────────────── */}
      <box justifyContent="flexEnd" gap={1} paddingTop={1} border={["top"]} borderColor={fg("borderSubtle")}>
        <text fg={fg("textMuted")} size="xsmall">
          Esc to close
        </text>
        <FooterButton api={props.api} label="Close" variant="primary" onClick={handleClose} />
      </box>
    </box>
  )
}

/* ─── Card ─────────────────────────────────
 *
 *   Bordered container with an optional header line.
 */

interface CardProps {
  api: TuiApi
  children?: unknown
  header?: string
}

export function Card(props: CardProps) {
  const fg = useTheme(props.api)
  return (
    <box
      flexDirection="column"
      gap={1}
      padding={1}
      border={["left"]}
      borderColor={fg("borderSubtle")}
      backgroundColor={fg("backgroundElement")}
    >
      {props.header !== undefined && props.header !== null && (
        <text fg={fg("textMuted")} size="xsmall" attributes={TextAttributes.BOLD}>
          {String(props.header)}
        </text>
      )}
      {props.children}
    </box>
  )
}

/* ─── Metric ────────────────────────────────
 *
 *   A label / value row, left-aligned label with
 *   right-aligned bold value.
 */

interface MetricProps {
  api: TuiApi
  label: string
  value: string | number
}

export function Metric(props: MetricProps) {
  const fg = useTheme(props.api)
  return (
    <box justifyContent="spaceBetween" width="100%">
      <text fg={fg("textMuted")} size="small">
        {props.label}
      </text>
      <text fg={fg("text")} attributes={TextAttributes.BOLD}>
        {String(props.value)}
      </text>
    </box>
  )
}

/* ─── Progress ──────────────────────────────
 *
 *   Horizontal progress bar using filled (█) and
 *   empty (░) block characters.  Bar width is 20
 *   characters by default.
 */

interface ProgressProps {
  api: TuiApi
  current: number
  max: number
  label?: string
  color?: string
  width?: number
}

export function Progress(props: ProgressProps) {
  const fg = useTheme(props.api)
  const pctValue = props.max > 0 ? Math.min(props.current / props.max, 1) : 0
  const barW = props.width ?? 20
  const filled = Math.round(pctValue * barW)
  const empty = barW - filled
  const barColor = props.color ?? fg("primary")

  return (
    <box gap={1} alignItems="center">
      {props.label !== undefined && props.label !== null && (
        <text fg={fg("textMuted")} size="xsmall">
          {props.label}
        </text>
      )}
      <text fg={barColor}>
        {"█".repeat(filled)}
        {"░".repeat(empty)}
      </text>
      <text fg={fg("textMuted")} size="xsmall">
        {props.max > 0 ? `${(pctValue * 100).toFixed(0)}%` : "—"}
      </text>
    </box>
  )
}

/* ─── PromptRow ─────────────────────────────
 *
 *   Displays a single prompt / message preview.
 */

interface PromptRowProps {
  api: TuiApi
  role: string
  preview: string
  tokens?: number
  color?: string
}

export function PromptRow(props: PromptRowProps) {
  const fg = useTheme(props.api)
  const roleColor = props.color ?? fg("textMuted")

  return (
    <box justifyContent="spaceBetween" width="100%" paddingX={1}>
      <box gap={1}>
        <text fg={roleColor} attributes={TextAttributes.BOLD} size="xsmall">
          {props.role}
        </text>
        <text fg={fg("text")} size="small" truncate>
          {props.preview}
        </text>
      </box>
      {props.tokens !== undefined && (
        <text fg={fg("textMuted")} size="xsmall">
          {props.tokens} tok
        </text>
      )}
    </box>
  )
}

/* ─── StatusPill ────────────────────────────
 *
 *   A small coloured badge indicating status.
 */

interface StatusPillProps {
  api: TuiApi
  status: "ok" | "warn" | "error" | "info" | "disabled"
  label?: string
}

const STATUS_COLORS: Record<string, string> = {
  ok: "success",
  warn: "warning",
  error: "error",
  info: "primary",
  disabled: "textMuted",
}

export function StatusPill(props: StatusPillProps) {
  const fg = useTheme(props.api)
  const key = STATUS_COLORS[props.status] ?? "textMuted"
  const color = fg(key)
  const display = props.label ?? props.status.toUpperCase()

  return (
    <box paddingX={1} border={["left"]} borderColor={color}>
      <text fg={color} size="xsmall" attributes={TextAttributes.BOLD}>
        {display}
      </text>
    </box>
  )
}

/* ─── ActionRow ─────────────────────────────
 *
 *   A clickable command entry showing a command
 *   name and its description.
 */

interface ActionRowProps {
  api: TuiApi
  command: string
  description: string
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
      <text fg={fg("primary")} attributes={TextAttributes.BOLD}>
        {props.command}
      </text>
      <text fg={fg("textMuted")} size="small">
        {props.description}
      </text>
    </box>
  )
}

/* ─── FooterButton ──────────────────────────
 *
 *   A clickable label used in footers.
 *   Variant "primary" uses the primary colour,
 *   "muted" uses textMuted.
 */

interface FooterButtonProps {
  api: TuiApi
  label: string
  variant?: "muted" | "primary" | "danger"
  onClick?: () => void | Promise<void>
}

export function FooterButton(props: FooterButtonProps) {
  const fg = useTheme(props.api)
  const accentMap: Record<string, string> = {
    muted: fg("textMuted"),
    primary: fg("primary"),
    danger: fg("error"),
  }
  const accent = accentMap[props.variant ?? "muted"] ?? fg("textMuted")
  const handle = props.onClick ?? (() => props.api.ui.dialog.close())

  return (
    <box
      paddingX={2}
      paddingY={1}
      border={["left"]}
      borderColor={accent}
      onMouseUp={handle}
    >
      <text fg={accent} attributes={TextAttributes.BOLD}>
        {props.label}
      </text>
    </box>
  )
}
