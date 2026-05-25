"""Unit tests for worker_protocol.py — pure functions, no LLM needed."""
import pytest
from worker_protocol import build_protocol_response, build_side_effects, WORKER_PROTOCOL_VERSION


# ---------------------------------------------------------------------------
# build_side_effects
# ---------------------------------------------------------------------------

def test_side_effects_defaults():
    se = build_side_effects()
    assert se["managed_work_count"] == 0
    assert se["artifact_count"] == 0
    assert se["innovation_count"] == 0
    assert se["hire_request_count"] == 0
    assert se["dispatch_count"] == 0
    assert se["sairgent_proposal_count"] == 0


def test_side_effects_non_default():
    se = build_side_effects(managed_work_count=3, artifact_count=2, innovation_count=1)
    assert se["managed_work_count"] == 3
    assert se["artifact_count"] == 2
    assert se["innovation_count"] == 1
    assert se["hire_request_count"] == 0


# ---------------------------------------------------------------------------
# build_protocol_response — required envelope fields
# ---------------------------------------------------------------------------

def test_protocol_version_always_present():
    resp = build_protocol_response(mode="execute_triage", agent_id="agent-1")
    assert resp["protocol_version"] == WORKER_PROTOCOL_VERSION


def test_default_status_completed():
    resp = build_protocol_response(mode="execute_triage", agent_id="agent-1")
    assert resp["status"] == "COMPLETED"


def test_error_forces_failed_status():
    resp = build_protocol_response(mode="execute_triage", agent_id="agent-1", error="boom")
    assert resp["status"] == "FAILED"
    assert resp["error"] == "boom"


def test_explicit_status_without_error():
    resp = build_protocol_response(mode="chat_mode", agent_id="agent-1", status="BLOCKED")
    assert resp["status"] == "BLOCKED"


def test_artifacts_default_empty_list():
    resp = build_protocol_response(mode="execute_triage", agent_id="agent-1")
    assert resp["artifacts"] == []


def test_side_effects_default_all_zeros():
    resp = build_protocol_response(mode="execute_triage", agent_id="agent-1")
    se = resp["side_effects"]
    assert all(v == 0 for v in se.values())


# ---------------------------------------------------------------------------
# build_protocol_response — optional payload fields
# ---------------------------------------------------------------------------

def test_triage_field_included_when_provided():
    triage = {"action": "ANSWER_DIRECTLY", "reasoning": "simple", "direct_answer": "42"}
    resp = build_protocol_response(mode="execute_triage", agent_id="a", triage=triage)
    assert resp["triage"] == triage


def test_triage_field_absent_when_not_provided():
    resp = build_protocol_response(mode="execute_triage", agent_id="a")
    assert "triage" not in resp


def test_synthesis_field_included():
    synthesis = {"action": "APPROVE_AND_REPLY", "reasoning": "good", "final_response": "done"}
    resp = build_protocol_response(mode="execute_synthesis", agent_id="a", synthesis=synthesis)
    assert resp["synthesis"]["action"] == "APPROVE_AND_REPLY"


def test_reply_field_included():
    resp = build_protocol_response(mode="chat_mode", agent_id="a", reply="Hello!")
    assert resp["reply"] == "Hello!"


def test_blocked_reason_included():
    resp = build_protocol_response(
        mode="execute_triage", agent_id="a",
        blocked_reason="waiting_for_approval"
    )
    assert resp["blocked_reason"] == "waiting_for_approval"
    # blocked_reason alone should not force FAILED
    assert resp["status"] == "COMPLETED"


def test_ideation_summary_included():
    resp = build_protocol_response(
        mode="execute_ideation", agent_id="a",
        ideation_summary="Reviewed team structure."
    )
    assert resp["ideation_summary"] == "Reviewed team structure."


def test_formatted_swo_included():
    resp = build_protocol_response(mode="format_swo", agent_id="a", formatted_swo="## Task\nDo the thing.")
    assert resp["formatted_swo"] == "## Task\nDo the thing."


def test_mode_and_agent_id_passed_through():
    resp = build_protocol_response(mode="some_mode", agent_id="uuid-xyz")
    assert resp["mode"] == "some_mode"
    assert resp["agent_id"] == "uuid-xyz"


# ---------------------------------------------------------------------------
# Artifact plumbing
# ---------------------------------------------------------------------------

def test_artifacts_passed_through():
    arts = [{"filename": "report.md", "absolute_path": "/tmp/report.md"}]
    resp = build_protocol_response(mode="execute_triage", agent_id="a", artifacts=arts)
    assert resp["artifacts"] == arts


def test_side_effects_passed_through():
    se = build_side_effects(artifact_count=5)
    resp = build_protocol_response(mode="execute_triage", agent_id="a", side_effects=se)
    assert resp["side_effects"]["artifact_count"] == 5
