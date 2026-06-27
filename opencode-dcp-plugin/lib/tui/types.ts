// @ts-nocheck
/* ───────────────────────────────────────
 *   lib/tui/types.ts — TUI type definitions
 * ─────────────────────────────────────── */

/** Minimal shape of the TUI API object provided by OpenCode. */
export interface TuiApi {
  theme?: {
    current: Theme
  }
  ui: {
    dialog: {
      replace(element: any): Promise<void>
      close(): void
    }
  }
  command?: {
    register(cmd: DcpCommandPaletteItem): void
  }
  keymap?: {
    registerLayer(layer: KeymapLayer): void
  }
  [key: any]: any
}

/** Theme colour palette (shaped like OpenCode's default theme). */
export interface Theme {
  background?: any
  backgroundElement?: any
  backgroundHover?: any
  border?: any
  primary?: any
  primaryMuted?: any
  text?: any
  textMuted?: any
  success?: any
  warning?: any
  error?: any
  info?: any
  [key: any]: any | undefined
}

/** A DCP command shown in the palette. */
export interface DcpCommand {
  command: any
  description: any
}

/** Item fed to api.command.register(). */
export interface DcpCommandPaletteItem {
  title: any
  value: any
  description: any
  category: any
  slash?: { name: any }
  onSelect: () => Promise<void>
}

/** Keymap layer for the newer API. */
export interface KeymapLayer {
  namespace: any
  commands: KeymapCommand[]
}

/** Single keymap command. */
export interface KeymapCommand {
  name: any
  title: any
  description: any
  category: any
  slashName: any
  run: () => Promise<void>
}
