"""Delegation tools — dispatch SWOs, submit innovations, hire subordinates.

These tools communicate with the Rust kernel via sidechannel messages.
"""

import json
import os
import uuid

from pydantic import BaseModel, Field
from typing import Optional

from ._common import (
    BlockedToolError,
    emit_debug,
    emit_sidechannel,
)
from . import ToolSpec, register


# ── Shared mutable state (injected from main.py) ───────────────────────────

_state = {
    "managed_work_count": 0,
    "innovation_count": 0,
    "hire_request_count": 0,
    "dispatch_count": 0,
    "sairgent_proposal_count": 0,
}


def init_delegation(state_ref: dict) -> None:
    """Inject shared mutable state dict from main.py."""
    global _state
    _state = state_ref


# ── Import HSM models ──────────────────────────────────────────────────────
# These are defined in hsm.py and used as structured inputs for tools.

import sqlite3
import sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from hsm import InnovationReport, HireSubordinateSpec, ManagedWorkRequest

SQLITE_READ_TIMEOUT_SEC = 2.0
SQLITE_BUSY_TIMEOUT_MS = 5000


# ── Tool functions ──────────────────────────────────────────────────────────

def submit_innovation_swo(report: InnovationReport) -> str:
    """Raise an upward innovation review to your manager.

    Use when — mid-execution — you identify an improvement
    opportunity, process optimisation, or strategic insight
    that is OUT OF SCOPE for your current task. This creates
    a review SWO without interrupting your current work.

    Args (InnovationReport): structured report payload — see
    the schema for the exact field list.

    Returns: confirmation string.

    Not available in execute_triage mode.
    """
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    originating_swo_id = os.getenv("AGENT_SWO_ID")
    emit_sidechannel({
        "__sairgent_sidechannel": "innovation_swo",
        "token": token,
        "originating_swo_id": int(originating_swo_id) if originating_swo_id else None,
        "report": report.model_dump(),
    })
    _state["innovation_count"] = _state.get("innovation_count", 0) + 1
    return "Innovation review SWO successfully submitted to manager."


def hire_subordinate_internal(spec: HireSubordinateSpec) -> str:
    """Create a NEW direct-report agent for a specific staffing gap.

    Use ONLY when you need a specialised role that does not yet
    exist on your team. Check your current subordinates first —
    do not duplicate roles.

    Write specific, narrow role definitions:
      BAD:  name="Helper", role="General assistant"
      GOOD: name="DataViz",
            role="Data Visualization Specialist",
            raison_detre="Transforms raw datasets into clear charts"

    Args (HireSubordinateSpec): see schema for required fields.

    Returns: confirmation string. Disabled (with a clear error
    message) if AGENT_CAN_HIRE is not set for this runtime.
    """
    if os.getenv("AGENT_CAN_HIRE", "1") != "1":
        return "Error: autonomous hiring is disabled for this agent in the current runtime."
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    emit_sidechannel({
        "__sairgent_sidechannel": "hire_subordinate",
        "token": token,
        "spec": spec.model_dump(),
    })
    _state["hire_request_count"] = _state.get("hire_request_count", 0) + 1
    return f"Direct-report hire submitted for {spec.name}."


def dispatch_swo_internal(dispatch_json: str) -> str:
    """Dispatch work directly to one of your registered subordinates.

    Use when you have identified the exact subordinate AND have
    a specific brief. You MUST call this tool to delegate —
    never narrate "I have assigned it" without invoking it.

    Brief quality matters — write SPECIFIC briefs, not paraphrases
    of the original request.
      BAD:  "Handle the market research task."
      GOOD: "Research top 5 competitors in the AI agent space.
             For each: pricing model, target customer, key
             differentiator. Deliver as a comparison table."

    Args:
      dispatch_json: JSON string with keys:
        "target_id": UUID of a registered subordinate.
        "payload":   specific brief with clear deliverable.

    Returns: confirmation string. Rejected if target_id is not
    a registered subordinate of this agent.
    """
    emit_debug(f"DEBUG TOOL EXECUTION: dispatch_swo_internal called with payload length {len(dispatch_json)}")

    try:
        parsed = json.loads(dispatch_json)
        target_id = parsed.get("target_id", "")
        payload = parsed.get("payload", "")
    except (json.JSONDecodeError, TypeError):
        return "Error: dispatch_json must be a valid JSON string with 'target_id' and 'payload' keys."

    if not target_id or not payload:
        return "Error: Both 'target_id' (UUID) and 'payload' are required in the dispatch JSON."

    subordinates_raw = os.getenv("AGENT_SUBORDINATES", "[]")
    try:
        authorized_ids = {s.get("id", "") for s in json.loads(subordinates_raw)}
    except Exception:
        authorized_ids = set()
    if target_id not in authorized_ids:
        return f"Error: target_id '{target_id}' is not a registered subordinate of this agent. Dispatch denied."

    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    emit_sidechannel({
        "__sairgent_sidechannel": "dispatch_swo",
        "token": token,
        "payload": json.dumps({"target_id": target_id, "payload": payload}),
    })
    _state["dispatch_count"] = _state.get("dispatch_count", 0) + 1
    return "SWO successfully dispatched to the Execution Pipeline. You may inform the user that work has begun."


def queue_managed_work(request: ManagedWorkRequest) -> str:
    """Queue a work order for async execution by a subordinate.

    Use for ANY task needing research, content creation,
    multi-step analysis, or specialist work. Each queued item
    MUST have ONE clear deliverable — never a bundle.
      BAD:  "Research competitors AND write positioning doc"
      GOOD: "Research top 5 competitors with pricing and
             differentiators, as a comparison table"

    Args (ManagedWorkRequest):
      payload:        specific brief with one clear deliverable.
      routing_policy: "HARD_ROUTE" | "PREFERENCE" | "NONE".

    Returns: confirmation string. Work runs asynchronously —
    poll with get_swo_queue_status to track progress.
    """
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    cleaned = request.payload.strip()
    if not cleaned:
        return "Error: queued work payload cannot be empty."
    routing_policy = (request.routing_policy or "NONE").strip().upper()
    if routing_policy not in ("HARD_ROUTE", "PREFERENCE", "NONE"):
        routing_policy = "NONE"
    payload = request.model_dump()
    payload["payload"] = cleaned
    payload["routing_policy"] = routing_policy
    emit_sidechannel({
        "__sairgent_sidechannel": "queue_managed_work",
        "token": token,
        "payload": payload,
    })
    _state["managed_work_count"] = _state.get("managed_work_count", 0) + 1
    return "Managed work successfully queued."


def get_swo_queue_status() -> str:
    """Check the live status of tasks assigned to YOUR subordinates.

    Use to give real-time progress updates, or decide whether
    to follow up on a subordinate before queueing more work.

    Args: none.

    Returns: multi-line status report (one line per active SWO)
    or "All tasks are completed or there are no active tasks"
    when idle. A transient error message is returned if the
    registry DB is momentarily busy.
    """
    registry_db_path = os.getenv("REGISTRY_DATABASE")
    if not registry_db_path:
        return "Internal Error: REGISTRY_DATABASE not found in environment."

    subordinates_json = os.getenv("AGENT_SUBORDINATES", "[]")
    try:
        subs = json.loads(subordinates_json)
        sub_ids = [s.get("id") for s in subs if s.get("id")]
    except Exception:
        sub_ids = []

    if not sub_ids:
        return "You do not have any subordinates to check the status of."

    placeholders = ",".join("?" for _ in sub_ids)
    query = f"SELECT id, assigned_agent_id, swo_payload, status FROM active_swos WHERE assigned_agent_id IN ({placeholders}) AND status != 'COMPLETED'"

    emit_debug(f"DEBUG get_swo_queue_status: db={registry_db_path}, subs={sub_ids}")

    try:
        db_uri = f"file:{os.path.abspath(registry_db_path)}?mode=ro"
        conn = sqlite3.connect(db_uri, uri=True, timeout=SQLITE_READ_TIMEOUT_SEC, isolation_level=None)
        conn.row_factory = sqlite3.Row
        conn.execute(f"PRAGMA busy_timeout = {SQLITE_BUSY_TIMEOUT_MS}")
        conn.execute("PRAGMA query_only = ON")
        cursor = conn.cursor()
        cursor.execute(query, sub_ids)
        rows = cursor.fetchall()

        if not rows:
            return "All tasks are completed or there are no active tasks in the queue."

        status_lines = []
        for row in rows:
            assigned_name = next((s.get("name", "Unknown") for s in subs if s.get("id") == row["assigned_agent_id"]), row["assigned_agent_id"])
            status_lines.append(f"- ID {row['id']}: {assigned_name} is [{row['status']}] on SWO pending task.")

        res = "\\n".join(status_lines)
        emit_debug(f"DEBUG get_swo_queue_status: SUCCESS. Found {len(rows)} tasks.")
        return res
    except sqlite3.OperationalError as e:
        msg = str(e).lower()
        if "locked" in msg or "busy" in msg:
            return "Queue status is temporarily unavailable because the registry database is busy. Please retry in a moment."
        return f"Database error checking queue: {str(e)}"
    except Exception as e:
        return f"Database error checking queue: {str(e)}"
    finally:
        if "conn" in locals():
            conn.close()


# ── Registration ────────────────────────────────────────────────────────────

register(
    ToolSpec(fn=queue_managed_work,    allowed_tool_key="queue_managed_work"),
    ToolSpec(fn=get_swo_queue_status,  allowed_tool_key="get_swo_queue_status"),
    ToolSpec(
        fn=dispatch_swo_internal,
        allowed_tool_key="dispatch_swo_internal",
        exclude_modes={"execute_triage"},  # triage must use delegation_swos schema
    ),
    ToolSpec(
        fn=submit_innovation_swo,
        allowed_tool_key="submit_innovation_swo",
        exclude_modes={"execute_triage"},  # innovation belongs in ideation cycle
    ),
    ToolSpec(
        fn=hire_subordinate_internal,
        allowed_tool_key="hire_subordinate_internal",
    ),
)
