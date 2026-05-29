#!/usr/bin/env bash
# =============================================================================
# pre-commit.sh — DCP pre-commit quality gate
# =============================================================================
# Runs before every `git commit` to ensure the workspace passes:
#   1. cargo fmt --check
#   2. cargo clippy --all-features -- -D warnings
#   3. cargo test --all-features
#   4. cargo build --all-features
#
# Any failure blocks the commit. Run manually with:
#   ./scripts/pre-commit.sh [--bead BEAD_ID]
# =============================================================================

set -euo pipefail

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
RESET='\033[0m'

# Track overall status
FAILED=0
CHECKS_PASSED=0

# -----------------------------------------------------------------------------
# Helper functions
# -----------------------------------------------------------------------------

header() {
    echo ""
    echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo -e "${BOLD}$1${RESET}"
    echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
}

pass() {
    echo -e "  ${GREEN}✓${RESET} $1"
    ((CHECKS_PASSED++))
}

fail() {
    echo -e "  ${RED}✗${RESET} $1"
    ((FAILED++))
}

warn() {
    echo -e "  ${YELLOW}!${RESET} $1"
}

info() {
    echo -e "  ${BOLD}•${RESET} $1"
}

section() {
    echo ""
    echo -e "${BOLD}▶ $1${RESET}"
}

# Check if a binary exists
has() {
    command -v "$1" >/dev/null 2>&1
}

# Run a cargo check with common flags
cargo_check() {
    cargo "$1" --all-features --manifest-path "${REPO_ROOT}/Cargo.toml" 2>&1
}

# -----------------------------------------------------------------------------
# Parse arguments
# -----------------------------------------------------------------------------

TARGET_BEAD=""
STAGED_FILES=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --bead)
            TARGET_BEAD="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--bead BEAD_ID]"
            echo ""
            echo "Options:"
            echo "  --bead BEAD_ID   Run checks only for files related to a specific bead"
            echo "  --help, -h       Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Resolve repo root
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Detect staged files (for future use — all files checked regardless)
if command -v git >/dev/null 2>&1 && git rev-parse --git-dir >/dev/null 2>&1; then
    mapfile -t STAGED_FILES < <(git diff --cached --name-only --diff-filter=ACM 2>/dev/null || true)
fi

# -----------------------------------------------------------------------------
# Environment checks
# -----------------------------------------------------------------------------

header "Environment"

if has cargo; then
    pass "cargo found: $(cargo --version | cut -d' ' -f2)"
else
    fail "cargo not found — install Rust 1.85+"
    exit 1
fi

if has rustfmt; then
    pass "rustfmt found"
else
    warn "rustfmt not found — install with: rustup component add rustfmt"
fi

if has clippy; then
    pass "clippy found"
else
    warn "clippy not found — install with: rustup component add clippy"
fi

echo ""
info "Repository: $REPO_ROOT"
if [[ -n "${TARGET_BEAD:-}" ]]; then
    info "Target bead: $TARGET_BEAD"
fi
if [[ ${#STAGED_FILES[@]} -gt 0 ]]; then
    info "Staged files: ${#STAGED_FILES[@]}"
fi

# -----------------------------------------------------------------------------
# Check 1: Formatting
# -----------------------------------------------------------------------------

section "Check 1: cargo fmt --check"

if ! cargo fmt --all -- --check >/dev/null 2>&1; then
    fail "code formatting violations — run: cargo fmt --all"
    echo ""
    echo "Diff:"
    cargo fmt --all -- --check 2>&1 | head -30 || true
    ((FAILED++))
else
    pass "formatting OK"
fi

# -----------------------------------------------------------------------------
# Check 2: Clippy linting
# -----------------------------------------------------------------------------

section "Check 2: cargo clippy --all-features -- -D warnings"

CLIPPY_OUTPUT=$(cargo clippy --all-targets --all-features -- \
    -D warnings 2>&1) || CLIPPY_EXIT=$?

if [[ ${CLIPPY_EXIT:-0} -eq 0 ]]; then
    pass "clippy OK — no warnings"
else
    fail "clippy found warnings or errors"
    echo ""
    echo "Clippy output (last 50 lines):"
    echo "$CLIPPY_OUTPUT" | tail -50
    ((FAILED++))
fi

# -----------------------------------------------------------------------------
# Check 3: Tests
# -----------------------------------------------------------------------------

section "Check 3: cargo test --all-features"

TEST_OUTPUT=$(cargo test --all-features 2>&1) || TEST_EXIT=$?

if [[ ${TEST_EXIT:-0} -eq 0 ]]; then
    # Extract test summary
    TEST_COUNT=$(echo "$TEST_OUTPUT" | grep -E '^test result:' | head -1 || echo "")
    if [[ -n "$TEST_COUNT" ]]; then
        pass "all tests pass — $TEST_COUNT"
    else
        pass "all tests pass"
    fi
else
    fail "tests failed"
    echo ""
    echo "Test output (last 50 lines):"
    echo "$TEST_OUTPUT" | tail -50
    ((FAILED++))
fi

# -----------------------------------------------------------------------------
# Check 4: Build
# -----------------------------------------------------------------------------

section "Check 4: cargo build --all-features"

BUILD_OUTPUT=$(cargo build --all-features 2>&1) || BUILD_EXIT=$?

if [[ ${BUILD_EXIT:-0} -eq 0 ]]; then
    pass "build OK"
else
    fail "build failed"
    echo ""
    echo "Build output (last 30 lines):"
    echo "$BUILD_OUTPUT" | tail -30
    ((FAILED++))
fi

# -----------------------------------------------------------------------------
# Summary
# -----------------------------------------------------------------------------

header "Summary"

TOTAL_CHECKS=4
echo ""
if [[ $FAILED -eq 0 ]]; then
    echo -e "  ${GREEN}${BOLD}All $TOTAL_CHECKS checks passed ✓${RESET}"
    echo ""
    echo -e "  ${GREEN}Ready to commit.${RESET}"
    exit 0
else
    echo -e "  ${RED}${BOLD}$FAILED of $TOTAL_CHECKS checks failed ✗${RESET}"
    echo ""
    echo -e "  ${RED}Commit blocked. Fix the issues above before committing.${RESET}"
    echo ""
    echo "Quick fixes:"
    echo "  cargo fmt --all           # fix formatting"
    echo "  cargo clippy --fix --allow-dirty --allow-staged --all-features  # auto-fix clippy"
    echo "  cargo test --all-features # run tests"
    exit 1
fi
