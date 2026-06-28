// @ts-nocheck
export function formatHelpMessage(state, config) {
    const commands = [
        ["/dcp help", "Show this help message"],
        ["/dcp context", "Show token usage breakdown"],
        ["/dcp stats", "Show pruning statistics"],
        ["/dcp sweep", "Flush pending prune strategies"],
        ["/dcp manual <on|off>", "Toggle manual mode"],
        ["/dcp decompress <id>", "Restore a compressed block"],
        ["/dcp recompress <id>", "Re-activate a decompressed block"],
        ["/dcp-compress [focus]", "Trigger manual compression"],
    ]
    const colWidth = Math.max(...commands.map(([cmd]) => cmd.length)) + 4
    const lines = []
    lines.push("╭─────────────────────────────────────────────────────────────────────────╮")
    lines.push("│                              DCP Commands                               │")
    lines.push("╰─────────────────────────────────────────────────────────────────────────╯")
    lines.push("")
    lines.push("  Manual mode: " + (state?.manualMode ? "ON" : "OFF"))
    lines.push("")
    for (const [cmd, desc] of commands) {
        lines.push("  " + cmd.padEnd(colWidth) + desc)
    }
    return lines.join("\n")
}
