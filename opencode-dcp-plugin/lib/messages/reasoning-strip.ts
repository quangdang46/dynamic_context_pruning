import type { WithParts } from "../state"
import { getLastUserMessage } from "./query"

/**
 * Mirrors opencode's differentModel handling by preserving part content while
 * dropping provider metadata on assistant parts that came from a different
 * model/provider than the current turn's user message.
 */
export function stripStaleMetadata(messages: WithParts[]): void {
    const lastUserMessage = getLastUserMessage(messages)
    if (lastUserMessage?.info.role !== "user") {
        return
    }

    // v2-adapted user messages may not carry the v1 `model` block.
    const userModel = (lastUserMessage.info as any).model
    if (!userModel || typeof userModel !== "object") {
        return
    }

    const modelID = userModel.modelID
    const providerID = userModel.providerID

    messages.forEach((message) => {
        if (message.info.role !== "assistant") {
            return
        }

        if (message.info.modelID === modelID && message.info.providerID === providerID) {
            return
        }

        message.parts = message.parts.map((part) => {
            if (part.type !== "text" && part.type !== "tool" && part.type !== "reasoning") {
                return part
            }

            if (!("metadata" in part)) {
                return part
            }

            const { metadata: _metadata, ...rest } = part
            return rest
        })
    })
}
