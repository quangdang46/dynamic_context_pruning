// @ts-nocheck
/* @jsxImportSource @opentui/solid */
/* ───────────────────────────────────────────
 *   lib/tui/types.ts — TUI type definitions
 *
 *   TuiApi        — minimal OpenCode TUI API
 *   Theme         — color palette
 *   ThemeColor    — color getter signature
 *   DcpCommand    — command descriptor
 *   StatsReport   — session + all-time stats
 *   ProgressData  — progress bar input
 * ─────────────────────────────────────────── */

/** Minimal OpenCode TUI API that our dialogs rely on. */
export interface TuiApi {
  theme?: {
    current: Record<string, string>
  }
  ui: {
    dialog: {
      replace(element: unknown): Promise<void>
      close(): void
    }
    notification?: {
      show(message: string): void
    }
  }
  command?: {
    register(cmd: DcpCommandPaletteItem): void
    unregister?(id: string): void
  }
  keymap?: {
    registerLayer(layer: KeymapLayer): void
    unregisterLayer?(namespace: string): void
  }
  /** Allow loose access for extra properties (e.g. "dcpConfig" on theme). */
  [key: string]: unknown
}

/** Theme colour palette, matching the shape of OpenCode's default theme. */
export interface Theme {
  primary?: string
  primaryMuted?: string
  text?: string
  textMuted?: string
  background?: string
  backgroundElement?: string
  backgroundHover?: string
  border?: string
  borderSubtle?: string
  accent?: string
  success?: string
  warning?: string
  error?: string
  info?: string
  [key: string]: unknown
}

/** Colour getter — bound to a theme, returns a hex string for a key. */
export interface ThemeColor {
  (key: string): string
}

/** Palette command descriptor. */
export interface DcpCommand {
  command: string
  description: string
  category?: string
  onClick?: () => void | Promise<void>
}

/** Item fed to `api.command.register()`. */
export interface DcpCommandPaletteItem {
  title: string
  value: string
  description: string
  category: string
  slash?: { name: string }
  onSelect: () => Promise<void>
}

/** Keymap layer for the newer keymap.registerLayer API. */
export interface KeymapLayer {
  namespace: string
  commands: KeymapCommand[]
}

/** Single keymap-registered command. */
export interface KeymapCommand {
  name: string
  title: string
  description: string
  category: string
  slashName: string
  run: () => Promise<void>
}

/** Session and all-time statistics report. */
export interface StatsReport {
  session: {
    totalTurns: number
    compressedBlocks: number
    decompressedBlocks: number
    tokensSaved: number
    compressionRatio: number
  }
  allTime: {
    totalCompressed: number
    totalDecompressed: number
    totalRecompressed: number
    tokensSaved: number
    compressionRatio: number
    activeBlocks: number
    pendingWork: number
  }
}

/** Input data for a Progress bar segment. */
export interface ProgressData {
  label: string
  current: number
  max: number
  color?: string
}
