/**
 * Minimal ambient declaration for `@opencode-ai/ai`, which ships inside the
 * opencode2 runtime (no npm types published). Only what the plugin uses.
 */
declare module "@opencode-ai/ai" {
    export interface SystemPart {
        type: "text"
        text: string
        cache?: unknown
        metadata?: Record<string, unknown>
    }
    export const SystemPart: {
        make(text: string): SystemPart
        content(
            input?: string | SystemPart | ReadonlyArray<SystemPart>,
        ): unknown[]
    }
}
