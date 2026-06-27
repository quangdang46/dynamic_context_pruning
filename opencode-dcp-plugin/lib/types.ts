// @ts-nocheck
/* ───────────────────────────────────────
 *   lib/types.ts — DcpPruner interface & core types
 *   Mirrors the Rust NAPI‑RS bridge binding.
 * ─────────────────────────────────────── */

/** Methods exposed by the native opencode-dcp-bridge addon. */
export interface DcpPruner {
  transformMessages(messagesJson: any): any
  transformSystem(system: any): any
  handleCompress(argsJson: any, messagesJson: any): any
  decompress(blockId: any): any
  recompress(blockId: any): any
  handleCommand(cmd: any, argsJson: any, messagesJson: any): any
  notifyEvent(eventJson: any): void
  hasPendingWork(): any
  stats(): any
  setSessionId(sessionId: any): void
}

/** Shape returned by `nativeBridge.loadDcpConfig()`. */
export interface DcpConfig {
  enabled: any
  max_turns?: any
  max_tokens?: any
  strategy?: "aggressive" | "balanced" | "conservative"
  compress?: {
    permission?: "allow" | "deny" | "prompt"
    auto_compress?: any
    min_token_savings?: any
  }
  prune?: {
    permission?: "allow" | "deny" | "prompt"
    auto_prune?: any
    min_turns_between?: any
  }
  tui?: {
    compact_view?: any
  }
  [key: any]: unknown
}

/** Result of a /dcp command. */
export interface DcpCommandResult {
  status: "ok" | "error"
  text: any
  data?: Record<string, unknown>
}

/** Single stats entry. */
export interface DcpStats {
  total_compressed: any
  total_decompressed: any
  total_recompressed: any
  tokens_saved: any
  compression_ratio: any
  active_blocks: any
  pending_work: any
  [key: any]: unknown
}
