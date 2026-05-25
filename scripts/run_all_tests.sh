#!/usr/bin/env bash
# run_all_tests.sh — Unified test runner for Sairgent UAT suite.
#
# Usage:
#   ./scripts/run_all_tests.sh             # run all layers
#   ./scripts/run_all_tests.sh kernel      # Rust kernel only
#   ./scripts/run_all_tests.sh harness     # Python harness only
#   ./scripts/run_all_tests.sh ui          # Playwright e2e only (requires bun run dev running)
#
# Exit code is 0 only if every requested layer passes.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LAYER="${1:-all}"
PASS=0
FAIL=0

_section() { echo; echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"; echo "  $1"; echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"; }
_ok()      { echo "[PASS] $1"; ((PASS++)) || true; }
_fail()    { echo "[FAIL] $1"; ((FAIL++)) || true; }

# ---------------------------------------------------------------------------
# Layer: Rust kernel
# ---------------------------------------------------------------------------
run_kernel() {
    _section "Rust kernel — cargo test"
    cd "$REPO_ROOT/sairgent_kernel"
    if cargo test 2>&1; then
        _ok "sairgent_kernel"
    else
        _fail "sairgent_kernel"
    fi
}

# ---------------------------------------------------------------------------
# Layer: Python harness unit tests
# ---------------------------------------------------------------------------
run_harness() {
    _section "Python harness — pytest"
    cd "$REPO_ROOT/sairgent_harness"

    # Prefer venv python if present
    if [ -f "venv/bin/python" ]; then
        PYTHON="venv/bin/python"
    else
        PYTHON="python3"
    fi

    # Ensure pytest is available
    if ! $PYTHON -m pytest --version &>/dev/null 2>&1; then
        echo "pytest not found — installing into venv..."
        $PYTHON -m pip install pytest -q
    fi

    if $PYTHON -m pytest tests/ -v 2>&1; then
        _ok "sairgent_harness pytest"
    else
        _fail "sairgent_harness pytest"
    fi
}

# ---------------------------------------------------------------------------
# Layer: Playwright e2e (browser mock — no Tauri required)
# ---------------------------------------------------------------------------
run_ui() {
    _section "Playwright e2e — apps/desktop"
    cd "$REPO_ROOT/apps/desktop"
    if bunx playwright test 2>&1; then
        _ok "playwright e2e"
    else
        _fail "playwright e2e"
    fi
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------
case "$LAYER" in
    kernel)  run_kernel ;;
    harness) run_harness ;;
    ui)      run_ui ;;
    all)
        run_kernel
        run_harness
        run_ui
        ;;
    *)
        echo "Unknown layer: $LAYER. Use: all | kernel | harness | ui"
        exit 1
        ;;
esac

echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Results: ${PASS} passed, ${FAIL} failed"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

[ "$FAIL" -eq 0 ]
