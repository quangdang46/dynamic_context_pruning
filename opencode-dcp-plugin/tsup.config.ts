import { defineConfig } from "tsup"

export default defineConfig({
    // Upstream entry: pure TS plugin with full OpenCode parity
    entry: ["index.ts"],
    format: ["esm"],
    dts: false,
    clean: true,
    sourcemap: true,
    // Bundle jsonc-parser to fix its broken ESM imports (same as upstream)
    noExternal: ["jsonc-parser"],
    // Keep native deps external
    external: [
        "@opencode-ai/plugin",
        "@opencode-ai/sdk",
        "@anthropic-ai/tokenizer",
        "@opentui/core",
        "@opentui/solid",
        "solid-js",
    ],
})
