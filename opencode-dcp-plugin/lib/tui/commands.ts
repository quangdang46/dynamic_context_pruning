import type { DcpCommand, TuiApi } from "./types"

export function registerCommands(api: TuiApi, commands: DcpCommand[]) {
    const keymap = (api as any).keymap

    // OpenCode 2.x: reactive keymap layer with palette + slash entries.
    if (keymap?.layer) {
        keymap.layer({
            commands: commands.map((command) => ({
                id: command.name,
                title: command.title,
                description: command.description,
                group: "DCP rust",
                palette: true,
                slash: {
                    name: command.slashName,
                    aliases: command.slashAliases,
                    arguments: true,
                },
                run: command.run,
            })),
        })
        return
    }

    // 1.x-era keymap.registerLayer shape.
    if (keymap?.registerLayer) {
        keymap.registerLayer({
            commands: commands.map((command) => ({
                namespace: "palette",
                name: command.name,
                title: command.title,
                desc: command.description,
                category: "DCP rust",
                slashName: command.slashName,
                slashAliases: command.slashAliases,
                run: command.run,
            })),
        })
        return
    }

    api.command?.register(() =>
        commands.map((command) => ({
            title: command.title,
            value: command.name,
            description: command.description,
            category: "DCP rust",
            slash: { name: command.slashName, aliases: command.slashAliases },
            onSelect: command.run,
        })),
    )
}
