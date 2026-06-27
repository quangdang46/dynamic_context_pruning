// @ts-nocheck
/* ───────────────────────────────────────
 *   lib/commands/help.ts — DCP help message
 * ─────────────────────────────────────── */

export function formatHelpMessage(): any {
  return [
    "╭──────────────────────────────────────────────────────────────╮",
    "│                      DCP Commands                           │",
    "╰──────────────────────────────────────────────────────────────╯",
    "",
    "  /dcp                    Show this help message",
    "  /dcp context            Show context analysis (turns, blocks, tokens)",
    "  /dcp stats              Show pruning statistics",
    "  /dcp sweep              Flush pending prune strategies",
    "  /dcp manual <on|off>    Toggle manual mode",
    "  /dcp decompress <id>    Restore a compressed block",
    "  /dcp recompress <id>    Re-activate a decompressed block",
    "  /dcp-compress [focus]   Trigger manual compression",
    "",
    "  Tools (available to the LLM):",
    "    compress    Replace stale content with summaries",
    "    decompress  Restore compressed blocks",
    "    recompress  Re-activate decompressed blocks",
    "",
  ].join("\n")
}
