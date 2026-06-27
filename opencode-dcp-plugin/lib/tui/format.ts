// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────────
 *   lib/tui/format.ts — formatting helpers
 *
 *   formatDuration  — seconds → human string
 *   formatRatio     — value / total → "N.m x"
 *   pct             — value / total → "XX.X%"
 *   truncate        — string with ellipsis
 * ─────────────────────────────────────────── */

/** Format a duration in seconds to a human-readable string. */
export function formatDuration(seconds: number): string {
  if (seconds < 0) return "—"
  if (seconds < 60) return `${seconds.toFixed(0)}s`
  const mins = Math.floor(seconds / 60)
  const secs = Math.floor(seconds % 60)
  if (mins < 60) return `${mins}m ${secs}s`
  const hrs = Math.floor(mins / 60)
  const remainMins = mins % 60
  return `${hrs}h ${remainMins}m`
}

/** Format a fraction as a "N.m x" ratio string. */
export function formatRatio(value: number, total: number): string {
  if (total <= 0) return "—"
  return `${(value / total).toFixed(1)}x`
}

/** Format a fraction as a "XX.X %" percentage string. */
export function pct(value: number, total: number): string {
  if (total <= 0) return "—"
  return `${((value / total) * 100).toFixed(1)} %`
}

/** Truncate a string, appending an ellipsis if it exceeds maxLen. */
export function truncate(str: string, maxLen: number): string {
  if (str.length <= maxLen) return str
  return str.slice(0, maxLen - 1) + "…"
}
