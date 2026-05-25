"""Shared utilities for tool modules.

Every tool module imports helpers from here instead of reaching into main.py.
This keeps the tool files self-contained and testable in isolation.
"""

import hashlib
import json
import os
import pathlib
import re
import sys
import threading
from typing import Optional

from pydantic_ai import ModelRetry  # re-exported; see error convention below


# ── Error convention ────────────────────────────────────────────────────────
#
# Two exception types, two meanings:
#
#   BlockedToolError   Terminal. Agent run aborts with status="BLOCKED".
#                      Use for: capability denial, config missing (AGENT_ROOT),
#                      hiring disabled, dedup cap hit — anything the agent
#                      should NOT retry.
#
#   ModelRetry         Recoverable. PydanticAI sends the message back to the
#                      LLM, which can adapt and try again.
#                      Use for: file-not-found, invalid path, empty query,
#                      provider misconfigured — anything the LLM might fix
#                      by calling a different tool or passing different args.
#
# Tools that already return data on success (dicts, strings) MAY keep their
# error-as-data returns for shape consistency. Prefer ModelRetry for errors
# in tools whose success return-type is a list (e.g. list_directory) where a
# mixed return shape would be awkward.
#
__all_exports__ = ("BlockedToolError", "ModelRetry")


class BlockedToolError(Exception):
    """Raised when a tool call must terminate the agent run (capability
    denied, config missing, guardrail tripped). Caught at the top level
    of main.py and converted into a structured BLOCKED status."""
    pass


# ── Sidechannel I/O ─────────────────────────────────────────────────────────

_sidechannel_lock = threading.Lock()
SIDECHANNEL_LOCK_TIMEOUT_SEC = 2.0


def emit_stderr_line(line: str, lock_timeout: Optional[float] = SIDECHANNEL_LOCK_TIMEOUT_SEC) -> bool:
    if lock_timeout is None:
        acquired = _sidechannel_lock.acquire()
    else:
        acquired = _sidechannel_lock.acquire(timeout=lock_timeout)
    if not acquired:
        return False
    try:
        if not line.endswith("\n"):
            line += "\n"
        sys.stderr.write(line)
        sys.stderr.flush()
    except Exception:
        return False
    finally:
        _sidechannel_lock.release()
    return True


def emit_sidechannel(payload: dict, lock_timeout: Optional[float] = None) -> bool:
    return emit_stderr_line(json.dumps(payload), lock_timeout=lock_timeout)


def emit_debug(message: str) -> bool:
    return emit_stderr_line(message, lock_timeout=1.0)


# ── Audit sidechannel (rate-limited, size-capped) ───────────────────────────

_MAX_AUDIT_EVENTS_PER_RUN = 2000
_MAX_AUDIT_EVENT_BYTES = 64 * 1024
_audit_event_count = 0
_audit_budget_exceeded_warned = False
_audit_counter_lock = threading.Lock()


def reset_audit_counters() -> None:
    """Reset per-run audit counters. Called once at worker start."""
    global _audit_event_count, _audit_budget_exceeded_warned
    _audit_event_count = 0
    _audit_budget_exceeded_warned = False


def emit_audit_sidechannel(event_type: str, payload: dict) -> None:
    global _audit_event_count, _audit_budget_exceeded_warned

    with _audit_counter_lock:
        if _audit_event_count >= _MAX_AUDIT_EVENTS_PER_RUN:
            should_warn = not _audit_budget_exceeded_warned
            _audit_budget_exceeded_warned = True
            over_budget = True
        else:
            _audit_event_count += 1
            should_warn = False
            over_budget = False

    if over_budget:
        if should_warn:
            emit_stderr_line("[WARN] audit sidechannel budget exceeded, dropping further events")
        return

    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    payload["__sairgent_sidechannel"] = event_type
    payload["token"] = token

    try:
        serialized = json.dumps(payload)
    except (TypeError, ValueError) as exc:
        emit_stderr_line(f"[WARN] audit sidechannel event not serializable ({exc}), dropping")
        return
    byte_len = len(serialized.encode("utf-8"))
    if byte_len > _MAX_AUDIT_EVENT_BYTES:
        emit_stderr_line(f"[WARN] audit sidechannel event oversized ({byte_len} bytes), dropping")
        return

    emit_sidechannel(payload)


# ── Capability checking ─────────────────────────────────────────────────────

def agent_capabilities() -> set:
    raw = os.getenv("AGENT_MANIFEST_JSON", "")
    if not raw:
        return set()
    try:
        manifest = json.loads(raw)
        caps = manifest.get("capabilities") or []
        return set(caps)
    except Exception:
        return set()


def require_capability(name: str) -> None:
    if name not in agent_capabilities():
        raise BlockedToolError(
            f"Capability `{name}` is not granted to this agent. "
            f"Ask an operator to enable it in the agent manifest."
        )


# ── Path safety ─────────────────────────────────────────────────────────────

def resolve_safe_path(path: str) -> pathlib.Path:
    workspace_root = os.getenv("AGENT_ROOT", "").strip()
    if not workspace_root:
        raise BlockedToolError("AGENT_ROOT is not configured for this agent.")
    requested = (path or "").strip()
    if not requested:
        raise ModelRetry(
            "path is required and cannot be empty. "
            "Pass a workspace-relative path like 'workspace/src/main.py'."
        )
    root = pathlib.Path(os.path.realpath(workspace_root))
    candidate = root / requested
    try:
        resolved = pathlib.Path(os.path.realpath(candidate))
        resolved.relative_to(root)
    except (ValueError, OSError):
        raise ModelRetry(
            f"Path '{path}' resolves outside the agent workspace. "
            f"Use a path relative to the agent root (no '..', no leading '/')."
        )
    return resolved


def workspace_dir() -> pathlib.Path:
    workspace_root = os.getenv("AGENT_ROOT", "").strip()
    if not workspace_root:
        raise BlockedToolError("AGENT_ROOT is not configured for this agent.")
    ws = pathlib.Path(workspace_root) / "workspace"
    ws.mkdir(parents=True, exist_ok=True)
    return ws


# ── Hashing / redaction ─────────────────────────────────────────────────────

def sha256_hex(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


_SECRET_PATTERNS = [
    (re.compile(r"(?i)(authorization:\s*bearer\s+)\S+"), r"\1[REDACTED]"),
    (re.compile(r"(?i)(authorization:\s*basic\s+)\S+"), r"\1[REDACTED]"),
    (re.compile(r"(https?://)[^@/\s:]+:[^@/\s]+@"), r"\1[REDACTED]@"),
    (re.compile(r"\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}"), r"[REDACTED_GH_TOKEN]"),
    (re.compile(r"\bsk-(?:proj-|ant-)?[A-Za-z0-9_-]{20,}"), r"[REDACTED_API_KEY]"),
    (re.compile(r"\bAKIA[0-9A-Z]{16}\b"), r"[REDACTED_AWS_KEY]"),
    (re.compile(r"(?i)(api[_-]?key[=:\s]+)\S+"), r"\1[REDACTED]"),
    (re.compile(r"(?i)(token[=:\s]+)[A-Za-z0-9_\-\.]{16,}"), r"\1[REDACTED]"),
    (re.compile(r"(?i)(password[=:\s]+)\S+"), r"\1[REDACTED]"),
]


def redact_secrets(text: str) -> str:
    if not isinstance(text, str):
        return text
    redacted = text
    for pattern, replacement in _SECRET_PATTERNS:
        redacted = pattern.sub(replacement, redacted)
    return redacted


# ── Artifact dedup (CHA-425) ────────────────────────────────────────────────

_ARTIFACT_PREFIX_CAP = 5
_artifact_prefix_counts: dict = {}
_artifact_dedup_warned: set = set()

_VERSION_SUFFIX_RE = re.compile(r"-v\d+$", re.IGNORECASE)
_SUFFIX_NOISE_WORDS = (
    "-final", "-response", "-answer", "-answered", "-answers",
    "-user", "-for-user", "-to-user", "-human", "-human-feedback",
    "-revision", "-revision-feedback", "-feedback", "-reply",
    "-resolved", "-where-hosted", "-where-run",
)


def normalize_artifact_prefix(filename: str) -> str:
    if not filename:
        return ""
    base = filename.rsplit("/", 1)[-1]
    if "." in base:
        base = base.rsplit(".", 1)[0]
    base = base.lower()
    changed = True
    while changed:
        changed = False
        m = _VERSION_SUFFIX_RE.search(base)
        if m:
            base = base[:m.start()]
            changed = True
        for suffix in _SUFFIX_NOISE_WORDS:
            if base.endswith(suffix):
                base = base[:-len(suffix)]
                changed = True
                break
    return base[:30]


def check_artifact_dedup(filename: str) -> Optional[str]:
    prefix = normalize_artifact_prefix(filename)
    if not prefix:
        return None
    current = _artifact_prefix_counts.get(prefix, 0)
    if current >= _ARTIFACT_PREFIX_CAP:
        if prefix not in _artifact_dedup_warned:
            _artifact_dedup_warned.add(prefix)
            emit_stderr_line(
                f"[WARN] artifact dedup: prefix '{prefix}' hit cap ({_ARTIFACT_PREFIX_CAP}). "
                f"Further writes with this prefix are rejected."
            )
        return (
            f"Too many artifacts with similar names (prefix '{prefix}', cap {_ARTIFACT_PREFIX_CAP}). "
            f"Consolidate your output into fewer files or put the content in "
            f"your direct_answer / final_response field instead of creating another "
            f"artifact. See CHA-425."
        )
    _artifact_prefix_counts[prefix] = current + 1
    return None


def reset_artifact_dedup() -> None:
    global _artifact_prefix_counts, _artifact_dedup_warned
    _artifact_prefix_counts = {}
    _artifact_dedup_warned = set()


# ── Filename validation ─────────────────────────────────────────────────────

def safe_validate_filename(filename: str, base_dir: str) -> Optional[str]:
    if not filename or not base_dir:
        return None
    if any(c in filename for c in ('/', '\\', '\x00')):
        return None
    candidate = pathlib.Path(base_dir).resolve() / filename
    try:
        real = pathlib.Path(os.path.realpath(candidate))
        real.relative_to(pathlib.Path(os.path.realpath(base_dir)))
    except (ValueError, OSError):
        return None
    return str(real)


# ── Child process env whitelist ──────────────────────────────────────────────

ALLOWED_CHILD_ENV: frozenset = frozenset({
    "PATH", "HOME", "USER", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TZ",
    "TMPDIR", "AGENT_WORKSPACE",
})
