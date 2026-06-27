// @ts-nocheck
/* ───────────────────────────────────────
 *   lib/notification.ts — sendIgnoredMessage
 *   Sends an ignored (silent) message to a session via
 *   the OpenCode SDK client.session.prompt().
 * ─────────────────────────────────────── */

/** Send an ignored message that won't appear in the visible chat history. */
export async function sendIgnoredMessage(
  client: any,
  sessionID: any,
  text: any,
): Promise<void> {
  try {
    await client.session.prompt({
      path: { id: sessionID },
      body: {
        noReply: true,
        parts: [{ type: "text", text, ignored: true }],
      },
    })
  } catch {
    // session.prompt may not be available in all OpenCode versions.
  }
}
