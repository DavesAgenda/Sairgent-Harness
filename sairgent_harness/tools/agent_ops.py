"""Agent-specific tools — artifact write, agent file read, skills, journal.

These tools use allowed_tool_key gating (not capability gating) because
they're part of the core agent workflow, not dark-factory capabilities.
"""

import json
import os
import pathlib
import sqlite3
from typing import Optional

from pydantic import BaseModel, Field

from ._common import (
    BlockedToolError,
    check_artifact_dedup,
    emit_sidechannel,
    emit_stderr_line,
    safe_validate_filename,
)
from . import ToolSpec, register


# ── Constants ───────────────────────────────────────────────────────────────

SQLITE_READ_TIMEOUT_SEC = 2.0
SQLITE_BUSY_TIMEOUT_MS = 5000
_MAX_READ_BYTES = 10 * 1024 * 1024  # 10 MB


# ── Shared mutable state (injected from main.py at worker startup) ──────────
# These are module-level references so tool functions can update counters
# that main.py reads back for side_effects.

_state = {
    "artifact_count": 0,
    "innovation_count": 0,
    "outbox_artifacts": [],
}


def init_agent_ops(state_ref: dict) -> None:
    """Inject shared mutable state dict from main.py."""
    global _state
    _state = state_ref


# ── Pydantic models for structured tool inputs ─────────────────────────────

class PulseJournalEntry(BaseModel):
    """A journal entry for a cadence run."""
    cadence: str = Field(description="Cadence type: 'dawn', 'heartbeat', or 'dusk'")
    entry_type: str = Field(description="Entry type: 'step_started', 'step_completed', 'observation', or 'escalation'")
    summary: str = Field(description="Human-readable summary of what happened")
    detail_json: Optional[str] = Field(default=None, description="Optional JSON string with structured details")


# ── Tool functions ──────────────────────────────────────────────────────────

def read_agent_file(relative_path: str) -> str:
    """Read a file from YOUR agent workspace (context/, artifacts/).

    Use to re-read context files, previously-saved artifacts, or
    anything under your own agent root. This is SEPARATE from
    read_file, which reads the shared workspace/ directory.

    Args:
      relative_path: path relative to your agent root, e.g.
                     "context/pricing.md" or "artifacts/report-v2.md".

    Returns: file contents as a string, or "(file is empty)" if
    empty. Returns an "Error: ..." string if the path is missing,
    unsafe, or larger than 10 MB.
    """
    workspace_root = os.getenv("AGENT_ROOT", "")
    if not workspace_root:
        return "Error: AGENT_ROOT not configured for this agent."
    requested = (relative_path or "").strip()
    if not requested:
        return "Error: relative_path is required."
    candidate = pathlib.Path(workspace_root).resolve() / requested
    try:
        safe_path = pathlib.Path(os.path.realpath(candidate))
        safe_path.relative_to(pathlib.Path(os.path.realpath(workspace_root)))
    except (ValueError, OSError):
        return f"Error: Path '{relative_path}' is invalid or unsafe. Use a workspace-relative path."
    try:
        with open(safe_path, "r", encoding="utf-8") as f:
            content = f.read(_MAX_READ_BYTES + 1)
        if len(content.encode("utf-8")) > _MAX_READ_BYTES:
            return f"Error: File '{relative_path}' exceeds the maximum readable size of 10MB."
        return content if content else "(file is empty)"
    except FileNotFoundError:
        return f"Error: File '{relative_path}' not found in agent workspace."
    except Exception as e:
        return f"Error reading file: {str(e)}"


def _versioned_artifact_path(artifacts_dir: str, filename: str) -> tuple[str, str]:
    safe_path = safe_validate_filename(filename, artifacts_dir)
    if safe_path is None:
        raise ValueError("invalid filename")
    candidate = pathlib.Path(safe_path)
    if not candidate.exists():
        return filename, str(candidate)
    stem = candidate.stem
    suffix = candidate.suffix
    version = 2
    while True:
        versioned_name = f"{stem}-v{version}{suffix}"
        versioned_path = safe_validate_filename(versioned_name, artifacts_dir)
        if versioned_path is None:
            raise ValueError("invalid versioned filename")
        if not pathlib.Path(versioned_path).exists():
            return versioned_name, versioned_path
        version += 1


def write_artifact_file(filename: str, content: str) -> str:
    """Save a substantial deliverable as a versioned artifact.

    Use for reports, plans, code files, analyses, or ANY output
    the user should read later. Do NOT use for short answers
    that fit in a chat reply.

    Args:
      filename: plain filename with extension, NO path separators,
                e.g. "competitor-analysis.md" or "launch-plan.md".
      content:  full artifact content (UTF-8).

    Returns: success string with the final saved path.

    Behaviour: never overwrites. Repeat writes get auto-versioned
    suffixes (-v2, -v3, ...). Near-duplicate repeated writes are
    blocked by the dedup guardrail — change the content
    meaningfully before retrying.
    """
    artifacts_dir = os.getenv("AGENT_ARTIFACTS", "")
    if not artifacts_dir:
        return "Error: AGENT_ARTIFACTS not configured for this agent."
    MAX_BYTES = 10 * 1024 * 1024
    if len(content.encode("utf-8")) > MAX_BYTES:
        return "Error: Content exceeds maximum allowed size of 10MB."
    dedup_err = check_artifact_dedup(filename)
    if dedup_err is not None:
        return dedup_err
    try:
        final_name, safe_path = _versioned_artifact_path(artifacts_dir, filename)
        with open(safe_path, "w", encoding="utf-8") as f:
            f.write(content)
        token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
        swo_id = os.getenv("AGENT_SWO_ID")
        if token and swo_id:
            emit_sidechannel({
                "__sairgent_sidechannel": "outbox_artifact",
                "token": token,
                "swo_id": int(swo_id),
                "filename": final_name,
                "absolute_path": safe_path,
            })
        else:
            reasons = []
            if not token:
                reasons.append("SAIRGENT_SIDECHANNEL_TOKEN is empty")
            if not swo_id:
                reasons.append("AGENT_SWO_ID is empty")
            emit_stderr_line(
                f"[WARNING] Skipping sidechannel artifact registration for '{final_name}': {', '.join(reasons)}"
            )
        _state["artifact_count"] = _state.get("artifact_count", 0) + 1
        _state.setdefault("outbox_artifacts", []).append({"filename": final_name, "absolute_path": safe_path})
        return f"Successfully wrote {len(content)} characters to {safe_path}."
    except Exception as e:
        return f"Error writing file: {str(e)}"


def _load_skill_index() -> list[dict]:
    raw = os.getenv("AGENT_SKILL_INDEX_JSON", "[]")
    try:
        return json.loads(raw) or []
    except Exception:
        return []


def _skill_index_map() -> dict:
    return {entry.get("id", ""): entry for entry in _load_skill_index() if entry.get("id")}


def _slugify_skill_name(name: str) -> str:
    import re
    return re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")


def list_available_skills(query: str = "") -> str:
    """List skills available to this agent, optionally filtered.

    Use to discover specialised procedures (competitive analysis,
    code review, report generation, etc.) before executing a
    complex task. Follow up with load_skill(skill_id) to fetch
    the actual step-by-step instructions.

    Args:
      query: optional keyword; matched case-insensitively against
             name, summary, tags, and trigger hints. Empty string
             returns every skill bound to this agent.

    Returns: JSON array of skill descriptor objects.
    """
    entries = _load_skill_index()
    query_lc = query.strip().lower()
    if query_lc:
        filtered = []
        for entry in entries:
            haystack = " ".join([
                entry.get("name", ""),
                entry.get("summary", ""),
                " ".join(entry.get("tags", []) or []),
                " ".join(entry.get("trigger_hints", []) or []),
            ]).lower()
            if query_lc in haystack:
                filtered.append(entry)
        entries = filtered
    return json.dumps(entries, indent=2)


def load_skill(skill_id: str) -> str:
    """Load the full markdown content of a specific skill.

    Use after list_available_skills has surfaced a relevant
    skill — this tool returns the step-by-step instructions,
    templates, and guidelines so you can execute the procedure.

    Args:
      skill_id: an ID returned from list_available_skills.

    Returns: markdown text starting with a header line. Returns
    an "Error: ..." string if the skill_id is not bound to this
    agent or cannot be loaded.
    """
    index = _skill_index_map()
    if skill_id not in index:
        return f"Error: skill_id '{skill_id}' is not bound to this agent."

    registry_db_path = os.getenv("REGISTRY_DATABASE", "")
    if not registry_db_path:
        return "Error: REGISTRY_DATABASE not configured."

    try:
        db_uri = f"file:{os.path.abspath(registry_db_path)}?mode=ro"
        conn = sqlite3.connect(db_uri, uri=True, timeout=SQLITE_READ_TIMEOUT_SEC, isolation_level=None)
        conn.execute(f"PRAGMA busy_timeout = {SQLITE_BUSY_TIMEOUT_MS}")
        conn.execute("PRAGMA query_only = ON")
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            """
            SELECT s.name, s.slug, sv.raw_markdown, sv.metadata_json, s.current_version
            FROM skills s
            JOIN skill_versions sv
              ON sv.skill_id = s.id AND sv.version = s.current_version
            WHERE s.id = ?
            """,
            (skill_id,),
        ).fetchone()
    except Exception as e:
        return f"Error loading skill: {e}"
    finally:
        if "conn" in locals():
            conn.close()

    if row is None:
        return f"Error: skill '{skill_id}' not found."

    runtime_dir = os.getenv("AGENT_RUNTIME_DIR", "")
    runtime_path = None
    if runtime_dir:
        try:
            skills_dir = os.path.join(runtime_dir, "skills")
            os.makedirs(skills_dir, exist_ok=True)
            slug = row["slug"] or _slugify_skill_name(row["name"])
            runtime_path = os.path.join(skills_dir, f"{slug}.md")
            with open(runtime_path, "w", encoding="utf-8") as handle:
                handle.write(row["raw_markdown"])
        except Exception:
            runtime_path = None

    metadata = {}
    try:
        metadata = json.loads(row["metadata_json"])
    except Exception:
        pass

    header = f"# Skill: {row['name']} (v{row['current_version']})\n\n"
    if runtime_path:
        header += f"_Materialized to: {runtime_path}_\n\n"
    return header + (row["raw_markdown"] or "")


def append_pulse_journal(entry: PulseJournalEntry) -> str:
    """Record a step, observation, or escalation during a cadence.

    Use CONTINUOUSLY during dawn/heartbeat/dusk execution to log
    what you did, what you noticed, and what needs attention.
    This is how the human-in-the-loop traces your autonomous work.

    Args (PulseJournalEntry):
      cadence:     "dawn" | "heartbeat" | "dusk"
      entry_type:  "step_started" | "step_completed" |
                   "observation" | "escalation"
      summary:     one-line human-readable description.
      detail_json: optional JSON string with structured details.

    Returns: confirmation string.
    """
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    run_id = os.getenv("AGENT_RUN_ID", "")
    emit_sidechannel({
        "__sairgent_sidechannel": "append_pulse_journal",
        "token": token,
        "cadence": entry.cadence,
        "entry_type": entry.entry_type,
        "summary": entry.summary,
        "detail_json": entry.detail_json,
        "run_id": run_id if run_id else None,
    })
    return f"Journal entry appended: [{entry.cadence}] {entry.entry_type} — {entry.summary}"


# ── Registration ────────────────────────────────────────────────────────────

register(
    ToolSpec(fn=read_agent_file,       allowed_tool_key="read_agent_file"),
    ToolSpec(fn=write_artifact_file,   allowed_tool_key="write_artifact_file"),
    ToolSpec(fn=list_available_skills,  allowed_tool_key="list_available_skills"),
    ToolSpec(fn=load_skill,            allowed_tool_key="load_skill"),
    ToolSpec(fn=append_pulse_journal,   allowed_tool_key="append_pulse_journal", always=True),
)
