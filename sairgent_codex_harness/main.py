import json
import os
import subprocess
import sys
import tempfile
import threading
import uuid
from pathlib import Path
from typing import Optional

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from sairgent_harness.memory import AgentMemory
from sairgent_harness.worker_protocol import build_protocol_response, build_side_effects

stderr_lock = threading.Lock()


def get_env_or_die(key: str) -> str:
    value = os.getenv(key)
    if not value:
        print(json.dumps({"error": f"Missing required env var: {key}"}))
        sys.exit(1)
    return value


def emit_stderr_line(line: str) -> None:
    with stderr_lock:
        if not line.endswith("\n"):
            line += "\n"
        sys.stderr.write(line)
        sys.stderr.flush()


def emit_sidechannel(payload: dict) -> None:
    emit_stderr_line(json.dumps(payload))


class HeartbeatEmitter(threading.Thread):
    def __init__(self, token: str, run_id: str):
        super().__init__(daemon=True)
        self.token = token
        self.run_id = run_id
        self._stop_event = threading.Event()
        self.seq = 0

    def stop(self) -> None:
        self._stop_event.set()

    def run(self) -> None:
        while not self._stop_event.is_set():
            emit_sidechannel(
                {
                    "__sairgent_sidechannel": "heartbeat",
                    "token": self.token,
                    "run_id": self.run_id,
                    "seq": self.seq,
                    "status": "COMPUTING",
                }
            )
            self.seq += 1
            self._stop_event.wait(2.0)


def normalize_subordinates(raw: str) -> str:
    try:
        items = json.loads(raw) if raw else []
    except Exception:
        items = []
    if not items:
        return "None."
    return "\n".join(
        f"- {item.get('name', 'Unknown')} ({item.get('role', 'Unknown')}) id={item.get('id', 'Unknown')}"
        for item in items
    )


def decision_log_retention_limit(default: int = 500) -> int:
    raw = os.getenv("DECISION_LOG_MAX_ENTRIES", "").strip()
    if not raw:
        return default
    try:
        val = int(raw)
        return val if val > 0 else default
    except ValueError:
        return default


def record_ideation_decision_log(
    memory: AgentMemory,
    result: dict,
    swo_id: Optional[int],
    run_id: Optional[str],
) -> None:
    decision_log = result.get("decision_log") or {}
    memory.append_decision_log_entry(
        entry_id=os.urandom(16).hex(),
        mode="ideation",
        summary=str(decision_log.get("summary", "")).strip(),
        rationale=str(decision_log.get("rationale", "")).strip(),
        outcome=str(decision_log.get("outcome", "UNKNOWN")).strip().upper() or "UNKNOWN",
        confidence=decision_log.get("confidence"),
        self_note=(str(decision_log.get("self_note", "")).strip() or None),
        linked_swo_id=swo_id,
        linked_run_id=run_id,
    )
    memory.prune_decision_log(decision_log_retention_limit())


def emit_artifacts(artifacts: list[dict], swo_id: Optional[int]) -> None:
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    if not token or swo_id is None:
        if artifacts:
            reasons = []
            if not token:
                reasons.append("SAIRGENT_SIDECHANNEL_TOKEN is empty")
            if swo_id is None:
                reasons.append("AGENT_SWO_ID is empty")
            emit_stderr_line(
                f"[WARNING] Skipping sidechannel registration for {len(artifacts)} artifact(s): {', '.join(reasons)}"
            )
        return

    for artifact in artifacts:
        absolute_path = artifact.get("absolute_path")
        filename = artifact.get("filename")
        if not absolute_path or not filename:
            continue
        emit_sidechannel(
            {
                "__sairgent_sidechannel": "outbox_artifact",
                "token": token,
                "swo_id": swo_id,
                "filename": filename,
                "absolute_path": absolute_path,
            }
        )


def _safe_outbox_path(filename: str) -> Optional[Path]:
    outbox_dir = os.getenv("AGENT_OUTBOX", "").strip()
    if not outbox_dir or not filename or any(ch in filename for ch in ("/", "\\", "\x00")):
        return None

    base = Path(outbox_dir).expanduser().resolve()
    candidate = (base / filename).resolve()
    try:
        candidate.relative_to(base)
    except ValueError:
        return None
    return candidate


def _pretty_or_text(value: str) -> tuple[str, str]:
    try:
        parsed = json.loads(value)
    except Exception:
        return value.strip(), "md"
    return json.dumps(parsed, indent=2), "json"


def _write_auto_artifact(filename: str, content: str) -> Optional[dict]:
    path = _safe_outbox_path(filename)
    if path is None:
        return None
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    return {"filename": path.name, "absolute_path": str(path)}


def _auto_artifact_for_mode(mode: str, result: dict) -> Optional[dict]:
    if result.get("artifacts"):
        return None

    if mode == "execute_triage" and result.get("action") == "ANSWER_DIRECTLY" and result.get("direct_answer"):
        body, ext = _pretty_or_text(result["direct_answer"])
        return _write_auto_artifact(f"triage-output.{ext}", body + ("\n" if not body.endswith("\n") else ""))

    if mode == "execute_synthesis" and result.get("final_response"):
        body, ext = _pretty_or_text(result["final_response"])
        return _write_auto_artifact(f"synthesis-output.{ext}", body + ("\n" if not body.endswith("\n") else ""))

    if mode == "chat_mode" and result.get("reply"):
        return _write_auto_artifact("chat-reply.md", result["reply"].strip() + "\n")

    if mode == "format_swo" and result.get("formatted_swo"):
        return _write_auto_artifact("formatted-swo.md", result["formatted_swo"].strip() + "\n")

    if mode == "execute_ideation" and result.get("ideation_summary"):
        return _write_auto_artifact("ideation-summary.md", result["ideation_summary"].strip() + "\n")

    return None


def _capability_notes() -> str:
    raw_manifest = os.getenv("AGENT_MANIFEST_JSON", "")
    if not raw_manifest:
        return "No additional governed capabilities were declared for this run."

    try:
        manifest = json.loads(raw_manifest)
    except Exception:
        return "No additional governed capabilities were declared for this run."

    capabilities = set(manifest.get("capabilities") or [])
    notes: list[str] = []
    if "web_search" in capabilities:
        notes.append(
            "- Web research is authorized for this run. Use built-in browser/search capabilities for time-sensitive factual claims when available, and include source-backed citations in the output."
        )
        notes.append(
            "- If browser/search is unavailable in the current backend, say so explicitly and do not fabricate citations."
        )
    return "\n".join(notes) if notes else "No additional governed capabilities were declared for this run."


def _load_skill_index() -> list[dict]:
    raw = os.getenv("AGENT_SKILL_INDEX_JSON", "[]")
    try:
        parsed = json.loads(raw)
    except Exception:
        parsed = []
    return [item for item in parsed if isinstance(item, dict)]


def _runtime_skill_section() -> str:
    entries = _load_skill_index()
    runtime_dir = os.getenv("AGENT_RUNTIME_DIR", "")
    if not entries:
        return "No bound skills are available for this run."

    lines = []
    for entry in entries:
        marker = "preselected" if entry.get("preselected") else "available"
        path = entry.get("runtime_path")
        path_suffix = f" path={path}" if path else ""
        lines.append(
            f"- {entry.get('name', 'Unknown')} [{marker}] :: {entry.get('summary', '')}{path_suffix}"
        )
    bundle_note = (
        f"Hidden runtime bundle root: {runtime_dir}. Inspect skill files only when relevant."
        if runtime_dir
        else "No runtime bundle path is available."
    )
    return "\n".join(lines) + f"\n{bundle_note}"


def build_prompt(
    mode: str,
    role: str,
    persona_prompt: str,
    raison: str,
    payload: str,
    subordinates_json: str,
    recent_decision_log: str = "No prior decision log entries recorded.",
) -> str:
    requested_assignee_id = os.getenv("AGENT_REQUESTED_ASSIGNEE_ID", "")
    requested_assignee_name = os.getenv("AGENT_REQUESTED_ASSIGNEE_NAME", "")
    routing_policy = os.getenv("AGENT_ROUTING_POLICY", "NONE")
    outbox_dir = os.getenv("AGENT_OUTBOX", "")
    subs = normalize_subordinates(subordinates_json)
    skill_section = _runtime_skill_section()

    if mode == "execute_triage":
        return f"""
[Persona]
{persona_prompt}

[Mission]
{raison}

[Available Skills]
{skill_section}

[Governed Capabilities]
{_capability_notes()}

[Task]
You are performing triage for a structured work order.

Work order:
{payload}

Available subordinates:
{subs}

Routing contract:
- requested assignee id: {requested_assignee_id or "(none)"}
- requested assignee name: {requested_assignee_name or "(none)"}
- routing policy: {routing_policy}

[Output Contract]
Return valid JSON that matches the supplied schema.

[Mode Rules]
- Choose `ANSWER_DIRECTLY` only if you can provide a concrete outcome now.
- Choose `DELEGATE` if the work should be split or assigned to a specialist.
- If you delegate, `delegation_swos` keys must be exact subordinate UUID strings.
- Do not treat acknowledgements, promises, or "I will" language as a direct answer.
- If a hard route cannot be honored, provide typed exception fields.
- If you create a deliverable file, write it under `{outbox_dir}` and include it in `artifacts`.
"""

    if mode == "execute_synthesis":
        return f"""
[Persona]
{persona_prompt}

[Mission]
{raison}

[Available Skills]
{skill_section}

[Governed Capabilities]
{_capability_notes()}

[Task]
You are reviewing subordinate work and deciding whether it is complete.

Review context:
{payload}

[Output Contract]
Return valid JSON that matches the supplied schema.

[Mode Rules]
- Use `APPROVE_AND_REPLY` only if the work contains a concrete result, artifact, or sufficient answer.
- Use `REJECT_AND_REVISE` for incomplete, vague, acknowledgement-only, or promise-only work.
- Do not claim files or actions that did not actually happen.
- If you create a deliverable file, write it under `{outbox_dir}` and include it in `artifacts`.
"""

    if mode == "chat_mode":
        return f"""
[Persona]
{persona_prompt}

[Mission]
{raison}

[Available Skills]
{skill_section}

[Governed Capabilities]
{_capability_notes()}

[Task]
You are replying in chat. If the work should be queued for asynchronous execution, populate `managed_work_requests` truthfully instead of pretending the work has started.

User message:
{payload}

Team context:
{subs}

[Output Contract]
Return valid JSON that matches the supplied schema.

[Mode Rules]
- Use `reply` for direct conversational content.
- Use `managed_work_requests` only for real follow-on work that should become managed SWOs.
- If a route is explicitly requested, put it on the managed work item.
- Do not fabricate filesystem paths or completion claims.
"""

    if mode == "format_swo":
        return f"""
[Persona]
{persona_prompt}

[Mission]
{raison}

[Available Skills]
{skill_section}

[Task]
Convert the request below into one clean subordinate work brief:

{payload}

[Output Contract]
Return valid JSON that matches the supplied schema.

[Mode Rules]
- Preserve objective, constraints, and deliverable.
- Return one concise standalone instruction in `formatted_swo`.
- Do not include JSON, UUIDs, or meta commentary in the brief itself.
"""

    if mode == "execute_ideation":
        return f"""
[Persona]
{persona_prompt}

[Mission]
{raison}

[Available Skills]
{skill_section}

[Task]
Generate 1-3 concrete proactive review ideas based on your mission and current context.

Prompt:
{payload}

[Recent Decision Log]
{recent_decision_log}

[Output Contract]
Return valid JSON that matches the supplied schema.

[Mode Rules]
- Use `ideation_summary` for the final answer.
- Always populate `decision_log.summary`, `decision_log.rationale`, and `decision_log.outcome`.
- Use `SUCCESS`, `PARTIAL`, `FAILED`, or `UNKNOWN` for `decision_log.outcome`.
- Use `blocked_reason` only if a meaningful proactive review cannot be produced.
- Do not fabricate external actions.
"""

    return f"""
[Persona]
{persona_prompt}

[Mission]
{raison}

[Available Skills]
{skill_section}

[Governed Capabilities]
{_capability_notes()}

[Task]
{payload}

[Output Contract]
Return valid JSON that matches the supplied schema.

[Mode Rules]
- Produce concrete output, not acknowledgements.
- If you create a deliverable file, write it under `{outbox_dir}` and include it in `artifacts`.
"""


def write_schema_file(schema: dict) -> str:
    handle = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
    with handle:
        json.dump(schema, handle)
    return handle.name


def _forward_stream(stream, label: str, sink: list[str]) -> None:
    try:
        for line in iter(stream.readline, ""):
            sink.append(line)
            emit_stderr_line(f"[codex_cli:{label}] {line.rstrip()}")
    finally:
        stream.close()


# CHA-401 (Kryptonite B1) — env whitelist for the codex child process.
# The codex binary authenticates via its own config file under HOME
# (typically ~/.codex/auth.json), so no LLM API keys need to cross the
# process boundary. Sairgent secrets (SAIRGENT_SIDECHANNEL_TOKEN,
# AGENT_MANIFEST_JSON, REGISTRY_DATABASE) are stripped by omission so a
# prompt-injected codex session cannot read them via `env` or `printenv`.
_ALLOWED_CODEX_ENV: frozenset[str] = frozenset({
    "PATH", "HOME", "USER", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TZ",
    "TMPDIR", "AGENT_WORKSPACE",
})


def run_codex(prompt: str, schema: dict) -> dict:
    schema_path = write_schema_file(schema)
    output_handle = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
    output_path = output_handle.name
    output_handle.close()
    codex_bin = os.getenv("CODEX_CLI_BIN", "codex")
    codex_workdir = os.getenv("AGENT_ARTIFACTS", str(REPO_ROOT))
    stdout_lines: list[str] = []
    stderr_lines: list[str] = []
    codex_env = {k: os.environ[k] for k in _ALLOWED_CODEX_ENV if k in os.environ}
    try:
        proc = subprocess.Popen(
            [
                codex_bin,
                "exec",
                "-C",
                codex_workdir,
                "--skip-git-repo-check",
                "--full-auto",
                "--output-schema",
                schema_path,
                "--output-last-message",
                output_path,
                "-",
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            env=codex_env,
        )
        assert proc.stdin is not None
        assert proc.stdout is not None
        assert proc.stderr is not None
        proc.stdin.write(prompt)
        proc.stdin.close()

        stdout_thread = threading.Thread(
            target=_forward_stream,
            args=(proc.stdout, "stdout", stdout_lines),
            daemon=True,
        )
        stderr_thread = threading.Thread(
            target=_forward_stream,
            args=(proc.stderr, "stderr", stderr_lines),
            daemon=True,
        )
        stdout_thread.start()
        stderr_thread.start()
        return_code = proc.wait()
        stdout_thread.join()
        stderr_thread.join()
    finally:
        try:
            os.unlink(schema_path)
        except OSError:
            pass

    if return_code != 0:
        try:
            os.unlink(output_path)
        except OSError:
            pass
        raise RuntimeError(
            "Codex CLI failed with exit code "
            f"{return_code}: "
            f"{''.join(stderr_lines).strip() or ''.join(stdout_lines).strip()}"
        )

    try:
        output = Path(output_path).read_text().strip()
    finally:
        try:
            os.unlink(output_path)
        except OSError:
            pass
    if not output:
        raise RuntimeError("Codex CLI returned empty output")
    return json.loads(output)


def schema_for_mode(mode: str) -> dict:
    artifact_schema = {
        "type": "array",
        "items": {
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "filename": {"type": "string"},
                "absolute_path": {"type": "string"},
            },
            "required": ["filename", "absolute_path"],
        },
    }
    if mode == "execute_triage":
        return {
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "action": {"type": "string", "enum": ["ANSWER_DIRECTLY", "DELEGATE"]},
                "reasoning": {"type": "string"},
                "direct_answer": {"type": ["string", "null"]},
                "delegation_swos": {
                    "type": ["object", "null"],
                    "additionalProperties": {"type": "string"},
                },
                "exception_code": {"type": ["string", "null"]},
                "exception_reason": {"type": ["string", "null"]},
                "user_message": {"type": ["string", "null"]},
                "artifacts": artifact_schema,
                "blocked_reason": {"type": ["string", "null"]},
            },
            "required": ["action", "reasoning", "direct_answer", "delegation_swos", "exception_code", "exception_reason", "user_message", "artifacts", "blocked_reason"],
        }
    if mode == "chat_mode":
        return {
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "reply": {"type": "string"},
                "managed_work_requests": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": False,
                        "properties": {
                            "payload": {"type": "string"},
                            "requested_assignee_agent_id": {"type": ["string", "null"]},
                            "requested_assignee_name": {"type": ["string", "null"]},
                            "routing_policy": {"type": "string", "enum": ["HARD_ROUTE", "PREFERENCE", "NONE"]},
                            "user_visible_summary": {"type": ["string", "null"]},
                        },
                        "required": ["payload", "requested_assignee_agent_id", "requested_assignee_name", "routing_policy", "user_visible_summary"],
                    },
                },
                "artifacts": artifact_schema,
                "blocked_reason": {"type": ["string", "null"]},
            },
            "required": ["reply", "managed_work_requests", "artifacts", "blocked_reason"],
        }
    if mode == "format_swo":
        return {
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "formatted_swo": {"type": "string"},
                "blocked_reason": {"type": ["string", "null"]},
            },
            "required": ["formatted_swo", "blocked_reason"],
        }
    if mode == "execute_ideation":
        return {
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "ideation_summary": {"type": "string"},
                "decision_log": {
                    "type": "object",
                    "additionalProperties": False,
                    "properties": {
                        "summary": {"type": "string"},
                        "rationale": {"type": "string"},
                        "outcome": {"type": "string", "enum": ["SUCCESS", "PARTIAL", "FAILED", "UNKNOWN"]},
                        "self_note": {"type": ["string", "null"]},
                        "confidence": {"type": ["number", "null"]},
                    },
                    "required": ["summary", "rationale", "outcome", "self_note", "confidence"],
                },
                "artifacts": artifact_schema,
                "blocked_reason": {"type": ["string", "null"]},
            },
            "required": ["ideation_summary", "decision_log", "artifacts", "blocked_reason"],
        }
    return {
        "type": "object",
        "additionalProperties": False,
        "properties": {
            "action": {"type": "string", "enum": ["APPROVE_AND_REPLY", "REJECT_AND_REVISE"]},
            "reasoning": {"type": "string"},
            "final_response": {"type": ["string", "null"]},
            "revision_swos": {
                "type": ["array", "null"],
                "items": {"type": "string"},
            },
            "artifacts": artifact_schema,
            "blocked_reason": {"type": ["string", "null"]},
        },
        "required": ["action", "reasoning", "final_response", "revision_swos", "artifacts", "blocked_reason"],
    }


def run_worker() -> None:
    agent_id = get_env_or_die("AGENT_ID")
    db_path = get_env_or_die("AGENT_DATABASE")
    role = os.getenv("AGENT_ROLE", "Agent")
    persona_prompt = os.getenv("AGENT_PERSONA_PROMPT", f"Role: {role}")
    raison = os.getenv("AGENT_RAISON", "Assist the user")
    subordinates_json = os.getenv("AGENT_SUBORDINATES", "[]")
    run_id = os.getenv("AGENT_RUN_ID", str(uuid.uuid4()))
    token = os.getenv("SAIRGENT_SIDECHANNEL_TOKEN", "")
    current_swo_id_raw = os.getenv("AGENT_SWO_ID", "")
    current_swo_id = int(current_swo_id_raw) if current_swo_id_raw.isdigit() else None
    heartbeat_emitter = HeartbeatEmitter(token, run_id)

    _ = sys.stdin.read()

    if len(sys.argv) < 3:
        print(json.dumps({"error": "Usage: worker.py <mode> <payload>"}))
        sys.exit(1)

    mode = sys.argv[1]
    payload = sys.argv[2]

    os.makedirs(os.path.dirname(db_path), exist_ok=True)
    memory = AgentMemory(db_path)
    try:
        heartbeat_emitter.start()
        if mode == "execute_triage":
            memory_mode = "triage"
        elif mode == "execute_synthesis":
            memory_mode = "review"
        elif mode == "chat_mode" or mode == "sairgent_chat":
            memory_mode = "chat"
        elif mode == "execute_ideation":
            memory_mode = "manual_review"
        else:
            memory_mode = "legacy"
        memory.append_interaction("user", payload, current_swo_id, memory_mode, run_id, "task")
        recent_decision_log = (
            memory.format_decision_log_context(limit=5)
            if mode == "execute_ideation"
            else "No prior decision log entries recorded."
        )
        result = run_codex(
            build_prompt(
                mode,
                role,
                persona_prompt,
                raison,
                payload,
                subordinates_json,
                recent_decision_log,
            ),
            schema_for_mode(mode),
        )
        auto_artifact = _auto_artifact_for_mode(mode, result)
        if auto_artifact:
            result.setdefault("artifacts", []).append(auto_artifact)
        emit_artifacts(result.get("artifacts", []), current_swo_id)
        side_effects = build_side_effects(
            managed_work_count=len(result.get("managed_work_requests", [])),
            artifact_count=len(result.get("artifacts", [])),
        )

        if mode == "execute_triage":
            output_payload = build_protocol_response(
                mode=mode,
                agent_id=agent_id,
                triage={
                    "action": result["action"],
                    "reasoning": result["reasoning"],
                    "direct_answer": result["direct_answer"],
                    "delegation_swos": result["delegation_swos"],
                    "exception_code": result["exception_code"],
                    "exception_reason": result["exception_reason"],
                    "user_message": result["user_message"],
                },
                blocked_reason=result.get("blocked_reason"),
                artifacts=result.get("artifacts", []),
                side_effects=side_effects,
            )
        elif mode == "execute_synthesis":
            output_payload = build_protocol_response(
                mode=mode,
                agent_id=agent_id,
                synthesis={
                    "action": result["action"],
                    "reasoning": result["reasoning"],
                    "final_response": result["final_response"],
                    "revision_swos": result["revision_swos"],
                },
                blocked_reason=result.get("blocked_reason"),
                artifacts=result.get("artifacts", []),
                side_effects=side_effects,
            )
        elif mode == "chat_mode":
            output_payload = build_protocol_response(
                mode=mode,
                agent_id=agent_id,
                reply=result["reply"],
                blocked_reason=result.get("blocked_reason"),
                artifacts=result.get("artifacts", []),
                side_effects=side_effects,
            )
            output_payload["managed_work_requests"] = result["managed_work_requests"]
        elif mode == "sairgent_chat":
            # Codex returns synthesis format; extract final_response as the reply.
            # On REJECT_AND_REVISE, final_response is null — compose from reasoning + blocked_reason.
            raw = result.get("final_response") or result.get("direct_answer") or result.get("reply") or ""
            if not raw:
                parts = []
                reasoning = result.get("reasoning") or ""
                blocked = result.get("blocked_reason") or ""
                revision_swos = result.get("revision_swos") or []
                if reasoning:
                    parts.append(reasoning)
                if blocked and blocked not in reasoning:
                    parts.append(f"**Blocked:** {blocked}")
                if revision_swos:
                    parts.append("**To proceed:**")
                    parts.extend(f"- {s}" for s in revision_swos)
                raw = "\n\n".join(parts)
            output_payload = build_protocol_response(
                mode=mode,
                agent_id=agent_id,
                reply=raw,
                blocked_reason=result.get("blocked_reason"),
                artifacts=result.get("artifacts", []),
                side_effects=side_effects,
            )
        elif mode == "format_swo":
            output_payload = build_protocol_response(
                mode=mode,
                agent_id=agent_id,
                formatted_swo=result["formatted_swo"],
                blocked_reason=result.get("blocked_reason"),
                side_effects=side_effects,
            )
        elif mode == "execute_ideation":
            record_ideation_decision_log(memory, result, current_swo_id, run_id)
            output_payload = build_protocol_response(
                mode=mode,
                agent_id=agent_id,
                ideation_summary=result["ideation_summary"],
                blocked_reason=result.get("blocked_reason"),
                artifacts=result.get("artifacts", []),
                side_effects=side_effects,
            )
        else:
            output_payload = build_protocol_response(
                mode=mode,
                agent_id=agent_id,
                error=f"Unsupported codex mode: {mode}",
                side_effects=side_effects,
            )

        memory.append_interaction(
            "assistant",
            json.dumps(output_payload),
            current_swo_id,
            memory_mode,
            run_id,
            "typed_result",
        )
        print(json.dumps(output_payload))
    except Exception as exc:
        print(json.dumps(build_protocol_response(
            mode=locals().get("mode", "unknown"),
            agent_id=locals().get("agent_id", "unknown"),
            error=f"Execution failed: {exc}",
            side_effects=build_side_effects(),
        )))
        sys.exit(1)
    finally:
        if heartbeat_emitter.is_alive():
            heartbeat_emitter.stop()
            heartbeat_emitter.join(timeout=1.0)
        memory.close()


if __name__ == "__main__":
    run_worker()
