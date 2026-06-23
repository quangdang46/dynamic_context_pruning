#!/usr/bin/env bash
# =============================================================================
# install.sh — Install DCP (Dynamic Context Pruning) from GitHub releases
# =============================================================================
# Downloads the dcp CLI binary for your platform and sets up a git
# pre-commit hook (optional).
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/quangdang46/dynamic_context_pruning/main/install.sh | bash
#   ./install.sh                          # install latest
#   ./install.sh --version v0.1.0         # install specific version
#   ./install.sh --dest ~/bin             # custom install directory
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
QUIET=0; EASY=0; VERIFY=0; FROM_SOURCE=0; UNINSTALL=0; DRY_RUN=0; CHECK=0
NO_GIT_HOOK=0
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
  --no-git-hook        Skip git pre-commit hook
  --check              Check DCP installation
  --dry-run            Preview what would be done without changes
  --quiet, -q          Suppress progress output
  --uninstall          Remove DCP and all configurations
  -h, --help           Show this help

${BOLD}Features:${RESET}
  • Git pre-commit hook (optional)

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
        # --no-mcp removed; MCP server no longer ships with DCP
        --no-git-hook)  NO_GIT_HOOK=1;          shift;;
        --dry-run)      DRY_RUN=1;              shift;;
--quiet|-q)     QUIET=1;                shift;;
        --check)       CHECK=1;                shift;;
        --uninstall)   UNINSTALL=1;            shift;;
        -h|--help)      usage;;
        *) shift;;
    esac
done

# ── Uninstall ─────────────────────────────────────────────────────────────────

do_uninstall() {
    log_step "Uninstalling DCP..."

    # Remove binaries
    for bin in dcp; do
        if [ -f "$DEST/$bin" ]; then
            rm -f "$DEST/$bin"
            log_info "Removed $DEST/$bin"
        fi
    done

    # Remove PATH entries

    # Remove PATH entries
    for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
        [ -f "$rc" ] && sed -i "/${BINARY_NAME} installer/d" "$rc" 2>/dev/null || true
    done

    echo "" >&2
    log_info "DCP uninstalled successfully"
    exit 0
}

[ "$UNINSTALL" -eq 1 ] && { acquire_lock; do_uninstall; }

# ── Health Check ────────────────────────────────────────────────────────────────

do_check() {
    echo "" >&2
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════${RESET}" >&2
    echo -e "${BOLD}  DCP Installation Check${RESET}" >&2
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════${RESET}" >&2
    echo "" >&2

    local exit_code=0

    # ── 1. Binary checks ──────────────────────────────────────────────────────
    echo -e "  ${BOLD}Binaries:${RESET}" >&2

    for bin in dcp; do
        local bin_path="$DEST/$bin"
        if [ -f "$bin_path" ] && [ -x "$bin_path" ]; then
            local ver
            ver=$("$bin_path" --version 2>/dev/null || echo "unknown")
            echo -e "    ${GREEN}✓${RESET} ${bin}: ${CYAN}${ver}${RESET}" >&2
        else
            echo -e "    ${RED}✗${RESET} ${bin}: ${DIM}not found or not executable${RESET}" >&2
            exit_code=1
        fi
    done

    echo "" >&2
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════${RESET}" >&2

    if [ $exit_code -eq 0 ]; then
        echo -e "  ${GREEN}All checks passed ✓${RESET}" >&2
    else
        echo -e "  ${YELLOW}Some checks failed — run installer to fix${RESET}" >&2
    fi
    echo "" >&2

    exit $exit_code
}

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
    for bin_name in dcp; do
        local pkg="dcp-cli"
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

    for bin_name in dcp; do
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

    if [ "$CHECK" -eq 1 ]; then
        do_check
    fi

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
