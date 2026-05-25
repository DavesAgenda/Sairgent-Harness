"""Shell execution tool — sandboxed run_command.

Capability gate: shell_exec.
"""

import os
import shlex
import subprocess
import time

from ._common import (
    ALLOWED_CHILD_ENV,
    BlockedToolError,
    emit_audit_sidechannel,
    redact_secrets,
    require_capability,
    resolve_safe_path,
    sha256_hex,
    workspace_dir,
)
from . import ToolSpec, register


_MAX_COMMAND_OUTPUT_BYTES = 100 * 1024  # 100 KB per stream
_MAX_COMMAND_TIMEOUT_SEC = 600


def run_command(command: str, working_dir: str | None = None, timeout_seconds: int = 120) -> dict:
    """Run a shell command inside the agent workspace.

    Use to run build commands, tests, scripts, or CLI tools.
    The command is parsed via shlex and executed directly — there
    is NO shell interpreter, so pipes (|), redirects (>), chains
    (&&), and globs (*) do NOT work. Chain multiple calls, or run
    "sh -c '...'" explicitly if you genuinely need those.

    Examples:
      run_command("pytest tests/test_util.py -v")
      run_command("bun run build", working_dir="apps/desktop")

    Args:
      command:         single command line (shlex-parsed).
      working_dir:     optional workspace-relative dir; defaults
                       to workspace root.
      timeout_seconds: clamped to [1, 600]; default 120.

    Returns: {"exit_code": int, "stdout": str, "stderr": str,
              "duration_ms": int, "truncated": bool}.
    stdout/stderr are truncated at 100 KB each.
    """
    require_capability("shell_exec")

    timeout_seconds = max(1, min(int(timeout_seconds), _MAX_COMMAND_TIMEOUT_SEC))

    if working_dir:
        cwd_path = resolve_safe_path(working_dir)
    else:
        cwd_path = workspace_dir()

    if not cwd_path.exists():
        cwd_path.mkdir(parents=True, exist_ok=True)

    try:
        args = shlex.split(command)
    except ValueError as e:
        return {
            "exit_code": -1, "stdout": "", "stderr": f"Failed to parse command: {e}",
            "duration_ms": 0, "truncated": False,
        }

    if not args:
        return {
            "exit_code": -1, "stdout": "", "stderr": "command is empty after parsing.",
            "duration_ms": 0, "truncated": False,
        }

    child_env = {k: os.environ[k] for k in ALLOWED_CHILD_ENV if k in os.environ}

    start = time.monotonic()
    try:
        proc = subprocess.run(
            args, cwd=str(cwd_path), capture_output=True,
            timeout=timeout_seconds, env=child_env,
        )
        duration_ms = int((time.monotonic() - start) * 1000)
        stdout_bytes = proc.stdout or b""
        stderr_bytes = proc.stderr or b""
        truncated = (
            len(stdout_bytes) > _MAX_COMMAND_OUTPUT_BYTES
            or len(stderr_bytes) > _MAX_COMMAND_OUTPUT_BYTES
        )
        stdout = stdout_bytes[:_MAX_COMMAND_OUTPUT_BYTES].decode("utf-8", errors="replace")
        stderr = stderr_bytes[:_MAX_COMMAND_OUTPUT_BYTES].decode("utf-8", errors="replace")
        emit_audit_sidechannel("shell_exec", {
            "command": redact_secrets(command), "cwd": str(cwd_path),
            "exit_code": proc.returncode, "duration_ms": duration_ms,
            "stdout_hash": sha256_hex(stdout_bytes[:_MAX_COMMAND_OUTPUT_BYTES]),
            "stderr_hash": sha256_hex(stderr_bytes[:_MAX_COMMAND_OUTPUT_BYTES]),
            "truncated": truncated,
        })
        return {
            "exit_code": proc.returncode, "stdout": stdout, "stderr": stderr,
            "duration_ms": duration_ms, "truncated": truncated,
        }
    except subprocess.TimeoutExpired:
        duration_ms = int((time.monotonic() - start) * 1000)
        emit_audit_sidechannel("shell_exec", {
            "command": redact_secrets(command), "cwd": str(cwd_path),
            "exit_code": -1, "duration_ms": duration_ms,
            "stdout_hash": sha256_hex(b""),
            "stderr_hash": sha256_hex(f"Command timed out after {timeout_seconds}s.".encode()),
            "truncated": False,
        })
        return {
            "exit_code": -1, "stdout": "",
            "stderr": f"Command timed out after {timeout_seconds}s.",
            "duration_ms": duration_ms, "truncated": False,
        }
    except FileNotFoundError:
        emit_audit_sidechannel("shell_exec", {
            "command": redact_secrets(command), "cwd": str(cwd_path),
            "exit_code": -1, "duration_ms": 0,
            "stdout_hash": sha256_hex(b""),
            "stderr_hash": sha256_hex(f"Command not found: {args[0]!r}".encode()),
            "truncated": False,
        })
        return {
            "exit_code": -1, "stdout": "",
            "stderr": f"Command not found: {args[0]!r}",
            "duration_ms": 0, "truncated": False,
        }
    except Exception as e:
        emit_audit_sidechannel("shell_exec", {
            "command": redact_secrets(command), "cwd": str(cwd_path),
            "exit_code": -1, "duration_ms": 0,
            "stdout_hash": sha256_hex(b""),
            "stderr_hash": sha256_hex(f"Error running command: {e}".encode()),
            "truncated": False,
        })
        return {
            "exit_code": -1, "stdout": "",
            "stderr": f"Error running command: {e}",
            "duration_ms": 0, "truncated": False,
        }


# ── Registration ────────────────────────────────────────────────────────────

register(
    ToolSpec(fn=run_command, capability="shell_exec"),
)
