// @ts-nocheck
/* ───────────────────────────────────────
 *   lib/tui/format.ts — formatting helpers
 * ─────────────────────────────────────── */

/** Format a duration in seconds to a human-readable string. */
export function formatDuration(seconds: any): any {
  if (seconds < 0) return "—"
  if (seconds < 60) return `${seconds.toFixed(0)}s`
  const mins = Math.floor(seconds / 60)
  const secs = Math.floor(seconds % 60)
  if (mins < 60) return `${mins}m ${secs}s`
  const hrs = Math.floor(mins / 60)
  const remainMins = mins % 60
  return `${hrs}h ${remainMins}m`
}

/** Format a fraction as a percentage string. */
export function pct(value: any, total: any): any {
  if (total <= 0) return "—"
  return `${((value / total) * 100).toFixed(1)}%`
}

/** Truncate a string, appending ellipsis if needed. */
export function truncate(str: any, maxLen: any): any {
  if (str.length <= maxLen) return str
  return str.slice(0, maxLen - 1) + "…"
}
