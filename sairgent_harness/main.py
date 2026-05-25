import hashlib
import os
import pathlib
import re
import shlex
import subprocess
import sys
import json
import sqlite3
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from typing import Optional
from pydantic import BaseModel, Field
from pydantic_ai import Agent, RunContext
from pydantic_ai.messages import ModelRequest, ModelResponse, UserPromptPart, TextPart
from pydantic_ai.usage import UsageLimits
from pydantic_ai.exceptions import UsageLimitExceeded
from memory import AgentMemory
from worker_protocol import build_protocol_response, build_side_effects, build_token_usage
from hsm import (
    BriefWritingResult,
    IdeationDecision,
    TriageDecision,
    TriageDecisionIC,
    TriageDecisionManager,
    SynthesisDecision,
    InnovationReport,
    ManagedWorkRequest,
    HireSubordinateSpec,
)
from tools import resolve_tools
from tools._common import (
    BlockedToolError as _RegistryBlockedToolError,
    reset_artifact_dedup as _registry_reset_artifact_dedup,
    reset_audit_counters as _registry_reset_audit_counters,
)
from tools.web_search import init_web_search
from tools.agent_ops import init_agent_ops, write_artifact_file as _registry_write_artifact
from tools.delegation import init_delegation

try:
    from pydantic_ai.models.openai import OpenAIChatModel
    from openai.types import chat as openai_chat
except Exception:
    OpenAIChatModel = None
    openai_chat = None

sidechannel_lock = threading.Lock()
SIDECHANNEL_LOCK_TIMEOUT_SEC = 2.0
SQLITE_READ_TIMEOUT_SEC = 2.0
SQLITE_BUSY_TIMEOUT_MS = 5000
_managed_work_count = 0
_innovation_count = 0
_hire_request_count = 0
_dispatch_count = 0
_sairgent_proposal_count = 0
_artifact_count = 0
_outbox_artifacts = []
_openai_chat_model_patched = False
_tool_api_keys_by_slug = {}
_mcp_credentials_by_slug = {}

# CHA-425 sub-fix 3 — per-SWO artifact dedup guardrail.
# Perry's calculator-app revision spiral produced 17 near-identical artifacts
# in one SWO turn (calculator-app-revision-feedback-response.md,
# -response-final.md, -v2.md, -answered.md, ...) before hitting the
# PydanticAI request_limit. Each create_file succeeded because the
# auto-versioned -vN suffix kept filenames distinct, so there was no
# pushback to teach the agent to stop. This dedup guardrail tracks
# normalized filename prefixes per SWO turn and rejects new writes once
# >5 artifacts share a similar prefix, emitting a stderr warning.
_ARTIFACT_PREFIX_CAP = 5
_artifact_prefix_counts: dict = {}  # normalized prefix -> count within current SWO turn
_artifact_dedup_warned: set = set()  # prefixes we've already warned about


_VERSION_SUFFIX_RE = re.compile(r"-v\d+$", re.IGNORECASE)
_SUFFIX_NOISE_WORDS = (
    "-final", "-response", "-answer", "-answered", "-answers",
    "-user", "-for-user", "-to-user", "-human", "-human-feedback",
    "-revision", "-revision-feedback", "-feedback", "-reply",
    "-resolved", "-where-hosted", "-where-run",
)


def _normalize_artifact_prefix(filename: str) -> str:
    """Return a coarse prefix of a filename for dedup comparison.

    Strips the extension, any -vN / -final / -response / -answer /
    -user suffix chains, normalizes case, and keeps the first ~30
    characters of the base name. Two files count as duplicates if
    their normalized prefixes are equal.
    """
    if not filename:
        return ""
    base = filename.rsplit("/", 1)[-1]
    # Drop extension
    if "." in base:
        base = base.rsplit(".", 1)[0]
    base = base.lower()
    # Strip common thrash suffixes that evidently drive the spiral.
    # Iterate because suffix chains can layer (e.g. "-response-final-v2").
    changed = True
    while changed:
        changed = False
        # Strip any -vN numeric version suffix first
        new_base = _VERSION_SUFFIX_RE.sub("", base)
        if new_base != base:
            base = new_base
            changed = True
            continue
        # Then strip known noise words
        for suf in _SUFFIX_NOISE_WORDS:
            if base.endswith(suf):
                base = base[: -len(suf)]
                changed = True
                break
    return base[:30]


def _check_artifact_dedup(filename: str) -> Optional[str]:
    """Check whether *filename* would breach the per-SWO dedup cap.

    Returns None if the write should proceed, or an error message
    explaining why it was rejected.
    """
    global _artifact_prefix_counts, _artifact_dedup_warned
    prefix = _normalize_artifact_prefix(filename)
    if not prefix:
        return None
    current = _artifact_prefix_counts.get(prefix, 0)
    if current >= _ARTIFACT_PREFIX_CAP:
        if prefix not in _artifact_dedup_warned:
            _artifact_dedup_warned.add(prefix)
            _emit_stderr_line(
                f"[WARN] CHA-425: artifact spam detected — prefix '{prefix}' "
                f"already has {current} files in this SWO turn. Dropping further "
                f"writes with this prefix. This usually means the agent is in a "
                f"revision spiral; return a direct answer instead of another file."
            )
        return (
            f"Artifact write rejected: prefix '{prefix}' has already been used "
            f"{current} times in this SWO turn. Do NOT write more near-duplicate "
            f"files. If you are iterating on an answer, put the final answer in "
            f"your direct_answer / final_response field instead of creating another "
            f"artifact. See CHA-425."
        )
    _artifact_prefix_counts[prefix] = current + 1
    return None


def _reset_artifact_dedup() -> None:
    """Clear the per-SWO dedup tracker. Called at the start of each run."""
    global _artifact_prefix_counts, _artifact_dedup_warned
    _artifact_prefix_counts = {}
    _artifact_dedup_warned = set()


class BlockedToolError(Exception):
    pass


def _extract_token_usage(result, provider: str) -> dict:
    """Extract and normalize token usage from a PydanticAI RunResult."""
    try:
        usage = result.usage()
    except Exception:
        return {}

    cost_usd = None
    # OpenRouter returns cost directly
    if provider == "openrouter":
        cost_usd = (usage.details or {}).get("usage_cost") or (usage.details or {}).get("cost")

    return build_token_usage(
        input_tokens=getattr(usage, 'input_tokens', 0) or 0,
        output_tokens=getattr(usage, 'output_tokens', 0) or 0,
        cache_read_tokens=getattr(usage, 'cache_read_tokens', 0) or 0,
        cache_write_tokens=getattr(usage, 'cache_write_tokens', 0) or 0,
        requests=getattr(usage, 'requests', 0) or 0,
        cost_usd=cost_usd,
    )


def _infer_openai_finish_reason(choice) -> str:
    message = getattr(choice, "message", None)
    if message is not None:
        if getattr(message, "tool_calls", None):
            return "tool_calls"
        if getattr(message, "function_call", None):
            return "function_call"
    return "stop"


def _normalize_openai_finish_reasons(response) -> bool:
    if openai_chat is None or not isinstance(response, openai_chat.ChatCompletion):
        return False

    updated = False
    for choice in getattr(response, "choices", []) or []:
        if getattr(choice, "finish_reason", None) is None:
            # openai>=2.24 can surface chat completions with a null finish_reason
            # that pydantic_ai 0.8.1 later rejects during re-validation.
            choice.finish_reason = _infer_openai_finish_reason(choice)
            updated = True
    return updated


def _install_openai_chat_completion_patch() -> None:
    global _openai_chat_model_patched
    if _openai_chat_model_patched or OpenAIChatModel is None:
        return

    original_process_response = OpenAIChatModel._process_response

    def _patched_process_response(self, response):
        _normalize_openai_finish_reasons(response)
        return original_process_response(self, response)

    OpenAIChatModel._process_response = _patched_process_response
    _openai_chat_model_patched = True


def _emit_stderr_line(line: str, lock_timeout: Optional[float] = SIDECHANNEL_LOCK_TIMEOUT_SEC) -> bool:
    """
    Emit one stderr line under sidechannel_lock.
    Returns True on successful write, False if lock could not be acquired.
    """
    if lock_timeout is None:
        acquired = sidechannel_lock.acquire()
    else:
        acquired = sidechannel_lock.acquire(timeout=lock_timeout)
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
        sidechannel_lock.release()
    return True


def _emit_sidechannel(payload: dict, lock_timeout: Optional[float] = None) -> bool:
    return _emit_stderr_line(json.dumps(payload), lock_timeout=lock_timeout)


def _emit_debug(message: str) -> bool:
    return _emit_stderr_line(message, lock_timeout=1.0)

class HeartbeatEmitter(threading.Thread):
    def __init__(self, token: str, run_id: str):
        super().__init__(daemon=True)
        self.token = token
        self.run_id = run_id
        self._stop_event = threading.Event()
        self.seq = 0

    def stop(self):
        self._stop_event.set()

    def run(self):
        while not self._stop_event.is_set():
            payload = {
                "__sairgent_sidechannel": "heartbeat",
                "token": self.token,
                "run_id": self.run_id,
                "seq": self.seq,
                "status": "COMPUTING"
            }
            _emit_sidechannel(payload, lock_timeout=SIDECHANNEL_LOCK_TIMEOUT_SEC)
            self.seq += 1
            # Wait up to 2 seconds, but exit early if _stop_event happens
            self._stop_event.wait(2.0)

def _get_failed_swo_ids_for_agent(agent_id: str) -> list:
    """Query the kernel registry for SWO IDs assigned to this agent that ended in FAILED status.
    Used to exclude contaminated history from prior failed triage runs."""
    registry_db_path = os.getenv("REGISTRY_DATABASE")
    if not registry_db_path:
        return []
    try:
        db_uri = f"file:{os.path.abspath(registry_db_path)}?mode=ro"
        conn = sqlite3.connect(db_uri, uri=True, timeout=SQLITE_READ_TIMEOUT_SEC)
        conn.execute(f"PRAGMA busy_timeout = {SQLITE_BUSY_TIMEOUT_MS}")
        cursor = conn.cursor()
        cursor.execute(
            "SELECT id FROM swos WHERE assigned_agent_id = ? AND status = 'FAILED'",
            (agent_id,)
        )
        failed_ids = [row[0] for row in cursor.fetchall()]
        conn.close()
        return failed_ids
    except Exception:
        return []


def _load_history(memory: AgentMemory, mode: str, exclude_failed_swos: bool = False):
    exclude_swo_ids = None
    if exclude_failed_swos:
        agent_id = os.getenv("AGENT_ID", "")
        if agent_id:
            exclude_swo_ids = _get_failed_swo_ids_for_agent(agent_id)
    history_rows = memory.get_history(mode=mode, exclude_swo_ids=exclude_swo_ids)
    message_history = []
    for row in history_rows:
        r = row["role"]
        c = row["content"]
        if r == "user":
            message_history.append(ModelRequest(parts=[UserPromptPart(content=c)]))
        elif r == "assistant":
            message_history.append(ModelResponse(parts=[TextPart(content=c)]))
    return message_history


def _decision_log_retention_limit(default: int = 500) -> int:
    raw = os.getenv("DECISION_LOG_MAX_ENTRIES", "").strip()
    if not raw:
        return default
    try:
        val = int(raw)
        return val if val > 0 else default
    except ValueError:
        return default


def _record_ideation_decision_log(
    memory: AgentMemory,
    result: IdeationDecision,
    swo_id: Optional[int],
    run_id: Optional[str],
) -> dict:
    entry = memory.append_decision_log_entry(
        entry_id=str(uuid.uuid4()),
        mode="ideation",
        summary=result.decision_log.summary.strip(),
        rationale=result.decision_log.rationale.strip(),
        outcome=result.decision_log.outcome.strip().upper() or "UNKNOWN",
        confidence=result.decision_log.confidence,
        self_note=(result.decision_log.self_note or "").strip() or None,
        linked_swo_id=swo_id,
        linked_run_id=run_id,
    )
    memory.prune_decision_log(_decision_log_retention_limit())
    return entry


def _render_prompt_sections(
    persona_prompt: str,
    raison: str,
    task: str,
    output_contract: str,
    mode_rules: str,
    extra_sections: str = "",
) -> str:
    extra = extra_sections.strip()
    if extra:
        extra = f"{extra}\n\n"
    return f"""
[Persona]
{persona_prompt.strip()}

[Mission]
{raison.strip()}

{extra}
[Task]
{task.strip()}

[Output Contract]
{output_contract.strip()}

[Mode Rules]
{mode_rules.strip()}
"""

def queue_managed_work(request: ManagedWorkRequest) -> str:
    """
    Queue a work order for async execution. Use for any task requiring research,
    content creation, multi-step analysis, or specialist work.

    Each queued task should have ONE clear deliverable — not a bundle.
    BAD:  "Research competitors and write a positioning doc" (two tasks)
    GOOD: "Research top 5 competitors with pricing and differentiators" (one task)
    Then separately: "Write positioning doc based on competitor research" (second task)

    Set routing_policy to HARD_ROUTE when a specific subordinate must handle it,
    PREFERENCE for a suggested assignee, or NONE for automatic routing.
    """
    global _managed_work_count
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
    sidechannel_payload = {
        "__sairgent_sidechannel": "queue_managed_work",
        "token": token,
        "payload": payload,
    }
    _emit_sidechannel(sidechannel_payload)
    _managed_work_count += 1
    return "Managed work successfully queued."


class PulseJournalEntry(BaseModel):
    """A journal entry for a cadence run."""
    cadence: str = Field(description="Cadence type: 'dawn', 'heartbeat', or 'dusk'")
    entry_type: str = Field(description="Entry type: 'step_started', 'step_completed', 'observation', or 'escalation'")
    summary: str = Field(description="Human-readable summary of what happened")
    detail_json: Optional[str] = Field(default=None, description="Optional JSON string with structured details")


def append_pulse_journal(entry: PulseJournalEntry) -> str:
    """
    Append an entry to the pulse journal for the current cadence run.
    Use this to log steps, observations, and escalations during dawn/heartbeat/dusk cadence execution.
    """
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    run_id = os.getenv("AGENT_RUN_ID", "")
    sidechannel_payload = {
        "__sairgent_sidechannel": "append_pulse_journal",
        "token": token,
        "cadence": entry.cadence,
        "entry_type": entry.entry_type,
        "summary": entry.summary,
        "detail_json": entry.detail_json,
        "run_id": run_id if run_id else None,
    }
    _emit_sidechannel(sidechannel_payload)
    return f"Journal entry appended: [{entry.cadence}] {entry.entry_type} — {entry.summary}"


def submit_innovation_swo(report: InnovationReport) -> str:
    """
    Raise an upward innovation review SWO for your manager without breaking the active task.
    """
    global _innovation_count
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    originating_swo_id = os.getenv("AGENT_SWO_ID")
    payload = {
        "__sairgent_sidechannel": "innovation_swo",
        "token": token,
        "originating_swo_id": int(originating_swo_id) if originating_swo_id else None,
        "report": report.model_dump(),
    }
    _emit_sidechannel(payload)
    _innovation_count += 1
    return "Innovation review SWO successfully submitted to manager."


def hire_subordinate_internal(spec: HireSubordinateSpec) -> str:
    """
    Create a new direct-report agent. Only for real staffing needs during execution —
    not for casual brainstorming or hypothetical org design.

    Write specific, narrow role definitions:
    BAD:  name="Helper", role="General assistant", raison_detre="Helps with things"
    GOOD: name="DataViz", role="Data Visualization Specialist",
          raison_detre="Transforms raw datasets into clear charts, dashboards, and
          visual narratives for executive reporting"
    """
    global _hire_request_count
    if os.getenv("AGENT_CAN_HIRE", "1") != "1":
        return "Error: autonomous hiring is disabled for this agent in the current runtime."
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    payload = {
        "__sairgent_sidechannel": "hire_subordinate",
        "token": token,
        "spec": spec.model_dump(),
    }
    _emit_sidechannel(payload)
    _hire_request_count += 1
    return f"Direct-report hire submitted for {spec.name}."

def dispatch_swo_internal(dispatch_json: str) -> str:
    """
    Dispatch work to a subordinate agent. You MUST call this tool to delegate — do not
    just say "I have assigned it" without invoking this function.

    Write SPECIFIC briefs for the payload — not copies of the original task.
    BAD:  "Handle the market research task."
    GOOD: "Research top 5 competitors in AI agent space. For each: pricing model,
           target customer, key differentiator, estimated ARR. Deliver as a comparison table."

    dispatch_json MUST be a JSON string with keys:
      - "target_id": UUID of the subordinate (from AGENT_SUBORDINATES env var)
      - "payload": a specific, actionable brief with clear deliverable

    Example: {"target_id": "a1b2c3d4-...", "payload": "Draft a 3-section blog post on ..."}
    """
    global _dispatch_count
    _emit_debug(f"DEBUG TOOL EXECUTION: dispatch_swo_internal called with payload length {len(dispatch_json)}")

    # Validate that the input is a properly structured JSON blob
    try:
        parsed = json.loads(dispatch_json)
        target_id = parsed.get("target_id", "")
        payload = parsed.get("payload", "")
    except (json.JSONDecodeError, TypeError):
        return "Error: dispatch_json must be a valid JSON string with 'target_id' and 'payload' keys."

    if not target_id or not payload:
        return "Error: Both 'target_id' (UUID) and 'payload' are required in the dispatch JSON."

    # Kryptonite: Enforce that target_id is an authorized subordinate of this agent.
    # An agent must not be able to dispatch SWOs to arbitrary agents outside its tree.
    subordinates_raw = os.getenv("AGENT_SUBORDINATES", "[]")
    try:
        authorized_ids = {s.get("id", "") for s in json.loads(subordinates_raw)}
    except Exception:
        authorized_ids = set()
    if target_id not in authorized_ids:
        return f"Error: target_id '{target_id}' is not a registered subordinate of this agent. Dispatch denied."

    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")

    # Emit structured sidechannel message consumed by the Rust orchestrator
    sidechannel_payload = {
        "__sairgent_sidechannel": "dispatch_swo",
        "token": token,
        "payload": json.dumps({"target_id": target_id, "payload": payload})
    }
    _emit_sidechannel(sidechannel_payload)
    _dispatch_count += 1
    return "SWO successfully dispatched to the Execution Pipeline. You may inform the user that work has begun."


def _emit_sairgent_proposal(tool_name: str, summary: str, arguments: dict) -> str:
    """Emit a governed sairgent proposal via sidechannel and return confirmation."""
    global _sairgent_proposal_count
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    call_id = str(uuid.uuid4())
    payload = {
        "__sairgent_sidechannel": "sairgent_proposal",
        "token": token,
        "call_id": call_id,
        "tool_name": tool_name,
        "summary": summary,
        "arguments": arguments,
    }
    _emit_sidechannel(payload)
    _sairgent_proposal_count += 1
    return f"Governed proposal submitted: {summary}. The operator will be asked to confirm before execution."


def sairgent_create_project(name: str, summary: str, priority: str = "NORMAL", target_outcome: str = "", tags: str = "") -> str:
    """
    Propose creating a new project. The operator must confirm before it is created.
    Args:
        name: Project name
        summary: Brief description of the project
        priority: NORMAL, HIGH, or CRITICAL
        target_outcome: What success looks like for this project
        tags: Comma-separated tags (e.g. "frontend,urgent")
    """
    if not name.strip():
        return "Error: project name is required."
    tag_list = [t.strip() for t in tags.split(",") if t.strip()] if tags else ["sairgent-agent"]
    arguments = {
        "name": name.strip(),
        "summary": summary.strip(),
        "priority": priority.strip().upper() or "NORMAL",
        "targetOutcome": target_outcome.strip(),
        "tags": tag_list,
    }
    return _emit_sairgent_proposal("create_project", f"Create project '{name.strip()}'", arguments)


def sairgent_create_work_order(title: str, outcome: str, constraints: str = "", priority: str = "NORMAL", project_id: str = "", assignee_agent_name: str = "") -> str:
    """
    Propose creating a new work order (SWO). The operator must confirm before it is created.
    Args:
        title: Work order title
        outcome: What the work order should achieve
        constraints: Any constraints or requirements
        priority: NORMAL, HIGH, or CRITICAL
        project_id: Optional project ID to associate with
        assignee_agent_name: Optional agent name to assign to
    """
    if not title.strip():
        return "Error: work order title is required."
    arguments = {
        "title": title.strip(),
        "outcome": outcome.strip(),
        "constraints": constraints.strip(),
        "priority": priority.strip().upper() or "NORMAL",
    }
    if project_id.strip():
        arguments["projectId"] = project_id.strip()
    if assignee_agent_name.strip():
        arguments["assigneeAgentName"] = assignee_agent_name.strip()
    return _emit_sairgent_proposal("create_work_order", f"Create work order '{title.strip()}'", arguments)


def sairgent_create_agent(name: str, role: str, mission: str, manager_agent_name: str = "") -> str:
    """
    Propose creating a new agent. The operator must confirm before the agent is created.
    Args:
        name: Agent name
        role: Agent's role (e.g. "Data Analyst", "Content Writer")
        mission: What the agent's purpose is
        manager_agent_name: Optional name of the manager agent
    """
    if not name.strip():
        return "Error: agent name is required."
    arguments = {
        "name": name.strip(),
        "role": role.strip(),
        "mission": mission.strip(),
    }
    if manager_agent_name.strip():
        arguments["managerAgentName"] = manager_agent_name.strip()
    return _emit_sairgent_proposal("create_agent", f"Create agent '{name.strip()}' as {role.strip()}", arguments)


def sairgent_update_agent_charter(agent_name: str, role: str = "", mission: str = "", persona_prompt: str = "") -> str:
    """
    Propose updating an agent's charter (role, mission, or persona prompt).
    The operator must confirm before the update is applied.
    Args:
        agent_name: Name of the agent to update
        role: New role (leave empty to keep current)
        mission: New mission (leave empty to keep current)
        persona_prompt: New persona prompt (leave empty to keep current)
    """
    if not agent_name.strip():
        return "Error: agent_name is required."
    arguments = {"agent_name": agent_name.strip()}
    if role.strip():
        arguments["role"] = role.strip()
    if mission.strip():
        arguments["mission"] = mission.strip()
    if persona_prompt.strip():
        arguments["persona_prompt"] = persona_prompt.strip()
    if len(arguments) == 1:
        return "Error: at least one of role, mission, or persona_prompt must be provided."
    return _emit_sairgent_proposal("update_agent_charter", f"Update charter for agent '{agent_name.strip()}'", arguments)


def sairgent_bind_tool_to_agent(agent_name: str, tool_slug: str) -> str:
    """
    Propose binding a tool to an agent. The operator must confirm.
    Args:
        agent_name: Name of the agent
        tool_slug: Slug identifier of the tool to bind (e.g. "tavily-search", "exa-search")
    """
    if not agent_name.strip() or not tool_slug.strip():
        return "Error: both agent_name and tool_slug are required."
    arguments = {"agent_name": agent_name.strip(), "tool_slug": tool_slug.strip()}
    return _emit_sairgent_proposal("bind_tool_to_agent", f"Bind tool '{tool_slug.strip()}' to agent '{agent_name.strip()}'", arguments)


def sairgent_unbind_tool_from_agent(agent_name: str, tool_slug: str) -> str:
    """
    Propose unbinding a tool from an agent. The operator must confirm.
    Args:
        agent_name: Name of the agent
        tool_slug: Slug identifier of the tool to unbind
    """
    if not agent_name.strip() or not tool_slug.strip():
        return "Error: both agent_name and tool_slug are required."
    arguments = {"agent_name": agent_name.strip(), "tool_slug": tool_slug.strip()}
    return _emit_sairgent_proposal("unbind_tool_from_agent", f"Unbind tool '{tool_slug.strip()}' from agent '{agent_name.strip()}'", arguments)


def sairgent_set_project_status(project_name: str, status: str, reason: str = "") -> str:
    """
    Propose changing a project's status. The operator must confirm.
    Args:
        project_name: Name of the project
        status: New status (ACTIVE, PAUSED, ARCHIVED, COMPLETED)
        reason: Optional reason for the status change
    """
    if not project_name.strip() or not status.strip():
        return "Error: both project_name and status are required."
    arguments = {"project_name": project_name.strip(), "status": status.strip().upper()}
    if reason.strip():
        arguments["reason"] = reason.strip()
    return _emit_sairgent_proposal("set_project_status", f"Set project '{project_name.strip()}' to {status.strip().upper()}", arguments)


def sairgent_set_reporting_line(agent_name: str, manager_agent_name: str) -> str:
    """
    Propose changing an agent's reporting line (manager). The operator must confirm.
    Args:
        agent_name: Name of the agent to move
        manager_agent_name: Name of the new manager agent
    """
    if not agent_name.strip() or not manager_agent_name.strip():
        return "Error: both agent_name and manager_agent_name are required."
    arguments = {"agent_name": agent_name.strip(), "manager_agent_name": manager_agent_name.strip()}
    return _emit_sairgent_proposal("set_reporting_line", f"Set '{agent_name.strip()}' to report to '{manager_agent_name.strip()}'", arguments)


def get_swo_queue_status() -> str:
    """
    Check the persistent queue for any active, pending, or recently completed SWOs assigned to your immediate subordinates.
    Use this to give the user real-time status updates on background tasks.
    """
    registry_db_path = os.getenv("REGISTRY_DATABASE")
    if not registry_db_path:
        return "Internal Error: REGISTRY_DATABASE not found in environment."
        
    subordinates_json = os.getenv("AGENT_SUBORDINATES", "[]")
    try:
        subs = json.loads(subordinates_json)
        sub_ids = [s.get("id") for s in subs if s.get("id")]
    except:
        sub_ids = []
        
    if not sub_ids:
        return "You do not have any subordinates to check the status of."
        
    # sqlite parameterized IN clause
    placeholders = ",".join("?" for _ in sub_ids)
    query = f"SELECT id, assigned_agent_id, swo_payload, status FROM active_swos WHERE assigned_agent_id IN ({placeholders}) AND status != 'COMPLETED'"
    
    _emit_debug(f"DEBUG get_swo_queue_status: db={registry_db_path}, subs={sub_ids}")

    try:
        # Read-only + fast-fail lock timeout keeps tool calls non-blocking under write contention.
        db_uri = f"file:{os.path.abspath(registry_db_path)}?mode=ro"
        conn = sqlite3.connect(
            db_uri,
            uri=True,
            timeout=SQLITE_READ_TIMEOUT_SEC,
            isolation_level=None,
        )
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
        _emit_debug(f"DEBUG get_swo_queue_status: SUCCESS. Found {len(rows)} tasks.")
        return res
    except sqlite3.OperationalError as e:
        msg = str(e).lower()
        if "locked" in msg or "busy" in msg:
            return "Queue status is temporarily unavailable because the registry database is busy. Please retry in a moment."
        return f"Database error checking queue: {str(e)}"
    except Exception as e:
        return f"Database error checking queue: {str(e)}"
    finally:
        if 'conn' in locals():
            conn.close()

def _open_registry_readonly():
    """Open a read-only connection to the kernel registry database."""
    registry_db_path = os.getenv("REGISTRY_DATABASE")
    if not registry_db_path:
        return None, "Registry database not available."
    db_uri = f"file:{os.path.abspath(registry_db_path)}?mode=ro"
    conn = sqlite3.connect(db_uri, uri=True, timeout=SQLITE_READ_TIMEOUT_SEC, isolation_level=None)
    conn.row_factory = sqlite3.Row
    conn.execute(f"PRAGMA busy_timeout = {SQLITE_BUSY_TIMEOUT_MS}")
    conn.execute("PRAGMA query_only = ON")
    return conn, None


# ---------------------------------------------------------------------------
# Sairgent "Eye of Sauron" query tools — full registry read access
# These are only bound to the sairgent_chat agent, not kernel-level agents.
# ---------------------------------------------------------------------------

def get_recent_tasks(limit: int = 10) -> str:
    """Get the most recent tasks across the entire system.

    Returns recent tasks with their status, title, and who they're assigned to.
    Use this when the user asks about recent work, last task, or what's been happening.

    Args:
        limit: Maximum number of tasks to return (default 10, max 50)
    """
    limit = min(max(1, limit), 50)
    conn, err = _open_registry_readonly()
    if err:
        return err
    try:
        rows = conn.execute(
            """SELECT s.id, s.work_order_title, s.status, s.kind, s.source,
                      s.created_at, a.name as assignee_name
               FROM active_swos s
               LEFT JOIN agents a ON a.id = s.assigned_agent_id
               ORDER BY s.id DESC
               LIMIT ?""",
            (limit,),
        ).fetchall()
        if not rows:
            return "No tasks found in the system yet."
        lines = []
        for r in rows:
            title = r["work_order_title"] or "(untitled)"
            assignee = r["assignee_name"] or "unassigned"
            lines.append(f"- **#{r['id']}** {title} — {r['status']} (assigned to {assignee}, created {r['created_at']})")
        return "\n".join(lines)
    except Exception as e:
        return f"Error querying tasks: {e}"
    finally:
        conn.close()


def get_task_detail(task_id: int) -> str:
    """Get full details on a specific task by ID number.

    Returns the task's title, status, payload/brief, assignee, and any results.

    Args:
        task_id: The numeric task ID (e.g. 42)
    """
    conn, err = _open_registry_readonly()
    if err:
        return err
    try:
        row = conn.execute(
            """SELECT s.id, s.work_order_title, s.work_order_outcome,
                      s.work_order_constraints, s.status, s.kind, s.source,
                      s.swo_payload, s.created_at, s.priority_class,
                      s.parent_swo_id, a.name as assignee_name,
                      m.name as manager_name
               FROM active_swos s
               LEFT JOIN agents a ON a.id = s.assigned_agent_id
               LEFT JOIN agents m ON m.id = s.manager_agent_id
               WHERE s.id = ?""",
            (task_id,),
        ).fetchone()
        if not row:
            return f"Task #{task_id} not found."
        parts = [
            f"**Task #{row['id']}**: {row['work_order_title'] or '(untitled)'}",
            f"Status: {row['status']}",
            f"Assigned to: {row['assignee_name'] or 'unassigned'}",
        ]
        if row["manager_name"]:
            parts.append(f"Manager: {row['manager_name']}")
        if row["priority_class"]:
            parts.append(f"Priority: {row['priority_class']}")
        if row["work_order_outcome"]:
            parts.append(f"Expected outcome: {row['work_order_outcome']}")
        if row["work_order_constraints"]:
            parts.append(f"Constraints: {row['work_order_constraints']}")
        if row["parent_swo_id"]:
            parts.append(f"Parent task: #{row['parent_swo_id']}")
        parts.append(f"Created: {row['created_at']}")
        # Check for results
        results = conn.execute(
            "SELECT result_payload, created_at FROM swo_results WHERE swo_id = ? ORDER BY created_at DESC LIMIT 1",
            (task_id,),
        ).fetchone()
        if results and results["result_payload"]:
            payload = results["result_payload"]
            if len(payload) > 1000:
                payload = payload[:1000] + "... (truncated)"
            parts.append(f"\n**Result** ({results['created_at']}):\n{payload}")
        return "\n".join(parts)
    except Exception as e:
        return f"Error querying task: {e}"
    finally:
        conn.close()


def get_projects() -> str:
    """Get all projects in the system with their status.

    Use this when the user asks about projects, what's active, or wants an overview.
    """
    conn, err = _open_registry_readonly()
    if err:
        return err
    try:
        rows = conn.execute(
            """SELECT p.id, p.name, p.summary, p.status, p.priority,
                      p.target_outcome, a.name as lead_name, p.created_at
               FROM projects p
               LEFT JOIN agents a ON a.id = p.lead_agent_id
               ORDER BY p.created_at DESC""",
        ).fetchall()
        if not rows:
            return "No projects exist yet. You can create one — just tell me what you're working on."
        lines = []
        for r in rows:
            lead = f", lead: {r['lead_name']}" if r["lead_name"] else ""
            summary = f" — {r['summary']}" if r["summary"] else ""
            lines.append(f"- **{r['name']}** [{r['status']}]{summary} (priority: {r['priority']}{lead})")
        return "\n".join(lines)
    except Exception as e:
        return f"Error querying projects: {e}"
    finally:
        conn.close()


def get_agents_overview() -> str:
    """Get an overview of all agents in the system.

    Shows each agent's name, role, and current status.
    Use when the user asks about the team, available agents, or capabilities.
    """
    conn, err = _open_registry_readonly()
    if err:
        return err
    try:
        rows = conn.execute(
            """SELECT a.id, a.name, a.role, a.raison_detre,
                      COALESCE(h.status, 'IDLE') as presence
               FROM agents a
               LEFT JOIN agent_heartbeats h ON h.agent_id = a.id
               ORDER BY a.name""",
        ).fetchall()
        if not rows:
            return "No agents registered."
        lines = []
        for r in rows:
            lines.append(f"- **{r['name']}** ({r['role']}) — {r['raison_detre'][:80]}")
        return "\n".join(lines)
    except Exception as e:
        return f"Error querying agents: {e}"
    finally:
        conn.close()


def search_history(query: str, limit: int = 20) -> str:
    """Search conversation history across all interactions.

    Searches message content for the given query string. Returns matching
    messages with their timestamps and context.

    Args:
        query: Text to search for in conversation history
        limit: Maximum results to return (default 20, max 50)
    """
    limit = min(max(1, limit), 50)
    conn, err = _open_registry_readonly()
    if err:
        return err
    try:
        # Search the agent's own memory DB first
        memory_db = os.getenv("MEMORY_DATABASE")
        if memory_db and os.path.exists(memory_db):
            mem_conn = sqlite3.connect(f"file:{os.path.abspath(memory_db)}?mode=ro", uri=True, timeout=SQLITE_READ_TIMEOUT_SEC, isolation_level=None)
            mem_conn.row_factory = sqlite3.Row
            mem_conn.execute("PRAGMA query_only = ON")
            rows = mem_conn.execute(
                """SELECT timestamp, role, content, mode
                   FROM interactions
                   WHERE content LIKE ?
                   ORDER BY timestamp DESC
                   LIMIT ?""",
                (f"%{query}%", limit),
            ).fetchall()
            mem_conn.close()
            if rows:
                lines = []
                for r in rows:
                    content = r["content"]
                    if len(content) > 200:
                        content = content[:200] + "..."
                    lines.append(f"- [{r['timestamp']}] **{r['role']}**: {content}")
                return "\n".join(lines)
        return f"No messages found matching '{query}'."
    except Exception as e:
        return f"Error searching history: {e}"
    finally:
        conn.close()


def get_artifacts(task_id: int = 0, limit: int = 20) -> str:
    """Get artifacts (deliverables, files) produced by agents.

    Returns recent artifacts with filenames, which task produced them, and when.
    If task_id is provided, returns only artifacts for that specific task.
    Can also read the content of text/markdown artifacts.

    Args:
        task_id: Optional task ID to filter by (0 = all recent artifacts)
        limit: Maximum number of artifacts to return (default 20, max 50)
    """
    limit = min(max(1, limit), 50)
    conn, err = _open_registry_readonly()
    if err:
        return err
    try:
        if task_id > 0:
            rows = conn.execute(
                """SELECT o.id, o.filename, o.absolute_path, o.created_at,
                          o.swo_id, a.name as agent_name,
                          s.work_order_title as task_title
                   FROM outbox_artifacts o
                   LEFT JOIN agents a ON a.id = o.agent_id
                   LEFT JOIN active_swos s ON s.id = o.swo_id
                   WHERE o.swo_id = ?
                   ORDER BY o.created_at DESC
                   LIMIT ?""",
                (task_id, limit),
            ).fetchall()
        else:
            rows = conn.execute(
                """SELECT o.id, o.filename, o.absolute_path, o.created_at,
                          o.swo_id, a.name as agent_name,
                          s.work_order_title as task_title
                   FROM outbox_artifacts o
                   LEFT JOIN agents a ON a.id = o.agent_id
                   LEFT JOIN active_swos s ON s.id = o.swo_id
                   ORDER BY o.created_at DESC
                   LIMIT ?""",
                (limit,),
            ).fetchall()
        if not rows:
            if task_id > 0:
                return f"No artifacts found for task #{task_id}."
            return "No artifacts have been produced yet."
        lines = []
        for r in rows:
            task_ref = f" (task #{r['swo_id']}: {r['task_title']})" if r["task_title"] else f" (task #{r['swo_id']})"
            agent = r["agent_name"] or "unknown"
            lines.append(f"- **{r['filename']}** by {agent}{task_ref} — {r['created_at']}")
        return "\n".join(lines)
    except Exception as e:
        return f"Error querying artifacts: {e}"
    finally:
        conn.close()


def read_artifact(filename: str) -> str:
    """Read the content of an artifact file by name.

    Returns the full text content of a text-based artifact (markdown, text, JSON, etc).
    For binary files, returns the file path instead.

    Args:
        filename: The artifact filename to read (e.g. 'report-v1.md')
    """
    conn, err = _open_registry_readonly()
    if err:
        return err
    try:
        row = conn.execute(
            """SELECT absolute_path, filename FROM outbox_artifacts
               WHERE filename = ? ORDER BY created_at DESC LIMIT 1""",
            (filename,),
        ).fetchone()
        if not row:
            # Try partial match
            row = conn.execute(
                """SELECT absolute_path, filename FROM outbox_artifacts
                   WHERE filename LIKE ? ORDER BY created_at DESC LIMIT 1""",
                (f"%{filename}%",),
            ).fetchone()
        if not row:
            return f"Artifact '{filename}' not found. Use get_artifacts() to see available files."
        path = row["absolute_path"]
        if not os.path.exists(path):
            return f"Artifact file exists in registry but not on disk: {path}"
        # Check if text-readable
        text_extensions = {'.md', '.txt', '.json', '.csv', '.yaml', '.yml', '.toml', '.html', '.xml', '.py', '.rs', '.ts', '.js'}
        ext = os.path.splitext(path)[1].lower()
        if ext not in text_extensions:
            return f"Binary artifact at: {path}\n(Use the desktop file browser to view this file.)"
        try:
            with open(path, 'r', encoding='utf-8') as f:
                content = f.read()
            if len(content) > 8000:
                return f"**{row['filename']}** ({len(content)} chars, showing first 8000):\n\n{content[:8000]}\n\n... (truncated)"
            return f"**{row['filename']}**:\n\n{content}"
        except UnicodeDecodeError:
            return f"Binary artifact at: {path}"
    except Exception as e:
        return f"Error reading artifact: {e}"
    finally:
        conn.close()


def build_ideation_prompt(
    persona_prompt: str,
    raison: str,
    subordinates_json: str,
    recent_decision_log: str,
) -> str:
    subs_formatted = _format_subordinates_rich(subordinates_json)

    return _render_prompt_sections(
        persona_prompt,
        raison,
        f"""You are on a manual proactive review cycle.

Available subordinates:
{subs_formatted}""",
        """Return structured JSON with:
- `ideation_summary`: brief plain-text summary after raising any valid innovation SWOs.
- `decision_log`: object with `summary`, `rationale`, `outcome`, optional `self_note`, and optional `confidence`.""",
        """Generate 1-3 concrete proposals at most.
Use `submit_innovation_swo` for each worthwhile idea.
Do not simulate delegation or completion if you did not create a real proposal.
Always fill `decision_log` with the most important lesson, operating decision, or heuristic from this cycle.
Use `SUCCESS`, `PARTIAL`, `FAILED`, or `UNKNOWN` for `decision_log.outcome`.""",
        _runtime_skill_section()
        + _capability_notes()
        + "[Recent Decision Log]\n"
        + recent_decision_log
        + "\n",
    )


def _safe_validate_filename(filename: str, base_dir: str) -> Optional[str]:
    """
    Validates that the resolved path of 'filename' inside 'base_dir' does
    not escape the base directory — including via symlinks.
    Returns the resolved (real) path or None if unsafe.
    """
    if not filename or not base_dir:
        return None
    # Reject separator characters and null bytes immediately
    if any(c in filename for c in ('/', '\\', '\x00')):
        return None
    candidate = pathlib.Path(base_dir).resolve() / filename
    # Resolve all symlinks in the candidate path
    try:
        real = pathlib.Path(os.path.realpath(candidate))
        real.relative_to(pathlib.Path(os.path.realpath(base_dir)))
    except (ValueError, OSError):
        return None
    return str(real)


def read_agent_file(relative_path: str) -> str:
    """
    Read a file from your agent workspace using a relative path from the agent root.
    Examples: 'context/pricing.md', 'artifacts/report-v2.md'.
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
    # Kryptonite: cap reads at 10MB to prevent memory exhaustion
    MAX_READ_BYTES = 10 * 1024 * 1024
    try:
        with open(safe_path, "r", encoding="utf-8") as f:
            content = f.read(MAX_READ_BYTES + 1)
        if len(content.encode("utf-8")) > MAX_READ_BYTES:
            return f"Error: File '{relative_path}' exceeds the maximum readable size of 10MB."
        return content if content else "(file is empty)"
    except FileNotFoundError:
        return f"Error: File '{relative_path}' not found in agent workspace."
    except Exception as e:
        return f"Error reading file: {str(e)}"


def _versioned_artifact_path(artifacts_dir: str, filename: str) -> tuple[str, str]:
    safe_path = _safe_validate_filename(filename, artifacts_dir)
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
        versioned_path = _safe_validate_filename(versioned_name, artifacts_dir)
        if versioned_path is None:
            raise ValueError("invalid versioned filename")
        if not pathlib.Path(versioned_path).exists():
            return versioned_name, versioned_path
        version += 1


def write_artifact_file(filename: str, content: str) -> str:
    """
    Write a substantial deliverable as a versioned artifact file.

    Use this for reports, plans, analyses, or any output the user should access later.
    Do NOT use for short answers that fit in a chat reply — only for content worth saving.

    The filename must be plain (e.g. 'competitor-analysis.md'), no path separators.
    Existing files are never overwritten; iterative writes get auto-versioned suffixes.
    """
    global _artifact_count, _outbox_artifacts
    artifacts_dir = os.getenv("AGENT_ARTIFACTS", "")
    if not artifacts_dir:
        return "Error: AGENT_ARTIFACTS not configured for this agent."
    # Kryptonite: cap at 10MB to prevent disk exhaustion
    MAX_BYTES = 10 * 1024 * 1024
    if len(content.encode("utf-8")) > MAX_BYTES:
        return f"Error: Content exceeds maximum allowed size of 10MB."
    # CHA-425 sub-fix 3 — refuse writes that would breach the dedup cap.
    dedup_err = _check_artifact_dedup(filename)
    if dedup_err is not None:
        return dedup_err
    try:
        final_name, safe_path = _versioned_artifact_path(artifacts_dir, filename)
        with open(safe_path, "w", encoding="utf-8") as f:
            f.write(content)
        token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
        swo_id = os.getenv("AGENT_SWO_ID")
        if token and swo_id:
            _emit_sidechannel({
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
            _emit_stderr_line(
                f"[WARNING] Skipping sidechannel artifact registration for '{final_name}': {', '.join(reasons)}"
            )
        _artifact_count += 1
        _outbox_artifacts.append({"filename": final_name, "absolute_path": safe_path})
        return f"Successfully wrote {len(content)} characters to {safe_path}."
    except Exception as e:
        return f"Error writing file: {str(e)}"


def read_inbox_file(filename: str) -> str:
    return read_agent_file(filename)


def write_outbox_file(filename: str, content: str) -> str:
    return write_artifact_file(filename, content)


def _pretty_or_text(value: str) -> tuple[str, str]:
    try:
        parsed = json.loads(value)
    except Exception:
        return value.strip(), "md"
    return json.dumps(parsed, indent=2), "json"


def _auto_materialize_output(mode: str, payload: str) -> None:
    if _outbox_artifacts:
        return

    filename: Optional[str] = None
    content: Optional[str] = None

    if mode == "execute_triage" and payload:
        body, ext = _pretty_or_text(payload)
        filename = f"triage-output.{ext}"
        content = body
    elif mode == "execute_synthesis" and payload:
        body, ext = _pretty_or_text(payload)
        filename = f"synthesis-output.{ext}"
        content = body
    elif mode == "chat_mode" and payload:
        filename = "chat-reply.md"
        content = payload.strip()
    elif mode == "format_swo" and payload:
        filename = "formatted-swo.md"
        content = payload.strip()
    elif mode == "execute_ideation" and payload:
        filename = "ideation-summary.md"
        content = payload.strip()

    if filename and content:
        write_artifact_file(filename, content + ("\n" if not content.endswith("\n") else ""))


def _load_skill_index() -> list[dict]:
    raw = os.getenv("AGENT_SKILL_INDEX_JSON", "[]")
    try:
        parsed = json.loads(raw)
    except Exception:
        parsed = []
    return [item for item in parsed if isinstance(item, dict)]


def _skill_index_map() -> dict[str, dict]:
    return {
        item.get("id", ""): item
        for item in _load_skill_index()
        if item.get("id")
    }


def _slugify_skill_name(name: str) -> str:
    slug = []
    last_dash = False
    for ch in name.lower():
        if ch.isalnum():
            slug.append(ch)
            last_dash = False
        elif not last_dash:
            slug.append("-")
            last_dash = True
    return "".join(slug).strip("-") or "skill"


def _runtime_skill_section() -> str:
    entries = _load_skill_index()
    if not entries:
        return "[Available Skills]\nNone.\n"

    lines = []
    for entry in entries:
        tags = ", ".join(entry.get("tags", [])[:4]) or "no-tags"
        marker = "preselected" if entry.get("preselected") else "available"
        summary = entry.get("summary", "")
        lines.append(f"- {entry.get('name', 'Unknown')} [{marker}] :: {summary} (tags: {tags})")
    return (
        "[Available Skills]\n"
        + "\n".join(lines)
        + "\nUse `list_available_skills` to inspect the catalog and `load_skill` only when a skill is relevant.\n"
    )


def _manager_execution_contract_section() -> str:
    """CHA-412: shared Manager Execution Contract prompt fragment.

    Returns the job-plan artifact convention that every Manager-class agent
    should follow when running a multi-turn Ralph loop. Injected into triage
    and synthesis prompts for agents with subordinates. Specialists and
    individual contributors do not see this section — they produce
    deliverables in one shot without durable plan state.

    The full contract is documented in ops/manager_execution_contract.md.
    """
    return (
        "[Manager Execution Contract]\n"
        "You are a Manager. On your first turn of a multi-step job, write a "
        "job_plan.md artifact to your own workspace/ directory (via write_artifact_file "
        "or create_file if file_write is granted). Structure it as a numbered task list:\n"
        "  1. <sub-task description> — status: pending | delegated | accepted | rejected\n"
        "  2. ...\n"
        "On every subsequent turn, read job_plan.md FIRST via read_agent_file to recover "
        "your plan, cross off completed items, and decide what to delegate next. Update the "
        "plan as you go.\n"
        "When the plan is fully complete, emit ACCEPT_AND_COMPLETE with a final_response that "
        "synthesizes all accepted deliverables. When the plan still has pending items, emit "
        "ACCEPT_AND_CONTINUE with next_step_brief naming what you intend to delegate next.\n"
        "This is how managers maintain durable state across turns. Without the plan, every "
        "turn re-discovers the work from scratch and the loop drifts.\n"
    )


def _capability_notes() -> str:
    raw_manifest = os.getenv("AGENT_MANIFEST_JSON", "")
    if not raw_manifest:
        return "[Governed Capabilities]\nNone.\n"
    try:
        manifest = json.loads(raw_manifest)
    except Exception:
        return "[Governed Capabilities]\nNone.\n"

    capabilities = set(manifest.get("capabilities") or [])
    lines = []
    if "web_search" in capabilities:
        search_provider = os.getenv("AGENT_SEARCH_PROVIDER_SLUG", "").strip()
        search_status = os.getenv("AGENT_SEARCH_PROVIDER_STATUS", "").strip()
        lines.append(
            "- Web research is authorized. Use browser/search capabilities for time-sensitive factual claims when available, and cite sources."
        )
        if search_provider and search_status == "configured":
            lines.append(
                f"- Live web search is configured through the assigned provider '{search_provider}'. Use the `web_search` tool when current facts matter."
            )
        elif search_status == "missing_credential" and search_provider:
            lines.append(
                f"- Web search is assigned to '{search_provider}', but its credential is missing. If current research is required, block and direct the operator to Settings."
            )
        else:
            lines.append(
                "- No web search provider is assigned. If current research is required, block and direct the operator to Tools to attach Tavily Search or Exa Search."
            )
    if not lines:
        lines.append("None.")
    return "[Governed Capabilities]\n" + "\n".join(lines) + "\n"


def _search_provider_slug() -> str:
    return os.getenv("AGENT_SEARCH_PROVIDER_SLUG", "").strip()


def _search_provider_status() -> str:
    return os.getenv("AGENT_SEARCH_PROVIDER_STATUS", "").strip()


def _search_blocked_reason() -> str:
    provider_slug = _search_provider_slug()
    provider_status = _search_provider_status()
    if not provider_slug or provider_status == "missing_binding":
        return (
            "Live web research is authorized for this agent, but no search provider is assigned. "
            "Open Tools and attach Tavily Search or Exa Search to this agent, then retry."
        )
    if provider_status == "missing_credential":
        return (
            f"Live web research is assigned to '{provider_slug}', but its credential is missing. "
            "Open Settings, save the provider API key, then retry."
        )
    return (
        f"Live web research through '{provider_slug}' is unavailable right now. "
        "Check Tools and Settings, then retry."
    )


def _perform_tavily_search(query: str, api_key: str) -> list[dict]:
    payload = json.dumps(
        {
            "query": query,
            "search_depth": "advanced",
            "max_results": 5,
            "include_answer": False,
        }
    ).encode("utf-8")
    request = urllib.request.Request(
        "https://api.tavily.com/search",
        data=payload,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {api_key}",
        },
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=20) as response:
        parsed = json.loads(response.read().decode("utf-8"))
    results = parsed.get("results", [])
    return [
        {
            "title": item.get("title", ""),
            "url": item.get("url", ""),
            "snippet": item.get("content", ""),
        }
        for item in results[:5]
    ]


def _perform_exa_search(query: str, api_key: str) -> list[dict]:
    payload = json.dumps(
        {
            "query": query,
            "type": "auto",
            "numResults": 5,
            "contents": {"text": True, "highlights": {"numSentences": 2}},
        }
    ).encode("utf-8")
    request = urllib.request.Request(
        "https://api.exa.ai/search",
        data=payload,
        headers={
            "Content-Type": "application/json",
            "x-api-key": api_key,
        },
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=20) as response:
        parsed = json.loads(response.read().decode("utf-8"))
    results = parsed.get("results", [])
    normalized = []
    for item in results[:5]:
        text = item.get("text") or ""
        highlights = item.get("highlights") or []
        snippet = highlights[0] if highlights else text[:400]
        normalized.append(
            {
                "title": item.get("title", ""),
                "url": item.get("url", ""),
                "snippet": snippet,
            }
        )
    return normalized


def web_search(query: str) -> str:
    cleaned = query.strip()
    if not cleaned:
        raise BlockedToolError("Web search query cannot be empty.")

    provider_slug = _search_provider_slug()
    provider_status = _search_provider_status()
    api_key = _tool_api_keys_by_slug.get(provider_slug, "")

    if provider_status != "configured" or not provider_slug or not api_key:
        raise BlockedToolError(_search_blocked_reason())

    try:
        if provider_slug == "tavily":
            results = _perform_tavily_search(cleaned, api_key)
        elif provider_slug == "exa":
            results = _perform_exa_search(cleaned, api_key)
        else:
            raise BlockedToolError(
                f"Assigned search provider '{provider_slug}' is not supported in this runtime yet."
            )
    except BlockedToolError:
        raise
    except urllib.error.HTTPError as exc:
        raise BlockedToolError(
            f"Web search provider '{provider_slug}' rejected the request ({exc.code}). Verify the credential in Settings and retry."
        ) from exc
    except urllib.error.URLError as exc:
        raise BlockedToolError(
            f"Web search provider '{provider_slug}' is unreachable right now. Retry when network access is available."
        ) from exc
    except Exception as exc:
        raise BlockedToolError(
            f"Web search through '{provider_slug}' failed unexpectedly: {exc}"
        ) from exc

    if not results:
        return json.dumps({"provider": provider_slug, "query": cleaned, "results": []}, indent=2)

    return json.dumps(
        {
            "provider": provider_slug,
            "query": cleaned,
            "results": results,
        },
        indent=2,
    )


def list_available_skills(query: str = "") -> str:
    entries = _load_skill_index()
    query_lc = query.strip().lower()
    if query_lc:
        filtered = []
        for entry in entries:
            haystack = " ".join(
                [
                    entry.get("name", ""),
                    entry.get("summary", ""),
                    " ".join(entry.get("tags", []) or []),
                    " ".join(entry.get("trigger_hints", []) or []),
                ]
            ).lower()
            if query_lc in haystack:
                filtered.append(entry)
        entries = filtered
    return json.dumps(entries, indent=2)


def load_skill(skill_id: str) -> str:
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
        metadata = {}

    payload = {
        "id": skill_id,
        "name": row["name"],
        "version": row["current_version"],
        "summary": metadata.get("summary", ""),
        "tags": metadata.get("tags", []),
        "trigger_hints": metadata.get("trigger_hints", []),
        "runtime_path": runtime_path,
        "markdown": row["raw_markdown"],
    }
    return json.dumps(payload, indent=2)


def _format_subordinates_rich(subordinates_json: str) -> str:
    """Format subordinate list with raison_detre for better routing signal.
    Uses name + role (no UUIDs) — the kernel resolves names to IDs."""
    try:
        subs = json.loads(subordinates_json) if subordinates_json else []
    except Exception:
        subs = []
    if not subs:
        return "None. You are an individual contributor — answer directly."
    lines = []
    for s in subs:
        name = s.get("name", "Unknown")
        role = s.get("role", "Unknown")
        raison = s.get("raison", "")
        line = f"- **{name}** (Role: {role})"
        if raison:
            line += f" — {raison}"
        lines.append(line)
    return "\n".join(lines)


def build_triage_prompt(persona_prompt: str, raison: str, subordinates_json: str, recent_judgments: str = "", revision_feedback: str = "") -> str:
    subs_formatted = _format_subordinates_rich(subordinates_json)
    requested_assignee_id = os.getenv("AGENT_REQUESTED_ASSIGNEE_ID", "")
    requested_assignee_name = os.getenv("AGENT_REQUESTED_ASSIGNEE_NAME", "")
    routing_policy = os.getenv("AGENT_ROUTING_POLICY", "NONE")

    has_subs = bool(subs_formatted and "individual contributor" not in subs_formatted)

    if has_subs:
        task_section = f"""You are a MANAGER. Your primary job is to ROUTE work to the right subordinate, not do it yourself.

A task has been assigned to you. Read it carefully, then decide who on your team should handle it.

Active routing contract:
- Requested assignee name: {requested_assignee_name or "(none)"}
- Routing policy: {routing_policy}

Your team:
{subs_formatted}"""
    else:
        task_section = f"""A task has been assigned to you. Read it carefully and produce the deliverable.

{subs_formatted}"""

    if has_subs:
        mode_rules = """# Triage decision rules

## Rule 1 — SINGLE-FIT ROUTING IS A HARD MUST (CHA-423 / CHA-426)

If ONE subordinate's role cleanly owns the deliverable type, delegate to that single person ONLY. \
This is not a preference — it is a HARD REQUIREMENT. Fan-out to multiple specialists on a single-fit \
task is the single worst failure mode this prompt exists to prevent.

### Self-check BEFORE returning your decision

If your `delegation_targets` list contains more than ONE name, stop and answer these questions in \
your `reasoning` field:

1. What deliverable does each specialist uniquely produce that the others cannot?
2. If I removed any one of them, what specific piece of the deliverable would be missing?
3. Am I adding specialists because the task genuinely needs multi-domain synthesis, or because \
I'm hedging, seeking validation, or trying to be thorough?

If you cannot answer #1 and #2 concretely for every specialist in your list, REDUCE to single-fit \
before returning. "Good to get another perspective" is not a valid reason — it is the failure mode.

### Single-fit examples (delegate to exactly ONE)

- "Build a calculator app" → delegation_targets: ["Felicity"] (engineering build, Felicity is the hands-on CTO)
- "Write a blog post about our launch" → delegation_targets: ["Stacker"] (long-form content)
- "Research top 5 competitors' pricing" → delegation_targets: ["Lois"] (research)
- "Design a landing page wireframe" → delegation_targets: ["Jimmy"] (UX/design)
- "Draft the Q3 email campaign sequence" → delegation_targets: ["Lucy"] (campaign execution)

### Multi-fit fan-out (2+ specialists) is legitimate ONLY when synthesis genuinely requires it

- "Product launch plan" → Cat Grant (positioning) + Lucy (campaign) + Felicity (eng readiness) — \
distinct deliverables that combine into the plan
- "Q3 competitive positioning dossier" → Lois (research) + Cat Grant (positioning) + Lex (market sizing) — \
each produces a distinct section
- "Post-mortem on the outage" → Felicity (technical root cause) + Kryptonite (security impact) + \
Perry (ops coordination) — each reviews the incident from a non-overlapping angle

Notice the pattern: in multi-fit cases, each specialist produces a *distinct section* of the final \
deliverable. If you cannot name the distinct section each specialist owns, you are over-delegating.

## Rule 2 — NO-FIT ESCALATION: hire or self-execute instead of fanning out (CHA-426 / CHA-427)

If NO subordinate's role cleanly owns the deliverable type — if the closest fit is "partial, loosely \
related" rather than "this is their job" — DO NOT fan out to 3+ partial-fit specialists as a substitute \
for the right role. Two correct actions in this situation:

### Option A — ANSWER_DIRECTLY if the deliverable fits your own expertise

You are a manager AND you have your own domain expertise. If the task genuinely falls within YOUR \
role's scope (e.g., you are a CTO and the task is "sketch the deployment architecture"; you are a \
COO and the task is "write the weekly ops digest"), execute it directly using ANSWER_DIRECTLY and \
produce the actual deliverable. Do not delegate work that is yours.

### Option B — Hire the missing role via hire_subordinate_internal (CHA-427 / CHA-428)

**ANY manager can hire — this is not a Perry-only privilege.** If the task requires a role your team \
does not have, use `hire_subordinate_internal` to bring in the missing role and delegate to the newly \
hired agent. The kernel gate will enforce the `autonomous_hiring_mode` policy (default: ANY_MANAGER) \
and a per-manager direct-reports cap to prevent sprawl — you do not need to ask permission.

When hiring, provide a precise role + raison_detre + cron schedule. Vague "Generalist" hires are \
rejected by the cultural standard even when the kernel allows them. Specific roles win.

**CHA-428 — hiring on behalf of another manager.** When the task is "build out X's team" or "hire \
engineers for Felicity", the new hires should report to X / Felicity, not to you. Set the \
`reports_to` field on HireSubordinateSpec to the target manager's exact name (e.g. "Felicity"). \
The kernel authorizes the placement only if:
- You are the root (Perry), OR
- You are an ancestor of the target in the org tree, OR
- You ARE the target (which is the normal "hire under me" case; reports_to can be omitted).

The per-manager cap is checked against the TARGET's direct reports, not yours — so Perry hiring \
five new devs "reports_to: Felicity" is gated by Felicity's headcount, not Perry's. Leave \
reports_to unset when you genuinely want the new agent on your own team.

Example — Perry asked "build out Felicity's dev team":
- HireSubordinateSpec(name="Alex", role="Frontend Dev", raison_detre="...", reports_to="Felicity")
- HireSubordinateSpec(name="Kai", role="Backend Dev", raison_detre="...", reports_to="Felicity")
- Each new agent's parent is set to Felicity's id, not Perry's. Felicity's direct_reports count
  climbs by 5 when she's the org-class Manager receiving them.

### Worked example — Felicity (CTO) asked to build a calculator app

- Felicity's team has one direct report: Workshop Operations Lead (operations, not a developer).
- No subordinate cleanly owns "build a web calculator".
- Felicity's own role (CTO & Lead Engineer) DOES cleanly own this — she writes the code herself \
using `create_file` and emits ANSWER_DIRECTLY with a pointer to workspace/calculator/ (Option A).
- Alternatively, if the calculator were part of a larger product and Felicity wanted dedicated \
frontend capacity, she could hire "Alex — Frontend Dev" with raison_detre "Ship React/HTML/CSS/JS \
frontends for Syllogism product features" and delegate the task to Alex (Option B).
- What Felicity MUST NOT do: fan out to Cat Grant + Lex + Lois + Lucy seeking "multiple perspectives" \
on a calculator. That is the anti-pattern this rule exists to prevent.

### The anti-pattern (do not do this)

A 5-agent fan-out on "build a calculator" when you have no dev is the worst of both worlds — you \
burn tokens, get inconsistent deliverables, and still do not end up with runnable code. The correct \
fixes are (A) self-execute if you own the skill or (B) hire the missing specialist. There is no third \
option where fanning out to partial-fit specialists is acceptable.

## Rule 3 — DELEGATE cleanly when single-fit applies

When Rule 1 fires (one clean single-fit specialist), delegate to that person with a SPECIFIC brief. \
Do not copy the original task verbatim — write a brief that names the exact deliverable, constraints, \
and acceptance criteria.

   Bad: "Handle this task about market research."
   Good: "Research the top 5 competitors in the AI agent space. For each, document: pricing model, \
target customer, key differentiator, estimated ARR. Deliver as a structured comparison table."

## Rule 4 — ANSWER_DIRECTLY when you own the deliverable OR it is trivial

ANSWER_DIRECTLY is allowed when ANY of these is true:

- The task is a simple factual question, trivial lookup, or clarification.
- The deliverable falls within YOUR OWN role's expertise and no subordinate is a better fit.
- The task is a clarification question about a prior deliverable (see Rule 5).

If you are unsure whether a subordinate is a better fit than you, ask yourself: "If a human operator \
looked at this task, would they say 'this is Felicity's job' or 'this is Perry's job'?" Answer that \
question honestly and act on it.

## Rule 5 — CLARIFICATION QUESTIONS about prior deliverables (CHA-425 sub-fix 1)

If the task you are seeing is a HUMAN REVISION FEEDBACK that asks a clarification question about a \
prior deliverable — "where do I run this", "how do I use this", "who built it", "what does X mean", \
"why did you pick Y" — answer it DIRECTLY from the prior synthesis context and your memory. Do NOT \
re-delegate to specialists to "go find the answer". The answer is in the context you already have. \
Clarification answers belong in your direct_answer, not in a new delegation fan-out. A spiral of \
re-delegations on a clarification is the single worst failure mode this prompt exists to prevent.

## Rule 6 — When delegating, use exact subordinate names

Use the subordinate's exact name in delegation_targets (e.g. "Lois", "Cat Grant", "Felicity"). If a \
hard route is present via the active routing contract, delegate to that exact subordinate or provide \
a typed exception.

## Rule 7 — A direct answer must contain the actual deliverable

Do not treat acknowledgements, meta-commentary, or plans ("I will research this") as a direct answer. \
A direct answer must contain the actual deliverable content requested.

## Rule 8 — BUILD, don't describe (CHA-428 — managers included)

When you ANSWER_DIRECTLY on a task that calls for code, files, scripts, configs, or any artifact a \
user needs to RUN or OPEN, use the dark factory tools to write the ACTUAL files. The triage toolbox \
now includes create_file, edit_file, delete_file, mkdir, run_command, and write_artifact_file — you \
have the full builder surface available from your first turn. There is no second "execution phase" \
where you get to build later. Build now, in triage, with real tools.

   Good: create_file("workspace/todo/index.html", "<html>...</html>"),
         create_file("workspace/todo/styles.css", "..."),
         create_file("workspace/todo/app.js", "..."),
         direct_answer = "Built at workspace/todo/ — open index.html in a browser. Uses localStorage for persistence."

   Bad: direct_answer = "Here is the todo app spec: ```html\\n<html>...</html>\\n``` ..."
   Bad: write_artifact_file("todo-app-runbook.md", "# Todo App Runbook\\n## Scope\\n...")

A markdown runbook about the code is NOT the deliverable. A markdown runbook is performance theatre. \
The user cannot open a `.md` file in a browser and use your todo app. If the task is "build X", the \
deliverable is the actual X, saved as real files to workspace/. Prose deliverables (research reports, \
strategy docs, analysis) still go in direct_answer as text — file-writing is for tasks where the \
deliverable IS a file, not every deliverable. The test: "can a human operator USE this output without \
reading prose first?" If the answer is no, you built the wrong thing."""
    else:
        mode_rules = """# Execution rules

1. READ the task thoroughly. Understand what deliverable is expected.

2. You are an individual contributor. Produce the deliverable directly using ANSWER_DIRECTLY.

3. Your direct_answer must contain the actual deliverable — not a promise, acknowledgement, or meta-commentary.

4. PRODUCE REAL FILES FOR CODE AND DOCUMENT TASKS (CHA-424). If the task calls for code, \
HTML/CSS/JS, configuration files, scripts, or any artifact a user needs to RUN or OPEN, and you \
have the file_write capability: use create_file to write the actual files to your workspace/ \
subtree (e.g. workspace/calculator/index.html, workspace/calculator/script.js). Your direct_answer \
should be a short pointer that lists the files you wrote and how to use them, NOT a markdown \
runbook with fenced code blocks describing the code. A runbook is not a runnable deliverable — \
the user cannot open a markdown file in a browser and use your calculator.

   Good: create_file("workspace/calculator/index.html", "<html>...</html>"),
         create_file("workspace/calculator/script.js", "..."),
         direct_answer = "Built at workspace/calculator/ — open index.html in a browser to use."

   Bad: direct_answer = "Here is the runbook: ```html\\n<html>...</html>\\n``` ..."

   For prose deliverables (research reports, strategy docs, synthesis summaries, analysis) return \
   the text directly in direct_answer — no file creation required. File-writing is for tasks where \
   the deliverable IS a file, not every deliverable.

5. If the task requires specialist skills you lack, say so clearly in your answer."""

    # CHA-425 sub-fix 1 — inject a high-priority clarification hint when the caller
    # supplied human revision feedback. The LLM sees this BEFORE the system rules
    # section so it biases toward ANSWER_DIRECTLY for clarification questions.
    revision_hint = ""
    if revision_feedback and revision_feedback.strip():
        revision_hint = (
            "[Human Revision Feedback Present]\n"
            "The user has provided revision feedback on your prior deliverable. Read the feedback "
            "carefully FIRST. If it is a clarification question about the prior work (how do I use "
            "it, where does it run, who built it, what does X mean), answer directly from your prior "
            "synthesis context — do NOT re-delegate. Only fan out to specialists if the feedback "
            "asks for genuinely new work (add a feature, change the design, rebuild with different "
            "constraints). See clarification-question rule #5 in the triage decision rules below.\n"
            f"Prior feedback: {revision_feedback.strip()[:500]}\n"
        )

    return _render_prompt_sections(
        persona_prompt,
        raison,
        task_section,
        "Respond strictly according to the TriageDecision schema.",
        mode_rules,
        _runtime_skill_section()
        + _capability_notes()
        + (_manager_execution_contract_section() if has_subs else "")
        + revision_hint
        + (
            "[Recent Judgments]\n"
            "These are your most recent decision-log entries. Use them to inform routing quality — "
            "avoid repeating past mistakes and build on approaches that worked.\n"
            + recent_judgments + "\n"
            if recent_judgments and recent_judgments != "No prior decision log entries recorded."
            else ""
        ),
    )

def build_write_briefs_prompt(persona_prompt: str, raison: str, routing_targets_json: str) -> str:
    """Build prompt for the write_briefs mode where the kernel has pre-selected routing targets."""
    try:
        targets = json.loads(routing_targets_json) if routing_targets_json else []
    except Exception:
        targets = []
    target_lines = []
    for t in targets:
        name = t.get("name", "Unknown")
        role = t.get("role", "Unknown")
        raison_text = t.get("raison", "")
        line = f"- **{name}** (Role: {role})"
        if raison_text:
            line += f" — {raison_text}"
        target_lines.append(line)
    target_block = "\n".join(target_lines) if target_lines else "(no targets)"

    return _render_prompt_sections(
        persona_prompt,
        raison,
        f"""You are writing work briefs for your team. The routing decision has already been made — \
your job is to write excellent, specific briefs that set each person up for success.

Assigned team members:
{target_block}""",
        "Respond strictly according to the BriefWritingResult schema. Every team member listed above MUST get a brief.",
        """# Brief-writing rules

1. READ the task thoroughly. Understand the deliverable and success criteria.

2. For EACH assigned team member, write a brief that:
   - Is SPECIFIC to their role and expertise
   - Clearly states what deliverable they should produce
   - Includes context they need (not just a copy of the original request)
   - Sets quality criteria where relevant

3. Bad brief: "Handle the market research part."
   Good brief: "Research the top 5 competitors in the AI HR space. For each, document: \
product capabilities, pricing model, target customer segment, and key differentiator. \
Deliver as a structured comparison table with a 1-paragraph executive summary."

4. If the task is simple enough that one person can handle it all, write one brief for the best-fit person.

5. Use exact subordinate names as they appear in the team list.""",
        _runtime_skill_section() + _capability_notes(),
    )


def build_synthesis_prompt(persona_prompt: str, raison: str, subordinates_json: str) -> str:
    return _render_prompt_sections(
        persona_prompt,
        raison,
        "You previously delegated work to subordinates and their results have returned for your review.",
        "Respond strictly according to the SynthesisDecision schema.",
        """# Synthesis review — three-question flow (CHA-409)

Answer these questions in order to pick the right action:

## Q1: Is each subordinate's deliverable acceptable?
- Acceptable: concrete output (analysis, content, data, artifact) that addresses the brief.
- NOT acceptable: acknowledgements, promises, plans, off-topic content, factual errors, \
missing critical parts.

If ANY deliverable is not acceptable → **REJECT_AND_REVISE** (see below).

## Q2 (only if all acceptable): Is the entire job complete?
- Complete: the deliverables collectively answer the original task fully. Nothing material is missing.
- NOT complete: this deliverable covers only one part of a larger job. More delegations are needed.

If complete → **ACCEPT_AND_COMPLETE**
If more work remains → **ACCEPT_AND_CONTINUE**

---

## Action guide

### ACCEPT_AND_COMPLETE
All deliverables are good AND the job is fully done. Synthesise the results into a coherent \
final_response — add managerial judgment, resolve contradictions, present a unified answer. \
Do not just concatenate.

Example: Lois delivered the competitor pricing table, Lex delivered the revenue impact analysis, \
and together they answer "should we raise prices?" Manager synthesizes both into a final \
recommendation. Job done.

### ACCEPT_AND_CONTINUE
All deliverables are good BUT the job is not done — more pieces need to be delegated before \
the original task is complete. Accept what was delivered, record what's done, and signal your \
intent to delegate the next piece on your next turn via next_step_brief.

Example: Lois delivered the competitor pricing table — accurate and complete. But the original \
task was "produce a full competitive positioning dossier" covering five sections. Pricing is one \
of five. Manager accepts Lois's piece, notes it in next_step_brief ("delegate feature comparison \
section to Lois next"), and will continue on next turn.

NOTE: The kernel currently finalizes the parent SWO as COMPLETED when it sees ACCEPT_AND_CONTINUE, \
pending the CHA-421 continuation loop. Your decision is still recorded and visible in logs and \
audit — use it honestly rather than forcing ACCEPT_AND_COMPLETE when the job isn't actually done.

### REJECT_AND_REVISE
One or more deliverables are not acceptable. Write specific revision briefs:

Bad: "This isn't good enough, try again."
Good: "Your competitor analysis is missing pricing data for Acme Corp and MegaAI. The comparison \
table needs a 'Pricing Model' column. Revise to include all 5 competitors with verified pricing."

Populate revision_swos as a dict mapping subordinate agent IDs to their specific revision payloads. \
Each brief must explain exactly what was wrong and what to produce instead.

---

## Hard rules
- Do not invent results that subordinates did not produce.
- Do not claim staffing or innovation actions unless present in the provided context.
- final_response is required for ACCEPT_AND_COMPLETE; omit it for ACCEPT_AND_CONTINUE.
- revision_swos is required for REJECT_AND_REVISE.

## BUILD, don't describe (CHA-428)

If the original task is to BUILD something — code, a web app, a script, a config file, a dataset, \
any artifact a user needs to RUN or OPEN — the synthesised deliverable is the ACTUAL THING, not a \
markdown report ABOUT the thing. The synthesis toolbox includes create_file, edit_file, run_command, \
git_*, and write_artifact_file. Use create_file / edit_file to write real source files to the \
workspace/ subtree and put only a short pointer in your final_response. Do NOT emit a \
`write_artifact_file("todo-app-runbook.md", ...)` call that contains a markdown spec of the code \
— the user cannot open a markdown file in a browser and use it. That is performance theatre.

   Good: create_file("workspace/todo/index.html", ...),
         create_file("workspace/todo/app.js", ...),
         final_response = "Built at workspace/todo/. Open index.html in a browser to use.
                           Add/edit/delete/toggle/filter implemented, localStorage persistence."

   Bad: write_artifact_file("todo-app-single-page-html-css-js-delivery.md",
                            "# Todo App\\n## Scope\\n## Exact code\\n```html\\n<html>...")

This applies EVEN WHEN your children's deliverables failed and you are self-salvaging. If Perry \
delegates "build a calculator" to five specialists and none of them produce working code, Perry's \
synthesis is NOT "write a 60-line runbook about what a calculator should look like". Perry's \
synthesis is "use create_file to write index.html / styles.css / script.js and emit a one-line \
final_response pointing at them". The self-salvage path still has access to the full builder \
toolbox — use it.

For prose deliverables (research reports, strategy docs, market analysis, synthesis summaries), \
return the text directly in final_response — no file writes required. File-writing is for tasks \
where the deliverable IS a file, not for every task. The test: "can a human USE this output without \
reading prose first?" If the answer is no for a build task, you built the wrong thing.""",
        _runtime_skill_section()
        + _capability_notes()
        + _manager_execution_contract_section(),
    )

_SAIRGENT_CHAT_PERSONA = """You are Sairgent — the user's AI work assistant.

# Identity
You are a single, unified assistant. The user talks to you — one voice, one name. Behind the scenes \
you can delegate to specialist agents, but the user never sees their names or the internal machinery. \
You own the conversation, you own the outcome.

# How to think
1. Understand before acting. Read the user's request carefully. If it references existing work, \
use get_swo_queue_status or read_agent_file to check current state before responding. \
Never guess at status — look it up.

2. Answer what you can immediately. If part of the request is conversational (a question, opinion, \
explanation), answer that part directly in your reply. Don't defer everything to background tasks.

3. Queue what requires execution. For work that needs async processing (research, content creation, \
multi-step analysis, code generation), use queue_managed_work with a clear, specific brief. \
Tell the user what you queued and what to expect.

4. Break complex work into discrete tasks. Each queued task should have one clear deliverable. \
"Research competitors and write a positioning doc" is two tasks, not one. Queue them separately \
with the second depending on the first if needed.

5. Use tools proactively.
   - web_search: Use when the user asks about current events, competitors, market data, or anything \
     where your training data may be stale. Don't hedge — search and cite.
   - list_available_skills / load_skill: Check what specialist knowledge is available before \
     answering domain questions. If a relevant skill exists, load it and apply its guidance.
   - read_agent_file: Check your context directory for relevant project files, prior deliverables, \
     or reference material before starting new work.
   - write_artifact_file: When you produce a substantial deliverable (report, plan, analysis), \
     write it as an artifact so the user can access it later.
   - get_swo_queue_status: Check before saying "I'll get that done" — it might already be in progress.

6. Be honest about what you can and can't do. If a tool is unavailable or a capability isn't \
configured, say so clearly and suggest what the user can do (e.g., "Web search isn't configured yet — \
add a Tavily or Exa key in Settings to enable it.").

# How to communicate
- Lead with the answer or action, not the reasoning.
- Use Markdown: headings for structure, bold for emphasis, lists for steps, code blocks for data.
- Be concise. One clear sentence beats three hedged ones.
- Never expose internal terminology: no "SWO", "HSM", "triage", "synthesis", "org chart", \
"subordinate", "delegation". Say "task", "working on it", "reviewing", "done".
- Never mention internal agent names to the user. You are Sairgent — one voice.
- When you queue work, describe it in the user's language: "I've started a market research task" \
not "I've dispatched a managed work order to the research agent."
- If something will take time, say so plainly: "This will run in the background. I'll have results \
shortly." Then offer to help with something else in the meantime.
- Don't add caveats, disclaimers, or meta-commentary about your own limitations unless directly relevant.

# Delegation strategy
When you have specialist agents available, route work to the right specialist based on their role. \
Write clear, specific briefs — not vague summaries of the user's request. A good delegation brief:
- States the specific deliverable expected
- Provides relevant context the specialist needs
- Sets constraints (format, length, focus areas)
- Specifies what "done" looks like

A bad delegation brief just parrots the user's message. Add your own judgment about what the \
specialist needs to know.
"""


def build_chat_prompt(persona_prompt: str, raison: str, subordinates_json: str) -> str:
    try:
        subs = json.loads(subordinates_json) if subordinates_json else []
    except:
        subs = []

    # Use the unified Sairgent persona for the primary chat agent.
    # Fall back to the injected persona for non-primary agents.
    effective_persona = _SAIRGENT_CHAT_PERSONA if not persona_prompt or persona_prompt.startswith("Role:") else persona_prompt

    if subs:
        team_list = "\n".join([f"- {s.get('name')} ({s.get('role')}): {s.get('raison', '')}" for s in subs])
        team_context = f"""You have specialist agents available for delegation:
{team_list}

Match tasks to the specialist whose role fits best. Use queue_managed_work to delegate.
If no specialist fits, handle it yourself. If the request is conversational, answer directly — \
don't delegate questions, only executable work."""
    else:
        team_context = "Handle all requests directly. No specialist agents are currently available."

    return _render_prompt_sections(
        effective_persona,
        raison,
        team_context,
        """Reply directly for conversational answers using well-structured Markdown.
Queue work via queue_managed_work only when the task requires async execution.
Always answer the conversational part of a request immediately, even if you also queue background work.""",
        """Do not roleplay queueing or completion — if you queued it, say so; if you didn't, don't claim you did.
Do not fabricate output paths or claim artifacts that were not written.
Do not mention internal agent names, SWO terminology, or organisational hierarchy to the user.
Check existing queue status before creating duplicate tasks.
When unsure whether to answer directly or queue, prefer answering directly for speed.""",
        _runtime_skill_section() + _capability_notes(),
    )


def _build_sairgent_chat_prompt(kernel_persona: str, raison: str, subordinates_json: str) -> str:
    """Build the sairgent_chat system prompt.

    The kernel injects runtime context (stats, roster, company info) + the agent's
    raw persona into ``kernel_persona``.  We strip out the stats dump and only keep
    actionable context (current project/task).  The model should use tools to look
    up status, not parrot numbers from its prompt.
    """
    try:
        subs = json.loads(subordinates_json) if subordinates_json else []
    except Exception:
        subs = []

    # Extract only actionable context from the kernel's runtime dump.
    # Skip stats lines, company_context XML, and agent roster — the model doesn't
    # need project counts or agent counts to be a good assistant.
    actionable_lines = []
    for line in kernel_persona.strip().splitlines():
        stripped = line.strip()
        # Skip stats, XML tags, roster headers, and empty lines
        if not stripped:
            continue
        if any(skip in stripped.lower() for skip in [
            "runtime context", "active projects", "open swos", "approvals waiting",
            "total agents", "ready/idle", "degraded", "<company_context>",
            "</company_context>", "operating principles", "non-goals",
            "## agent roster", "| name", "| ---", "| perry",
        ]):
            continue
        # Keep current project/task context and highlights
        if any(keep in stripped.lower() for keep in [
            "current project", "current swo", "recent", "highlight",
        ]):
            actionable_lines.append(stripped)

    if subs:
        team_list = "\n".join([f"- {s.get('name')} ({s.get('role')})" for s in subs])
        team_section = f"""You have specialist agents available for delegation:
{team_list}

Match tasks to the specialist whose role fits best. Use queue_managed_work to delegate.
If no specialist fits, handle it yourself."""
    else:
        team_section = "Handle all requests directly."

    context_section = ""
    if actionable_lines:
        context_section = "\n# Current context\n" + "\n".join(actionable_lines)

    return f"""{_SAIRGENT_CHAT_PERSONA}

{team_section}

# Tools
Use your tools proactively. When the user asks about tasks, status, or history, \
call get_swo_queue_status to look it up — don't guess or say you don't have the information.

{_runtime_skill_section()}{_capability_notes()}
{context_section}

[Mission]
{raison.strip()}

[Mode Rules]
Answer the user's question directly and conversationally.
Use tools to look up information you don't have — never say "insufficient context" \
if you have a tool that could answer the question.
Do not output structured data dumps, JSON, key-value lists, or status reports.
Do not mention internal agent names, SWO terminology, or organisational hierarchy.
Do not fabricate output paths or claim artifacts that were not written.
When unsure whether to answer directly or queue work, prefer answering directly.
"""


def build_swo_formatter_prompt(persona_prompt: str, raison: str) -> str:
    return _render_prompt_sections(
        persona_prompt,
        raison,
        "Convert a delegation request plus recent context into one clean SWO brief.",
        "Return plain text only.",
        """Preserve the user's objective, constraints, and requested deliverable.
Do not mention UUIDs, raw JSON, or meta commentary.""",
        _runtime_skill_section(),
    )

def get_env_or_die(key: str) -> str:
    val = os.getenv(key)
    if not val:
        print(json.dumps({"error": f"Missing required env var: {key}"}))
        sys.exit(1)
    return val


def _load_worker_secrets_from_stdin() -> tuple[str, dict, dict]:
    raw = sys.stdin.read().strip()
    if not raw:
        print(json.dumps({"error": "Missing worker secrets on stdin"}))
        sys.exit(1)

    try:
        payload = json.loads(raw)
    except Exception:
        return raw, {}, {}

    if isinstance(payload, dict):
        llm_api_key = str(payload.get("llm_api_key", "") or "").strip()
        tool_api_keys = payload.get("tool_api_keys_by_slug", {})
        if not llm_api_key:
            print(json.dumps({"error": "Missing LLM_API_KEY in worker secret bundle"}))
            sys.exit(1)
        if not isinstance(tool_api_keys, dict):
            tool_api_keys = {}
        sanitized = {
            str(slug).strip(): str(secret).strip()
            for slug, secret in tool_api_keys.items()
            if str(slug).strip() and str(secret).strip()
        }
        mcp_credentials = payload.get("mcp_credentials_by_slug", {})
        if not isinstance(mcp_credentials, dict):
            mcp_credentials = {}
        sanitized_mcp = {
            str(slug).strip(): str(secret).strip()
            for slug, secret in mcp_credentials.items()
            if str(slug).strip() and str(secret).strip()
        }
        return llm_api_key, sanitized, sanitized_mcp

    return str(payload).strip(), {}, {}


def _build_mcp_servers(mcp_credentials: dict) -> list:
    """Build PydanticAI MCP server instances from kernel-provided configs."""
    mcp_connectors_json = os.getenv("AGENT_MCP_CONNECTORS_JSON", "[]")
    try:
        mcp_connectors = json.loads(mcp_connectors_json)
    except (json.JSONDecodeError, TypeError):
        mcp_connectors = []

    if not mcp_connectors:
        return []

    try:
        from pydantic_ai.mcp import MCPServerStdio, MCPServerHTTP
    except ImportError:
        print(json.dumps({"warning": "pydantic_ai.mcp not available, skipping MCP servers"}), file=sys.stderr)
        return []

    servers = []
    for conn in mcp_connectors:
        slug = conn.get("slug", "")
        transport = conn.get("transport", "")

        if transport == "stdio":
            command = conn.get("command")
            if not command:
                continue
            # Build sanitized env: start EMPTY, only add declared vars
            child_env = {}
            for k, v in (conn.get("env") or {}).items():
                child_env[k] = str(v)
            # Inject credential if present
            cred = mcp_credentials.get(slug)
            if cred:
                child_env["MCP_API_KEY"] = cred
            # Add minimal PATH for command resolution
            child_env["PATH"] = "/usr/local/bin:/usr/bin:/bin"

            servers.append(MCPServerStdio(
                command,
                args=conn.get("args") or [],
                env=child_env,
                cwd=conn.get("cwd"),
            ))
        elif transport == "sse":
            url = conn.get("url")
            if not url:
                continue
            headers = dict(conn.get("headers") or {})
            cred = mcp_credentials.get(slug)
            if cred:
                headers["Authorization"] = f"Bearer {cred}"
            servers.append(MCPServerHTTP(
                url,
                headers=headers if headers else None,
            ))

    return servers


# CHA-425 sub-fix 2 — PydanticAI request_limit override.
# PydanticAI's default request_limit is 50. When a manager gets into a
# reasoning spiral (see CHA-425 calculator-app test), 50 model calls are
# burned quickly and the exception propagates as a raw Internal error.
# Override to a sensible ceiling that accommodates legitimate multi-step
# Ralph-loop sessions, env-configurable for ops tuning. UsageLimitExceeded
# is caught at the top-level dispatch and emitted as a clean FAILED
# synthesis with exception_code=request_limit_exceeded.
_DEFAULT_AGENT_REQUEST_LIMIT = 200


def _agent_usage_limits() -> UsageLimits:
    """Construct the UsageLimits policy for every agent.run() call."""
    raw = os.getenv("SAIRGENT_AGENT_REQUEST_LIMIT", "").strip()
    try:
        limit = int(raw) if raw else _DEFAULT_AGENT_REQUEST_LIMIT
    except ValueError:
        limit = _DEFAULT_AGENT_REQUEST_LIMIT
    if limit <= 0:
        limit = _DEFAULT_AGENT_REQUEST_LIMIT
    return UsageLimits(request_limit=limit)


def _run_agent_with_mcp(agent, prompt, mcp_servers, message_history=None):
    """Run a PydanticAI agent, using async context managers if MCP servers are present."""
    usage_limits = _agent_usage_limits()
    if mcp_servers:
        import asyncio
        async def _run_with_mcp():
            async with agent:
                if message_history is not None:
                    return await agent.run(prompt, message_history=message_history, usage_limits=usage_limits)
                else:
                    return await agent.run(prompt, usage_limits=usage_limits)
        return asyncio.run(_run_with_mcp())
    else:
        if message_history is not None:
            return agent.run_sync(prompt, message_history=message_history, usage_limits=usage_limits)
        else:
            return agent.run_sync(prompt, usage_limits=usage_limits)


def _run_agent_with_streaming(agent, prompt, mcp_servers, message_history=None, message_id=None, agent_id=None):
    """Run a PydanticAI agent with streaming, emitting deltas to stderr.

    Falls back to non-streaming if run_stream() is unavailable.
    """
    import asyncio

    if message_id is None:
        message_id = str(uuid.uuid4())

    if not hasattr(agent, 'run_stream'):
        # Fallback to non-streaming
        return _run_agent_with_mcp(agent, prompt, mcp_servers, message_history=message_history)

    async def _stream():
        ctx = agent if mcp_servers else _nullcontext(agent)
        async with ctx:
            kwargs = {"usage_limits": _agent_usage_limits()}
            if message_history is not None:
                kwargs["message_history"] = message_history
            async with agent.run_stream(prompt, **kwargs) as stream_result:
                full_text = ""
                async for chunk in stream_result.stream_text(delta=True):
                    if chunk:
                        full_text += chunk
                        _emit_sidechannel({
                            "__sairgent_delta": True,
                            "delta": chunk,
                            "message_id": message_id,
                            "agent_id": agent_id,
                            "is_final": False,
                        }, lock_timeout=None)
                # Capture usage after the stream context closes
                stream_usage = None
                try:
                    stream_usage = stream_result.usage()
                except Exception:
                    pass  # TODO: capture streaming usage if PydanticAI exposes it post-close

        class _StreamedResult:
            def __init__(self, output, usage_data=None):
                self.output = output
                self._usage_data = usage_data

            def usage(self):
                if self._usage_data is not None:
                    return self._usage_data
                raise AttributeError("usage not available")

        return _StreamedResult(full_text, stream_usage)

    try:
        return asyncio.run(_stream())
    except Exception:
        # Fallback to non-streaming on any streaming error
        return _run_agent_with_mcp(agent, prompt, mcp_servers, message_history=message_history)


class _nullcontext:
    """Minimal async context manager that yields the wrapped value."""
    def __init__(self, val):
        self._val = val
    async def __aenter__(self):
        return self._val
    async def __aexit__(self, *args):
        pass


def _allowed_tools() -> set[str]:
    raw = os.getenv("AGENT_ALLOWED_TOOLS", "[]")
    try:
        parsed = json.loads(raw)
    except Exception:
        parsed = []
    return {item for item in parsed if isinstance(item, str)}


def _collect_side_effects() -> dict:
    # Merge counts from old globals (sairgent_* governed tools still use them)
    # and from the shared state dict (registry tools update it).
    return build_side_effects(
        managed_work_count=_managed_work_count + _shared_state.get("managed_work_count", 0),
        artifact_count=_artifact_count + _shared_state.get("artifact_count", 0),
        innovation_count=_innovation_count + _shared_state.get("innovation_count", 0),
        hire_request_count=_hire_request_count + _shared_state.get("hire_request_count", 0),
        dispatch_count=_dispatch_count + _shared_state.get("dispatch_count", 0),
        sairgent_proposal_count=_sairgent_proposal_count,
    )


# Shared state dict for registry tool modules; initialized in run_worker().
_shared_state: dict = {}

# ─────────────────────────────────────────────────────────────────────────────
# Dark Factory Phase 7 — shared helpers (CHA-392 capability gate, path guard)
# ─────────────────────────────────────────────────────────────────────────────

def _agent_capabilities() -> set:
    """Return the set of capability slugs from AGENT_MANIFEST_JSON (snake_case strings)."""
    raw = os.getenv("AGENT_MANIFEST_JSON", "")
    if not raw:
        return set()
    try:
        manifest = json.loads(raw)
        caps = manifest.get("capabilities") or []
        return set(caps)
    except Exception:
        return set()


def _require_capability(name: str) -> None:
    """Raise BlockedToolError if the agent does not hold the named capability grant.

    name should match the snake_case serialization used by the kernel, e.g.
    'shell_exec', 'file_read', 'file_write', 'git_ops'.
    """
    if name not in _agent_capabilities():
        raise BlockedToolError(
            f"Capability `{name}` is not granted to this agent. "
            f"Ask an operator to enable it in the agent manifest."
        )


def _resolve_safe_path(path: str) -> pathlib.Path:
    """Resolve *path* relative to AGENT_ROOT and enforce it stays inside AGENT_ROOT.

    Raises BlockedToolError with an actionable message if the path escapes the root
    or if AGENT_ROOT is not configured.
    """
    workspace_root = os.getenv("AGENT_ROOT", "").strip()
    if not workspace_root:
        raise BlockedToolError("AGENT_ROOT is not configured for this agent.")
    requested = (path or "").strip()
    if not requested:
        raise BlockedToolError("path is required and cannot be empty.")
    root = pathlib.Path(os.path.realpath(workspace_root))
    candidate = root / requested
    try:
        resolved = pathlib.Path(os.path.realpath(candidate))
        resolved.relative_to(root)
    except (ValueError, OSError):
        raise BlockedToolError(
            f"Path '{path}' resolves outside the agent root. "
            f"Use a path relative to the agent workspace."
        )
    return resolved


def _workspace_dir() -> pathlib.Path:
    """Return (and create if missing) the agent's workspace/ subdirectory."""
    workspace_root = os.getenv("AGENT_ROOT", "").strip()
    if not workspace_root:
        raise BlockedToolError("AGENT_ROOT is not configured for this agent.")
    ws = pathlib.Path(workspace_root) / "workspace"
    ws.mkdir(parents=True, exist_ok=True)
    return ws


# ─────────────────────────────────────────────────────────────────────────────
# Audit helpers (CHA-396)
# ─────────────────────────────────────────────────────────────────────────────

def _sha256_hex(data: bytes) -> str:
    """Return a 'sha256:<hex>' digest of *data*."""
    return "sha256:" + hashlib.sha256(data).hexdigest()


# Secret redaction patterns — applied to command strings and URLs before they
# land in the tamper-evident audit chain. These cannot be removed after the
# fact without breaking the chain, so redact aggressively at emission time.
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


def _redact_secrets(text: str) -> str:
    """Scrub common secret shapes from a string before auditing.

    Applied to command strings and URLs that get recorded into the
    tamper-evident audit chain. Matches bearer tokens, basic auth,
    URL-embedded credentials, GitHub tokens, API keys, and the
    api_key=/token=/password= shell idioms.
    """
    if not isinstance(text, str):
        return text
    redacted = text
    for pattern, replacement in _SECRET_PATTERNS:
        redacted = pattern.sub(replacement, redacted)
    return redacted


# CHA-402 — audit sidechannel rate limit + size cap (H4: declared above
# _emit_audit_sidechannel so a refactor that calls it at module-import time
# cannot NameError).
#
# Budget rationale: 2000 events/run covers a Barry-Allen-style multi-iteration
# loop ceiling (~20 reads per iteration × 100 iterations) while still capping
# a hostile `for i in 1..100000: read_file(f)` prompt injection. Env var
# overrides are a follow-up (see CHA-415 H-class findings).
#
# Truncation of CONTENT is NOT an option: content hashes in the audit chain
# must reflect the exact bytes recorded. Oversized events are dropped with
# a warning, not trimmed. The exception is `files_changed` manifests in
# git_operation events — those are file-list sentinels, not hashed content,
# so git_commit truncates them at emit time to stay under the size cap.
_MAX_AUDIT_EVENTS_PER_RUN = 2000
_MAX_AUDIT_EVENT_BYTES = 64 * 1024  # 64 KB per serialized JSON line
_audit_event_count = 0
_audit_budget_exceeded_warned = False
# CHA-402 B1 — counter lock. PydanticAI runs sync tool functions on anyio
# worker threads when an agent is inside an MCP async context, so
# `_emit_audit_sidechannel` can be called concurrently from multiple threads.
# Without this lock, two simultaneous emissions can both observe the counter
# at N-1 and both pass the budget check, corrupting the event ceiling.
# Do NOT reuse sidechannel_lock — that one is held across stderr writes and
# would serialize all tool emissions behind a single blocking lock.
_audit_counter_lock = threading.Lock()


def _emit_audit_sidechannel(event_type: str, payload: dict) -> None:
    """Emit a sidechannel audit event, injecting token automatically.

    CHA-402: Subject to per-run budget (_MAX_AUDIT_EVENTS_PER_RUN) and per-event
    size cap (_MAX_AUDIT_EVENT_BYTES).  The counter is incremented on attempt, not
    on success — counting on success would allow a rate-limit bypass via deliberate
    lock contention.  Dropped events emit a one-time stderr warning.

    Thread-safe: counter check, increment, and warning-flag live under
    _audit_counter_lock.  Called from HeartbeatEmitter and PydanticAI worker
    threads concurrently.
    """
    global _audit_event_count, _audit_budget_exceeded_warned

    # 1. Budget check + counter reservation under the lock.
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
            _emit_stderr_line("[WARN] audit sidechannel budget exceeded, dropping further events")
        return

    # 2. Mutate payload in place (preserved behaviour from original function).
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    payload["__sairgent_sidechannel"] = event_type
    payload["token"] = token

    # 3. Size cap — check AFTER mutation so the token/event_type fields are
    #    included in the measured size, matching what actually hits the wire.
    try:
        serialized = json.dumps(payload)
    except (TypeError, ValueError) as exc:
        _emit_stderr_line(
            f"[WARN] audit sidechannel event not serializable ({exc}), dropping"
        )
        return
    byte_len = len(serialized.encode("utf-8"))
    if byte_len > _MAX_AUDIT_EVENT_BYTES:
        _emit_stderr_line(
            f"[WARN] audit sidechannel event oversized ({byte_len} bytes), dropping"
        )
        return

    _emit_sidechannel(payload)


# ─────────────────────────────────────────────────────────────────────────────
# CHA-393 — Sandboxed run_command
# ─────────────────────────────────────────────────────────────────────────────

_MAX_COMMAND_OUTPUT_BYTES = 100 * 1024  # 100 KB per stream
_MAX_COMMAND_TIMEOUT_SEC = 600

# CHA-401 — env whitelist for child processes.
# Only these vars are forwarded; secrets (SAIRGENT_SIDECHANNEL_TOKEN,
# AGENT_MANIFEST_JSON, REGISTRY_DATABASE, *_API_KEY, ANTHROPIC_*, etc.)
# are stripped automatically by omission. DO NOT add SSH_AUTH_SOCK,
# GITHUB_TOKEN, GIT_*, GH_*, or PYTHONPATH here — git uses HTTPS + explicit
# --author/--message flags, and PYTHONPATH is a sitecustomize hijack vector.
_ALLOWED_CHILD_ENV: frozenset[str] = frozenset({
    "PATH", "HOME", "USER", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TZ",
    "TMPDIR", "AGENT_WORKSPACE",
})


def run_command(command: str, working_dir: str | None = None, timeout_seconds: int = 120) -> dict:
    """Run a shell command inside the agent workspace with a configurable timeout.

    *command* is parsed via shlex.split() — it is NOT run through a shell.
    *working_dir* must be relative to the agent root; defaults to workspace/.
    *timeout_seconds* is clamped to [1, 600].

    Returns a dict with keys:
      exit_code (int), stdout (str), stderr (str), duration_ms (int), truncated (bool).

    Requires the shell_exec capability grant.
    """
    _require_capability("shell_exec")

    timeout_seconds = max(1, min(int(timeout_seconds), _MAX_COMMAND_TIMEOUT_SEC))

    if working_dir:
        cwd_path = _resolve_safe_path(working_dir)
    else:
        cwd_path = _workspace_dir()

    if not cwd_path.exists():
        cwd_path.mkdir(parents=True, exist_ok=True)

    try:
        args = shlex.split(command)
    except ValueError as e:
        return {
            "exit_code": -1,
            "stdout": "",
            "stderr": f"Failed to parse command: {e}",
            "duration_ms": 0,
            "truncated": False,
        }

    if not args:
        return {
            "exit_code": -1,
            "stdout": "",
            "stderr": "command is empty after parsing.",
            "duration_ms": 0,
            "truncated": False,
        }

    child_env = {k: os.environ[k] for k in _ALLOWED_CHILD_ENV if k in os.environ}

    start = time.monotonic()
    try:
        proc = subprocess.run(
            args,
            cwd=str(cwd_path),
            capture_output=True,
            timeout=timeout_seconds,
            env=child_env,
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
        result = {
            "exit_code": proc.returncode,
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms,
            "truncated": truncated,
        }
        _emit_audit_sidechannel("shell_exec", {
            "command": _redact_secrets(command),
            "cwd": str(cwd_path),
            "exit_code": proc.returncode,
            "duration_ms": duration_ms,
            "stdout_hash": _sha256_hex(stdout_bytes[:_MAX_COMMAND_OUTPUT_BYTES]),
            "stderr_hash": _sha256_hex(stderr_bytes[:_MAX_COMMAND_OUTPUT_BYTES]),
            "truncated": truncated,
        })
        return result
    except subprocess.TimeoutExpired:
        duration_ms = int((time.monotonic() - start) * 1000)
        _emit_audit_sidechannel("shell_exec", {
            "command": _redact_secrets(command),
            "cwd": str(cwd_path),
            "exit_code": -1,
            "duration_ms": duration_ms,
            "stdout_hash": _sha256_hex(b""),
            "stderr_hash": _sha256_hex(f"Command timed out after {timeout_seconds}s.".encode()),
            "truncated": False,
        })
        return {
            "exit_code": -1,
            "stdout": "",
            "stderr": f"Command timed out after {timeout_seconds}s.",
            "duration_ms": duration_ms,
            "truncated": False,
        }
    except FileNotFoundError:
        _emit_audit_sidechannel("shell_exec", {
            "command": _redact_secrets(command),
            "cwd": str(cwd_path),
            "exit_code": -1,
            "duration_ms": 0,
            "stdout_hash": _sha256_hex(b""),
            "stderr_hash": _sha256_hex(f"Command not found: {args[0]!r}".encode()),
            "truncated": False,
        })
        return {
            "exit_code": -1,
            "stdout": "",
            "stderr": f"Command not found: {args[0]!r}",
            "duration_ms": 0,
            "truncated": False,
        }
    except Exception as e:
        _emit_audit_sidechannel("shell_exec", {
            "command": _redact_secrets(command),
            "cwd": str(cwd_path),
            "exit_code": -1,
            "duration_ms": 0,
            "stdout_hash": _sha256_hex(b""),
            "stderr_hash": _sha256_hex(f"Error running command: {e}".encode()),
            "truncated": False,
        })
        return {
            "exit_code": -1,
            "stdout": "",
            "stderr": f"Error running command: {e}",
            "duration_ms": 0,
            "truncated": False,
        }


# ─────────────────────────────────────────────────────────────────────────────
# CHA-394 — File operations (FileRead + FileWrite tiers)
# ─────────────────────────────────────────────────────────────────────────────

_MAX_READ_BYTES_DARK = 10 * 1024 * 1024  # 10 MB cap (matches existing read_agent_file)


def read_file(path: str) -> str:
    """Read any file within the agent workspace using a workspace-relative path.

    Examples: 'workspace/main.py', 'context/notes.md'.
    Maximum file size: 10 MB.

    Requires the file_read capability grant.
    """
    _require_capability("file_read")
    safe_path = _resolve_safe_path(path)
    if not safe_path.exists():
        return f"Error: file does not exist: '{path}'"
    if safe_path.is_dir():
        return f"Error: '{path}' is a directory, not a file. Use list_directory instead."
    try:
        with open(safe_path, "r", encoding="utf-8") as f:
            content = f.read(_MAX_READ_BYTES_DARK + 1)
        content_bytes = content.encode("utf-8")
        if len(content_bytes) > _MAX_READ_BYTES_DARK:
            _emit_audit_sidechannel("file_read", {
                "operation": "read",
                "path": str(safe_path),
                "size": len(content_bytes),
                "content_hash": _sha256_hex(content_bytes),
                "truncated": True,
            })
            return f"Error: file '{path}' exceeds the maximum readable size of 10 MB."
        _emit_audit_sidechannel("file_read", {
            "operation": "read",
            "path": str(safe_path),
            "size": len(content_bytes),
            "content_hash": _sha256_hex(content_bytes),
            "truncated": False,
        })
        return content if content else "(file is empty)"
    except Exception as e:
        return f"Error reading file '{path}': {e}"


def list_directory(path: str) -> list:
    """List the contents of a directory within the agent workspace.

    Returns a list of dicts with keys: name (str), kind ('file'|'dir'), size (int), mtime (float).
    *path* is workspace-relative.

    Requires the file_read capability grant.
    """
    _require_capability("file_read")
    safe_path = _resolve_safe_path(path)
    if not safe_path.exists():
        raise BlockedToolError(f"directory does not exist: '{path}'")
    if not safe_path.is_dir():
        raise BlockedToolError(f"'{path}' is not a directory.")
    entries = []
    try:
        for entry in sorted(safe_path.iterdir(), key=lambda e: (e.is_file(), e.name)):
            stat = entry.stat()
            entries.append({
                "name": entry.name,
                "kind": "file" if entry.is_file() else "dir",
                "size": stat.st_size if entry.is_file() else 0,
                "mtime": stat.st_mtime,
            })
    except Exception as e:
        raise BlockedToolError(f"Error listing directory '{path}': {e}")
    _emit_audit_sidechannel("file_read", {
        "operation": "list",
        "path": str(safe_path),
        "entry_count": len(entries),
    })
    return entries


def create_file(path: str, content: str) -> dict:
    """Create a new file at the given workspace-relative path.

    Fails if the file already exists — use edit_file to overwrite.
    Parent directories are created automatically.

    Requires the file_write capability grant.
    """
    _require_capability("file_write")
    safe_path = _resolve_safe_path(path)
    if safe_path.exists():
        return {"ok": False, "error": f"file already exists: '{path}'. Use edit_file to overwrite."}
    # CHA-425 sub-fix 3 — refuse writes that would breach the dedup cap.
    dedup_err = _check_artifact_dedup(path)
    if dedup_err is not None:
        return {"ok": False, "error": dedup_err}
    try:
        safe_path.parent.mkdir(parents=True, exist_ok=True)
        content_bytes = content.encode("utf-8")
        safe_path.write_bytes(content_bytes)
        _emit_audit_sidechannel("file_mutation", {
            "operation": "create",
            "path": str(safe_path),
            "size": len(content_bytes),
            "content_hash": _sha256_hex(content_bytes),
        })
        return {"ok": True, "path": str(safe_path), "bytes_written": len(content_bytes)}
    except Exception as e:
        return {"ok": False, "error": f"Error creating file '{path}': {e}"}


def edit_file(path: str, content: str) -> dict:
    """Overwrite an existing file at the given workspace-relative path.

    Fails if the file does not exist — use create_file for new files.

    Requires the file_write capability grant.
    """
    _require_capability("file_write")
    safe_path = _resolve_safe_path(path)
    if not safe_path.exists():
        return {"ok": False, "error": f"file does not exist: '{path}'. Use create_file to create it."}
    if safe_path.is_dir():
        return {"ok": False, "error": f"'{path}' is a directory, not a file."}
    try:
        content_bytes = content.encode("utf-8")
        safe_path.write_bytes(content_bytes)
        _emit_audit_sidechannel("file_mutation", {
            "operation": "edit",
            "path": str(safe_path),
            "size": len(content_bytes),
            "content_hash": _sha256_hex(content_bytes),
        })
        return {"ok": True, "path": str(safe_path), "bytes_written": len(content_bytes)}
    except Exception as e:
        return {"ok": False, "error": f"Error editing file '{path}': {e}"}


def delete_file(path: str) -> dict:
    """Delete a file within the agent workspace. Does NOT delete directories.

    Requires the file_write capability grant.
    """
    _require_capability("file_write")
    safe_path = _resolve_safe_path(path)
    if not safe_path.exists():
        return {"ok": False, "error": f"file does not exist: '{path}'"}
    if safe_path.is_dir():
        return {"ok": False, "error": f"'{path}' is a directory. delete_file only removes files."}
    try:
        safe_path.unlink()
        _emit_audit_sidechannel("file_mutation", {
            "operation": "delete",
            "path": str(safe_path),
            "size": None,
            "content_hash": None,
        })
        return {"ok": True, "path": str(safe_path)}
    except Exception as e:
        return {"ok": False, "error": f"Error deleting file '{path}': {e}"}


def mkdir(path: str) -> dict:
    """Create a directory (and any missing parents) within the agent workspace.

    Safe to call if the directory already exists.

    Requires the file_write capability grant.
    """
    _require_capability("file_write")
    safe_path = _resolve_safe_path(path)
    try:
        safe_path.mkdir(parents=True, exist_ok=True)
        _emit_audit_sidechannel("file_mutation", {
            "operation": "mkdir",
            "path": str(safe_path),
            "size": None,
            "content_hash": None,
        })
        return {"ok": True, "path": str(safe_path)}
    except Exception as e:
        return {"ok": False, "error": f"Error creating directory '{path}': {e}"}


# ─────────────────────────────────────────────────────────────────────────────
# CHA-395 — Git tooling (GitOps capability)
# ─────────────────────────────────────────────────────────────────────────────

def _git(args: list, cwd: pathlib.Path) -> dict:
    """Run a git sub-command and return {ok, stdout, stderr, exit_code}."""
    git_env = {k: os.environ[k] for k in _ALLOWED_CHILD_ENV if k in os.environ}
    try:
        proc = subprocess.run(
            ["git"] + args,
            cwd=str(cwd),
            capture_output=True,
            timeout=120,
            env=git_env,
        )
        stdout = (proc.stdout or b"").decode("utf-8", errors="replace").strip()
        stderr = (proc.stderr or b"").decode("utf-8", errors="replace").strip()
        return {
            "ok": proc.returncode == 0,
            "exit_code": proc.returncode,
            "stdout": stdout,
            "stderr": stderr,
        }
    except subprocess.TimeoutExpired:
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "git command timed out after 120s."}
    except FileNotFoundError:
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "git is not available on PATH."}
    except Exception as e:
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": f"Error running git: {e}"}


def git_clone(repo_url: str, branch: str | None = None) -> dict:
    """Clone a git repository into the agent workspace/ directory.

    The clone target is determined by the repo name extracted from *repo_url*.
    The working directory is always under AGENT_ROOT/workspace/.

    Requires the git_ops capability grant.
    """
    _require_capability("git_ops")
    ws = _workspace_dir()
    args = ["clone"]
    if branch:
        args += ["--branch", branch]
    args.append(repo_url)
    result = _git(args, cwd=ws)
    _emit_audit_sidechannel("git_operation", {
        "operation": "clone",
        "repo": _redact_secrets(repo_url),
        "branch": branch,
        "commit_hash": None,
        "files_changed": None,
    })
    return result


def git_status(working_dir: str | None = None) -> dict:
    """Return git status --porcelain output for the working directory.

    *working_dir* is workspace-relative; defaults to workspace/.

    Requires the git_ops capability grant.
    """
    _require_capability("git_ops")
    cwd = _resolve_safe_path(working_dir) if working_dir else _workspace_dir()
    result = _git(["status", "--porcelain"], cwd=cwd)
    _emit_audit_sidechannel("git_operation", {
        "operation": "status",
        "repo": str(cwd),
        "branch": None,
        "commit_hash": None,
        "files_changed": None,
    })
    return result


def git_diff(staged: bool = False, working_dir: str | None = None) -> str:
    """Return git diff output.

    Set *staged* to True for 'git diff --staged'.
    *working_dir* is workspace-relative; defaults to workspace/.

    Requires the git_ops capability grant.
    """
    _require_capability("git_ops")
    cwd = _resolve_safe_path(working_dir) if working_dir else _workspace_dir()
    args = ["diff"]
    if staged:
        args.append("--staged")
    result = _git(args, cwd=cwd)
    _emit_audit_sidechannel("git_operation", {
        "operation": "diff",
        "repo": str(cwd),
        "staged": staged,
        "branch": None,
        "commit_hash": None,
        "files_changed": None,
    })
    return result.get("stdout", "") if result["ok"] else f"Error: {result.get('stderr', '')}"


def git_commit(message: str, files: list, working_dir: str | None = None) -> dict:
    """Stage specific files and commit them with the given message.

    *files* is a list of workspace-relative file paths to stage.
    *working_dir* is workspace-relative; defaults to workspace/.

    Requires the git_ops capability grant.
    """
    _require_capability("git_ops")
    if not message or not message.strip():
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "commit message cannot be empty."}
    if not files:
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "files list cannot be empty."}
    cwd = _resolve_safe_path(working_dir) if working_dir else _workspace_dir()
    add_result = _git(["add", "--"] + [str(f) for f in files], cwd=cwd)
    if not add_result["ok"]:
        return add_result
    result = _git(["commit", "-m", message], cwd=cwd)
    # Extract commit hash from stdout if available (e.g. "[main abc1234] message")
    commit_hash = None
    if result["ok"] and result.get("stdout"):
        import re
        m = re.search(r"\b([0-9a-f]{7,40})\b", result["stdout"])
        if m:
            commit_hash = m.group(1)
    # CHA-402 B2 — cap files_changed at a forensic-triage threshold. A 5000-file
    # commit would otherwise blow the 64KB per-event size cap and the ENTIRE
    # event would be dropped — the most interesting commit (mass rewrite) is
    # the one most likely to get silently lost. files_changed is a file-list
    # manifest, not hashed content, so truncating it is safe: commit_hash
    # carries full forensic reconstructability via git lookup.
    _FILES_CHANGED_CAP = 100
    all_files = [str(f) for f in files]
    files_changed_truncated = len(all_files) > _FILES_CHANGED_CAP
    _emit_audit_sidechannel("git_operation", {
        "operation": "commit",
        "repo": str(cwd),
        "branch": None,
        "commit_hash": commit_hash,
        "files_changed": all_files[:_FILES_CHANGED_CAP],
        "files_changed_total": len(all_files),
        "files_changed_truncated": files_changed_truncated,
    })
    return result


def git_create_branch(name: str, working_dir: str | None = None) -> dict:
    """Create and switch to a new git branch.

    *working_dir* is workspace-relative; defaults to workspace/.

    Requires the git_ops capability grant.
    """
    _require_capability("git_ops")
    if not name or not name.strip():
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "branch name cannot be empty."}
    cwd = _resolve_safe_path(working_dir) if working_dir else _workspace_dir()
    result = _git(["checkout", "-b", name.strip()], cwd=cwd)
    _emit_audit_sidechannel("git_operation", {
        "operation": "create_branch",
        "repo": str(cwd),
        "branch": name.strip(),
        "commit_hash": None,
        "files_changed": None,
    })
    return result


def git_push(remote: str = "origin", branch: str = "", working_dir: str | None = None) -> dict:
    """Push to an explicit remote and branch.

    Both *remote* and *branch* must be provided; no implicit upstream tracking.
    *working_dir* is workspace-relative; defaults to workspace/.

    Requires the git_ops capability grant.
    """
    _require_capability("git_ops")
    if not remote or not remote.strip():
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "remote cannot be empty."}
    if not branch or not branch.strip():
        return {"ok": False, "exit_code": -1, "stdout": "", "stderr": "branch cannot be empty. Provide an explicit branch name."}
    cwd = _resolve_safe_path(working_dir) if working_dir else _workspace_dir()
    result = _git(["push", remote.strip(), branch.strip()], cwd=cwd)
    _emit_audit_sidechannel("git_operation", {
        "operation": "push",
        "repo": str(cwd),
        "branch": branch.strip(),
        "commit_hash": None,
        "files_changed": None,
    })
    return result


# ─────────────────────────────────────────────────────────────────────────────
# Dark Factory tool lists — REPLACED by tools/ registry (resolve_tools).
# The _dark_factory_tools() function is no longer needed; capability-gated
# tools are auto-discovered from tools/file_ops.py, tools/shell.py, and
# tools/git_ops.py via their ToolSpec.capability declarations.
# ─────────────────────────────────────────────────────────────────────────────


def run_worker():
    global _managed_work_count, _innovation_count, _hire_request_count, _dispatch_count, _sairgent_proposal_count, _artifact_count, _outbox_artifacts, _tool_api_keys_by_slug, _mcp_credentials_by_slug
    _managed_work_count = 0
    _innovation_count = 0
    _hire_request_count = 0
    _dispatch_count = 0
    _sairgent_proposal_count = 0
    _artifact_count = 0
    _outbox_artifacts = []
    _reset_artifact_dedup()
    _registry_reset_artifact_dedup()
    _registry_reset_audit_counters()
    _install_openai_chat_completion_patch()

    # 1. Read Environment injected by Rust Orchestrator
    agent_id = get_env_or_die("AGENT_ID")
    db_path = get_env_or_die("AGENT_DATABASE")
    provider = get_env_or_die("LLM_PROVIDER")
    model = get_env_or_die("LLM_MODEL")

    # Secure secrets cross the process boundary through stdin instead of environment variables.
    api_key, _tool_api_keys_by_slug, _mcp_credentials_by_slug = _load_worker_secrets_from_stdin()
    role = os.getenv("AGENT_ROLE", "Agent")
    persona_prompt = os.getenv("AGENT_PERSONA_PROMPT", f"Role: {role}")
    raison = os.getenv("AGENT_RAISON", "Assist the user")
    subordinates_json = os.getenv("AGENT_SUBORDINATES", "[]")
    can_hire = os.getenv("AGENT_CAN_HIRE", "1") == "1"
    allowed_tools = _allowed_tools()

    # ── Initialize tool registry modules ────────────────────────────────────
    # Inject runtime dependencies (API keys, shared counters) into tool modules
    # so they don't need to reach back into main.py.
    _shared_state = {
        "artifact_count": 0,
        "outbox_artifacts": _outbox_artifacts,
        "managed_work_count": 0,
        "innovation_count": 0,
        "hire_request_count": 0,
        "dispatch_count": 0,
        "sairgent_proposal_count": 0,
    }
    init_web_search(_tool_api_keys_by_slug)
    init_agent_ops(_shared_state)
    init_delegation(_shared_state)

    # ── Tool Registry ───────────────────────────────────────────────────────
    # Resolve capabilities + allowed_tools into concrete tool lists.
    # One call replaces the old imperative if-chain + dual gating systems.
    capabilities = _agent_capabilities()
    execution_tools, triage_tools = resolve_tools(
        capabilities=capabilities,
        allowed_tools=allowed_tools,
        can_hire=can_hire,
    )
    current_swo_id_raw = os.getenv("AGENT_SWO_ID", "")
    current_swo_id = int(current_swo_id_raw) if current_swo_id_raw.isdigit() else None
    revision_feedback = os.getenv("AGENT_REVISION_FEEDBACK", "").strip()

    # Ensure memory dir exists
    os.makedirs(os.path.dirname(db_path), exist_ok=True)
    os.environ["MEMORY_DATABASE"] = db_path

    # 2. Setup Sovereign Memory
    memory = AgentMemory(db_path)

    # 3. Setup PydanticAI Agent Model String
    model_str = f"{provider}:{model}"
    # Set env var for PydanticAI model resolution; cleaned up in finally block
    # to prevent leaking to child processes (HIGH-6).
    _env_key_for_cleanup = f"{provider.upper()}_API_KEY"
    os.environ[_env_key_for_cleanup] = api_key
    
    # 4. Parse Command Line Arguments
    if len(sys.argv) < 3:
        print(json.dumps(build_protocol_response(
            mode="unknown",
            agent_id=agent_id,
            error="Usage: worker.py <mode> <payload>",
            artifacts=_outbox_artifacts,
            side_effects=_collect_side_effects(),
        )))
        sys.exit(1)
        
    mode = sys.argv[1]
    payload = sys.argv[2]
    
    # Kryptonite fix #3b: Use the run_id provided by the orchestrator (AGENT_RUN_ID) so
    # heartbeats are keyed to the same run_id stored in active_swos.current_run_id.
    # Fall back to a fresh uuid4 for standalone / test invocations.
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    run_id = os.getenv("AGENT_RUN_ID", str(uuid.uuid4()))
    heartbeat_emitter = HeartbeatEmitter(token, run_id)

    # 5. Build MCP server instances (if any bound connectors)
    mcp_servers = _build_mcp_servers(_mcp_credentials_by_slug)

    try:
        heartbeat_emitter.start()
        
        if mode == "write_briefs":
            # Kernel has determined this is a manager with subordinates.
            # Perry picks routing targets AND writes briefs in one call.
            # No ANSWER_DIRECTLY option — the schema only produces briefs.
            routing_targets_json = os.getenv("AGENT_ROUTING_TARGETS", subordinates_json)
            agent = Agent(
                model=model_str,
                output_type=BriefWritingResult,
                system_prompt=build_write_briefs_prompt(persona_prompt, raison, routing_targets_json),
                tools=[],  # no tools needed — just brief writing
                retries=3,
                output_retries=3,
            )

            memory.append_interaction("user", f"WRITE BRIEFS: {payload}", swo_id=current_swo_id, mode="write_briefs", run_id=run_id, interaction_kind="task")

            wrapped_payload = f"<delegated_task>\n{payload}\n</delegated_task>"
            result = _run_agent_with_mcp(agent, wrapped_payload, [], message_history=[])
            briefs: BriefWritingResult = result.output

            # Convert to triage format expected by kernel (delegation_swos dict)
            triage_dump = {
                "action": "DELEGATE",
                "reasoning": f"Kernel-routed delegation. {len(briefs.delegation_targets)} brief(s) written.",
                "delegation_swos": {
                    target.subordinate_name: target.brief
                    for target in briefs.delegation_targets
                },
            }

            output_payload = {
                **build_protocol_response(
                    mode=mode,
                    agent_id=agent_id,
                    triage=triage_dump,
                    artifacts=_outbox_artifacts,
                    side_effects=_collect_side_effects(),
                )
            }
            memory.append_interaction("assistant", json.dumps(output_payload), swo_id=current_swo_id, mode="write_briefs", run_id=run_id, interaction_kind="typed_result")
            print(json.dumps(output_payload))

        elif mode == "execute_triage":
            recent_judgments = memory.format_decision_log_context(limit=3)

            # CHA-361: ICs physically cannot emit DELEGATE — the Literal constraint
            # removes the option from the JSON schema handed to the LLM.
            try:
                _parsed_subs = json.loads(subordinates_json)
            except (ValueError, TypeError):
                _parsed_subs = []
            triage_output_type = TriageDecisionManager if _parsed_subs else TriageDecisionIC

            agent = Agent(
                model=model_str,
                output_type=triage_output_type,
                system_prompt=build_triage_prompt(persona_prompt, raison, subordinates_json, recent_judgments, revision_feedback),
                tools=triage_tools,
                mcp_servers=mcp_servers if mcp_servers else [],
                retries=3,
                output_retries=3,
            )

            # Load context from SQLite
            message_history = _load_history(memory, "triage", exclude_failed_swos=True)

            memory.append_interaction("user", f"TRIAGE SWO: {payload}", swo_id=current_swo_id, mode="triage", run_id=run_id, interaction_kind="task")

            if revision_feedback:
                wrapped_payload = (
                    f"<human_revision_feedback>\n{revision_feedback}\n</human_revision_feedback>\n\n"
                    f"<previous_deliverable_note>Your previous deliverable has been reviewed. "
                    f"Apply the feedback above to improve your output.</previous_deliverable_note>\n\n"
                    f"<delegated_task>\n{payload}\n</delegated_task>"
                )
            else:
                wrapped_payload = f"<delegated_task>\n{payload}\n</delegated_task>"
            result = _run_agent_with_mcp(agent, wrapped_payload, mcp_servers, message_history=message_history)
            token_usage = _extract_token_usage(result, provider)
            decision: TriageDecision = result.output
            if decision.action == "ANSWER_DIRECTLY" and decision.direct_answer:
                _auto_materialize_output(mode, decision.direct_answer)

            # Convert new delegation_targets (name-based list) to legacy
            # delegation_swos (dict) format expected by the kernel protocol.
            # The kernel resolves names to UUIDs via case-insensitive lookup.
            triage_dump = decision.model_dump(exclude={"delegation_targets"})
            if decision.delegation_targets:
                triage_dump["delegation_swos"] = {
                    target.subordinate_name: target.brief
                    for target in decision.delegation_targets
                }

            output_payload = {
                **build_protocol_response(
                    mode=mode,
                    agent_id=agent_id,
                    triage=triage_dump,
                    artifacts=_outbox_artifacts,
                    side_effects=_collect_side_effects(),
                    token_usage=token_usage if token_usage else None,
                )
            }
            memory.append_interaction("assistant", json.dumps(output_payload), swo_id=current_swo_id, mode="triage", run_id=run_id, interaction_kind="typed_result")
            print(json.dumps(output_payload))

        elif mode == "execute_synthesis":
            agent = Agent(
                model=model_str,
                output_type=SynthesisDecision,
                system_prompt=build_synthesis_prompt(persona_prompt, raison, subordinates_json),
                tools=execution_tools,
                mcp_servers=mcp_servers if mcp_servers else [],
                retries=3,
                output_retries=3,
            )

            # Load context from SQLite
            message_history = _load_history(memory, "review")

            memory.append_interaction("user", f"SYNTHESIS CONTEXT: {payload}", swo_id=current_swo_id, mode="review", run_id=run_id, interaction_kind="task")

            # Synthesis JSON from potentially compromised or hallucinating delegates
            # is encapsulated in structural tags to preserve managerial prompt integrity.
            if revision_feedback:
                wrapped_payload = (
                    f"<human_revision_feedback>\n{revision_feedback}\n</human_revision_feedback>\n\n"
                    f"<synthesis_context>\n{payload}\n</synthesis_context>"
                )
            else:
                wrapped_payload = f"<synthesis_context>\n{payload}\n</synthesis_context>"
            result = _run_agent_with_mcp(agent, wrapped_payload, mcp_servers, message_history=message_history)
            token_usage = _extract_token_usage(result, provider)
            decision: SynthesisDecision = result.output
            if decision.final_response:
                _auto_materialize_output(mode, decision.final_response)

            output_payload = {
                **build_protocol_response(
                    mode=mode,
                    agent_id=agent_id,
                    synthesis=decision.model_dump(),
                    artifacts=_outbox_artifacts,
                    side_effects=_collect_side_effects(),
                    token_usage=token_usage if token_usage else None,
                )
            }
            memory.append_interaction("assistant", json.dumps(output_payload), swo_id=current_swo_id, mode="review", run_id=run_id, interaction_kind="typed_result")
            print(json.dumps(output_payload))
            
        elif mode == "chat_mode":
            registry_db_path = os.getenv("REGISTRY_DATABASE")
            
            prompt = build_chat_prompt(persona_prompt, raison, subordinates_json)
            
            _emit_debug(f"DEBUG CHAT_MODE INIT: db={registry_db_path}")
            _emit_debug(f"DEBUG CHAT_MODE INIT: subs={subordinates_json}")
            _emit_debug(f"DEBUG CHAT_MODE PROMPT:\n{prompt}")

            agent = Agent(
                model=model_str,
                output_type=str,
                system_prompt=prompt,
                tools=execution_tools,
                mcp_servers=mcp_servers if mcp_servers else [],
                retries=3,
                output_retries=3,
            )

            # Load context from SQLite
            message_history = _load_history(memory, "chat")

            memory.append_interaction("user", payload, swo_id=current_swo_id, mode="chat", run_id=run_id, interaction_kind="message")

            result = _run_agent_with_streaming(agent, payload, mcp_servers, message_history=message_history, message_id=run_id, agent_id=agent_id)
            token_usage = _extract_token_usage(result, provider)
            reply: str = result.output
            _emit_sidechannel({"__sairgent_delta": True, "delta": "", "message_id": run_id, "agent_id": agent_id, "is_final": True}, lock_timeout=None)
            _auto_materialize_output(mode, reply)

            output_payload = {
                **build_protocol_response(
                    mode=mode,
                    agent_id=agent_id,
                    reply=reply,
                    artifacts=_outbox_artifacts,
                    side_effects=_collect_side_effects(),
                    token_usage=token_usage if token_usage else None,
                )
            }
            if _managed_work_count == 0 and _shared_state.get("managed_work_count", 0) == 0:
                memory.append_interaction("assistant", reply, swo_id=current_swo_id, mode="chat", run_id=run_id, interaction_kind="message")
            print(json.dumps(output_payload))

        elif mode == "format_swo":
            agent = Agent(
                model=model_str,
                output_type=str,
                system_prompt=build_swo_formatter_prompt(persona_prompt, raison),
                retries=2,
                output_retries=2,
            )

            wrapped_payload = f"<delegation_request>\n{payload}\n</delegation_request>"
            result = agent.run_sync(wrapped_payload)
            token_usage = _extract_token_usage(result, provider)
            formatted_swo: str = result.output.strip()
            _auto_materialize_output(mode, formatted_swo)

            print(json.dumps(build_protocol_response(
                mode=mode,
                agent_id=agent_id,
                formatted_swo=formatted_swo,
                artifacts=_outbox_artifacts,
                side_effects=_collect_side_effects(),
                token_usage=token_usage if token_usage else None,
            )))
            
        elif mode == "execute_ideation":
            recent_decision_log = memory.format_decision_log_context(limit=5)
            agent = Agent(
                model=model_str,
                output_type=IdeationDecision,
                system_prompt=build_ideation_prompt(
                    persona_prompt,
                    raison,
                    subordinates_json,
                    recent_decision_log,
                ),
                tools=execution_tools,
                mcp_servers=mcp_servers if mcp_servers else [],
                retries=3,
                output_retries=3,
            )

            # Load context from SQLite
            message_history = _load_history(memory, "manual_review")

            memory.append_interaction("user", f"CRON_IDEATION: {payload}", swo_id=current_swo_id, mode="manual_review", run_id=run_id, interaction_kind="review_request")

            result = _run_agent_with_mcp(agent, payload, mcp_servers, message_history=message_history)
            token_usage = _extract_token_usage(result, provider)
            ideation: IdeationDecision = result.output
            ideation_summary = ideation.ideation_summary
            _record_ideation_decision_log(memory, ideation, current_swo_id, run_id)
            _auto_materialize_output(mode, ideation_summary)

            output_payload = {
                **build_protocol_response(
                    mode=mode,
                    agent_id=agent_id,
                    ideation_summary=ideation_summary,
                    artifacts=_outbox_artifacts,
                    side_effects=_collect_side_effects(),
                    token_usage=token_usage if token_usage else None,
                )
            }
            memory.append_interaction("assistant", ideation_summary, swo_id=current_swo_id, mode="manual_review", run_id=run_id, interaction_kind="review_summary")
            print(json.dumps(output_payload))

        elif mode == "sairgent_chat":
            # Sairgent super-agent mode: all standard tools + governed platform tools
            sairgent_governed_tools = [
                sairgent_create_project,
                sairgent_create_work_order,
                sairgent_create_agent,
                sairgent_update_agent_charter,
                sairgent_bind_tool_to_agent,
                sairgent_unbind_tool_from_agent,
                sairgent_set_project_status,
                sairgent_set_reporting_line,
            ]
            # "Eye of Sauron" query tools — full registry read access
            sairgent_query_tools = [
                get_recent_tasks,
                get_task_detail,
                get_projects,
                get_agents_overview,
                search_history,
                get_artifacts,
                read_artifact,
            ]

            # Standard tools from registry + governed platform tools
            all_tools = execution_tools + sairgent_governed_tools + sairgent_query_tools
            _emit_debug(f"DEBUG SAIRGENT_CHAT tools: registry={[t.__name__ if hasattr(t, '__name__') else str(t) for t in execution_tools]}")
            _emit_debug(f"DEBUG SAIRGENT_CHAT tools: governed={[t.__name__ if hasattr(t, '__name__') else str(t) for t in sairgent_governed_tools]}")
            _emit_debug(f"DEBUG SAIRGENT_CHAT tools: all_tools count={len(all_tools)}")

            # Build the unified Sairgent prompt using our chat persona + the kernel's runtime context.
            # The kernel injects runtime stats and roster into persona_prompt — we extract it as
            # context but wrap it in the Sairgent persona so the agent behaves like a helpful
            # assistant, not a sysadmin dumping stats.
            prompt = _build_sairgent_chat_prompt(persona_prompt, raison, subordinates_json)
            _emit_debug(f"DEBUG SAIRGENT_CHAT PROMPT:\n{prompt}")

            agent = Agent(
                model=model_str,
                output_type=str,
                system_prompt=prompt,
                tools=all_tools,
                mcp_servers=mcp_servers if mcp_servers else [],
                retries=3,
                output_retries=3,
            )
            # Log what PydanticAI actually registered
            try:
                if hasattr(agent, 'toolsets'):
                    for ts in agent.toolsets:
                        ts_tools = getattr(ts, 'tools', None)
                        if ts_tools:
                            _emit_debug(f"DEBUG SAIRGENT_CHAT toolset({type(ts).__name__}): {list(ts_tools.keys()) if isinstance(ts_tools, dict) else ts_tools}")
                elif hasattr(agent, '_function_tools'):
                    _emit_debug(f"DEBUG SAIRGENT_CHAT _function_tools: {list(agent._function_tools.keys())}")
                else:
                    _emit_debug(f"DEBUG SAIRGENT_CHAT agent attrs with 'tool': {[a for a in dir(agent) if 'tool' in a.lower()]}")
            except Exception as e:
                _emit_debug(f"DEBUG SAIRGENT_CHAT tool introspection error: {e}")

            message_history = _load_history(memory, "chat")
            memory.append_interaction("user", payload, swo_id=current_swo_id, mode="chat", run_id=run_id, interaction_kind="message")

            result = _run_agent_with_streaming(agent, payload, mcp_servers, message_history=message_history, message_id=run_id, agent_id=agent_id)
            token_usage = _extract_token_usage(result, provider)
            reply: str = result.output
            _emit_sidechannel({"__sairgent_delta": True, "delta": "", "message_id": run_id, "agent_id": agent_id, "is_final": True}, lock_timeout=None)
            _auto_materialize_output(mode, reply)

            output_payload = {
                **build_protocol_response(
                    mode=mode,
                    agent_id=agent_id,
                    reply=reply,
                    artifacts=_outbox_artifacts,
                    side_effects=_collect_side_effects(),
                    token_usage=token_usage if token_usage else None,
                )
            }
            if (_managed_work_count + _shared_state.get("managed_work_count", 0)) == 0 and _sairgent_proposal_count == 0:
                memory.append_interaction("assistant", reply, swo_id=current_swo_id, mode="chat", run_id=run_id, interaction_kind="message")
            print(json.dumps(output_payload))

        else:
            print(json.dumps(build_protocol_response(
                mode=mode,
                agent_id=agent_id,
                error=f"Unknown execution mode: {mode}",
                artifacts=_outbox_artifacts,
                side_effects=_collect_side_effects(),
            )))
            sys.exit(1)
            
    except (BlockedToolError, _RegistryBlockedToolError) as e:
        print(json.dumps(build_protocol_response(
            mode=locals().get("mode", "unknown"),
            agent_id=locals().get("agent_id", "unknown"),
            status="BLOCKED",
            blocked_reason=str(e),
            artifacts=_outbox_artifacts,
            side_effects=_collect_side_effects(),
        )))
        sys.exit(0)
    except UsageLimitExceeded as e:
        # CHA-425 sub-fix 2 — PydanticAI request_limit breach lands here instead
        # of propagating as a raw Internal error upstream. Emit a structured FAILED
        # response with an exception_code prefix in the error field so the kernel
        # can pattern-match and surface a clean failure to the operator.
        _request_limit = _agent_usage_limits().request_limit
        _mode = locals().get("mode", "unknown")
        _emit_stderr_line(
            f"[CHA-425] UsageLimitExceeded in mode={_mode}: {e}. "
            f"Emitted as FAILED with exception_code=request_limit_exceeded."
        )
        # For synthesis / triage modes, also emit a minimal synthesis stub so the
        # kernel's synthesis action dispatch has something structured to ingest.
        _synthesis_stub = None
        if _mode in ("execute_synthesis", "execute_triage"):
            _synthesis_stub = {
                "action": "REJECT_AND_REVISE",
                "reasoning": (
                    f"Manager exhausted the PydanticAI request_limit of {_request_limit} "
                    f"without reaching a synthesis decision. This usually indicates a "
                    f"reasoning spiral or runaway delegation loop. See CHA-425. "
                    f"Original error: {e}"
                ),
                "final_response": None,
                "revision_swos": None,
                "next_step_brief": None,
                "exception_code": "request_limit_exceeded",
            }
        print(json.dumps(build_protocol_response(
            mode=_mode,
            agent_id=locals().get("agent_id", "unknown"),
            error=f"[exception_code=request_limit_exceeded] Agent exhausted request_limit={_request_limit}: {e}",
            synthesis=_synthesis_stub,
            artifacts=_outbox_artifacts,
            side_effects=_collect_side_effects(),
        )))
        sys.exit(1)
    except Exception as e:
        # Prevent silent failure on run_sync errors by ensuring JSON goes to stdout
        print(json.dumps(build_protocol_response(
            mode=locals().get("mode", "unknown"),
            agent_id=locals().get("agent_id", "unknown"),
            error=f"Execution failed: {str(e)}",
            artifacts=_outbox_artifacts,
            side_effects=_collect_side_effects(),
        )))
        sys.exit(1)
    finally:
        # Clean up LLM API key from env to prevent leaking to child processes (HIGH-6)
        if _env_key_for_cleanup in os.environ:
            del os.environ[_env_key_for_cleanup]
        if 'heartbeat_emitter' in locals() and heartbeat_emitter.is_alive():
            heartbeat_emitter.stop()
            heartbeat_emitter.join(timeout=1.0)
        memory.close()

if __name__ == "__main__":
    run_worker()
