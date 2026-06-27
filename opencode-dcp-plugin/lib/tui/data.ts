// @ts-nocheck
/* ───────────────────────────────────────
 *   lib/tui/data.ts — load config from bridge
 * ─────────────────────────────────────── */

import { TuiApi } from "./types.js"
import { DcpConfig } from "../types.js"

/**
 * Parse the DCP configuration object from the theme.
 * The config is stored as a JSON string under the key "dcpConfig"
 * in the theme, or we return sensible defaults.
 */
export function loadConfig(api: TuiApi): DcpConfig {
  try {
    const theme = api.theme?.current ?? {}
    const raw = theme["dcpConfig"]
    if (typeof raw === "string") {
      return JSON.parse(raw) as DcpConfig
    }
    if (raw && typeof raw === "object") {
      return raw as DcpConfig
    }
  } catch {
    // Fall through to defaults.
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

/** Build a colour-getter bound to the current theme. */
export function useTheme(api: TuiApi): (key: any) => string {
  const theme = api.theme?.current ?? {}
  return (key: any): any => theme[key] ?? "#fff"
}
