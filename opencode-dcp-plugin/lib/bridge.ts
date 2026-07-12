import { createRequire } from "module"
import { fileURLToPath } from "url"
import { dirname, join, resolve } from "path"
import { existsSync } from "fs"

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)
const _require = createRequire(import.meta.url)

export interface DcpPruner {
  transformMessages(messagesJson: string): string
  transformSystem(system: string): string
  handleCompress(argsJson: string, messagesJson: string): string
  decompress(blockId: number): string
  recompress(blockId: number): string
  handleCommand(cmd: string, argsJson: string, messagesJson: string): string
  notifyEvent(eventJson: string): void
  hasPendingWork(): boolean
  stats(): string
  contextSnapshot(): string
  setSessionId(sessionId: string): void
  isEnabled(): boolean
  configJson(): string
}

export interface BridgeExports {
  DcpPruner: new (configJson: string) => DcpPruner
  loadDcpConfig(): string
}

let cached: BridgeExports | null = null

/** Package root whether loaded from lib/, dist/lib/, or dist/ts_src/. */
function packageRoot(): string {
  // lib/ → root; dist/lib/ → root; dist/ts_src/ → root
  let dir = __dirname
  // Walk up until package.json with our name, or hit node_modules boundary.
  for (let i = 0; i < 5; i++) {
    const pkg = join(dir, "package.json")
    if (existsSync(pkg)) {
      try {
        const j = _require(pkg)
        if (j?.name === "@qdang46/opencode-dcp-plugin" || existsSync(join(dir, "opencode-dcp-bridge.darwin-arm64.node")) || existsSync(join(dir, "npm"))) {
          return dir
        }
      } catch {
        /* continue */
      }
    }
    const parent = resolve(dir, "..")
    if (parent === dir) break
    dir = parent
  }
  return resolve(__dirname, "..")
}

function platformTriple(): string {
  const platform = process.platform
  const arch = process.arch
  if (platform === "darwin" && arch === "arm64") return "darwin-arm64"
  if (platform === "darwin" && arch === "x64") return "darwin-x64"
  if (platform === "linux" && arch === "x64") return "linux-x64-gnu"
  if (platform === "win32" && arch === "x64") return "win32-x64-msvc"
  return `${platform}-${arch}`
}

function candidatePaths(): string[] {
  const root = packageRoot()
  const triple = platformTriple()
  const name = `opencode-dcp-bridge.${triple}.node`
  return [
    join(root, "npm", triple, name),
    join(root, name),
    join(root, "node_modules", `@qdang46/opencode-dcp-bridge-${triple}`, name),
    // monorepo dev builds
    join(root, "..", "target", "release", "libopencode_dcp_bridge.dylib"),
    join(root, "..", "target", "debug", "libopencode_dcp_bridge.dylib"),
  ]
}

export function loadBridge(): BridgeExports {
  if (cached) return cached

  const errors: string[] = []
  for (const path of candidatePaths()) {
    try {
      if (!existsSync(path)) {
        errors.push(`${path}: missing`)
        continue
      }
      cached = _require(path) as BridgeExports
      return cached
    } catch (err) {
      errors.push(`${path}: ${(err as Error).message}`)
    }
  }

  throw new Error(
    "Cannot load opencode-dcp-bridge native addon.\n" +
      "Build: cargo build -p opencode-dcp-bridge --release\n" +
      "Tried:\n  " +
      errors.slice(0, 10).join("\n  "),
  )
}

export function createPruner(): { bridge: BridgeExports; pruner: DcpPruner; config: any } {
  const bridge = loadBridge()
  const configJson = bridge.loadDcpConfig()
  let config: any = {}
  try {
    config = JSON.parse(configJson)
  } catch {
    config = { enabled: true }
  }
  const pruner = new bridge.DcpPruner(configJson)
  return { bridge, pruner, config }
}
