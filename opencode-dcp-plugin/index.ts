import type { Plugin } from "@opencode-ai/plugin"
import { getConfig } from "./lib/config"
import { RANGE_FORMAT_EXTENSION } from "./lib/prompts/extensions/tool"
import {
    applyV1MutationsToV2,
    fromV2Messages,
} from "./lib/v2/adapter"
import { ensureSessionInitialized } from "./lib/state"
import { createCompressMessageTool, createCompressRangeTool } from "./lib/compress"
import {
    compressDisabledByOpencode,
    hasExplicitToolPermission,
    type HostPermissionSnapshot,
} from "./lib/host-permissions"
import { Logger } from "./lib/logger"
import { createSessionState } from "./lib/state"
import { PromptStore } from "./lib/prompts/store"
import {
    createChatMessageTransformHandler,
    createCommandExecuteHandler,
    createEventHandler,
    createSystemPromptHandler,
    createTextCompleteHandler,
} from "./lib/hooks"
import { configureClientAuth, isSecureMode } from "./lib/auth"
import { startAutoUpdate } from "./lib/update"

const server: Plugin = (async (ctx) => {
    const config = getConfig(ctx)

    if (!config.enabled) {
        return {}
    }

    const logger = new Logger(config.debug)
    const state = createSessionState()
    const prompts = new PromptStore(logger, ctx.directory, config.experimental.customPrompts)
    const hostPermissions: HostPermissionSnapshot = {
        global: undefined,
        agents: {},
    }

    if (isSecureMode()) {
        configureClientAuth(ctx.client)
        // logger.info("Secure mode detected, configured client authentication")
    }

    logger.info("DCP initialized", {
        strategies: config.strategies,
    })

    startAutoUpdate(ctx, config.autoUpdate)

    const compressToolContext = {
        client: ctx.client,
        state,
        logger,
        config,
        prompts,
    }

    return {
        "experimental.chat.system.transform": createSystemPromptHandler(
            state,
            logger,
            config,
            prompts,
        ),
        "experimental.chat.messages.transform": createChatMessageTransformHandler(
            ctx.client,
            state,
            logger,
            config,
            prompts,
            hostPermissions,
        ) as any,
        "experimental.text.complete": createTextCompleteHandler(),
        "command.execute.before": createCommandExecuteHandler(
            ctx.client,
            state,
            logger,
            config,
            ctx.directory,
            hostPermissions,
        ),
        event: createEventHandler(state, logger),
        tool: {
            ...(config.compress.permission !== "deny" && {
                compress:
                    config.compress.mode === "message"
                        ? createCompressMessageTool(compressToolContext)
                        : createCompressRangeTool(compressToolContext),
            }),
        },
        config: async (opencodeConfig) => {
            if (
                config.compress.permission !== "deny" &&
                compressDisabledByOpencode(opencodeConfig.permission)
            ) {
                config.compress.permission = "deny"
            }

            if (config.commands.enabled && config.compress.permission !== "deny") {
                opencodeConfig.command ??= {}
                opencodeConfig.command["dcp-compress"] = {
                    template: "",
                    description: "Trigger DCP manual compression with: /dcp-compress [focus]",
                }
            }

            const toolsToAdd: string[] = []
            if (config.compress.permission !== "deny" && !config.experimental.allowSubAgents) {
                toolsToAdd.push("compress")
            }

            if (toolsToAdd.length > 0) {
                const existingPrimaryTools = opencodeConfig.experimental?.primary_tools ?? []
                opencodeConfig.experimental = {
                    ...opencodeConfig.experimental,
                    primary_tools: [...existingPrimaryTools, ...toolsToAdd],
                }
            }

            if (!hasExplicitToolPermission(opencodeConfig.permission, "compress")) {
                const permission = opencodeConfig.permission ?? {}
                opencodeConfig.permission = {
                    ...permission,
                    compress: config.compress.permission,
                } as typeof permission
            }

            hostPermissions.global = opencodeConfig.permission
            hostPermissions.agents = Object.fromEntries(
                Object.entries(opencodeConfig.agent ?? {}).map(([name, agent]) => [
                    name,
                    agent?.permission,
                ]),
            )
        },
    }
}) satisfies Plugin

/**
 * Dual-target export:
 * - OpenCode 1.x: `{ id, server }` V1 PluginModule shape.
 * - OpenCode 2.x (opencode2): requires `{ id, setup }` or `{ id, effect }`.
 *
 * The V2 `setup` registers the same DCP behavior through the V2 hook API.
 * The context hook adapts live V2 messages onto the internal V1 shape, runs
 * the exact same pipeline handlers as `server`, then writes mutations back
 * into the live objects before model dispatch.
 */
const pluginModule = {
    id: "dcp-rust",
    server,
    setup: async (ctx: any) => {
        const config = getConfig(ctx)
        if (!config.enabled) return

        const logger = new Logger(config.debug)
        const state = createSessionState()
        const prompts = new PromptStore(
            logger,
            ctx.directory ?? ".",
            config.experimental.customPrompts,
        )

        prompts.reload()
        const runtimePrompts = prompts.getRuntimePrompts()
        const compressDescription =
            runtimePrompts.compressRange + RANGE_FORMAT_EXTENSION

        // Per-dispatch pipeline runner: system prompt injection + full message
        // pipeline on the adapted shape, then write-back into live objects.
        await ctx.session.hook("context", (event: any) => {
            const { sessionID, system: systemParts, messages: v2Messages } = event
            if (!sessionID || !Array.isArray(v2Messages)) return

            void ensureSessionInitialized(
                ctx.client ?? ctx.session,
                state,
                sessionID,
                logger,
                [],
                config.manualMode.enabled,
            )

            const { messages, sources } = fromV2Messages(v2Messages)
            if (messages.length === 0) return

            const systemHandler = createSystemPromptHandler(state, logger, config, prompts)
            const systemOutput = { system: systemParts.map((p: any) => p.text) }
            void systemHandler(
                { sessionID, model: { limit: { context: 0 } } } as any,
                systemOutput,
            )
            for (let i = 0; i < systemOutput.system.length; i++) {
                if (systemParts[i]) {
                    systemParts[i].text = systemOutput.system[i]
                } else {
                    systemParts.push({ type: "text", text: systemOutput.system[i] })
                }
            }

            const hostPermissions: HostPermissionSnapshot = {
                global: undefined,
                agents: {},
            }
            const messageHandler = createChatMessageTransformHandler(
                ctx.client ?? ctx.session,
                state,
                logger,
                config,
                prompts,
                hostPermissions,
            ) as any
            void messageHandler({}, { messages })

            applyV1MutationsToV2(messages, sources)
        })

        if (config.compress.permission !== "deny") {
            await ctx.tool.transform((tools: any) => {
                const toolCtxBase = {
                    client: undefined,
                    state,
                    logger,
                    config,
                    prompts,
                }
                const compressTool =
                    config.compress.mode === "message"
                        ? createCompressMessageTool(toolCtxBase)
                        : createCompressRangeTool(toolCtxBase)

                tools.add({
                    name: "compress",
                    description: compressDescription,
                    input: {
                        type: "object",
                        properties: {
                            topic: {
                                type: "string",
                                description:
                                    "Short label (3-5 words) for display - e.g., 'Auth System Exploration'",
                            },
                            content: {
                                type: "array",
                                description:
                                    "Ranges or messages to compress with a technical summary each",
                                items: {
                                    type: "object",
                                    properties: {
                                        startId: {
                                            type: "string",
                                            description:
                                                "Message/block ID at range start (e.g. m0001, b2)",
                                        },
                                        endId: {
                                            type: "string",
                                            description:
                                                "Message/block ID at range end (e.g. m0012, b5)",
                                        },
                                        summary: {
                                            type: "string",
                                            description:
                                                "Complete technical summary replacing all content in range",
                                        },
                                    },
                                    required: ["startId", "endId", "summary"],
                                    additionalProperties: false,
                                },
                            },
                        },
                        required: ["topic", "content"],
                        additionalProperties: false,
                    },
                    execute: async (input: any, toolCtx: any) => {
                        const result = await (compressTool as any).execute(input, {
                            sessionID: toolCtx.sessionID,
                            messageID: toolCtx.messageID,
                            callID: toolCtx.id,
                            agent: toolCtx.agent,
                            ask: async () => {},
                            metadata: () => {},
                        })
                        return typeof result === "string" ? { content: result } : result
                    },
                })
            })
        }
    },
}

export default pluginModule
export { server }
