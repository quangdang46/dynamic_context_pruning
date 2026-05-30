# =============================================================================
# install.ps1 — Install DCP (Dynamic Context Pruning) from GitHub releases
# =============================================================================
# Downloads binaries for your platform and auto-configures:
#   - MCP servers (Claude Code, Cursor, Cline, Windsurf, VS Code Copilot)
#   - Claude Code hooks (PreToolUse, SessionStart)
#   - Git pre-commit hook (optional)
#
# Usage:
#   irm https://raw.githubusercontent.com/quangdang46/dynamic_context_pruning/main/install.ps1 | iex
#   .\install.ps1                          # install latest
#   .\install.ps1 -Version v0.1.0          # install specific version
#   .\install.ps1 -InstallDir C:\Tools     # custom install directory
#   .\install.ps1 -NoMcp                   # skip MCP provider config
#   .\install.ps1 -NoHooks                 # skip Claude Code hooks
#   .\install.ps1 -Uninstall               # remove everything
#   .\install.ps1 -DryRun                  # preview without changes
# =============================================================================

[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$InstallDir = "",
    [switch]$System = $false,
    [switch]$EasyMode = $false,
    [switch]$Verify = $false,
    [switch]$FromSource = $false,
    [switch]$NoMcp = $false,
    [switch]$NoHooks = $false,
    [switch]$NoGitHook = $false,
    [switch]$DryRun = $false,
    [switch]$Quiet = $false,
    [switch]$Uninstall = $false,
    [switch]$Help = $false
)

# ── Config ────────────────────────────────────────────────────────────────────

$Script:Owner = "quangdang46"
$Script:Repo  = "dynamic_context_pruning"
$Script:GitHubApi = "https://api.github.com/repos/$($Script:Owner)/$($Script:Repo)"
$Script:GitHubReleases = "https://github.com/$($Script:Owner)/$($Script:Repo)/releases"

# ── Colors (Write-Host with colors) ───────────────────────────────────────────

function Write-Step   { param([string]$Msg) if (-not $Quiet) { Write-Host "`n  $Msg" -ForegroundColor Cyan } }
function Write-Success{ param([string]$Msg) if (-not $Quiet) { Write-Host "  ✓ $Msg" -ForegroundColor Green } }
function Write-WarnMsg{ param([string]$Msg) Write-Host "  ! $Msg" -ForegroundColor Yellow }
function Write-ErrMsg { param([string]$Msg) Write-Host "  ✗ $Msg" -ForegroundColor Red }
function Write-DebugMsg { param([string]$Msg) if ($VerbosePreference -eq 'Continue') { Write-Host "  (debug) $Msg" -ForegroundColor DarkGray } }
function Die         { param([string]$Msg) Write-ErrMsg $Msg; exit 1 }

# ── Help ──────────────────────────────────────────────────────────────────────

if ($Help) {
    Write-Host @"

  dcp installer — Dynamic Context Pruning for LLM coding agents

  Usage:
    .\install.ps1 [options]

  Options:
    -Version <tag>        Install specific version (default: latest)
    -InstallDir <dir>     Custom install directory (default: ~\AppData\Local\DCP\bin)
    -System               Install to C:\Program Files\DCP (requires admin)
    -EasyMode             Auto-add to PATH (current user)
    -Verify               Run self-test after install
    -FromSource           Build from source (requires Rust/Cargo)
    -NoMcp                Skip MCP provider configuration
    -NoHooks              Skip Claude Code hooks configuration
    -NoGitHook            Skip git pre-commit hook
    -DryRun               Preview without changes
    -Quiet                Suppress progress output
    -Uninstall            Remove DCP and all configurations
    -Help                 Show this help

  Providers auto-configured:
    * Claude Code  (hooks + MCP)
    * Cursor       (MCP)
    * Cline        (MCP)
    * Windsurf     (MCP)
    * VS Code      (Copilot MCP)
    * OpenCode     (MCP, env as array)
    * Codex CLI    (hooks + MCP, TOML)
    * Gemini CLI   (hooks + MCP)
    * Amazon Q     (hooks + MCP)
    * Warp         (MCP)

"@
    exit 0
}

# ── Resolve Install Directory ─────────────────────────────────────────────────

function Resolve-InstallDir {
    if ($InstallDir -ne "") { return $InstallDir }
    if ($System) { return "C:\Program Files\DCP" }
    return Join-Path $env:LOCALAPPDATA "DCP"
}

# ── Resolve Latest Version ────────────────────────────────────────────────────

function Resolve-LatestVersion {
    try {
        $response = Invoke-RestMethod -Uri "$($Script:GitHubApi)/releases/latest" -TimeoutSec 30
        $tag = $response.tag_name
        if ($tag -match '^v\d') { return $tag }
    } catch {}

    # Fallback: scrape the releases page
    try {
        $resp = Invoke-WebRequest -Uri "$($Script:GitHubReleases)/latest" -MaximumRedirection 0 -ErrorAction SilentlyContinue
        $loc = $resp.Headers.Location
        if ($loc -match '/tag/(v[^/]+)$') { return $Matches[1] }
    } catch {
        if ($_.Exception.Response.StatusCode -eq 302) {
            $loc = $_.Exception.Response.Headers['Location']
            if ($loc -match '/tag/(v[^/]+)$') { return $Matches[1] }
        }
    }

    Die "Could not determine latest version. Use -Version <tag> to specify."
}

# ── Download & Install ───────────────────────────────────────────────────────

function Download-AndInstall {
    param(
        [string]$Version,
        [string]$Target,
        [string]$Dest
    )

    $baseUrl = "$($Script:GitHubReleases)/download/$Version"
    $tmpDir = Join-Path $env:TEMP "dcp-install-$(Get-Random)"
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

    try {
        foreach ($binName in @("dcp", "dcp-mcp", "dcp-claude-hook")) {
            $url = "$baseUrl/$binName-$Target.zip"
            $archive = Join-Path $tmpDir "$binName.zip"

            Write-Step "Downloading $binName..."
            Write-DebugMsg "URL: $url"

            if ($DryRun) {
                Write-Success "(dry-run) Would download $binName from $url"
                continue
            }

            try {
                Invoke-WebRequest -Uri $url -OutFile $archive -TimeoutSec 120 -ErrorAction Stop
            } catch {
                Write-WarnMsg "Could not download $binName — it may not be in this release"
                continue
            }

            # Extract
            $extractDir = Join-Path $tmpDir "$binName-extract"
            New-Item -ItemType Directory -Path $extractDir -Force | Out-Null
            Expand-Archive -Path $archive -DestinationPath $extractDir -Force

            # Find the binary
            $exePath = Get-ChildItem -Path $extractDir -Filter "$binName.exe" -Recurse | Select-Object -First 1
            if ($exePath) {
                $destPath = Join-Path $Dest "$binName.exe"
                Copy-Item -Path $exePath.FullName -Destination $destPath -Force
                Write-Success "Installed $binName.exe -> $destPath"
            } else {
                Write-WarnMsg "Binary '$binName.exe' not found in archive"
            }
        }
    } finally {
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ── Build from Source ─────────────────────────────────────────────────────────

function Build-FromSource {
    param([string]$Dest)

    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Die "cargo not found. Install Rust from https://rustup.rs"
    }

    $tmpDir = Join-Path $env:TEMP "dcp-src-$(Get-Random)"
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

    try {
        Write-Step "Cloning repository..."
        git clone --depth 1 "https://github.com/$($Script:Owner)/$($Script:Repo).git" $tmpDir

        foreach ($info in @(
            @{ Bin="dcp"; Pkg="dcp-cli" },
            @{ Bin="dcp-mcp"; Pkg="dcp-mcp" },
            @{ Bin="dcp-claude-hook"; Pkg="dcp-claude-hook" }
        )) {
            Write-Step "Building $($info.Bin)..."
            Push-Location $tmpDir
            try {
                cargo build --release -p $info.Pkg --bin $info.Bin 2>&1 | Write-DebugMsg
                $exePath = Join-Path $tmpDir "target\release\$($info.Bin).exe"
                if (Test-Path $exePath) {
                    $destPath = Join-Path $Dest "$($info.Bin).exe"
                    Copy-Item -Path $exePath -Destination $destPath -Force
                    Write-Success "Built and installed $($info.Bin).exe"
                }
            } finally {
                Pop-Location
            }
        }
    } finally {
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ── PATH Management ──────────────────────────────────────────────────────────

function Add-ToPath {
    param([string]$Dir)

    # Check if already in PATH
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -like "*$Dir*") { return }

    if ($EasyMode -and -not $DryRun) {
        $newPath = "$Dir;$currentPath"
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        Write-Success "Added $Dir to user PATH"
    } else {
        Write-WarnMsg "Add to PATH manually: `$env:Path += `";$Dir`""
    }
}

# ── JSON Helpers ──────────────────────────────────────────────────────────────

function Merge-JsonIntoFile {
    param(
        [string]$FilePath,
        [string]$Key,
        [hashtable]$Value
    )

    if ($DryRun) {
        Write-Success "(dry-run) Would configure $FilePath"
        return
    }

    # Ensure parent directory exists
    $parent = Split-Path -Parent $FilePath
    if (-not (Test-Path $parent)) { return }

    # Read or create the file
    $data = @{}
    if (Test-Path $FilePath) {
        try {
            $raw = Get-Content -Path $FilePath -Raw -ErrorAction Stop
            $data = $raw | ConvertFrom-Json -AsHashtable
        } catch {
            $data = @{}
        }
    }

    # Merge the value
    if (-not $data.ContainsKey($Key)) {
        $data[$Key] = @{}
    }
    foreach ($k in $Value.Keys) {
        $data[$Key][$k] = $Value[$k]
    }

    # Write back
    $data | ConvertTo-Json -Depth 10 | Set-Content -Path $FilePath -Encoding UTF8
}

function Remove-JsonKeyFromFile {
    param(
        [string]$FilePath,
        [string]$ParentKey,
        [string]$ChildKey
    )

    if (-not (Test-Path $FilePath)) { return }

    try {
        $raw = Get-Content -Path $FilePath -Raw
        $data = $raw | ConvertFrom-Json -AsHashtable

        if ($data.ContainsKey($ParentKey) -and $data[$ParentKey].ContainsKey($ChildKey)) {
            $data[$ParentKey].Remove($ChildKey) | Out-Null
            $data | ConvertTo-Json -Depth 10 | Set-Content -Path $FilePath -Encoding UTF8
        }
    } catch {
        # Silently ignore JSON parse errors
    }
}

# ── MCP Provider Configuration ───────────────────────────────────────────────

function Set-McpServer {
    param(
        [string]$ProviderName,
        [string]$SettingsFile,
        [string]$JsonKey,
        [string]$BinaryPath
    )

    if (-not (Test-Path $BinaryPath)) { return }

    # Check if config directory exists
    $configDir = Split-Path -Parent $SettingsFile
    if (-not (Test-Path $configDir)) {
        Write-DebugMsg "$ProviderName config dir not found, skipping"
        return
    }

    Write-Step "Configuring MCP for $ProviderName..."

    $mcpEntry = @{
        dcp = @{
            command = $BinaryPath
            args    = @()
            env     = @{}
        }
    }

    Merge-JsonIntoFile -FilePath $SettingsFile -Key $JsonKey -Value $mcpEntry
    Write-Success "$ProviderName MCP configured in $SettingsFile"
}

function Configure-AllMcpProviders {
    param([string]$BinaryDir)

    $mcpBinary = Join-Path $BinaryDir "dcp-mcp.exe"
    if (-not (Test-Path $mcpBinary)) {
        Write-WarnMsg "dcp-mcp.exe not installed — skipping MCP setup"
        return
    }

    # ── JSON-based providers ───────────────────────────────────────────────

    # Claude Code — write to both config locations
    Set-McpServer -ProviderName "Claude Code" `
                  -SettingsFile (Join-Path $env:USERPROFILE ".claude\settings.json") `
                  -JsonKey "mcpServers" `
                  -BinaryPath $mcpBinary
    Set-McpServer -ProviderName "Claude Code (~/.claude.json)" `
                  -SettingsFile (Join-Path $env:USERPROFILE ".claude.json") `
                  -JsonKey "mcpServers" `
                  -BinaryPath $mcpBinary

    # Cursor
    Set-McpServer -ProviderName "Cursor" `
                  -SettingsFile (Join-Path $env:APPDATA "Cursor\User\settings.json") `
                  -JsonKey "mcp.servers" `
                  -BinaryPath $mcpBinary

    # Cline
    Set-McpServer -ProviderName "Cline" `
                  -SettingsFile (Join-Path $env:USERPROFILE ".cline\mcp_settings.json") `
                  -JsonKey "mcpServers" `
                  -BinaryPath $mcpBinary

    # Windsurf
    Set-McpServer -ProviderName "Windsurf" `
                  -SettingsFile (Join-Path $env:USERPROFILE ".codeium\windsurf\mcp_config.json") `
                  -JsonKey "mcpServers" `
                  -BinaryPath $mcpBinary

    # VS Code Copilot
    Set-McpServer -ProviderName "VS Code" `
                  -SettingsFile (Join-Path $env:APPDATA "Code\User\settings.json") `
                  -JsonKey "github.copilot.chat.mcp" `
                  -BinaryPath $mcpBinary

    # OpenCode (env as array of strings)
    Set-McpServer-OpenCode -BinaryPath $mcpBinary

    # Gemini CLI
    Set-McpServer -ProviderName "Gemini CLI" `
                  -SettingsFile (Join-Path $env:USERPROFILE ".gemini\settings.json") `
                  -JsonKey "mcpServers" `
                  -BinaryPath $mcpBinary

    # Amazon Q
    Set-McpServer -ProviderName "Amazon Q" `
                  -SettingsFile (Join-Path $env:USERPROFILE ".aws\amazonq\mcp.json") `
                  -JsonKey "mcpServers" `
                  -BinaryPath $mcpBinary

    # Warp
    Set-McpServer -ProviderName "Warp" `
                  -SettingsFile (Join-Path $env:USERPROFILE ".warp\.mcp.json") `
                  -JsonKey "mcpServers" `
                  -BinaryPath $mcpBinary

    # ── TOML-based providers ───────────────────────────────────────────────

    # Codex CLI
    Set-McpServer-Codex -BinaryPath $mcpBinary
}

# ── OpenCode MCP (special: env as string array, type field) ──────────────────

function Set-McpServer-OpenCode {
    param([string]$BinaryPath)

    if (-not (Test-Path $BinaryPath)) { return }

    # Check multiple possible locations
    $settingsFile = Join-Path $env:USERPROFILE ".opencode.json"
    if (-not (Test-Path (Split-Path -Parent $settingsFile))) {
        $xdgConfig = if ($env:XDG_CONFIG_HOME) { $env:XDG_CONFIG_HOME } else { Join-Path $env:USERPROFILE ".config" }
        $alt = Join-Path $xdgConfig "opencode\.opencode.json"
        if (Test-Path (Split-Path -Parent $alt)) { $settingsFile = $alt } else { return }
    }

    Write-Step "Configuring MCP for OpenCode..."

    # OpenCode uses env as array of "KEY=VALUE" strings, not an object
    $mcpEntry = @{
        dcp = @{
            type    = "stdio"
            command = $BinaryPath
            args    = @()
            env     = @()
        }
    }

    Merge-JsonIntoFile -FilePath $settingsFile -Key "mcpServers" -Value $mcpEntry
    Write-Success "OpenCode MCP configured in $settingsFile"
}

# ── Codex CLI MCP (TOML-based config) ────────────────────────────────────────

function Set-McpServer-Codex {
    param([string]$BinaryPath)

    if (-not (Test-Path $BinaryPath)) { return }

    $configDir = Join-Path $env:USERPROFILE ".codex"
    if (-not (Test-Path $configDir)) { return }

    $configFile = Join-Path $configDir "config.toml"

    Write-Step "Configuring MCP for Codex CLI..."

    if ($DryRun) {
        Write-Success "(dry-run) Would configure $configFile"
        return
    }

    # Read existing content
    $content = ""
    if (Test-Path $configFile) {
        $content = Get-Content -Path $configFile -Raw
    }

    # Check if section already exists
    if ($content -match '\[mcp_servers\.dcp\]') {
        # Update command in existing section
        $content = $content -replace '(?m)(^\[mcp_servers\.dcp\][\s\S]*?command = ").*?(")', "`${1}$BinaryPath`${2}"
    } else {
        # Append new section
        $content += @"

[mcp_servers.dcp]
type = "stdio"
command = "$BinaryPath"
args = []
"@
    }

    Set-Content -Path $configFile -Value $content -Encoding UTF8
    Write-Success "Codex CLI MCP configured in $configFile"
}

# ── Claude Code Hooks ─────────────────────────────────────────────────────────

function Configure-ClaudeHooks {
    param([string]$BinaryDir)

    $hookBinary = Join-Path $BinaryDir "dcp-claude-hook.exe"
    if (-not (Test-Path $hookBinary)) {
        Write-WarnMsg "dcp-claude-hook.exe not installed — skipping hook setup"
        return
    }

    Write-Step "Configuring Claude Code hooks..."

    $settingsDir = Join-Path $env:USERPROFILE ".claude"
    $settingsFile = Join-Path $settingsDir "settings.json"

    if ($DryRun) {
        Write-Success "(dry-run) Would configure hooks in $settingsFile"
        return
    }

    if (-not (Test-Path $settingsDir)) {
        New-Item -ItemType Directory -Path $settingsDir -Force | Out-Null
    }

    $hooksConfig = @{
        PreToolUse = @(
            @{
                matcher = "*"
                hooks   = @(
                    @{
                        type    = "command"
                        command = $hookBinary
                    }
                )
            }
        )
        SessionStart = @(
            @{
                matcher = "compact"
                hooks   = @(
                    @{
                        type    = "command"
                        command = "$hookBinary --on-compact"
                    }
                )
            }
        )
    }

    Merge-JsonIntoFile -FilePath $settingsFile -Key "hooks" -Value $hooksConfig
    Write-Success "Claude Code hooks configured in $settingsFile"
}

# ── Codex CLI Hooks (TOML) ───────────────────────────────────────────────────

function Configure-CodexHooks {
    param([string]$BinaryDir)

    $hookBinary = Join-Path $BinaryDir "dcp-claude-hook.exe"
    if (-not (Test-Path $hookBinary)) { return }

    $configDir = Join-Path $env:USERPROFILE ".codex"
    if (-not (Test-Path $configDir)) { return }

    $configFile = Join-Path $configDir "config.toml"

    Write-Step "Configuring hooks for Codex CLI..."

    if ($DryRun) {
        Write-Success "(dry-run) Would configure hooks in $configFile"
        return
    }

    $content = ""
    if (Test-Path $configFile) {
        $content = Get-Content -Path $configFile -Raw
    }

    # Only add if not already present
    if ($content -match 'dcp-claude-hook') { return }

    $content += @"

[[hooks.PreToolUse]]
matcher = "*"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "$hookBinary"

[[hooks.SessionStart]]
matcher = "compact"

[[hooks.SessionStart.hooks]]
type = "command"
command = "$hookBinary --on-compact"
"@

    Set-Content -Path $configFile -Value $content -Encoding UTF8
    Write-Success "Codex CLI hooks configured in $configFile"
}

# ── Gemini CLI Hooks ─────────────────────────────────────────────────────────

function Configure-GeminiHooks {
    param([string]$BinaryDir)

    $hookBinary = Join-Path $BinaryDir "dcp-claude-hook.exe"
    if (-not (Test-Path $hookBinary)) { return }

    $settingsFile = Join-Path $env:USERPROFILE ".gemini\settings.json"
    if (-not (Test-Path (Split-Path -Parent $settingsFile))) { return }

    Write-Step "Configuring hooks for Gemini CLI..."

    if ($DryRun) {
        Write-Success "(dry-run) Would configure hooks in $settingsFile"
        return
    }

    $hooksConfig = @{
        BeforeTool = @(
            @{
                matcher    = ".*"
                sequential = $false
                hooks      = @(
                    @{
                        type        = "command"
                        command     = $hookBinary
                        name        = "dcp-prune"
                        timeout     = 5000
                        description = "DCP context pruning"
                    }
                )
            }
        )
        SessionStart = @(
            @{
                matcher = "startup"
                hooks   = @(
                    @{
                        type        = "command"
                        command     = "$hookBinary --on-compact"
                        name        = "dcp-compact"
                        description = "DCP compact handler"
                    }
                )
            }
        )
    }

    Merge-JsonIntoFile -FilePath $settingsFile -Key "hooks" -Value $hooksConfig
    Write-Success "Gemini CLI hooks configured in $settingsFile"
}

# ── Amazon Q Hooks ───────────────────────────────────────────────────────────

function Configure-AmazonQHooks {
    param([string]$BinaryDir)

    $hookBinary = Join-Path $BinaryDir "dcp-claude-hook.exe"
    if (-not (Test-Path $hookBinary)) { return }

    $settingsFile = Join-Path $env:USERPROFILE ".aws\amazonq\mcp.json"
    if (-not (Test-Path (Split-Path -Parent $settingsFile))) { return }

    Write-Step "Configuring hooks for Amazon Q..."

    if ($DryRun) {
        Write-Success "(dry-run) Would configure hooks in $settingsFile"
        return
    }

    $hooksConfig = @{
        preToolUse = @(
            @{
                matcher = "*"
                command = $hookBinary
            }
        )
        stop = @(
            @{
                command = "$hookBinary --on-compact"
            }
        )
    }

    Merge-JsonIntoFile -FilePath $settingsFile -Key "hooks" -Value $hooksConfig
    Write-Success "Amazon Q hooks configured in $settingsFile"
}

# ── Git Pre-commit Hook ──────────────────────────────────────────────────────

function Configure-GitHook {
    Write-Step "Configuring git pre-commit hook..."

    if (-not (Test-Path ".git")) {
        Write-WarnMsg "Not in a git repository — skipping git hook"
        return
    }

    if ($DryRun) {
        Write-Success "(dry-run) Would install pre-commit hook"
        return
    }

    if (Test-Path "scripts\install-hooks.sh") {
        # On Windows with Git Bash
        bash scripts/install-hooks.sh --force 2>$null
    } elseif (Test-Path "scripts\pre-commit.sh") {
        # Direct copy
        $hooksDir = ".git\hooks"
        if (-not (Test-Path $hooksDir)) { New-Item -ItemType Directory -Path $hooksDir -Force | Out-Null }
        Copy-Item "scripts\pre-commit.sh" "$hooksDir\pre-commit" -Force
        Write-Success "Installed pre-commit hook"
    } else {
        Write-WarnMsg "scripts\install-hooks.sh not found — skipping"
    }
}

# ── Uninstall ─────────────────────────────────────────────────────────────────

function Do-Uninstall {
    Write-Step "Uninstalling DCP..."

    $dest = Resolve-InstallDir

    # Remove binaries
    foreach ($bin in @("dcp", "dcp-mcp", "dcp-claude-hook")) {
        $exePath = Join-Path $dest "$bin.exe"
        if (Test-Path $exePath) {
            Remove-Item $exePath -Force
            Write-Success "Removed $exePath"
        }
    }

    # Remove MCP config from all providers (both Claude Code locations)
    Remove-JsonKeyFromFile -FilePath (Join-Path $env:USERPROFILE ".claude\settings.json") `
                           -ParentKey "mcpServers" -ChildKey "dcp"
    Remove-JsonKeyFromFile -FilePath (Join-Path $env:USERPROFILE ".claude.json") `
                           -ParentKey "mcpServers" -ChildKey "dcp"
    Remove-JsonKeyFromFile -FilePath (Join-Path $env:USERPROFILE ".cline\mcp_settings.json") `
                           -ParentKey "mcpServers" -ChildKey "dcp"
    Remove-JsonKeyFromFile -FilePath (Join-Path $env:USERPROFILE ".codeium\windsurf\mcp_config.json") `
                           -ParentKey "mcpServers" -ChildKey "dcp"
    Remove-JsonKeyFromFile -FilePath (Join-Path $env:USERPROFILE ".opencode.json") `
                           -ParentKey "mcpServers" -ChildKey "dcp"
    Remove-JsonKeyFromFile -FilePath (Join-Path $env:USERPROFILE ".gemini\settings.json") `
                           -ParentKey "mcpServers" -ChildKey "dcp"
    Remove-JsonKeyFromFile -FilePath (Join-Path $env:USERPROFILE ".aws\amazonq\mcp.json") `
                           -ParentKey "mcpServers" -ChildKey "dcp"
    Remove-JsonKeyFromFile -FilePath (Join-Path $env:USERPROFILE ".warp\.mcp.json") `
                           -ParentKey "mcpServers" -ChildKey "dcp"

    # Remove from Codex CLI (TOML)
    $codexConfig = Join-Path $env:USERPROFILE ".codex\config.toml"
    if (Test-Path $codexConfig) {
        $content = Get-Content -Path $codexConfig -Raw
        # Remove dcp-claude-hook references and [mcp_servers.dcp] section
        $content = ($content -split "`n" | Where-Object {
            $_ -notmatch 'dcp-claude-hook' -and $_ -notmatch '^\[mcp_servers\.dcp\]'
        }) -join "`n"
        Set-Content -Path $codexConfig -Value $content.TrimEnd() -Encoding UTF8
        Write-Success "Removed DCP from Codex CLI config"
    }

    # Remove PATH
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -like "*$dest*") {
        $newPath = ($currentPath -split ';' | Where-Object { $_ -ne $dest }) -join ';'
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        Write-Success "Removed from user PATH"
    }

    Write-Host ""
    Write-Success "DCP uninstalled successfully"
    exit 0
}

# ── Main ──────────────────────────────────────────────────────────────────────

if ($Uninstall) { Do-Uninstall }

Write-Host ""
Write-Host "  ═════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host "    DCP Installer — Dynamic Context Pruning" -ForegroundColor White
Write-Host "  ═════════════════════════════════════════════" -ForegroundColor Cyan

$dest = Resolve-InstallDir
$target = "x86_64-pc-windows-msvc"

Write-Step "Platform: windows-x86_64 ($target)"
Write-Step "Install dir: $dest"

# ── Resolve version ───────────────────────────────────────────────────────
$resolvedVersion = $Version
if ([string]::IsNullOrEmpty($resolvedVersion)) {
    $resolvedVersion = Resolve-LatestVersion
}
Write-Step "Version: $resolvedVersion"

# ── Install binaries ─────────────────────────────────────────────────────
if ($FromSource) {
    Build-FromSource -Dest $dest
} else {
    if ($DryRun) {
        Write-Success "(dry-run) Would download and install binaries to $dest"
    } else {
        New-Item -ItemType Directory -Path $dest -Force | Out-Null
        Download-AndInstall -Version $resolvedVersion -Target $target -Dest $dest
    }
}

# ── Add to PATH ──────────────────────────────────────────────────────────
Add-ToPath -Dir $dest

# ── Configure MCP providers ──────────────────────────────────────────────
if (-not $NoMcp) {
    Configure-AllMcpProviders -BinaryDir $dest
}

# ── Configure hooks for all providers ─────────────────────────────────────
if (-not $NoHooks) {
    Configure-ClaudeHooks -BinaryDir $dest
    Configure-CodexHooks -BinaryDir $dest
    Configure-GeminiHooks -BinaryDir $dest
    Configure-AmazonQHooks -BinaryDir $dest
}

# ── Configure git pre-commit hook ────────────────────────────────────────
if (-not $NoGitHook) {
    Configure-GitHook
}

# ── Verify ───────────────────────────────────────────────────────────────
if ($Verify) {
    $dcpExe = Join-Path $dest "dcp.exe"
    if (Test-Path $dcpExe) {
        & $dcpExe --version
    }
}

# ── Summary ──────────────────────────────────────────────────────────────
Write-Host ""
Write-Host "  ═════════════════════════════════════════════" -ForegroundColor Green
Write-Host "    DCP $resolvedVersion installed successfully!" -ForegroundColor White
Write-Host "  ═════════════════════════════════════════════" -ForegroundColor Green
Write-Host ""
Write-Host "  Binaries installed to:  $dest" -ForegroundColor Cyan
Write-Host "  Configuration:          $env:USERPROFILE\.dynamic_context_pruning\config.jsonc" -ForegroundColor Cyan
Write-Host ""

if (-not $NoMcp) {
    Write-Host "  MCP Providers:" -ForegroundColor White
    Write-Host "    Claude Code    ✓" -ForegroundColor Green
    $cursorDir = Join-Path $env:APPDATA "Cursor"
    if (Test-Path $cursorDir) { Write-Host "    Cursor         ✓" -ForegroundColor Green }
    else { Write-Host "    Cursor         —" -ForegroundColor DarkGray }
    $clineDir = Join-Path $env:USERPROFILE ".cline"
    if (Test-Path $clineDir) { Write-Host "    Cline          ✓" -ForegroundColor Green }
    else { Write-Host "    Cline          —" -ForegroundColor DarkGray }
    $windsurfDir = Join-Path $env:USERPROFILE ".codeium"
    if (Test-Path $windsurfDir) { Write-Host "    Windsurf       ✓" -ForegroundColor Green }
    else { Write-Host "    Windsurf       —" -ForegroundColor DarkGray }
    $vscodeDir = Join-Path $env:APPDATA "Code"
    if (Test-Path $vscodeDir) { Write-Host "    VS Code        ✓" -ForegroundColor Green }
    else { Write-Host "    VS Code        —" -ForegroundColor DarkGray }
    $opencodeFile = Join-Path $env:USERPROFILE ".opencode.json"
    if (Test-Path $opencodeFile) { Write-Host "    OpenCode       ✓" -ForegroundColor Green }
    else { Write-Host "    OpenCode       —" -ForegroundColor DarkGray }
    $codexDir = Join-Path $env:USERPROFILE ".codex"
    if (Test-Path $codexDir) { Write-Host "    Codex CLI      ✓" -ForegroundColor Green }
    else { Write-Host "    Codex CLI      —" -ForegroundColor DarkGray }
    $geminiDir = Join-Path $env:USERPROFILE ".gemini"
    if (Test-Path $geminiDir) { Write-Host "    Gemini CLI     ✓" -ForegroundColor Green }
    else { Write-Host "    Gemini CLI     —" -ForegroundColor DarkGray }
    $amazonqDir = Join-Path $env:USERPROFILE ".aws\amazonq"
    if (Test-Path $amazonqDir) { Write-Host "    Amazon Q       ✓" -ForegroundColor Green }
    else { Write-Host "    Amazon Q       —" -ForegroundColor DarkGray }
    $warpDir = Join-Path $env:USERPROFILE ".warp"
    if (Test-Path $warpDir) { Write-Host "    Warp           ✓" -ForegroundColor Green }
    else { Write-Host "    Warp           —" -ForegroundColor DarkGray }
}

if (-not $NoHooks) {
    Write-Host ""
    Write-Host "  Hooks configured:" -ForegroundColor White
    Write-Host "    Claude Code    ✓  (PreToolUse + SessionStart)" -ForegroundColor Green
    if (Test-Path (Join-Path $env:USERPROFILE ".codex")) {
        Write-Host "    Codex CLI      ✓  (PreToolUse + SessionStart)" -ForegroundColor Green
    } else { Write-Host "    Codex CLI      —" -ForegroundColor DarkGray }
    if (Test-Path (Join-Path $env:USERPROFILE ".gemini")) {
        Write-Host "    Gemini CLI     ✓  (BeforeTool + SessionStart)" -ForegroundColor Green
    } else { Write-Host "    Gemini CLI     —" -ForegroundColor DarkGray }
    if (Test-Path (Join-Path $env:USERPROFILE ".aws\amazonq")) {
        Write-Host "    Amazon Q       ✓  (preToolUse + stop)" -ForegroundColor Green
    } else { Write-Host "    Amazon Q       —" -ForegroundColor DarkGray }
}

Write-Host ""
Write-Host "  Quick start:" -ForegroundColor White
Write-Host "    dcp --help           Show CLI help" -ForegroundColor Cyan
Write-Host "    dcp context          Show current session context" -ForegroundColor Cyan
Write-Host "    dcp stats            Show pruning statistics" -ForegroundColor Cyan
Write-Host ""

# Print installed version
$dcpExe = Join-Path $dest "dcp.exe"
if ((Test-Path $dcpExe) -and -not $DryRun) {
    $installedVersion = & $dcpExe --version 2>$null
    if ($installedVersion) { Write-Success "Verified: dcp $installedVersion" }
}
