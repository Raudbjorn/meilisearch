#!/usr/bin/env bash
#
# VIGIL: Verification gate for Anthropic integration
#
# Exit codes:
#   0 = All checks pass (proceed)
#   1 = Build failed
#   2 = Clippy failed (warnings are failures)
#   3 = Tests failed
#
# Usage: ./vigil.sh [--quick]
#   --quick: Skip tests (build + clippy only)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

# Colors (if terminal supports them)
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_step() {
    echo -e "${YELLOW}[VIGIL]${NC} $1"
}

log_pass() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

log_fail() {
    echo -e "${RED}[FAIL]${NC} $1"
}

# Parse arguments
QUICK=false
for arg in "$@"; do
    case $arg in
        --quick)
            QUICK=true
            shift
            ;;
    esac
done

cd "$PROJECT_ROOT"

# Step 1: Build
log_step "Building meilisearch..."
if cargo build -p meilisearch 2>&1; then
    log_pass "Build succeeded"
else
    log_fail "Build failed"
    exit 1
fi

# Step 2: Clippy (warnings are failures)
log_step "Running clippy..."
if cargo clippy -p meilisearch -- -D warnings 2>&1; then
    log_pass "Clippy passed (no warnings)"
else
    log_fail "Clippy failed (warnings present)"
    exit 2
fi

# Step 3: Tests (unless --quick)
if [ "$QUICK" = false ]; then
    log_step "Running anthropic tests..."
    if cargo test -p meilisearch anthropic 2>&1; then
        log_pass "Tests passed"
    else
        log_fail "Tests failed"
        exit 3
    fi
else
    log_step "Skipping tests (--quick mode)"
fi

echo ""
echo -e "${GREEN}[VIGIL]${NC} All checks passed"
exit 0
