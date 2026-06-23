# =============================================================================
# install.ps1 — Install DCP (Dynamic Context Pruning) from GitHub releases
# =============================================================================
# Downloads the dcp CLI binary for your platform and sets up a git
# pre-commit hook (optional).
#
# Usage:
#   irm https://raw.githubusercontent.com/quangdang46/dynamic_context_pruning/main/install.ps1 | iex
#   .\install.ps1                          # install latest
#   .\install.ps1 -Version v0.1.0          # install specific version
#   .\install.ps1 -InstallDir C:\Tools     # custom install directory
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
    -NoGitHook            Skip git pre-commit hook
    -DryRun               Preview without changes
    -Quiet                Suppress progress output
    -Uninstall            Remove DCP and all configurations
    -Help                 Show this help

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
        foreach ($binName in @("dcp")) {
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

        Write-Step "Building dcp..."
        Push-Location $tmpDir
        try {
            cargo build --release -p dcp-cli --bin dcp 2>&1 | Write-DebugMsg
            $exePath = Join-Path $tmpDir "target\release\dcp.exe"
            if (Test-Path $exePath) {
                $destPath = Join-Path $Dest "dcp.exe"
                Copy-Item -Path $exePath -Destination $destPath -Force
                Write-Success "Built and installed dcp.exe"
            }
        } finally {
            Pop-Location
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
    foreach ($bin in @("dcp")) {
        $exePath = Join-Path $dest "$bin.exe"
        if (Test-Path $exePath) {
            Remove-Item $exePath -Force
            Write-Success "Removed $exePath"
        }
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
