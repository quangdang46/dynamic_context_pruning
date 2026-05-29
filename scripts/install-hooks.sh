#!/usr/bin/env bash
# =============================================================================
# install-hooks.sh — Install DCP git hooks
# =============================================================================
# Symlinks scripts/pre-commit.sh into .git/hooks/pre-commit so it runs before
# every `git commit`.
#
# Usage:
#   ./scripts/install-hooks.sh          # install
#   ./scripts/install-hooks.sh --force # re-install (overwrite)
#   ./scripts/install-hooks.sh --uninstall # remove hook
# =============================================================================

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
RESET='\033[0m'

HOOK_NAME="pre-commit"
HOOK_SOURCE="../../scripts/pre-commit.sh"
HOOK_TARGET_DIR=".git/hooks"
HOOK_TARGET="${HOOK_TARGET_DIR}/${HOOK_NAME}"

# Resolve repo root (scripts/ -> repo root)
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
    echo "Usage: $0 [--force|--uninstall]"
    echo ""
    echo "Options:"
    echo "  --force       Overwrite existing hook (default: skip if exists)"
    echo "  --uninstall    Remove the installed pre-commit hook"
    echo "  --help, -h     Show this help message"
}

uninstall() {
    if [[ -L "${REPO_ROOT}/${HOOK_TARGET}" ]]; then
        rm "${REPO_ROOT}/${HOOK_TARGET}"
        echo -e "  ${GREEN}✓${RESET} Removed ${HOOK_TARGET}"
    elif [[ -f "${REPO_ROOT}/${HOOK_TARGET}" ]]; then
        echo -e "  ${YELLOW}!${RESET} ${HOOK_TARGET} exists but is not a symlink — manual removal needed:"
        echo "    rm ${REPO_ROOT}/${HOOK_TARGET}"
    else
        echo -e "  ${GREEN}✓${RESET} No pre-commit hook installed — nothing to remove"
    fi
    exit 0
}

FORCE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --force) FORCE=true; shift ;;
        --uninstall) uninstall ;;
        --help|-h) usage; exit 0 ;;
        *) echo "Unknown option: $1"; usage; exit 1 ;;
    esac
done

# Ensure .git/hooks directory exists
mkdir -p "${REPO_ROOT}/${HOOK_TARGET_DIR}"

# Check if hook already exists
if [[ -e "${REPO_ROOT}/${HOOK_TARGET}" && "${FORCE}" == "false" ]]; then
    echo -e "  ${YELLOW}!${RESET} ${HOOK_TARGET} already exists"
    echo ""
    echo -e "  To re-install: ${BOLD}$0 --force${RESET}"
    echo -e "  To uninstall: ${BOLD}$0 --uninstall${RESET}"
    echo ""
    echo -e "  Current hook:"
    ls -la "${REPO_ROOT}/${HOOK_TARGET}"
    exit 1
fi

# Remove existing hook if force
if [[ -e "${REPO_ROOT}/${HOOK_TARGET}" || -L "${REPO_ROOT}/${HOOK_TARGET}" ]]; then
    rm "${REPO_ROOT}/${HOOK_TARGET}"
fi

# Create symlink (relative path from .git/hooks/ to scripts/)
cd "${REPO_ROOT}/${HOOK_TARGET_DIR}"
ln -s "../../scripts/pre-commit.sh" "${HOOK_NAME}"

echo -e "  ${GREEN}✓${RESET} Installed pre-commit hook"
echo ""
echo -e "  ${GREEN}Pre-commit hook active!${RESET}"
echo "  It will run before every commit:"
echo "    1. cargo fmt --check"
echo "    2. cargo clippy --all-features -- -D warnings"
echo "    3. cargo test --all-features"
echo "    4. cargo build --all-features"
echo ""
echo -e "  ${YELLOW}Tip:${RESET} To bypass the hook temporarily: git commit --no-verify"
