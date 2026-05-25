"""Unit tests for hsm.py Pydantic models — no LLM, pure validation."""
import pytest
from pydantic import ValidationError
from hsm import (
    TriageDecision,
    SynthesisDecision,
    InnovationReport,
    ManagedWorkRequest,
    HireSubordinateSpec,
    IdeationDecision,
    DecisionLogEntryDraft,
)


# ---------------------------------------------------------------------------
# TriageDecision
# ---------------------------------------------------------------------------

def test_triage_answer_directly():
    td = TriageDecision(
        action="ANSWER_DIRECTLY",
        reasoning="simple task",
        direct_answer="Here is the result.",
    )
    assert td.action == "ANSWER_DIRECTLY"
    assert td.direct_answer == "Here is the result."
    assert td.delegation_swos is None


def test_triage_delegate():
    td = TriageDecision(
        action="DELEGATE",
        reasoning="needs specialization",
        delegation_swos={"agent-uuid-1": "Do the analysis", "agent-uuid-2": "Review costs"},
    )
    assert td.action == "DELEGATE"
    assert len(td.delegation_swos) == 2
    assert td.direct_answer is None


def test_triage_requires_action_and_reasoning():
    with pytest.raises(ValidationError):
        TriageDecision(reasoning="no action here")
    with pytest.raises(ValidationError):
        TriageDecision(action="ANSWER_DIRECTLY")


def test_triage_exception_fields_optional():
    td = TriageDecision(
        action="ANSWER_DIRECTLY",
        reasoning="hard route failed",
        exception_code="NO_SUBORDINATE",
        exception_reason="Named agent not found",
        user_message="Sorry, that agent is unavailable.",
    )
    assert td.exception_code == "NO_SUBORDINATE"
    assert td.user_message is not None


def test_triage_serializes_to_dict():
    td = TriageDecision(action="ANSWER_DIRECTLY", reasoning="ok", direct_answer="done")
    d = td.model_dump()
    assert d["action"] == "ANSWER_DIRECTLY"
    assert "delegation_swos" in d


# ---------------------------------------------------------------------------
# SynthesisDecision
# ---------------------------------------------------------------------------

def test_synthesis_approve_and_reply():
    sd = SynthesisDecision(
        action="APPROVE_AND_REPLY",
        reasoning="all results look good",
        final_response="The consolidated answer is 42.",
    )
    assert sd.action == "APPROVE_AND_REPLY"
    assert sd.final_response is not None
    assert sd.revision_swos is None


def test_synthesis_reject_and_revise():
    sd = SynthesisDecision(
        action="REJECT_AND_REVISE",
        reasoning="result was incomplete",
        revision_swos={"agent-uuid-1": "Please redo section 3 with more detail."},
    )
    assert sd.action == "REJECT_AND_REVISE"
    assert len(sd.revision_swos) == 1
    assert sd.final_response is None


def test_synthesis_accept_and_complete():
    """CHA-410: ACCEPT_AND_COMPLETE with final_response validates correctly."""
    sd = SynthesisDecision(
        action="ACCEPT_AND_COMPLETE",
        reasoning="All deliverables answered the original question.",
        final_response="The final synthesized answer.",
    )
    assert sd.action == "ACCEPT_AND_COMPLETE"
    assert sd.final_response == "The final synthesized answer."
    assert sd.revision_swos is None
    assert sd.next_step_brief is None


def test_synthesis_accept_and_continue():
    """CHA-410: ACCEPT_AND_CONTINUE with next_step_brief validates correctly."""
    sd = SynthesisDecision(
        action="ACCEPT_AND_CONTINUE",
        reasoning="Pricing table is good, but dossier has 4 more sections to go.",
        next_step_brief="Delegate feature comparison section to Lois next.",
    )
    assert sd.action == "ACCEPT_AND_CONTINUE"
    assert sd.final_response is None
    assert sd.next_step_brief == "Delegate feature comparison section to Lois next."
    assert sd.revision_swos is None


def test_synthesis_reject_and_revise_new_action():
    """CHA-410: REJECT_AND_REVISE still works with the new schema shape."""
    sd = SynthesisDecision(
        action="REJECT_AND_REVISE",
        reasoning="Analysis missing pricing data for Acme Corp.",
        revision_swos={"agent-uuid-1": "Add Acme Corp pricing column to the comparison table."},
    )
    assert sd.action == "REJECT_AND_REVISE"
    assert len(sd.revision_swos) == 1
    assert sd.final_response is None
    assert sd.next_step_brief is None


def test_synthesis_legacy_approve_and_reply_accepted():
    """CHA-410: Legacy APPROVE_AND_REPLY is still accepted as a valid action string."""
    sd = SynthesisDecision(
        action="APPROVE_AND_REPLY",
        reasoning="All results look good.",
        final_response="The consolidated answer is 42.",
    )
    assert sd.action == "APPROVE_AND_REPLY"
    assert sd.final_response is not None


def test_synthesis_requires_action_and_reasoning():
    with pytest.raises(ValidationError):
        SynthesisDecision(reasoning="missing action")


# ---------------------------------------------------------------------------
# InnovationReport
# ---------------------------------------------------------------------------

def test_innovation_report_valid():
    ir = InnovationReport(
        title="Repetitive Task",
        context="Monthly report generation",
        proposed_solution="Automate with a cron agent",
        estimated_impact="Saves 2h/week",
    )
    assert ir.title == "Repetitive Task"


def test_innovation_report_requires_all_fields():
    with pytest.raises(ValidationError):
        InnovationReport(title="Missing fields")


# ---------------------------------------------------------------------------
# ManagedWorkRequest
# ---------------------------------------------------------------------------

def test_managed_work_request_defaults():
    mwr = ManagedWorkRequest(payload="Do the thing.")
    assert mwr.routing_policy == "NONE"
    assert mwr.requested_assignee_agent_id is None


def test_managed_work_request_hard_route():
    mwr = ManagedWorkRequest(
        payload="Run financial analysis.",
        requested_assignee_agent_id="lex-uuid",
        requested_assignee_name="Lex",
        routing_policy="HARD_ROUTE",
        user_visible_summary="Routing to Lex for financial work.",
    )
    assert mwr.routing_policy == "HARD_ROUTE"
    assert mwr.requested_assignee_name == "Lex"


# ---------------------------------------------------------------------------
# HireSubordinateSpec
# ---------------------------------------------------------------------------

def test_hire_subordinate_spec_valid():
    spec = HireSubordinateSpec(
        name="DataBot",
        role="Data Analyst",
        raison_detre="Analyze datasets and produce reports.",
        provider="openrouter",
        model="deepseek/deepseek-v3.2",
    )
    assert spec.name == "DataBot"
    assert spec.cron_interval_seconds is None


def test_hire_subordinate_spec_with_cron():
    spec = HireSubordinateSpec(
        name="Watchdog",
        role="Monitor",
        raison_detre="Watch system health.",
        provider="openrouter",
        model="deepseek/deepseek-v3.2",
        cron_interval_seconds=300,
    )
    assert spec.cron_interval_seconds == 300


def test_hire_subordinate_spec_requires_core_fields():
    with pytest.raises(ValidationError):
        HireSubordinateSpec(name="NoRole", raison_detre="...", provider="x", model="y")


# ---------------------------------------------------------------------------
# IdeationDecision + DecisionLogEntryDraft
# ---------------------------------------------------------------------------

def test_ideation_decision_valid():
    entry = DecisionLogEntryDraft(
        summary="Chose delegation over direct answer",
        rationale="Task required specialized knowledge",
        outcome="SUCCESS",
        confidence=0.9,
    )
    decision = IdeationDecision(
        ideation_summary="Reviewed SWO routing strategy.",
        decision_log=entry,
    )
    assert decision.decision_log.outcome == "SUCCESS"
    assert decision.decision_log.confidence == 0.9


def test_decision_log_optional_fields():
    entry = DecisionLogEntryDraft(
        summary="Test",
        rationale="Because",
        outcome="UNKNOWN",
    )
    assert entry.self_note is None
    assert entry.confidence is None
