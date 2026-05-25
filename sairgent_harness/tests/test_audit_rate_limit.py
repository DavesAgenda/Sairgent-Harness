"""CHA-402 — audit sidechannel rate limit + size cap.

Verifies:
- Events under budget pass through to _emit_sidechannel normally.
- Events over the budget are dropped with a single warning.
- Oversized events (> 64 KB serialized) are dropped with a warning.
- Normal _emit_sidechannel (non-audit) calls are NOT subject to the budget.
- Concurrent _emit_audit_sidechannel from multiple threads respects the cap
  (CHA-402 B1 — counter lock prevents TOCTOU races between budget check
  and increment).
- git_commit truncates files_changed at the forensic-triage cap and sets
  the files_changed_total/files_changed_truncated sentinels (CHA-402 B2).
"""
import threading
from unittest.mock import MagicMock, patch

import pytest


@pytest.fixture(autouse=True)
def reset_audit_counters(monkeypatch):
    """Reset CHA-402 module-level state between every test."""
    import main

    monkeypatch.setattr(main, "_audit_event_count", 0)
    monkeypatch.setattr(main, "_audit_budget_exceeded_warned", False)
    yield


@pytest.fixture
def sidechannel_env(monkeypatch):
    """Provide the bare minimum env for _emit_audit_sidechannel to run."""
    monkeypatch.setenv("SAIRGENT_SIDECHANNEL_TOKEN", "test-token")


# ---------------------------------------------------------------------------
# Test 1 — under budget: all events pass through
# ---------------------------------------------------------------------------

def test_audit_events_under_budget_emit_normally(sidechannel_env):
    import main

    mock_emit = MagicMock(return_value=True)
    with patch.object(main, "_emit_sidechannel", mock_emit):
        for i in range(5):
            main._emit_audit_sidechannel("file_read", {"path": f"/tmp/file{i}"})

    assert mock_emit.call_count == 5


# ---------------------------------------------------------------------------
# Test 2 — over budget: only 1000 events pass through, one warning emitted
# ---------------------------------------------------------------------------

def test_audit_events_over_budget_are_dropped(sidechannel_env):
    import main

    budget = main._MAX_AUDIT_EVENTS_PER_RUN
    mock_emit = MagicMock(return_value=True)
    warnings = []

    def capture_stderr(line):
        warnings.append(line)
        return True

    with patch.object(main, "_emit_sidechannel", mock_emit), \
         patch.object(main, "_emit_stderr_line", side_effect=capture_stderr):
        for i in range(budget + 5):
            main._emit_audit_sidechannel("file_read", {"path": f"/tmp/file{i}"})

    assert mock_emit.call_count == budget, (
        f"Expected {budget} emissions, got {mock_emit.call_count}"
    )

    budget_warnings = [w for w in warnings if "audit sidechannel budget exceeded" in w]
    assert len(budget_warnings) == 1, (
        f"Expected exactly one budget-exceeded warning, got {len(budget_warnings)}"
    )


# ---------------------------------------------------------------------------
# Test 3 — oversized payload: event dropped with one warning
# ---------------------------------------------------------------------------

def test_oversized_audit_event_dropped(sidechannel_env):
    import main

    mock_emit = MagicMock(return_value=True)
    warnings = []

    def capture_stderr(line):
        warnings.append(line)
        return True

    # Build a payload that serializes to > 64 KB after token injection
    big_payload = {"data": "x" * (100 * 1024)}  # 100 KB string

    with patch.object(main, "_emit_sidechannel", mock_emit), \
         patch.object(main, "_emit_stderr_line", side_effect=capture_stderr):
        main._emit_audit_sidechannel("file_read", big_payload)

    assert mock_emit.call_count == 0, "Oversized event must NOT reach _emit_sidechannel"

    size_warnings = [w for w in warnings if "audit sidechannel event oversized" in w]
    assert len(size_warnings) == 1, (
        f"Expected exactly one oversized warning, got {len(size_warnings)}"
    )


# ---------------------------------------------------------------------------
# Test 4 — _emit_sidechannel itself is NOT rate-limited
# ---------------------------------------------------------------------------

def test_normal_sidechannel_events_not_rate_limited(monkeypatch):
    """Calling _emit_sidechannel directly (bypassing _emit_audit_sidechannel)
    must never be subject to the audit budget.  This guards against accidental
    mis-wiring where the budget check is placed too high up the call chain.
    """
    import main

    captured_lines = []

    def capture_stderr(line, lock_timeout=None):
        captured_lines.append(line)
        return True

    with patch.object(main, "_emit_stderr_line", side_effect=capture_stderr):
        for i in range(2000):
            main._emit_sidechannel({"event": "normal", "seq": i})

    # All 2000 non-audit sidechannel events must have gone through.
    assert len(captured_lines) == 2000, (
        f"Expected 2000 normal sidechannel lines, got {len(captured_lines)}"
    )

    # Budget counter must still be zero — _emit_sidechannel doesn't touch it.
    assert main._audit_event_count == 0


# ---------------------------------------------------------------------------
# Test 5 — concurrent audit emissions respect the cap (CHA-402 B1)
# ---------------------------------------------------------------------------

def test_concurrent_audit_emissions_respect_cap(sidechannel_env):
    """10 worker threads each emit 500 audit events (5000 total, well over the
    2000-event budget).  With the counter lock from B1, the total number of
    events that reach _emit_sidechannel must equal _MAX_AUDIT_EVENTS_PER_RUN
    exactly — not one over, not one under.  Without the lock, two threads can
    observe the counter at N-1, both pass the check, and overshoot the cap.
    """
    import main

    budget = main._MAX_AUDIT_EVENTS_PER_RUN
    mock_emit = MagicMock(return_value=True)

    thread_count = 10
    per_thread = 500  # 5000 > 2000 budget

    def capture_stderr(line):
        return True

    with patch.object(main, "_emit_sidechannel", mock_emit), \
         patch.object(main, "_emit_stderr_line", side_effect=capture_stderr):
        def worker(worker_id):
            for i in range(per_thread):
                main._emit_audit_sidechannel(
                    "file_read",
                    {"path": f"/tmp/w{worker_id}/f{i}"},
                )

        threads = [
            threading.Thread(target=worker, args=(wid,))
            for wid in range(thread_count)
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

    assert mock_emit.call_count == budget, (
        f"Expected exactly {budget} emissions under concurrency, "
        f"got {mock_emit.call_count}"
    )
    assert main._audit_event_count == budget


# ---------------------------------------------------------------------------
# Test 6 — git_commit truncates files_changed at the forensic-triage cap
# ---------------------------------------------------------------------------

def test_git_commit_files_changed_truncated_past_cap(sidechannel_env, monkeypatch, tmp_path):
    """A commit of 5000 files would otherwise blow the 64KB per-event size cap.
    Verify files_changed is truncated at 100 entries with the sentinel fields
    set, and that the resulting payload stays well under the cap.
    """
    import main

    agent_root = tmp_path / "agent_root"
    (agent_root / "workspace").mkdir(parents=True)
    monkeypatch.setenv("AGENT_ROOT", str(agent_root))
    monkeypatch.setenv("AGENT_MANIFEST_JSON", '{"capabilities":["git_ops"]}')

    captured_payloads = []

    def capture_emit(payload):
        captured_payloads.append(dict(payload))
        return True

    big_file_list = [f"src/file_{i:05d}.rs" for i in range(5000)]

    fake_add_result = {"ok": True, "exit_code": 0, "stdout": "", "stderr": ""}
    fake_commit_result = {
        "ok": True,
        "exit_code": 0,
        "stdout": "[main abc1234] 5000-file mass rewrite",
        "stderr": "",
    }

    def fake_git(args, cwd):
        if args[:1] == ["add"]:
            return fake_add_result
        if args[:1] == ["commit"]:
            return fake_commit_result
        return {"ok": True, "exit_code": 0, "stdout": "", "stderr": ""}

    with patch.object(main, "_emit_sidechannel", side_effect=capture_emit), \
         patch.object(main, "_git", side_effect=fake_git):
        main.git_commit("mass rewrite", big_file_list)

    assert len(captured_payloads) == 1, "Expected exactly one git_operation event"
    event = captured_payloads[0]
    assert event["__sairgent_sidechannel"] == "git_operation"
    assert event["operation"] == "commit"
    assert event["files_changed_total"] == 5000
    assert event["files_changed_truncated"] is True
    assert len(event["files_changed"]) == 100, (
        f"Expected 100 files in truncated manifest, got {len(event['files_changed'])}"
    )

    import json
    serialized = json.dumps(event)
    byte_len = len(serialized.encode("utf-8"))
    assert byte_len <= main._MAX_AUDIT_EVENT_BYTES, (
        f"Truncated payload is still {byte_len} bytes, over the {main._MAX_AUDIT_EVENT_BYTES} cap"
    )


def test_git_commit_small_list_not_marked_truncated(sidechannel_env, monkeypatch, tmp_path):
    """A normal commit with 5 files should NOT set the truncation sentinel."""
    import main

    agent_root = tmp_path / "agent_root"
    (agent_root / "workspace").mkdir(parents=True)
    monkeypatch.setenv("AGENT_ROOT", str(agent_root))
    monkeypatch.setenv("AGENT_MANIFEST_JSON", '{"capabilities":["git_ops"]}')

    captured_payloads = []

    def capture_emit(payload):
        captured_payloads.append(dict(payload))
        return True

    fake_git_result = {
        "ok": True,
        "exit_code": 0,
        "stdout": "[main abc1234] small commit",
        "stderr": "",
    }
    with patch.object(main, "_emit_sidechannel", side_effect=capture_emit), \
         patch.object(main, "_git", return_value=fake_git_result):
        main.git_commit("small commit", ["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"])

    assert len(captured_payloads) == 1
    event = captured_payloads[0]
    assert event["files_changed_total"] == 5
    assert event["files_changed_truncated"] is False
    assert len(event["files_changed"]) == 5
