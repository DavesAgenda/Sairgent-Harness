from __future__ import annotations

from typing import Any, Dict, List, Optional

WORKER_PROTOCOL_VERSION = "worker-protocol-v1"


def build_token_usage(
    input_tokens: int = 0,
    output_tokens: int = 0,
    cache_read_tokens: int = 0,
    cache_write_tokens: int = 0,
    requests: int = 0,
    cost_usd: Optional[float] = None,
) -> Dict[str, Any]:
    usage: Dict[str, Any] = {
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "cache_read_tokens": cache_read_tokens,
        "cache_write_tokens": cache_write_tokens,
        "requests": requests,
    }
    if cost_usd is not None:
        usage["cost_usd"] = cost_usd
    return usage


def build_side_effects(
    managed_work_count: int = 0,
    artifact_count: int = 0,
    innovation_count: int = 0,
    hire_request_count: int = 0,
    dispatch_count: int = 0,
    sairgent_proposal_count: int = 0,
) -> Dict[str, int]:
    return {
        "managed_work_count": managed_work_count,
        "artifact_count": artifact_count,
        "innovation_count": innovation_count,
        "hire_request_count": hire_request_count,
        "dispatch_count": dispatch_count,
        "sairgent_proposal_count": sairgent_proposal_count,
    }


def build_protocol_response(
    *,
    mode: str,
    agent_id: str,
    status: str = "COMPLETED",
    triage: Optional[Dict[str, Any]] = None,
    synthesis: Optional[Dict[str, Any]] = None,
    reply: Optional[str] = None,
    formatted_swo: Optional[str] = None,
    ideation_summary: Optional[str] = None,
    blocked_reason: Optional[str] = None,
    error: Optional[str] = None,
    artifacts: Optional[List[Dict[str, Any]]] = None,
    side_effects: Optional[Dict[str, int]] = None,
    token_usage: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    payload: Dict[str, Any] = {
        "protocol_version": WORKER_PROTOCOL_VERSION,
        "mode": mode,
        "agent_id": agent_id,
        "status": status,
        "artifacts": artifacts or [],
        "side_effects": side_effects or build_side_effects(),
    }

    if triage is not None:
        payload["triage"] = triage
    if synthesis is not None:
        payload["synthesis"] = synthesis
    if reply is not None:
        payload["reply"] = reply
    if formatted_swo is not None:
        payload["formatted_swo"] = formatted_swo
    if ideation_summary is not None:
        payload["ideation_summary"] = ideation_summary
    if blocked_reason is not None:
        payload["blocked_reason"] = blocked_reason
    if error is not None:
        payload["error"] = error

    if token_usage is not None:
        payload["token_usage"] = token_usage

    if error:
        payload["status"] = "FAILED"

    return payload
