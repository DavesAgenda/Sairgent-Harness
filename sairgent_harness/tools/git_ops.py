"""Git tools — clone, status, diff, commit, branch, push.

Capability gate: git_ops.
"""

import os
import re
import subprocess

from ._common import (
    ALLOWED_CHILD_ENV,
    BlockedToolError,
    emit_audit_sidechannel,
    redact_secrets,
    require_capability,
    resolve_safe_path,
    workspace_dir,
)
from . import ToolSpec, register


def _git(args: list, cwd) -> dict:
    """Run a git sub-command and return {ok, stdout, stderr, exit_code}."""
    git_env = {k: os.environ[k] for k in ALLOWED_CHILD_ENV if k in os.environ}
    try:
        proc = subprocess.run(
            ["git"] + args, cwd=str(cwd),
            capture_output=True, timeout=120, env=git_env,
        )
        stdout = (proc.stdout or b"").decode("utf-8", errors="replace").strip()
        stderr = (proc.stderr or b"").decode("utf-8", errors="replace").strip()
        return {"ok": proc.returncode == 0, "exit_code": proc.returncode, "stdout": stdout, "stderr": stderr}
    except subprocess.TimeoutExpired:
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "git command timed out after 120s."}
    except FileNotFoundError:
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "git is not available on PATH."}
    except Exception as e:
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": f"Error running git: {e}"}


def git_clone(repo_url: str, branch: str | None = None) -> dict:
    """Clone a git repository into the agent workspace.

    Use to bring an external repo into scope for inspection,
    building, or modification. Credentials must already be
    configured in the environment — this tool does not prompt.

    Args:
      repo_url: HTTPS or SSH URL.
      branch:   optional branch to check out immediately.

    Returns: {"ok": bool, "exit_code": int,
              "stdout": str, "stderr": str}.
    """
    require_capability("git_ops")
    ws = workspace_dir()
    args = ["clone"]
    if branch:
        args += ["--branch", branch]
    args.append(repo_url)
    result = _git(args, cwd=ws)
    emit_audit_sidechannel("git_operation", {
        "operation": "clone", "repo": redact_secrets(repo_url),
        "branch": branch, "commit_hash": None, "files_changed": None,
    })
    return result


def git_status(working_dir: str | None = None) -> dict:
    """Show a repo's git status in porcelain format (machine-readable).

    Use to check what has changed before staging or committing.
    Returns one line per modified/untracked file — short, stable
    prefixes like " M file.py" (modified) or "?? file.py" (new).

    Args:
      working_dir: optional workspace-relative repo path;
                   defaults to workspace root.

    Returns: {"ok": bool, "exit_code": int,
              "stdout": str, "stderr": str}.
    """
    require_capability("git_ops")
    cwd = resolve_safe_path(working_dir) if working_dir else workspace_dir()
    result = _git(["status", "--porcelain"], cwd=cwd)
    emit_audit_sidechannel("git_operation", {
        "operation": "status", "repo": str(cwd),
        "branch": None, "commit_hash": None, "files_changed": None,
    })
    return result


def git_diff(staged: bool = False, working_dir: str | None = None) -> str:
    """Show file diffs for a git repository.

    Use to review exact changes before committing, or inspect
    a repo's current delta against HEAD.

    Args:
      staged:      True for staged-only (git diff --staged);
                   False (default) for unstaged working-tree.
      working_dir: optional workspace-relative repo path.

    Returns: diff text, or "Error: ..." string on failure.
    """
    require_capability("git_ops")
    cwd = resolve_safe_path(working_dir) if working_dir else workspace_dir()
    args = ["diff"]
    if staged:
        args.append("--staged")
    result = _git(args, cwd=cwd)
    emit_audit_sidechannel("git_operation", {
        "operation": "diff", "repo": str(cwd), "staged": staged,
        "branch": None, "commit_hash": None, "files_changed": None,
    })
    return result.get("stdout", "") if result["ok"] else f"Error: {result.get('stderr', '')}"


def git_commit(message: str, files: list, working_dir: str | None = None) -> dict:
    """Stage specific files and commit them.

    Use to save work as a git commit. Always list the exact
    files — never rely on "git add .".

    Args:
      message:     non-empty commit message.
      files:       list of paths relative to the repo, e.g.
                   ["src/util.py", "tests/test_util.py"].
      working_dir: optional workspace-relative repo path.

    Returns: {"ok": bool, "exit_code": int,
              "stdout": str, "stderr": str}.
    """
    require_capability("git_ops")
    if not message or not message.strip():
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "commit message cannot be empty."}
    if not files:
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "files list cannot be empty."}
    cwd = resolve_safe_path(working_dir) if working_dir else workspace_dir()
    add_result = _git(["add", "--"] + [str(f) for f in files], cwd=cwd)
    if not add_result["ok"]:
        return add_result
    result = _git(["commit", "-m", message], cwd=cwd)
    commit_hash = None
    if result["ok"] and result.get("stdout"):
        m = re.search(r"\b([0-9a-f]{7,40})\b", result["stdout"])
        if m:
            commit_hash = m.group(1)
    _FILES_CHANGED_CAP = 100
    all_files = [str(f) for f in files]
    files_changed_truncated = len(all_files) > _FILES_CHANGED_CAP
    emit_audit_sidechannel("git_operation", {
        "operation": "commit", "repo": str(cwd), "branch": None,
        "commit_hash": commit_hash,
        "files_changed": all_files[:_FILES_CHANGED_CAP],
        "files_changed_total": len(all_files),
        "files_changed_truncated": files_changed_truncated,
    })
    return result


def git_create_branch(name: str, working_dir: str | None = None) -> dict:
    """Create a new branch and switch to it.

    Use to start isolated work before making commits, so the
    main branch is never modified directly.

    Args:
      name:        branch name, e.g. "feat/llm-telemetry".
      working_dir: optional workspace-relative repo path.

    Returns: {"ok": bool, "exit_code": int,
              "stdout": str, "stderr": str}.
    """
    require_capability("git_ops")
    if not name or not name.strip():
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "branch name cannot be empty."}
    cwd = resolve_safe_path(working_dir) if working_dir else workspace_dir()
    result = _git(["checkout", "-b", name.strip()], cwd=cwd)
    emit_audit_sidechannel("git_operation", {
        "operation": "create_branch", "repo": str(cwd),
        "branch": name.strip(), "commit_hash": None, "files_changed": None,
    })
    return result


def git_push(remote: str = "origin", branch: str = "", working_dir: str | None = None) -> dict:
    """Push commits to a remote repository.

    Use to publish committed changes. Both remote AND branch
    are REQUIRED — there is no implicit upstream tracking.

    Args:
      remote:      e.g. "origin".
      branch:      e.g. "feat/llm-telemetry".
      working_dir: optional workspace-relative repo path.

    Returns: {"ok": bool, "exit_code": int,
              "stdout": str, "stderr": str}.
    """
    require_capability("git_ops")
    if not remote or not remote.strip():
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "remote cannot be empty."}
    if not branch or not branch.strip():
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "branch cannot be empty. Provide an explicit branch name."}
    cwd = resolve_safe_path(working_dir) if working_dir else workspace_dir()
    result = _git(["push", remote.strip(), branch.strip()], cwd=cwd)
    emit_audit_sidechannel("git_operation", {
        "operation": "push", "repo": str(cwd),
        "branch": branch.strip(), "commit_hash": None, "files_changed": None,
    })
    return result


# ── Registration ────────────────────────────────────────────────────────────

register(
    ToolSpec(fn=git_clone,         capability="git_ops"),
    ToolSpec(fn=git_status,        capability="git_ops"),
    ToolSpec(fn=git_diff,          capability="git_ops"),
    ToolSpec(fn=git_commit,        capability="git_ops"),
    ToolSpec(fn=git_create_branch, capability="git_ops"),
    ToolSpec(fn=git_push,          capability="git_ops"),
)
