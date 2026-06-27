// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────────
 *   lib/tui/data.ts — DCP data helpers
 *
 *   loadConfig  — read DcpConfig from the theme slot or
 *                 parse it from a JSON string stored under
 *                 api.theme.current["dcpConfig"]
 *   useTheme    — build a colour getter bound to the API
 *   loadStats   — fetch stats from the native bridge via
 *                 a theme slot (opencode bridge integration)
 * ─────────────────────────────────────────── */

import type { TuiApi } from "./types.js"
import type { DcpConfig, StatsReport } from "../types.js"

/**
 * Read the DCP configuration from the OpenCode theme slot.
 * The bridge stores the config as a JSON-in-string under the
 * key "dcpConfig" on the current theme object.
 *
 * Returns sensible defaults when the slot is unavailable or
 * the JSON cannot be parsed.
 */
export function loadConfig(api: TuiApi): DcpConfig {
  try {
    const theme = (api.theme?.current ?? {}) as Record<string, unknown>
    const raw = theme["dcpConfig"]

    if (typeof raw === "string") {
      return JSON.parse(raw) as DcpConfig
    }
    if (raw && typeof raw === "object") {
      return raw as DcpConfig
    }
  } catch {
    /* Fall through to defaults. */
  }

  return {
    enabled: true,
    max_turns: 50,
    max_tokens: 4096,
    strategy: "balanced",
    compress: { permission: "allow", auto_compress: true },
    prune: { permission: "allow", auto_prune: true },
  }
}

/**
 * Build a colour-getter function bound to the current theme.
 *
 * Usage:
 *   const fg = useTheme(api)
 *   <text fg={fg("primary")}>Hello</text>
 */
export function useTheme(api: TuiApi): (key: string) => string {
  const theme = (api.theme?.current ?? {}) as Record<string, string>
  return (key: string): string => theme[key] ?? "#ffffff"
}

/**
 * Load a stats report from a theme slot.
 *
 * The bridge can push a serialised StatsReport under
 * "dcpStats" on the current theme.  When none is available
 * we return placeholder dashes.
 */
export function loadStats(api: TuiApi): StatsReport {
  try {
    const theme = (api.theme?.current ?? {}) as Record<string, unknown>
    const raw = theme["dcpStats"]

    if (typeof raw === "string") {
      return JSON.parse(raw) as StatsReport
    }
    if (raw && typeof raw === "object") {
      return raw as StatsReport
    }
  } catch {
    /* Fall through to defaults. */
  }

  return {
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
}
