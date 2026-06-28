// @ts-nocheck
export async function sendIgnoredMessage(client, sessionID, text, params, logger) {
    try {
        await client.session.prompt({
            path: { id: sessionID },
            body: {
                noReply: true,
                parts: [{ type: "text", text, ignored: true }],
            },
        })
    } catch (error) {
        if (logger) logger.error("Failed to send notification", { error: error.message })
    }
}

export async function sendUnifiedNotification(client, logger, config, state, sessionId,
    pruneToolIds, toolMetadata, reason, params, workingDirectory) {
    if (config.notification === "off") return false
    const message = "DCP: " + (pruneToolIds.length > 0 ? "Compressed " + pruneToolIds.length + " items" : "No items to compress")
    await sendIgnoredMessage(client, sessionId, message, params, logger)
    return true
}
