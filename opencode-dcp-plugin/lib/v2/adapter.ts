/**
 * OpenCode V2 message-shape adapter.
 *
 * Bridges the opencode2 session surface (the `session.hook("context")`
 * payload: `Message[]` from @opencode-ai/ai with `text`/`tool-call`/
 * `tool-result` content parts) onto the plugin's internal V1 `WithParts`
 * shape (`{info, parts}`, tool parts keyed by `callID`, output under
 * `state.output`). All DCP logic keeps running on the V1 shape; only this
 * layer knows both.
 *
 * Mutations made on the adapted V1 objects (pruned outputs, injected ids,
 * nudges) are written back to the live V2 message objects via
 * `applyV1MutationsToV2`.
 */
import type { WithParts } from "../state"

/** A live V2 message from the context hook (@opencode-ai/ai Message). */
export interface V2LiveMessage {
    id?: string
    role: "system" | "user" | "assistant" | "tool"
    content: Array<Record<string, any>>
    time?: { created?: number }
    [key: string]: unknown
}

function toolResultToText(result: any): string {
    if (!result || typeof result !== "object") return ""
    if (result.type === "json" && result.value !== undefined) {
        return typeof result.value === "string" ? result.value : JSON.stringify(result.value)
    }
    if (result.type === "text") return String(result.value ?? "")
    if (result.type === "error") return `Error: ${String(result.value ?? "unknown")}`
    return ""
}

/**
 * Convert a V2 live message list into V1 WithParts entries plus the parallel
 * `sources` array used by write-back to locate the original V2 parts.
 *
 * Mapping notes:
 * - assistant `tool-call` + following same-id `tool-result` merge into one V1
 *   tool part keyed by callID = tool-call.id; prune replaces
 *   `state.output`, which write-back turns into a replaced `tool-result`.
 * - user messages become text parts; `metadata.ignored` round-trips.
 * - system/tool-role messages become ignored user turns so anchors and turn
 *   counting skip them.
 */
export function fromV2Messages(v2Messages: V2LiveMessage[]): {
    messages: WithParts[]
    sources: V2LiveMessage[]
} {
    const sources: V2LiveMessage[] = []
    const messages: WithParts[] = []

    for (const msg of v2Messages) {
        sources.push(msg)

        if (msg.role === "assistant") {
            const resultsById = new Map<string, any>()
            for (const part of msg.content ?? []) {
                if (part.type === "tool-result") {
                    resultsById.set(part.id, part)
                }
            }

            // step-start first: V1 turn counting keys off it.
            const parts: Record<string, any>[] = [{ type: "step-start" }]

            for (const part of msg.content ?? []) {
                if (part.type === "text") {
                    parts.push({ type: "text", text: part.text })
                } else if (part.type === "reasoning") {
                    parts.push({ type: "reasoning", text: part.text })
                } else if (part.type === "tool-call") {
                    const result = resultsById.get(part.id)
                    const status = result ? "completed" : "running"
                    const state: Record<string, any> = { status }
                    state.input = part.input
                    if (result) {
                        const value = result.result?.value
                        state.output =
                            typeof value === "string"
                                ? value
                                : value !== undefined
                                  ? JSON.stringify(value)
                                  : ""
                        state.metadata = result.metadata
                    } else if (msg.content.some((p) => p.type === "tool-result" && p.id === part.id)) {
                        state.status = "error"
                    }
                    parts.push({
                        type: "tool",
                        callID: part.id,
                        tool: part.name,
                        state,
                        metadata: {},
                        sourceCall: part,
                        sourceResult: result,
                    })
                }
                // other v2 part kinds have no V1 counterpart — skipped.
            }

            messages.push({
                info: {
                    id: msg.id,
                    sessionID: "",
                    role: "assistant",
                    time: { created: msg.time?.created ?? Date.now() },
                } as WithParts["info"],
                parts: parts as any[],
            })
        } else if (msg.role === "user") {
            const ignored = (msg as any).metadata?.ignored === true
            const texts = (msg.content ?? []).filter((p) => p.type === "text")
            messages.push({
                info: {
                    id: msg.id,
                    sessionID: "",
                    role: "user",
                    time: { created: msg.time?.created ?? Date.now() },
                } as WithParts["info"],
                parts: texts.map((p) => ({
                    type: "text",
                    text: p.text,
                    ignored,
                    sourcePart: p,
                })) as any[],
            })
        } else {
            // system / tool role → ignored user turns so strategy anchors and
            // turn counting skip them.
            const firstText =
                (msg.content ?? []).find((p) => p.type === "text")?.text ?? ""
            messages.push({
                info: {
                    id: msg.id,
                    sessionID: "",
                    role: "user",
                    time: { created: msg.time?.created ?? Date.now() },
                } as WithParts["info"],
                parts: [
                    { type: "text", text: firstText, ignored: true },
                ] as any[],
            })
        }
    }

    return { messages, sources }
}

/**
 * Write mutations back into the live V2 messages:
 * - pruned/replaced tool outputs become replaced `tool-result` values
 * - pruned inputs become replaced `tool-call.input`
 * - injected DCP text parts (nudges, block placeholders) become new v2 text
 *   parts appended after their anchor part
 * - ignored flag round-trips through user message metadata
 */
export function applyV1MutationsToV2(adapted: WithParts[], sources: V2LiveMessage[]): void {
    for (let i = 0; i < adapted.length && i < sources.length; i++) {
        const v1 = adapted[i]
        const v2 = sources[i]
        if (!v1 || !v2 || v1.info.id !== v2.id) continue

        for (const part of v1.parts as Record<string, any>[]) {
            if (part.type === "tool") {
                const call = part.sourceCall as any
                const result = part.sourceResult as any
                if (!call) continue

                // input pruning
                if (
                    call.input !== undefined &&
                    part.state?.input !== undefined &&
                    part.state.input !== call.input &&
                    JSON.stringify(call.input) !== JSON.stringify(part.state.input)
                ) {
                    call.input = part.state.input
                }

                // output pruning → replace tool-result value
                if (
                    result &&
                    typeof part.state?.output === "string" &&
                    part.state.output !== toolResultToText(result.result)
                ) {
                    result.result = { type: "text", value: part.state.output }
                }
            } else if (part.type === "text" && part.__dcpInjected === true) {
                // Injected parts (nudges, ids, manual-compress prompts): append
                // as new text content right after the last existing part.
                v2.content.push({
                    type: "text",
                    text: part.text,
                    ...(part.metadata ? { metadata: part.metadata } : {}),
                })
            } else if (part.type === "text" && part.sourcePart) {
                // In-place edits of existing user text.
                if (part.sourcePart.text !== part.text) {
                    part.sourcePart.text = part.text
                }
                if (part.ignored) {
                    if (v2.metadata === undefined) (v2 as any).metadata = {}
                    ;(v2.metadata as any).ignored = true
                }
            }
        }
    }
}

/**
 * Mark an adapted part as DCP-injected so write-back appends it to the live
 * V2 message instead of trying to match an existing source part.
 */
export function markInjected(part: Record<string, any>): void {
    part.__dcpInjected = true
}
