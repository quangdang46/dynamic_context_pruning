import { tool, type ToolDefinition } from "@opencode-ai/plugin"
import type { DcpPruner } from "../lib/bridge.js"

export type ToolContext = {
  pruner: DcpPruner
  /** Fetch current session messages as OpenCode {info,parts}[] JSON. */
  getMessagesJson?: () => Promise<string> | string
}

async function messagesJson(ctx: ToolContext): Promise<string> {
  if (!ctx.getMessagesJson) return "[]"
  try {
    const v = await ctx.getMessagesJson()
    return typeof v === "string" ? v : JSON.stringify(v ?? [])
  } catch {
    return "[]"
  }
}

export function createTools(ctx: ToolContext): Record<string, ToolDefinition> {
  const { pruner } = ctx

  return {
    compress: tool({
      description:
        "Replace stale conversation content with technical summaries. " +
        "Use for closed/discussed topics to free context space. " +
        "Provide one or more ranges identified by message/block ids (e.g. m0001, b2).",
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
        const msgs = await messagesJson(ctx)
        const resultJson = pruner.handleCompress(JSON.stringify(args), msgs)
        let result: any = {}
        try {
          result = JSON.parse(resultJson)
        } catch {
          return resultJson
        }
        const n =
          result.blocks?.length ??
          result.compressed_count ??
          result.compressedCount ??
          0
        return `Compressed ${n} block(s). Topic: ${args.topic || "(none)"}.`
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
