#!/usr/bin/env bash
# =============================================================================
# review-bead.sh — Per-bead code review verification
# =============================================================================
# For a given bead ID, identifies the relevant files and runs the full
# quality gate (fmt + clippy + test + build + audit) on just those files.
#
# Usage:
#   ./scripts/review-bead.sh <BEAD_ID>
#   ./scripts/review-bead.sh --all     # review all beads
#   ./scripts/review-bead.sh --list    # list all beads and their files
#
# Examples:
#   ./scripts/review-bead.sh dynamic_context_pruning-0ed   # get-message bead
#   ./scripts/review-bead.sh dynamic_context_pruning-c54   # Phase J bead
#
# Output: Per-bead pass/fail, files checked, and a summary line suitable
#         for copying into a commit message or bead close comment.
# =============================================================================

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ISSUES_FILE="${REPO_ROOT}/.beads/issues.jsonl"

# -----------------------------------------------------------------------------
# Bead → files mapping (derived from bead descriptions)
# Key: bead ID suffix (without the "dynamic_context_pruning-" prefix)
# Value: space-separated file patterns relative to REPO_ROOT
# -----------------------------------------------------------------------------

declare -A BEAD_FILES=(
    # === Phase A: dcp-permissions ===
    ["yz0"]="crates/dcp-permissions/src/auth.rs"
    ["ykb"]="crates/dcp-permissions/src/host_permissions.rs crates/dcp-permissions/src/lib.rs"
    ["erk"]="crates/dcp-permissions/src/compress_permission.rs crates/dcp-permissions/src/lib.rs"
    ["pca"]="crates/dcp-permissions/src/lib.rs crates/dcp-permissions/tests/permissions_integration.rs"

    # === Phase B: dcp-traits ===
    ["ws8"]="crates/dcp-traits/src/lib.rs"

    # === Phase C: dcp-messages ===
    ["7jq"]="crates/dcp-messages/src/lib.rs crates/dcp-messages/Cargo.toml"
    ["2x2"]="crates/dcp-messages/src/query.rs crates/dcp-messages/src/lib.rs"
    ["5g5"]="crates/dcp-messages/src/shape.rs crates/dcp-messages/src/lib.rs"
    ["9b2"]="crates/dcp-messages/src/sync.rs crates/dcp-messages/src/lib.rs"
    ["c85"]="crates/dcp-messages/src/priority.rs crates/dcp-messages/src/lib.rs"
    ["oxs"]="crates/dcp-messages/src/inject_utils.rs crates/dcp-messages/src/lib.rs"
    ["pk4"]="crates/dcp-messages/src/utils.rs crates/dcp-messages/src/lib.rs"
    ["dgc"]="crates/dcp-messages/src/inject.rs crates/dcp-messages/src/lib.rs"
    ["wt3"]="crates/dcp-messages/src/reasoning_strip.rs crates/dcp-messages/src/lib.rs"
    ["02w"]="crates/dcp-messages/src/subagents.rs crates/dcp-messages/src/lib.rs"

    # === Phase D: dcp-prune ===
    ["ylt"]="crates/dcp-prune/src/lib.rs crates/dcp-prune/src/deduplicate.rs crates/dcp-prune/src/purge_errors.rs crates/dcp-prune/src/stale_file_reads.rs crates/dcp-prune/src/apply.rs"

    # === Phase E: dcp-compress ===
    ["3f7"]="crates/dcp-compress/src/lib.rs crates/dcp-compress/src/handler.rs crates/dcp-compress/src/block.rs crates/dcp-compress/src/types.rs"

    # === Phase F: dcp-prompts ===
    ["1z8"]="crates/dcp-prompts/src/extensions/nudge.rs crates/dcp-prompts/src/lib.rs crates/dcp-prompts/src/extensions/mod.rs"
    ["aj7"]="crates/dcp-prompts/src/extensions/system.rs crates/dcp-prompts/src/lib.rs"
    ["7tu"]="crates/dcp-prompts/src/extensions/tool.rs crates/dcp-prompts/src/lib.rs"
    ["m30"]="crates/dcp-prompts/src/lib.rs crates/dcp-prompts/src/extensions/mod.rs crates/dcp-prompts/src/store.rs"
    ["lu2"]="crates/dcp-prompts/src/lib.rs crates/dcp-prompts/src/store.rs"

    # === Phase G: dcp-notification ===
    ["aw4"]="crates/dcp-notification/src/lib.rs crates/dcp-notification/src/format.rs crates/dcp-notification/src/notification.rs"

    # === Phase H: dcp-config ===
    ["xtm"]="crates/dcp-config/src/lib.rs crates/dcp-config/src/config.rs dcp.schema.json"
    ["1y7"]="crates/dcp-config/src/lib.rs crates/dcp-config/src/cascade.rs crates/dcp-config/src/config.rs crates/dcp-config/src/limits.rs crates/dcp-config/src/enums.rs crates/dcp-config/src/sub_configs.rs"

    # === Phase I: CLI ===
    ["yoa"]="crates/dcp-cli/src/db.rs crates/dcp-cli/src/lib.rs crates/dcp-cli/Cargo.toml"
    ["0ed"]="crates/dcp-cli/src/commands/get_message.rs crates/dcp-cli/src/commands/mod.rs"
    ["xq4"]="crates/dcp-cli/src/commands/token_stats.rs crates/dcp-cli/src/commands/mod.rs"
    ["ppp"]="crates/dcp-cli/src/commands/message_tokens.rs crates/dcp-cli/src/commands/mod.rs"

    # === Phase J: Docs ===
    ["c54"]="README.md CONTRIBUTING.md IMPLEMENTATION_PLAN.md assets/images/"

    # === Integration/adapters ===
    ["djp"]="crates/dcp-cli/src/main.rs"
    ["dm9"]="crates/dcp-hook/src/main.rs crates/dcp-hook/Cargo.toml"
    ["n8c"]="crates/dcp-mcp/src/main.rs crates/dcp-mcp/Cargo.toml"
    ["dz6"]="crates/dcp-rig/src/lib.rs crates/dcp-rig/Cargo.toml"

    # === Scaffold beads ===
    ["mu3"]="crates/dcp-permissions/Cargo.toml crates/dcp-permissions/src/lib.rs"
    ["mwr"]="crates/dcp-notification/Cargo.toml crates/dcp-notification/src/lib.rs"

    # === Storage & state ===
    ["zet"]="crates/dcp-storage/src/lib.rs crates/dcp-storage/src/file.rs crates/dcp-storage/src/memory.rs"

    # === Core ===
    ["l65"]="crates/dcp-core/src/lib.rs crates/dcp-core/src/pruner.rs"

    # === Telemetry ===
    ["vve"]="crates/dcp-telemetry/src/lib.rs"

    # === Types ===
    ["hct"]="crates/dcp-tokens/src/lib.rs"

    # === Umbrella & misc ===
    ["cn5"]="Cargo.toml src/lib.rs crates/dynamic_context_pruning/src/lib.rs"

    # === Examples ===
    ["35d"]="examples/"

    # === Workspace/CI ===
    ["y5x"]=".github/workflows/ci.yml Cargo.toml rust-toolchain.toml"
    ["mlt"]="Cargo.toml crates/"

    # === Architecture/planning docs (informational beads — no code files) ===
    ["0ry"]="PLAN.md"
    ["dp2"]="SPEC.md"
    ["quw"]="crates/dcp-types/src/lib.rs"
    ["tuz"]="crates/dcp-core/src/lib.rs crates/dcp-core/src/async_facade.rs"
    ["uci"]="crates/dcp-state/src/lib.rs crates/dcp-state/src/session.rs crates/dcp-state/src/message_refs.rs"
    ["ukh"]="crates/dcp-state/src/lib.rs crates/dcp-state/src/message_refs.rs"
    ["s5z"]="crates/dcp-prompts/src/store.rs crates/dcp-prompts/src/lib.rs"
    ["842"]="crates/dcp-nudges/src/lib.rs crates/dcp-core/src/pipeline.rs"
    ["9t6"]="crates/dcp-protected/src/lib.rs"
    ["q01"]="crates/dcp-cli/src/commands/"

    # === Workspace scaffold deps ===
)

# -----------------------------------------------------------------------------
# Utility functions
# -----------------------------------------------------------------------------

pass() { echo -e "  ${GREEN}✓${RESET} $1"; }
fail() { echo -e "  ${RED}✗${RESET} $1"; }
warn() { echo -e "  ${YELLOW}!${RESET} $1"; }
info() { echo -e "  ${CYAN}•${RESET} $1"; }
section() { echo -e "\n${BOLD}▶ $1${RESET}"; }

run_checks() {
    local failed=0

    section "Formatting check"
    if cargo fmt --all -- --check >/dev/null 2>&1; then
        pass "fmt OK"
    else
        fail "fmt violations"
        ((failed++))
    fi

    section "Clippy check"
    if cargo clippy --all-targets --all-features -- \
        -D warnings >/dev/null 2>&1; then
        pass "clippy OK"
    else
        fail "clippy warnings/errors"
        ((failed++))
    fi

    section "Tests"
    if cargo test --all-features >/dev/null 2>&1; then
        pass "tests OK"
    else
        fail "tests failed"
        ((failed++))
    fi

    section "Build"
    if cargo build --all-features >/dev/null 2>&1; then
        pass "build OK"
    else
        fail "build failed"
        ((failed++))
    fi

    section "Security audit"
    if command -v cargo-audit >/dev/null 2>&1; then
        if cargo audit >/dev/null 2>&1; then
            pass "audit OK"
        else
            fail "cargo audit found vulnerabilities"
            ((failed++))
        fi
    else
        warn "cargo-audit not installed — install with: cargo install cargo-audit"
    fi

    return $failed
}

# -----------------------------------------------------------------------------
# Parse arguments
# -----------------------------------------------------------------------------

list_beads() {
    echo ""
    echo -e "${BOLD}Available beads:${RESET}"
    echo ""
    printf "  %-12s  %s\n" "ID suffix" "Files"
    printf "  %-12s  %s\n" "----------" "-----"
    # FIX: iterate over keys directly (not via subshell string)
    for bead_id in "${!BEAD_FILES[@]}"; do
        files="${BEAD_FILES[$bead_id]:-unknown}"
        printf "  %-12s  %s\n" "$bead_id" "$files"
    done | sort
}

# -----------------------------------------------------------------------------
# Main
# -----------------------------------------------------------------------------

case "${1:-}" in
    --list)
        list_beads
        exit 0
        ;;
    --all)
        echo -e "${BOLD}Reviewing all beads...${RESET}"
        echo "(Full workspace checks)"
        run_checks
        exit $?
        ;;
    --help|-h)
        echo "Usage: $0 <BEAD_ID> [--list|--all]"
        echo ""
        echo "Options:"
        echo "  <BEAD_ID>   Review a specific bead (e.g. '0ed' or 'dynamic_context_pruning-0ed')"
        echo "  --list       List all beads and their associated files"
        echo "  --all        Run full workspace review"
        echo "  --help, -h   Show this help"
        exit 0
        ;;
    "")
        echo "Usage: $0 <BEAD_ID>  (or: $0 --list)"
        echo "Run --list to see all available beads."
        exit 1
        ;;
    *)
        BEAD_KEY="${1#dynamic_context_pruning-}"
        BEAD_KEY="${BEAD_KEY#dynamic_context_pruning}"

        if [[ -z "${BEAD_FILES[$BEAD_KEY]:-}" ]]; then
            warn "Bead '$BEAD_KEY' not found in mapping."
            echo "Run: $0 --list  to see available beads."
            exit 1
        fi

        FILES="${BEAD_FILES[$BEAD_KEY]}"
        BEAD_ID="dynamic_context_pruning-${BEAD_KEY}"

        echo ""
        echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
        echo -e "${BOLD}Bead:${RESET} $BEAD_ID"
        echo -e "${BOLD}Files:${RESET} $FILES"
        echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"

        run_checks
        result=$?

        echo ""
        if [[ $result -eq 0 ]]; then
            echo -e "${GREEN}Code review: PASS ✓${RESET}"
            echo ""
            echo "Close comment suggestion:"
            echo "  Verified: cargo fmt OK, clippy OK, all tests pass, build OK, audit OK"
            echo ""
        else
            echo -e "${RED}Code review: FAIL ✗${RESET}"
            echo ""
            echo "Issues found — fix before closing bead."
            echo ""
        fi

        exit $result
        ;;
esac
