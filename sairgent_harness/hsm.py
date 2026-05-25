from enum import Enum
from pydantic import BaseModel, Field
from typing import Literal, Optional, List, Dict, Any

class HSMState(Enum):
    READY = "READY"
    TRIAGE = "TRIAGE"
    DELEGATING = "DELEGATING"
    SYNTHESIS = "SYNTHESIS"
    COMPLETED = "COMPLETED"

class DelegationTarget(BaseModel):
    """A single delegation assignment from a manager to a subordinate."""
    subordinate_name: str = Field(description="Exact name of the subordinate to delegate to (e.g. 'Lois', 'Cat Grant', 'Felicity')")
    brief: str = Field(description="A clear, specific work brief for this subordinate explaining what to produce")

class _TriageDecisionBase(BaseModel):
    """Shared fields for all triage decisions. Do not instantiate directly."""
    reasoning: str = Field(description="Why this decision was made against the Goal context")
    direct_answer: Optional[str] = Field(description="If ANSWER_DIRECTLY, provide the final response here", default=None)
    delegation_targets: Optional[List[DelegationTarget]] = Field(description="If DELEGATE, list the subordinates and their work briefs", default=None)
    # Legacy field — kept for backwards compatibility with in-flight workers
    delegation_swos: Optional[Dict[str, str]] = Field(description="Deprecated. Use delegation_targets instead.", default=None, exclude=True)
    exception_code: Optional[str] = Field(description="If a hard-route request cannot be honored, provide a short machine-readable exception code.", default=None)
    exception_reason: Optional[str] = Field(description="Explain why the required subordinate could not be used.", default=None)
    user_message: Optional[str] = Field(description="A truthful user-facing explanation when a hard-route request cannot be fulfilled as asked.", default=None)

class TriageDecisionManager(_TriageDecisionBase):
    """Triage output for agents that have subordinates. Can answer directly, delegate, or raise an exception."""
    action: Literal["ANSWER_DIRECTLY", "DELEGATE", "EXCEPTION"] = Field(description="ANSWER_DIRECTLY to respond yourself, DELEGATE to assign work to subordinates, or EXCEPTION if you cannot honor a hard-route request.")

class TriageDecisionIC(_TriageDecisionBase):
    """Triage output for individual contributors with no subordinates. DELEGATE is unavailable."""
    action: Literal["ANSWER_DIRECTLY", "EXCEPTION"] = Field(description="ANSWER_DIRECTLY to produce the final response yourself, or EXCEPTION if you cannot fulfill the request.")

# Backwards-compat alias — existing code that imports TriageDecision keeps working.
TriageDecision = TriageDecisionManager

class BriefWritingResult(BaseModel):
    """Result of a manager writing tailored briefs for pre-selected subordinates."""
    delegation_targets: List[DelegationTarget] = Field(description="One brief per routing target. Every subordinate listed in the prompt MUST get a brief.")

class SynthesisDecision(BaseModel):
    """Manager synthesis decision on child deliverables.

    CHA-410: action is now split into three semantically distinct values:

    - ACCEPT_AND_COMPLETE: child deliverable acceptable AND parent job is done.
      The manager produces a final_response and the parent SWO is finalized.
    - ACCEPT_AND_CONTINUE: child deliverable acceptable BUT the job is not done.
      The manager intends to delegate another piece of work on their next turn.
      (Kernel continuation loop is a follow-up — CHA-421 — so for now the kernel
       records the intent and finalizes the SWO as if ACCEPT_AND_COMPLETE.)
    - REJECT_AND_REVISE: deliverable not acceptable. revision_swos must be
      populated with a map of subordinate agent IDs to revision payloads.

    Legacy value APPROVE_AND_REPLY is accepted as an alias for ACCEPT_AND_COMPLETE
    during migration. New managers should emit the three-valued form.
    """
    action: str = Field(
        description="One of: ACCEPT_AND_COMPLETE, ACCEPT_AND_CONTINUE, REJECT_AND_REVISE. "
                    "Legacy APPROVE_AND_REPLY is accepted as alias for ACCEPT_AND_COMPLETE."
    )
    reasoning: str = Field(description="Brief rationale for the decision — why this is acceptable or not, and why the job is or isn't done.")
    final_response: Optional[str] = Field(
        description="Required when action is ACCEPT_AND_COMPLETE. The synthesized final answer for the requesting party.",
        default=None,
    )
    revision_swos: Optional[Dict[str, str]] = Field(
        description="Required when action is REJECT_AND_REVISE. Map of subordinate agent ID to revision payload describing what needs to be redone.",
        default=None,
    )
    next_step_brief: Optional[str] = Field(
        description="Optional for ACCEPT_AND_CONTINUE: a short note describing what the manager intends to delegate next. Surfaces intent in logs.",
        default=None,
    )

class InnovationReport(BaseModel):
    """A report detailing a discovered systemic inefficiency or a novel solution."""
    title: str = Field(description="A concise title for the innovation or efficiency discovery")
    context: str = Field(description="The context or specific SWO that triggered this discovery")
    proposed_solution: str = Field(description="The actionable change recommended to the Manager")
    estimated_impact: str = Field(description="The estimated impact of implementing this solution")


class DecisionLogEntryDraft(BaseModel):
    summary: str = Field(description="The key decision, lesson, or heuristic reinforced by this ideation cycle.")
    rationale: str = Field(description="Why this decision or lesson matters based on the run context.")
    outcome: str = Field(description="One of SUCCESS, PARTIAL, FAILED, or UNKNOWN.")
    self_note: Optional[str] = Field(description="Optional short reminder for the next ideation cycle.", default=None)
    confidence: Optional[float] = Field(description="Optional self-assessed confidence from 0.0 to 1.0.", default=None)


class IdeationDecision(BaseModel):
    ideation_summary: str = Field(description="A concise summary of the ideation or proactive review output.")
    decision_log: DecisionLogEntryDraft = Field(description="Structured decision memory recorded from this ideation cycle.")


class ManagedWorkRequest(BaseModel):
    payload: str = Field(description="The actual async work payload that should enter the managed SWO queue.")
    requested_assignee_agent_id: Optional[str] = Field(description="Exact subordinate UUID if the user named a direct report for this work.", default=None)
    requested_assignee_name: Optional[str] = Field(description="Human-readable subordinate name for audit/UI projection.", default=None)
    routing_policy: str = Field(description="One of HARD_ROUTE, PREFERENCE, or NONE.", default="NONE")
    user_visible_summary: Optional[str] = Field(description="Concise truthful acknowledgement summary for the user-facing feed.", default=None)


class HireSubordinateSpec(BaseModel):
    name: str = Field(description="Name of the direct-report agent to create.")
    role: str = Field(description="Role or title of the new agent.")
    raison_detre: str = Field(description="System prompt purpose / raison d'etre for the new agent.")
    provider: str = Field(description="LLM provider identifier to use for this agent.")
    model: str = Field(description="Default model identifier to use for this agent.")
    cron_interval_seconds: Optional[int] = Field(description="Optional heartbeat interval for the new agent.", default=None)
    reports_to: Optional[str] = Field(
        description=(
            "CHA-428: optional 'hire on behalf of another manager'. Set to the target "
            "manager's exact name (e.g. 'Felicity') when the new agent should report to "
            "someone OTHER than yourself. If unset, the new agent reports to you (the caller). "
            "The kernel enforces authorization — you must be an ancestor of the target in the "
            "org tree, or be the root (Perry), to place hires under another manager. The "
            "per-manager direct-reports cap is checked against the target, not against you."
        ),
        default=None,
    )
