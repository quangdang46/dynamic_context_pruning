import type { Plugin } from "@opencode-ai/plugin"
import { getConfig } from "./lib/config"
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
 * The 2.x `setup` registers the same DCP behavior through the V2 hook API:
 * - ctx.session.hook("context") replaces experimental.chat.{system,messages}.transform
 *   (system + messages are mutated in place before model dispatch).
 * - ctx.tool.transform(tools.add(...)) registers the compress tool
 *   (JSON Schema input, plain-string content result).
 */
const pluginModule = {
    id: "dcp-rust",
    server,
    setup: async (ctx) => {
        // Session context hook: runs right before model dispatch with mutable
        // system parts and messages. Reuses the V1 handlers against a shimmed
        // input/output pair so both hosts share one code path.
        await ctx.session.hook("context", (event) => {
            const { sessionID, agent, model, system, messages, tools } = event
            void agent
            void model

            // System prompt injection (V1 system.transform equivalent).
            const systemOutput = { system: system.map((part) => part.text) }
            const systemHandler = getSystemHandler(sessionID)
            if (systemHandler) {
                void systemHandler({ sessionID }, systemOutput)
                for (let i = 0; i < systemOutput.system.length; i++) {
                    if (system[i]) {
                        system[i].text = systemOutput.system[i]
                    } else {
                        system.push(SystemPart.make(systemOutput.system[i]))
                    }
                }
            }

            // Message pipeline (V1 messages.transform equivalent).
            const messageHandler = getMessageHandler(sessionID)
            if (messageHandler) {
                void messageHandler({}, { messages })
            }
            void tools
        })

        // Compress tool registration.
        await ctx.tool.transform((tools) => {
            tools.add({
                name: "compress",
                description: COMPRESS_TOOL_DESCRIPTION,
                input: {
                    type: "object",
                    properties: {
                        topic: { type: "string", description: "Short label (3-5 words) for the batch" },
                        startId: { type: "string", description: "Message/block ID beginning of range (e.g. m0001, b2)" },
                        endId: { type: "string", description: "Message/block ID end of range (e.g. m0012, b5)" },
                        summary: { type: "string", description: "Technical summary replacing all content in range" },
                    },
                    required: ["topic", "startId", "endId", "summary"],
                    additionalProperties: false,
                },
                execute: async (input) => {
                    return compressViaBridge(input)
                },
            })
        })
    },
}

export default pluginModule
export { server }
