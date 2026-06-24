import { tool, type ToolDefinition } from "@opencode-ai/plugin"

interface DcpPruner {
  transformMessages(messagesJson: string): string
  transformSystem(system: string): string
  handleCompress(argsJson: string, messagesJson: string): string
  decompress(blockId: number): string
  recompress(blockId: number): string
  handleCommand(cmd: string, argsJson: string, messagesJson: string): string
  notifyEvent(eventJson: string): void
  hasPendingWork(): boolean
  stats(): string
  setSessionId(sessionId: string): void
}

export function createTools(pruner: DcpPruner): Record<string, ToolDefinition> {
  return {
    compress: tool({
      description:
        "Replace stale conversation content with technical summaries. " +
        "Use for closed/discussed topics to free context space.",
      args: {
        topic: tool.schema
          .string()
          .describe("Short label (3-5 words) for the batch - e.g. 'Auth Exploration'"),
        content: tool.schema
          .array(
            tool.schema.object({
              startId: tool.schema
                .string()
                .describe("Message or block ID beginning of range (e.g. m0001, b2)"),
              endId: tool.schema
                .string()
                .describe("Message or block ID end of range (e.g. m0012, b5)"),
              summary: tool.schema
                .string()
                .describe("Complete technical summary replacing all content in range"),
            }),
          )
          .describe("One or more ranges to compress"),
      },
      async execute(args) {
        const resultJson = pruner.handleCompress(JSON.stringify(args), "[]")
        const result = JSON.parse(resultJson)
        return `Compressed ${result.blocks?.length || result.compressed_count || 0} messages.`
      },
    }),

    decompress: tool({
      description: "Restore a compressed block to its original messages.",
      args: {
        blockId: tool.schema.number().describe("Block ID to restore (e.g. 1, 2, 3)"),
      },
      async execute(args) {
        pruner.decompress(args.blockId)
        return `Decompressed block ${args.blockId}.`
      },
    }),

    recompress: tool({
      description: "Re-activate a user-decompressed block for future compression.",
      args: {
        blockId: tool.schema.number().describe("Block ID to re-compress (e.g. 1, 2, 3)"),
      },
      async execute(args) {
        pruner.recompress(args.blockId)
        return `Recompressed block ${args.blockId}.`
      },
    }),
  }
}
