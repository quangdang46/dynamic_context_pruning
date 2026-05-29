#!/usr/bin/env bash
# =============================================================================
# install.sh — Install DCP (Dynamic Context Pruning) from GitHub releases
# =============================================================================
# Downloads binaries for your platform and auto-configures:
#   - MCP servers (Claude Code, Cursor, Cline, Windsurf, VS Code Copilot)
#   - Claude Code hooks (PreToolUse, SessionStart)
#   - Git pre-commit hook (optional)
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/quangdang46/dynamic_context_pruning/main/install.sh | bash
#   ./install.sh                          # install latest
#   ./install.sh --version v0.1.0         # install specific version
#   ./install.sh --dest ~/bin             # custom install directory
#   ./install.sh --no-mcp                 # skip MCP provider config
#   ./install.sh --no-hooks               # skip Claude Code hooks
#   ./install.sh --uninstall              # remove everything
#   ./install.sh --dry-run                # preview without changes
# =============================================================================
set -euo pipefail
umask 022

# ── Config ────────────────────────────────────────────────────────────────────

BINARY_NAME="dcp"
OWNER="quangdang46"
REPO="dynamic_context_pruning"
DEST="${DEST:-$HOME/.local/bin}"
VERSION="${VERSION:-}"
QUIET=0; EASY=0; VERIFY=0; FROM_SOURCE=0; UNINSTALL=0; DRY_RUN=0
NO_MCP=0; NO_HOOKS=0; NO_GIT_HOOK=0
MAX_RETRIES=3; DOWNLOAD_TIMEOUT=120
LOCK_DIR="/tmp/${BINARY_NAME}-install.lock.d"
TMP=""

# ── Colors ────────────────────────────────────────────────────────────────────

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; BOLD='\033[1m'
DIM='\033[2m'; RESET='\033[0m'

# ── Logging ───────────────────────────────────────────────────────────────────

log_info()    { [ "$QUIET" -eq 1 ] && return; echo -e "  ${GREEN}✓${RESET} $*" >&2; }
log_warn()    { echo -e "  ${YELLOW}!${RESET} $*" >&2; }
log_error()   { echo -e "  ${RED}✗${RESET} $*" >&2; }
log_step()    { [ "$QUIET" -eq 1 ] && return; echo -e "\n${BOLD}${BLUE}▸${RESET} $*" >&2; }
log_debug()   { [ "$QUIET" -eq 1 ] && return; echo -e "  ${DIM}(debug)${RESET} $*" >&2; }
die()         { log_error "$@"; exit 1; }

# ── Cleanup & lock ────────────────────────────────────────────────────────────

cleanup() { rm -rf "$TMP" "$LOCK_DIR" 2>/dev/null || true; }
trap cleanup EXIT
acquire_lock() {
    mkdir "$LOCK_DIR" 2>/dev/null || die "Another install running. rm -rf $LOCK_DIR"
    echo $$ > "$LOCK_DIR/pid"
}

# ── Args ───────────────────────────────────────────────────────────────────────

usage() {
    cat <<EOF
${BOLD}dcp installer${RESET} — Dynamic Context Pruning for LLM coding agents

${BOLD}Usage:${RESET}
  $0 [options]

${BOLD}Options:${RESET}
  --dest PATH          Installation directory (default: ~/.local/bin)
  --version VERSION    Install specific version (default: latest)
  --system             Install to /usr/local/bin (requires sudo)
  --easy-mode          Auto-add to PATH in shell RC files
  --verify             Run self-test after install
  --from-source        Build from source instead of downloading
  --no-mcp             Skip MCP provider configuration
  --no-hooks           Skip Claude Code hooks configuration
  --no-git-hook        Skip git pre-commit hook
  --dry-run            Preview what would be done without changes
  --quiet, -q          Suppress progress output
  --uninstall          Remove DCP and all configurations
  -h, --help           Show this help

${BOLD}Providers auto-configured:${RESET}
  • Claude Code  (hooks + MCP)
  • Cursor       (MCP)
  • Cline        (MCP)
  • Windsurf     (MCP)
  • VS Code      (Copilot MCP)
  • OpenCode     (MCP, env as array)
  • Codex CLI    (hooks + MCP, TOML)
  • Gemini CLI   (hooks + MCP)
  • Amazon Q     (hooks + MCP)
  • Warp         (MCP)

${BOLD}Examples:${RESET}
  # Install latest with all providers
  curl -fsSL https://raw.githubusercontent.com/${OWNER}/${REPO}/main/install.sh | bash

  # Install specific version, auto-add to PATH
  ./install.sh --version v0.2.0 --easy-mode

  # Preview installation
  ./install.sh --dry-run --verbose
EOF
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dest)         DEST="$2";              shift 2;;
        --dest=*)       DEST="${1#*=}";         shift;;
        --version)      VERSION="$2";           shift 2;;
        --version=*)    VERSION="${1#*=}";      shift;;
        --system)       DEST="/usr/local/bin";  shift;;
        --easy-mode)    EASY=1;                 shift;;
        --verify)       VERIFY=1;               shift;;
        --from-source)  FROM_SOURCE=1;           shift;;
        --no-mcp)       NO_MCP=1;               shift;;
        --no-hooks)     NO_HOOKS=1;             shift;;
        --no-git-hook)  NO_GIT_HOOK=1;          shift;;
        --dry-run)      DRY_RUN=1;              shift;;
        --quiet|-q)     QUIET=1;                shift;;
        --uninstall)    UNINSTALL=1;            shift;;
        -h|--help)      usage;;
        *) shift;;
    esac
done

# ── Uninstall ─────────────────────────────────────────────────────────────────

do_uninstall() {
    log_step "Uninstalling DCP..."

    # Remove binaries
    for bin in dcp dcp-mcp dcp-claude-hook; do
        if [ -f "$DEST/$bin" ]; then
            rm -f "$DEST/$bin"
            log_info "Removed $DEST/$bin"
        fi
    done

    # Remove from Claude Code settings
    _remove_mcp_from_file "$HOME/.claude/settings.json" "dcp"
    _remove_hooks_from_claude_settings "$HOME/.claude/settings.json"

    # Remove from other JSON providers
    _remove_mcp_from_file "$HOME/.cline/mcp_settings.json" "dcp"
    _remove_mcp_from_file "$HOME/.codeium/windsurf/mcp_config.json" "dcp"
    _remove_mcp_from_file "$HOME/.opencode.json" "dcp"
    _remove_mcp_from_file "$HOME/.gemini/settings.json" "dcp"
    _remove_mcp_from_file "$HOME/.aws/amazonq/mcp.json" "dcp"
    _remove_mcp_from_file "$HOME/.warp/.mcp.json" "dcp"

    # Remove from Codex CLI (TOML)
    if [ -f "$HOME/.codex/config.toml" ]; then
        local tmpf; tmpf="$(mktemp)"
        sed '/^\[mcp_servers\.dcp\]/,/^\[/{ /^\[mcp_servers\.dcp\]/d; /^\[/{p;d}; d }' "$HOME/.codex/config.toml" > "$tmpf" && mv "$tmpf" "$HOME/.codex/config.toml"
        sed '/dcp-claude-hook/d' "$HOME/.codex/config.toml" > "$tmpf" && mv "$tmpf" "$HOME/.codex/config.toml"
        log_info "Removed DCP from Codex CLI config"
    fi

    # Remove PATH entries
    for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
        [ -f "$rc" ] && sed -i "/${BINARY_NAME} installer/d" "$rc" 2>/dev/null || true
    done

    echo "" >&2
    log_info "DCP uninstalled successfully"
    exit 0
}

[ "$UNINSTALL" -eq 1 ] && { acquire_lock; do_uninstall; }

# ── Platform Detection ────────────────────────────────────────────────────────

detect_platform() {
    local os arch
    case "$(uname -s)" in
        Linux*)  os="linux";;
        Darwin*) os="darwin";;
        MINGW*|MSYS*|CYGWIN*) os="windows";;
        *) die "Unsupported OS: $(uname -s)";;
    esac
    case "$(uname -m)" in
        x86_64|amd64)  arch="x86_64";;
        aarch64|arm64) arch="aarch64";;
        *) die "Unsupported arch: $(uname -m)";;
    esac
    echo "${os}_${arch}"
}

resolve_target() {
    local platform="$1"
    case "$platform" in
        linux_x86_64)   echo "x86_64-unknown-linux-musl";;
        linux_aarch64)  echo "aarch64-unknown-linux-musl";;
        darwin_x86_64)  echo "x86_64-apple-darwin";;
        darwin_aarch64) echo "aarch64-apple-darwin";;
        windows_x86_64) echo "x86_64-pc-windows-msvc";;
        *) die "No target triple for: $platform";;
    esac
}

# ── Version Resolution ────────────────────────────────────────────────────────

resolve_version() {
    [ -n "$VERSION" ] && return 0
    VERSION=$(curl -fsSL --connect-timeout 10 --max-time 30 \
        "https://api.github.com/repos/${OWNER}/${REPO}/releases/latest" 2>/dev/null \
        | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/') || true
    if ! [[ "$VERSION" =~ ^v[0-9] ]]; then
        VERSION=$(curl -fsSL -o /dev/null -w '%{url_effective}' \
            "https://github.com/${OWNER}/${REPO}/releases/latest" 2>/dev/null \
            | sed -E 's|.*/tag/||') || true
    fi
    [[ "$VERSION" =~ ^v[0-9] ]] || die "Could not resolve version. Use --version <tag>."
    log_info "Latest: $VERSION"
}

# ── Download Helpers ──────────────────────────────────────────────────────────

download_file() {
    local url="$1" dest="$2" partial="${2}.part" attempt=0
    while [ $attempt -lt $MAX_RETRIES ]; do
        attempt=$((attempt + 1))
        curl -fL --connect-timeout 30 --max-time "$DOWNLOAD_TIMEOUT" \
             -sS --retry 2 \
             $( [ -s "$partial" ] && echo "--continue-at -") \
             -o "$partial" "$url" \
          && mv -f "$partial" "$dest" && return 0
        [ $attempt -lt $MAX_RETRIES ] && { log_warn "Retry $attempt..."; sleep 3; }
    done
    return 1
}

install_binary_atomic() {
    local src="$1" dest="$2" tmp="${2}.tmp.$$"
    install -m 0755 "$src" "$tmp" && mv -f "$tmp" "$dest" || { rm -f "$tmp"; die "Install failed"; }
}

# ── Source Build ──────────────────────────────────────────────────────────────

build_from_source() {
    command -v cargo >/dev/null || die "cargo not found — install Rust: https://rustup.rs"
    git clone --depth 1 "https://github.com/${OWNER}/${REPO}.git" "$TMP/src"
    for bin_name in dcp dcp-mcp dcp-claude-hook; do
        local pkg="dcp-cli"
        [ "$bin_name" = "dcp-mcp" ] && pkg="dcp-mcp"
        [ "$bin_name" = "dcp-claude-hook" ] && pkg="dcp-claude-hook"
        (cd "$TMP/src" && CARGO_TARGET_DIR="$TMP/target" cargo build --release -p "$pkg" --bin "$bin_name") || true
        [ -f "$TMP/target/release/$bin_name" ] && install_binary_atomic "$TMP/target/release/$bin_name" "$DEST/$bin_name"
    done
}

# ── Download All Binaries ────────────────────────────────────────────────────

download_all_binaries() {
    local platform="$1" target="$2"

    local ext="tar.gz"
    [ "${platform}" == windows* ] && ext="zip"
    local base_url="https://github.com/${OWNER}/${REPO}/releases/download/${VERSION}"

    for bin_name in dcp dcp-mcp dcp-claude-hook; do
        local url="${base_url}/${bin_name}-${target}.${ext}"
        local archive="${TMP}/${bin_name}.${ext}"

        log_step "Downloading ${bin_name}..."
        log_debug "URL: $url"

        if download_file "$url" "$archive" 2>/dev/null; then
            # Extract
            case "$archive" in
                *.tar.gz) tar -xzf "$archive" -C "$TMP";;
                *.zip)    unzip -qo "$archive" -d "$TMP/${bin_name}-extract";;
            esac

            # Find the binary
            local bin_path=""
            if [ "$ext" = "zip" ]; then
                bin_path=$(find "$TMP/${bin_name}-extract" -name "${bin_name}.exe" -type f 2>/dev/null | head -1)
            else
                bin_path=$(find "$TMP" -maxdepth 2 -name "$bin_name" -type f -perm -111 2>/dev/null | head -1)
            fi

            if [ -n "$bin_path" ]; then
                install_binary_atomic "$bin_path" "$DEST/$bin_name"
                log_info "Installed ${bin_name} → $DEST/$bin_name"
            else
                log_warn "Binary '${bin_name}' not found in archive"
            fi
        else
            log_warn "Could not download ${bin_name} — it may not be in this release"
        fi
    done
}

# ── PATH Management ──────────────────────────────────────────────────────────

maybe_add_path() {
    case ":$PATH:" in *":$DEST:"*) return 0;; esac
    if [ "$EASY" -eq 1 ]; then
        for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
            [ -f "$rc" ] && [ -w "$rc" ] || continue
            grep -qF "$DEST" "$rc" && continue
            printf '\nexport PATH="%s:$PATH"  # %s installer\n' "$DEST" "$BINARY_NAME" >> "$rc"
            log_info "Added $DEST to PATH in $rc"
        done
    fi
    log_warn "Restart shell or: export PATH=\"$DEST:\$PATH\""
}

# ── JSON Helpers ──────────────────────────────────────────────────────────────

# Merge a JSON object into a file's top-level key.
# Usage: _json_merge <file> <key> <json_object>
_json_merge() {
    local file="$1" key="$2" value="$3"

    [ "$DRY_RUN" -eq 1 ] && { log_info "(dry-run) Would configure $file"; return 0; }

    if [ ! -f "$file" ]; then
        echo "{ \"${key}\": ${value} }" > "$file"
        return 0
    fi

    if command -v jq &>/dev/null; then
        local tmpf; tmpf="$(mktemp)"
        jq --argjson val "$value" ".$key += \$val // .${key} = \$val" "$file" > "$tmpf" && mv "$tmpf" "$file"
    elif command -v node &>/dev/null; then
        node -e "
            const fs=require('fs'),f='${file}';
            const d=JSON.parse(fs.readFileSync(f,'utf8')||'{}');
            d['${key}']=Object.assign(d['${key}']||{},${value});
            fs.writeFileSync(f,JSON.stringify(d,null,2)+'\n');
        "
    elif command -v python3 &>/dev/null; then
        python3 -c "
import json,os
f='${file}'; k='${key}'; v=${value}
d=json.load(open(f)) if os.path.exists(f) and os.path.getsize(f)>0 else {}
d.setdefault(k,{}).update(v)
json.dump(d,open(f,'w'),indent=2); print()
"
    else
        log_warn "No JSON tool (jq/node/python3) — skipping $file"
        return 1
    fi
}

# Remove a key from mcpServers in a JSON file
_remove_mcp_from_file() {
    local file="$1" key="$2"
    [ -f "$file" ] || return 0
    command -v jq &>/dev/null || return 0
    local tmpf; tmpf="$(mktemp)"
    jq "del(.mcpServers.${key})" "$file" > "$tmpf" && mv "$tmpf" "$file"
}

# Remove DCP hooks from Claude Code settings
_remove_hooks_from_claude_settings() {
    local file="$1"
    [ -f "$file" ] || return 0
    command -v jq &>/dev/null || return 0
    local tmpf; tmpf="$(mktemp)"
    jq 'del(.hooks.PreToolUse[] | select(.hooks[]?.command | test("dcp-claude-hook"))) // . | del(.hooks.SessionStart[] | select(.hooks[]?.command | test("dcp-claude-hook"))) // .' "$file" > "$tmpf" && mv "$tmpf" "$file"
}

# ── MCP Provider Configuration ───────────────────────────────────────────────

configure_mcp_provider() {
    local provider_name="$1" settings_file="$2" json_key="$3" binary="$4"

    [ -f "$binary" ] || return 0

    # Check if the config dir exists (skip providers that aren't installed)
    local config_dir
    config_dir="$(dirname "$settings_file")"
    if [ ! -d "$config_dir" ]; then
        log_debug "${provider_name}: config dir not found, skipping"
        return 0
    fi

    log_step "Configuring MCP for ${provider_name}..."

    local mcp_entry
    mcp_entry=$(cat <<EOF
{
  "dcp": {
    "command": "${binary}",
    "args": [],
    "env": {}
  }
}
EOF
)

    if _json_merge "$settings_file" "$json_key" "$mcp_entry"; then
        log_info "${provider_name} MCP configured in ${settings_file}"
    fi
}

configure_all_mcp_providers() {
    local binary="$DEST/dcp-mcp"
    [ -f "$binary" ] || { log_warn "dcp-mcp not installed — skipping MCP setup"; return; }

    # ── JSON-based providers ───────────────────────────────────────────────

    # Claude Code
    configure_mcp_provider "Claude Code" "$HOME/.claude/settings.json" "mcpServers" "$binary"

    # Cursor
    local cursor_settings=""
    case "$(uname -s)" in
        Darwin) cursor_settings="$HOME/Library/Application Support/Cursor/User/settings.json" ;;
        *)      cursor_settings="$HOME/.config/Cursor/User/settings.json" ;;
    esac
    configure_mcp_provider "Cursor" "$cursor_settings" "mcp.servers" "$binary"

    # Cline
    configure_mcp_provider "Cline" "$HOME/.cline/mcp_settings.json" "mcpServers" "$binary"

    # Windsurf
    configure_mcp_provider "Windsurf" "$HOME/.codeium/windsurf/mcp_config.json" "mcpServers" "$binary"

    # VS Code Copilot
    local vscode_settings=""
    case "$(uname -s)" in
        Darwin) vscode_settings="$HOME/Library/Application Support/Code/User/settings.json" ;;
        *)      vscode_settings="$HOME/.config/Code/User/settings.json" ;;
    esac
    configure_mcp_provider "VS Code" "$vscode_settings" "github.copilot.chat.mcp" "$binary"

    # OpenCode — ~/.opencode.json (uses "type":"stdio" + env as array)
    configure_mcp_opencode "$binary"

    # Gemini CLI — ~/.gemini/settings.json
    configure_mcp_provider "Gemini CLI" "$HOME/.gemini/settings.json" "mcpServers" "$binary"

    # Amazon Q — ~/.aws/amazonq/mcp.json
    configure_mcp_provider "Amazon Q" "$HOME/.aws/amazonq/mcp.json" "mcpServers" "$binary"

    # Warp — ~/.warp/.mcp.json
    configure_mcp_provider "Warp" "$HOME/.warp/.mcp.json" "mcpServers" "$binary"

    # ── TOML-based providers ───────────────────────────────────────────────

    # Codex CLI — ~/.codex/config.toml
    configure_mcp_codex "$binary"
}

# ── OpenCode MCP (special: uses "type":"stdio" + env as array of "KEY=VALUE") ──

configure_mcp_opencode() {
    local binary="$1"
    local settings_file="$HOME/.opencode.json"
    local config_dir
    config_dir="$(dirname "$settings_file")"

    # OpenCode config can be in $XDG_CONFIG_HOME/opencode/.opencode.json too
    if [ ! -d "$config_dir" ] && [ -n "${XDG_CONFIG_HOME:-}" ] && [ -d "${XDG_CONFIG_HOME}/opencode" ]; then
        settings_file="${XDG_CONFIG_HOME}/opencode/.opencode.json"
    elif [ ! -f "$settings_file" ] && [ -d "$HOME/.config/opencode" ]; then
        settings_file="$HOME/.config/opencode/.opencode.json"
    fi

    config_dir="$(dirname "$settings_file")"
    if [ ! -d "$config_dir" ]; then
        log_debug "OpenCode: config dir not found, skipping"
        return 0
    fi

    log_step "Configuring MCP for OpenCode..."

    # OpenCode uses env as array of "KEY=VALUE" strings, not an object
    local mcp_entry
    mcp_entry=$(cat <<EOF
{
  "dcp": {
    "type": "stdio",
    "command": "${binary}",
    "args": [],
    "env": []
  }
}
EOF
)

    if _json_merge "$settings_file" "mcpServers" "$mcp_entry"; then
        log_info "OpenCode MCP configured in ${settings_file}"
    fi
}

# ── Codex CLI MCP (TOML-based config) ─────────────────────────────────────────

_toml_upsert_mcp() {
    local file="$1" server_name="$2" command_path="$3"

    [ "$DRY_RUN" -eq 1 ] && { log_info "(dry-run) Would configure $file"; return 0; }

    mkdir -p "$(dirname "$file")"

    # Ensure file exists
    [ -f "$file" ] || touch "$file"

    # Check if section already exists
    if grep -q "^\[mcp_servers\.${server_name}\]" "$file" 2>/dev/null; then
        # Update existing section: replace the command line
        local tmpf; tmpf="$(mktemp)"
        sed "s|^\(command = \).*|\1\"${command_path}\"|" "$file" > "$tmpf" && mv "$tmpf" "$file"
    else
        # Append new section
        cat >> "$file" <<TOML

[mcp_servers.${server_name}]
type = "stdio"
command = "${command_path}"
args = []
TOML
    fi
}

_toml_upsert_hooks() {
    local file="$1" hook_binary="$2"

    [ "$DRY_RUN" -eq 1 ] && { log_info "(dry-run) Would configure hooks in $file"; return 0; }

    mkdir -p "$(dirname "$file")"
    [ -f "$file" ] || touch "$file"

    # Only add if not already present
    if grep -q "dcp-claude-hook" "$file" 2>/dev/null; then
        return 0
    fi

    cat >> "$file" <<TOML

[[hooks.PreToolUse]]
matcher = "*"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "${hook_binary}"

[[hooks.SessionStart]]
matcher = "compact"

[[hooks.SessionStart.hooks]]
type = "command"
command = "${hook_binary} --on-compact"
TOML
}

configure_mcp_codex() {
    local binary="$1"
    local config_file="$HOME/.codex/config.toml"

    if [ ! -d "$(dirname "$config_file")" ]; then
        log_debug "Codex CLI: config dir not found, skipping"
        return 0
    fi

    log_step "Configuring MCP for Codex CLI..."

    if _toml_upsert_mcp "$config_file" "dcp" "$binary"; then
        log_info "Codex CLI MCP configured in ${config_file}"
    fi
}

# ── Gemini CLI Hooks ──────────────────────────────────────────────────────────

configure_gemini_hooks() {
    local hook_binary="$DEST/dcp-claude-hook"
    [ -f "$hook_binary" ] || return 0

    local settings_file="$HOME/.gemini/settings.json"
    [ -d "$(dirname "$settings_file")" ] || return 0

    log_step "Configuring hooks for Gemini CLI..."

    [ "$DRY_RUN" -eq 1 ] && { log_info "(dry-run) Would configure hooks in $settings_file"; return; }

    local hooks_json
    hooks_json=$(cat <<EOJSON
{
  "BeforeTool": [
    {
      "matcher": ".*",
      "sequential": false,
      "hooks": [
        {
          "type": "command",
          "command": "${hook_binary}",
          "name": "dcp-prune",
          "timeout": 5000,
          "description": "DCP context pruning"
        }
      ]
    }
  ],
  "SessionStart": [
    {
      "matcher": "startup",
      "hooks": [
        {
          "type": "command",
          "command": "${hook_binary} --on-compact",
          "name": "dcp-compact",
          "description": "DCP compact handler"
        }
      ]
    }
  ]
}
EOJSON
)

    if _json_merge "$settings_file" "hooks" "$hooks_json"; then
        log_info "Gemini CLI hooks configured in $settings_file"
    fi
}

# ── Amazon Q Hooks ────────────────────────────────────────────────────────────

configure_amazonq_hooks() {
    local hook_binary="$DEST/dcp-claude-hook"
    [ -f "$hook_binary" ] || return 0

    local config_file="$HOME/.aws/amazonq/mcp.json"
    [ -d "$(dirname "$config_file")" ] || return 0

    log_step "Configuring hooks for Amazon Q..."

    [ "$DRY_RUN" -eq 1 ] && { log_info "(dry-run) Would configure hooks in $config_file"; return; }

    local hooks_json
    hooks_json=$(cat <<EOJSON
{
  "preToolUse": [
    {
      "matcher": "*",
      "command": "${hook_binary}"
    }
  ],
  "stop": [
    {
      "command": "${hook_binary} --on-compact"
    }
  ]
}
EOJSON
)

    if _json_merge "$config_file" "hooks" "$hooks_json"; then
        log_info "Amazon Q hooks configured in $config_file"
    fi
}

# ── Codex CLI Hooks ───────────────────────────────────────────────────────────

configure_codex_hooks() {
    local hook_binary="$DEST/dcp-claude-hook"
    [ -f "$hook_binary" ] || return 0

    local config_file="$HOME/.codex/config.toml"
    [ -d "$(dirname "$config_file")" ] || return 0

    log_step "Configuring hooks for Codex CLI..."

    _toml_upsert_hooks "$config_file" "$hook_binary"
    log_info "Codex CLI hooks configured in $config_file"
}

# ── Claude Code Hooks ─────────────────────────────────────────────────────────

configure_claude_hooks() {
    local hook_binary="$DEST/dcp-claude-hook"
    [ -f "$hook_binary" ] || { log_warn "dcp-claude-hook not installed — skipping hook setup"; return; }

    log_step "Configuring Claude Code hooks..."

    local settings_dir="$HOME/.claude"
    local settings_file="${settings_dir}/settings.json"

    [ "$DRY_RUN" -eq 1 ] && { log_info "(dry-run) Would configure hooks in $settings_file"; return; }

    mkdir -p "$settings_dir"

    local hooks_json
    hooks_json=$(cat <<EOJSON
{
  "PreToolUse": [
    {
      "matcher": "*",
      "hooks": [
        {
          "type": "command",
          "command": "${hook_binary}"
        }
      ]
    }
  ],
  "SessionStart": [
    {
      "matcher": "compact",
      "hooks": [
        {
          "type": "command",
          "command": "${hook_binary} --on-compact"
        }
      ]
    }
  ]
}
EOJSON
)

    if _json_merge "$settings_file" "hooks" "$hooks_json"; then
        log_info "Claude Code hooks configured in $settings_file"
    fi
}

# ── Git Pre-commit Hook ──────────────────────────────────────────────────────

configure_git_hook() {
    log_step "Configuring git pre-commit hook..."

    if [ ! -d ".git" ]; then
        log_warn "Not in a git repository — skipping git hook"
        return
    fi

    [ "$DRY_RUN" -eq 1 ] && { log_info "(dry-run) Would install pre-commit hook"; return; }

    if [ -f "scripts/install-hooks.sh" ]; then
        bash scripts/install-hooks.sh --force
    else
        log_warn "scripts/install-hooks.sh not found — skipping"
    fi
}

# ── Main ──────────────────────────────────────────────────────────────────────

main() {
    echo -e "\n${BOLD}${CYAN}═══════════════════════════════════════════════${RESET}" >&2
    echo -e "${BOLD}  DCP Installer — Dynamic Context Pruning${RESET}" >&2
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════${RESET}" >&2

    acquire_lock
    TMP=$(mktemp -d)
    mkdir -p "$DEST"

    local platform; platform=$(detect_platform)
    local target;   target=$(resolve_target "$platform")

    log_step "Platform: $platform ($target)"
    log_step "Install dir: $DEST"

    # ── Install binaries ──────────────────────────────────────────────────
    if [ "$FROM_SOURCE" -eq 1 ]; then
        log_step "Building from source..."
        build_from_source
    else
        resolve_version
        log_step "Version: $VERSION"

        if [ "$DRY_RUN" -eq 1 ]; then
            log_info "(dry-run) Would download and install binaries to $DEST"
        else
            download_all_binaries "$platform" "$target"

            # Fallback: if dcp CLI didn't download, build from source
            if [ ! -f "$DEST/dcp" ]; then
                log_warn "Binary download failed — building from source..."
                build_from_source
            fi
        fi
    fi

    # ── Ensure in PATH ────────────────────────────────────────────────────
    maybe_add_path

    # ── Configure MCP providers ───────────────────────────────────────────
    if [ "$NO_MCP" -eq 0 ]; then
        configure_all_mcp_providers
    fi

    # ── Configure Claude Code hooks ───────────────────────────────────────
    if [ "$NO_HOOKS" -eq 0 ]; then
        configure_claude_hooks
        configure_codex_hooks
        configure_gemini_hooks
        configure_amazonq_hooks
    fi

    # ── Configure git pre-commit hook ─────────────────────────────────────
    if [ "$NO_GIT_HOOK" -eq 0 ]; then
        configure_git_hook
    fi

    # ── Verify ────────────────────────────────────────────────────────────
    if [ "$VERIFY" -eq 1 ] && [ -x "$DEST/dcp" ]; then
        "$DEST/dcp" --version
    fi

    # ── Summary ───────────────────────────────────────────────────────────
    echo "" >&2
    echo -e "${BOLD}${GREEN}═══════════════════════════════════════════════${RESET}" >&2
    echo -e "${BOLD}  DCP ${VERSION} installed successfully!${RESET}" >&2
    echo -e "${BOLD}${GREEN}═══════════════════════════════════════════════${RESET}" >&2
    echo "" >&2
    echo -e "  Binaries installed to:  ${CYAN}${DEST}${RESET}" >&2
    echo -e "  Configuration:          ${CYAN}~/.dynamic_context_pruning/config.jsonc${RESET}" >&2
    echo "" >&2

    if [ "$NO_MCP" -eq 0 ]; then
        echo -e "  ${BOLD}MCP Providers:${RESET}" >&2
        echo -e "    Claude Code    ${GREEN}✓${RESET}  (~/.claude/settings.json)" >&2
        [ -d "$HOME/.config/Cursor" ] || [ -d "$HOME/Library/Application Support/Cursor" ] \
            && echo -e "    Cursor         ${GREEN}✓${RESET}" >&2 \
            || echo -e "    Cursor         ${DIM}—${RESET}" >&2
        [ -d "$HOME/.cline" ] \
            && echo -e "    Cline          ${GREEN}✓${RESET}" >&2 \
            || echo -e "    Cline          ${DIM}—${RESET}" >&2
        [ -d "$HOME/.codeium" ] \
            && echo -e "    Windsurf       ${GREEN}✓${RESET}" >&2 \
            || echo -e "    Windsurf       ${DIM}—${RESET}" >&2
        [ -d "$HOME/.config/Code" ] || [ -d "$HOME/Library/Application Support/Code" ] \
            && echo -e "    VS Code        ${GREEN}✓${RESET}" >&2 \
            || echo -e "    VS Code        ${DIM}—${RESET}" >&2
        [ -f "$HOME/.opencode.json" ] || [ -d "$HOME/.config/opencode" ] \
            && echo -e "    OpenCode       ${GREEN}✓${RESET}" >&2 \
            || echo -e "    OpenCode       ${DIM}—${RESET}" >&2
        [ -d "$HOME/.codex" ] \
            && echo -e "    Codex CLI      ${GREEN}✓${RESET}" >&2 \
            || echo -e "    Codex CLI      ${DIM}—${RESET}" >&2
        [ -d "$HOME/.gemini" ] \
            && echo -e "    Gemini CLI     ${GREEN}✓${RESET}" >&2 \
            || echo -e "    Gemini CLI     ${DIM}—${RESET}" >&2
        [ -d "$HOME/.aws/amazonq" ] \
            && echo -e "    Amazon Q       ${GREEN}✓${RESET}" >&2 \
            || echo -e "    Amazon Q       ${DIM}—${RESET}" >&2
        [ -d "$HOME/.warp" ] \
            && echo -e "    Warp           ${GREEN}✓${RESET}" >&2 \
            || echo -e "    Warp           ${DIM}—${RESET}" >&2
    fi

    if [ "$NO_HOOKS" -eq 0 ]; then
        echo "" >&2
        echo -e "  ${BOLD}Hooks configured:${RESET}" >&2
        echo -e "    Claude Code    ${GREEN}✓${RESET}  (PreToolUse + SessionStart)" >&2
        [ -d "$HOME/.codex" ] \
            && echo -e "    Codex CLI      ${GREEN}✓${RESET}  (PreToolUse + SessionStart)" >&2 \
            || echo -e "    Codex CLI      ${DIM}—${RESET}" >&2
        [ -d "$HOME/.gemini" ] \
            && echo -e "    Gemini CLI     ${GREEN}✓${RESET}  (BeforeTool + SessionStart)" >&2 \
            || echo -e "    Gemini CLI     ${DIM}—${RESET}" >&2
        [ -d "$HOME/.aws/amazonq" ] \
            && echo -e "    Amazon Q       ${GREEN}✓${RESET}  (preToolUse + stop)" >&2 \
            || echo -e "    Amazon Q       ${DIM}—${RESET}" >&2
    fi

    echo "" >&2
    echo -e "  ${BOLD}Quick start:${RESET}" >&2
    echo -e "    ${CYAN}dcp --help${RESET}           Show CLI help" >&2
    echo -e "    ${CYAN}dcp context${RESET}          Show current session context" >&2
    echo -e "    ${CYAN}dcp stats${RESET}            Show pruning statistics" >&2
    echo "" >&2

    # Print installed version
    if [ -x "$DEST/dcp" ]; then
        local installed_version
        installed_version="$("$DEST/dcp" --version 2>/dev/null || echo "unknown")"
        log_info "Verified: dcp $installed_version"
    fi
}

# curl|bash safety: buffer entire script before executing
if [[ "${BASH_SOURCE[0]:-}" == "${0:-}" ]] || [[ -z "${BASH_SOURCE[0]:-}" ]]; then
    { main "$@"; }
fi
