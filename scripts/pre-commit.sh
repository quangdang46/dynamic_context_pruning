#!/usr/bin/env bash
# =============================================================================
# pre-commit.sh — DCP pre-commit quality gate
# =============================================================================
# Runs before every `git commit` to ensure the workspace passes:
#   1. cargo fmt --check
#   2. cargo clippy --all-features -- -D warnings
#   3. cargo test --all-features
#   4. cargo build --all-features
#   5. cargo audit (security vulnerabilities)
#
# When staged files are detected, checks are scoped to the relevant crates.
# Any failure blocks the commit.
# =============================================================================

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
RESET='\033[0m'

FAILED=0
CHECKS_PASSED=0

header() {
    echo ""
    echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo -e "${BOLD}$1${RESET}"
    echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
}

pass() { echo -e "  ${GREEN}✓${RESET} $1"; ((CHECKS_PASSED++)); }
fail() { echo -e "  ${RED}✗${RESET} $1"; ((FAILED++)); }
warn() { echo -e "  ${YELLOW}!${RESET} $1"; }
info() { echo -e "  ${BOLD}•${RESET} $1"; }
section() { echo -e "\n${BOLD}▶ $1${RESET}"; }
has() { command -v "$1" >/dev/null 2>&1; }

# -----------------------------------------------------------------------------
# Bead → crate/files mapping
# Maps bead ID suffix → crate name (for scoped checks)
# -----------------------------------------------------------------------------
declare -A BEAD_CRATES=(
    ["yz0"]="dcp-permissions" ["ykb"]="dcp-permissions"
    ["erk"]="dcp-permissions" ["pca"]="dcp-permissions"
    ["ws8"]="dcp-traits"
    ["7jq"]="dcp-messages" ["2x2"]="dcp-messages" ["5g5"]="dcp-messages"
    ["9b2"]="dcp-messages" ["c85"]="dcp-messages" ["oxs"]="dcp-messages"
    ["pk4"]="dcp-messages" ["dgc"]="dcp-messages" ["wt3"]="dcp-messages"
    ["02w"]="dcp-messages"
    ["ylt"]="dcp-prune"
    ["3f7"]="dcp-compress"
    ["1z8"]="dcp-prompts" ["aj7"]="dcp-prompts" ["7tu"]="dcp-prompts"
    ["m30"]="dcp-prompts" ["lu2"]="dcp-prompts"
    ["aw4"]="dcp-notification"
    ["xtm"]="dcp-config" ["1y7"]="dcp-config"
    ["yoa"]="dcp-cli" ["0ed"]="dcp-cli" ["xq4"]="dcp-cli" ["ppp"]="dcp-cli"
    ["djp"]="dcp-cli" ["dm9"]="dcp-claude-hook" ["n8c"]="dcp-mcp"
    ["dz6"]="dcp-rig"
    ["mu3"]="dcp-permissions" ["mwr"]="dcp-notification"
    ["zet"]="dcp-storage"
    ["l65"]="dcp-core"
    ["vve"]="dcp-telemetry"
    ["hct"]="dcp-tokens"
    ["cn5"]="dynamic_context_pruning"
    ["35d"]="dynamic_context_pruning"
    ["y5x"]="dynamic_context_pruning" ["mlt"]="dynamic_context_pruning"
    ["0ry"]="dynamic_context_pruning" ["dp2"]="dynamic_context_pruning"
    ["quw"]="dcp-types" ["tuz"]="dcp-core" ["uci"]="dcp-state"
    ["ukh"]="dcp-state" ["s5z"]="dcp-prompts"
    ["842"]="dcp-nudges" ["9t6"]="dcp-protected" ["q01"]="dcp-cli"
)

# Staged files → relevant crates (deduplicated)
get_staged_crates() {
    local crates=()
    local seen=()
    # Sort staged files by depth (deepest first) to avoid duplicates
    for f in "${STAGED_FILES[@]}"; do
        local crate=""
        case "$f" in
            crates/dcp-permissions/*)    crate="dcp-permissions" ;;
            crates/dcp-traits/*)         crate="dcp-traits" ;;
            crates/dcp-messages/*)       crate="dcp-messages" ;;
            crates/dcp-prune/*)          crate="dcp-prune" ;;
            crates/dcp-compress/*)       crate="dcp-compress" ;;
            crates/dcp-prompts/*)        crate="dcp-prompts" ;;
            crates/dcp-notification/*)   crate="dcp-notification" ;;
            crates/dcp-config/*)        crate="dcp-config" ;;
            crates/dcp-cli/*)           crate="dcp-cli" ;;
            crates/dcp-claude-hook/*)   crate="dcp-claude-hook" ;;
            crates/dcp-mcp/*)           crate="dcp-mcp" ;;
            crates/dcp-rig/*)           crate="dcp-rig" ;;
            crates/dcp-storage/*)       crate="dcp-storage" ;;
            crates/dcp-core/*)          crate="dcp-core" ;;
            crates/dcp-telemetry/*)     crate="dcp-telemetry" ;;
            crates/dcp-tokens/*)        crate="dcp-tokens" ;;
            crates/dcp-state/*)         crate="dcp-state" ;;
            crates/dcp-protected/*)     crate="dcp-protected" ;;
            crates/dcp-nudges/*)        crate="dcp-nudges" ;;
            crates/dcp-types/*)         crate="dcp-types" ;;
            examples/*)                  crate="dynamic_context_pruning" ;;
            .github/workflows/*)         crate="dynamic_context_pruning" ;;
            *.rs)                        crate="dynamic_context_pruning" ;;
            Cargo.toml|rust-toolchain.toml) crate="dynamic_context_pruning" ;;
        esac
        if [[ -n "$crate" ]]; then
            local is_new=true
            for c in "${seen[@]}"; do
                [[ "$c" == "$crate" ]] && is_new=false && break
            done
            if $is_new; then
                seen+=("$crate")
                crates+=("$crate")
            fi
        fi
    done
    echo "${crates[@]}"
}

# Determine which crates to check
get_check_target() {
    local target=""
    if [[ ${#STAGED_FILES[@]} -gt 0 && ${#STAGED_FILES[0]} -gt 0 ]]; then
        local staged_crates=($(get_staged_crates))
        if [[ ${#staged_crates[@]} -gt 0 ]]; then
            target="${staged_crates[*]}"
        fi
    fi
    echo "${target:-all}"
}

# Run cargo fmt check (scope-aware)
run_fmt() {
    section "Check 1: cargo fmt"
    if cargo fmt --all -- --check >/dev/null 2>&1; then
        pass "fmt OK"
    else
        fail "fmt violations — run: cargo fmt --all"
        ((FAILED++))
    fi
}

# Run cargo clippy (scope-aware)
run_clippy() {
    section "Check 2: cargo clippy"
    local output
    if output=$(cargo clippy --all-targets --all-features -- -D warnings 2>&1); then
        pass "clippy OK"
    else
        fail "clippy warnings/errors"
        echo "$output" | grep -E "^error" | head -10 || true
        ((FAILED++))
    fi
}

# Run cargo test (scope-aware)
run_tests() {
    section "Check 3: cargo test"
    local output
    if output=$(cargo test --all-features 2>&1); then
        local test_result=$(echo "$output" | grep -E '^test result:' | head -1 || echo "")
        [[ -n "$test_result" ]] && pass "$test_result" || pass "tests OK"
    else
        fail "tests failed"
        ((FAILED++))
    fi
}

# Run cargo build (scope-aware)
run_build() {
    section "Check 4: cargo build"
    if cargo build --all-features >/dev/null 2>&1; then
        pass "build OK"
    else
        fail "build failed"
        ((FAILED++))
    fi
}

# Run cargo audit
run_audit() {
    section "Check 5: cargo audit"
    if has cargo-audit; then
        local output
        if output=$(cargo audit 2>&1); then
            pass "audit OK"
        else
            fail "cargo audit found vulnerabilities"
            echo "$output" | grep -E "(vulnerable|advisory)" | head -5 || true
            ((FAILED++))
        fi
    else
        warn "cargo-audit not installed — install: cargo install cargo-audit"
    fi
}

# -----------------------------------------------------------------------------
# Parse arguments
# -----------------------------------------------------------------------------

TARGET_BEAD=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --bead)
            TARGET_BEAD="${2#dynamic_context_pruning-}"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--bead BEAD_ID]"
            echo "  --bead ID   Scope checks to a specific bead's crate"
            echo "  Runs automatically as git pre-commit hook."
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Detect staged files
if has git && git rev-parse --git-dir >/dev/null 2>&1; then
    mapfile -t STAGED_FILES < <(git diff --cached --name-only --diff-filter=ACM 2>/dev/null || true)
fi

# -----------------------------------------------------------------------------
# Environment
# -----------------------------------------------------------------------------

header "Environment"
if has cargo; then
    pass "cargo found: $(cargo --version | cut -d' ' -f2)"
else
    fail "cargo not found"
    exit 1
fi
has rustfmt && pass "rustfmt found" || warn "rustfmt not found"
has clippy && pass "clippy found" || warn "clippy not found"
has cargo-audit && pass "cargo-audit found" || warn "cargo-audit not installed"

echo ""
info "Repo: $REPO_ROOT"
[[ -n "$TARGET_BEAD" ]] && info "Bead: $TARGET_BEAD"
[[ ${#STAGED_FILES[@]} -gt 0 ]] && info "Staged files: ${#STAGED_FILES[@]}"

# Determine scope
CHECK_TARGET=$(get_check_target)
echo ""
if [[ "$CHECK_TARGET" == "all" ]]; then
    info "Scope: full workspace"
else
    info "Scope: ${CHECK_TARGET}"
fi

# -----------------------------------------------------------------------------
# Run checks (always full workspace for now — Rust cross-crate analysis)
# -----------------------------------------------------------------------------

run_fmt
run_clippy
run_tests
run_build
run_audit

# -----------------------------------------------------------------------------
# Summary
# -----------------------------------------------------------------------------

header "Summary"
echo ""
if [[ $FAILED -eq 0 ]]; then
    echo -e "  ${GREEN}${BOLD}All checks passed ✓${RESET}"
    echo -e "  ${GREEN}Ready to commit.${RESET}"
    exit 0
else
    echo -e "  ${RED}${BOLD}$FAILED check(s) failed ✗${RESET}"
    echo ""
    echo -e "  ${RED}Commit blocked.${RESET}"
    echo ""
    echo "Quick fixes:"
    echo "  cargo fmt --all"
    echo "  cargo clippy --fix --allow-dirty --allow-staged --all-features"
    echo "  cargo test --all-features"
    echo "  cargo build --all-features"
    exit 1
fi
