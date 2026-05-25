"""File I/O tools — read, create, edit, delete, mkdir, list_directory.

Capability gate: file_read, file_write.
"""

import os
import pathlib

from ._common import (
    BlockedToolError,
    ModelRetry,
    check_artifact_dedup,
    emit_audit_sidechannel,
    require_capability,
    resolve_safe_path,
    sha256_hex,
)
from . import ToolSpec, register


_MAX_READ_BYTES = 10 * 1024 * 1024  # 10 MB


def read_file(path: str) -> str:
    """Read a UTF-8 text file from the agent workspace.

    Use when you need to inspect existing source, config, data,
    or previously-written artifacts before editing or summarising.
    If you are unsure of the exact path, run list_directory first.

    Args:
      path: workspace-relative path, e.g. "workspace/src/main.py"
            or "context/notes.md". Path traversal is blocked.

    Returns: file contents as a string. On a recoverable error
    (file missing, wrong type, too large) the LLM is asked to
    retry with a corrected path. Run list_directory first if
    you are guessing.
    """
    require_capability("file_read")
    safe_path = resolve_safe_path(path)
    if not safe_path.exists():
        raise ModelRetry(
            f"File not found: '{path}'. Run list_directory on the parent "
            f"directory to see what is actually there, then retry."
        )
    if safe_path.is_dir():
        raise ModelRetry(
            f"'{path}' is a directory, not a file. "
            f"Use list_directory('{path}') to see its contents."
        )
    try:
        content_bytes = safe_path.read_bytes()
    except Exception as e:
        raise ModelRetry(f"Error reading file '{path}': {e}")
    if len(content_bytes) > _MAX_READ_BYTES:
        raise ModelRetry(
            f"File '{path}' exceeds the 10 MB read limit. "
            f"Use run_command with grep/head/tail to extract the section you need."
        )
    emit_audit_sidechannel("file_read", {
        "path": str(safe_path),
        "size": len(content_bytes),
        "content_hash": sha256_hex(content_bytes),
    })
    return content_bytes.decode("utf-8", errors="replace")


def list_directory(path: str) -> list:
    """List entries in a workspace directory.

    Use before reading or creating files when you are not sure
    what exists. Always list before guessing a path.

    Args:
      path: workspace-relative directory, e.g. "workspace" or
            "workspace/src". Use "." for the workspace root.

    Returns: list of {"name": str, "type": "file" | "dir"},
    sorted alphabetically. On a recoverable error (path missing
    or wrong type) the LLM is asked to retry with a corrected
    path.
    """
    require_capability("file_read")
    safe_path = resolve_safe_path(path)
    if not safe_path.exists():
        raise ModelRetry(
            f"Directory not found: '{path}'. "
            f"Try list_directory('.') to see the workspace root first."
        )
    if not safe_path.is_dir():
        raise ModelRetry(
            f"'{path}' is a file, not a directory. "
            f"Use read_file('{path}') to read its contents instead."
        )
    try:
        entries = []
        for entry in sorted(safe_path.iterdir()):
            entries.append({
                "name": entry.name,
                "type": "dir" if entry.is_dir() else "file",
            })
        return entries
    except Exception as e:
        raise ModelRetry(f"Error listing directory '{path}': {e}")


def create_file(path: str, content: str) -> dict:
    """Create a new file with the given content.

    Use to write new source code, scripts, or config that does
    not exist yet. Parent directories are created automatically.
    Fails if the file already exists — use edit_file to overwrite.

    Args:
      path:    workspace-relative path.
      content: full file contents as UTF-8.

    Returns: {"ok": True, "path": str, "bytes_written": int}
    on success, or {"ok": False, "error": str} on failure.
    """
    require_capability("file_write")
    safe_path = resolve_safe_path(path)
    if safe_path.exists():
        return {"ok": False, "error": f"file already exists: '{path}'. Use edit_file to overwrite."}
    dedup_err = check_artifact_dedup(path)
    if dedup_err is not None:
        return {"ok": False, "error": dedup_err}
    try:
        safe_path.parent.mkdir(parents=True, exist_ok=True)
        content_bytes = content.encode("utf-8")
        safe_path.write_bytes(content_bytes)
        emit_audit_sidechannel("file_mutation", {
            "operation": "create",
            "path": str(safe_path),
            "size": len(content_bytes),
            "content_hash": sha256_hex(content_bytes),
        })
        return {"ok": True, "path": str(safe_path), "bytes_written": len(content_bytes)}
    except Exception as e:
        return {"ok": False, "error": f"Error creating file '{path}': {e}"}


def edit_file(path: str, content: str) -> dict:
    """Overwrite an existing file's entire contents.

    Use to rewrite a file wholesale. For small surgical changes,
    read the file first, mutate the string, then pass the full
    result here — this tool takes the full replacement content,
    NOT a diff or patch. Fails if the file does not exist.

    Args:
      path:    workspace-relative path to an existing file.
      content: full replacement contents as UTF-8.

    Returns: {"ok": True, "path": str, "bytes_written": int}
    on success, or {"ok": False, "error": str} on failure.
    """
    require_capability("file_write")
    safe_path = resolve_safe_path(path)
    if not safe_path.exists():
        return {"ok": False, "error": f"file does not exist: '{path}'. Use create_file to create it."}
    if safe_path.is_dir():
        return {"ok": False, "error": f"'{path}' is a directory, not a file."}
    try:
        content_bytes = content.encode("utf-8")
        safe_path.write_bytes(content_bytes)
        emit_audit_sidechannel("file_mutation", {
            "operation": "edit",
            "path": str(safe_path),
            "size": len(content_bytes),
            "content_hash": sha256_hex(content_bytes),
        })
        return {"ok": True, "path": str(safe_path), "bytes_written": len(content_bytes)}
    except Exception as e:
        return {"ok": False, "error": f"Error editing file '{path}': {e}"}


def delete_file(path: str) -> dict:
    """Delete a single file. Does not remove directories.

    Use to remove obsolete files. Be deliberate — this is not
    undoable from the agent side.

    Args:
      path: workspace-relative path to an existing file.

    Returns: {"ok": True, "path": str} on success, or
    {"ok": False, "error": str} on failure (file missing, or
    path is a directory).
    """
    require_capability("file_write")
    safe_path = resolve_safe_path(path)
    if not safe_path.exists():
        return {"ok": False, "error": f"file does not exist: '{path}'"}
    if safe_path.is_dir():
        return {"ok": False, "error": f"'{path}' is a directory. delete_file only removes files."}
    try:
        safe_path.unlink()
        emit_audit_sidechannel("file_mutation", {
            "operation": "delete",
            "path": str(safe_path),
            "size": None,
            "content_hash": None,
        })
        return {"ok": True, "path": str(safe_path)}
    except Exception as e:
        return {"ok": False, "error": f"Error deleting file '{path}': {e}"}


def mkdir(path: str) -> dict:
    """Create a directory, including any missing parents.

    Use to set up folder structure before writing several files.
    Idempotent — safe to call on directories that already exist.

    Args:
      path: workspace-relative path, e.g. "workspace/tests/fixtures".

    Returns: {"ok": True, "path": str} on success, or
    {"ok": False, "error": str} on failure.
    """
    require_capability("file_write")
    safe_path = resolve_safe_path(path)
    try:
        safe_path.mkdir(parents=True, exist_ok=True)
        emit_audit_sidechannel("file_mutation", {
            "operation": "mkdir",
            "path": str(safe_path),
            "size": None,
            "content_hash": None,
        })
        return {"ok": True, "path": str(safe_path)}
    except Exception as e:
        return {"ok": False, "error": f"Error creating directory '{path}': {e}"}


# ── Registration ────────────────────────────────────────────────────────────

register(
    ToolSpec(fn=read_file,       capability="file_read"),
    ToolSpec(fn=list_directory,  capability="file_read"),
    ToolSpec(fn=create_file,     capability="file_write"),
    ToolSpec(fn=edit_file,       capability="file_write"),
    ToolSpec(fn=delete_file,     capability="file_write"),
    ToolSpec(fn=mkdir,           capability="file_write"),
)
