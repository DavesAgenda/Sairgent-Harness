#!/bin/bash
# Wrapper script to execute the configured worker harness from the Rust Orchestrator
DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"

PYDANTIC_VENV_PYTHON="$DIR/sairgent_harness/venv/bin/python"
CODEX_VENV_PYTHON="$DIR/sairgent_harness/venv/bin/python"

MODE="${1:-}"
BACKEND="${SAIRGENT_WORKER_BACKEND:-}"

if [ -z "$BACKEND" ]; then
    case "$MODE" in
        chat_mode|format_swo|sairgent_chat)
            BACKEND="pydantic_ai"
            ;;
        *)
            BACKEND="codex_cli"
            ;;
    esac
fi

# Allowlist validation for SAIRGENT_WORKER_BACKEND
if [ -n "$BACKEND" ] && ! echo "pydantic_ai codex_cli" | grep -qw "$BACKEND"; then
    echo "ERROR: Invalid SAIRGENT_WORKER_BACKEND value: $BACKEND" >&2
    exit 1
fi

case "$BACKEND" in
    codex_cli)
        PYTHON_EXEC="$CODEX_VENV_PYTHON"
        if [ ! -f "$PYTHON_EXEC" ]; then PYTHON_EXEC="$(command -v python3)"; fi
        exec "$PYTHON_EXEC" "$DIR/sairgent_codex_harness/main.py" "$@"
        ;;
    *)
        PYTHON_EXEC="$PYDANTIC_VENV_PYTHON"
        if [ ! -f "$PYTHON_EXEC" ]; then PYTHON_EXEC="$(command -v python3)"; fi
        exec "$PYTHON_EXEC" "$DIR/sairgent_harness/main.py" "$@"
        ;;
esac
