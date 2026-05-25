"""CHA-401 — verify run_command strips secrets from the child env.

Kryptonite H1 follow-up on CHA-396. The harness historically forwarded its
full process environment to every subprocess.run child, leaking the per-run
sidechannel token, the registry database path, the full agent manifest JSON,
and any LLM API keys an operator had set at the process level. This test
locks in the whitelist behavior so future edits cannot regress it silently.
"""
from unittest.mock import MagicMock, patch

import pytest


@pytest.fixture
def harness_env(monkeypatch, tmp_path):
    """Minimal env required for run_command to reach subprocess.run."""
    agent_root = tmp_path / "agent_root"
    agent_root.mkdir()
    (agent_root / "workspace").mkdir()

    monkeypatch.setenv("AGENT_ROOT", str(agent_root))
    monkeypatch.setenv(
        "AGENT_MANIFEST_JSON",
        '{"capabilities":["shell_exec","file_read","file_write"]}',
    )
    monkeypatch.setenv("SAIRGENT_SIDECHANNEL_TOKEN", "top-secret-token")
    monkeypatch.setenv("REGISTRY_DATABASE", "/tmp/fake-registry.sqlite")
    monkeypatch.setenv("OPENAI_API_KEY", "sk-should-not-leak")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "should-not-leak")
    monkeypatch.setenv("PATH", "/usr/local/bin:/usr/bin:/bin")
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USER", "test")
    monkeypatch.setenv("LANG", "en_US.UTF-8")
    monkeypatch.setenv("TZ", "UTC")
    monkeypatch.setenv("TMPDIR", str(tmp_path / "tmp"))
    return agent_root


def _fake_subprocess_run_capture(captured: dict):
    def _run(*args, **kwargs):
        captured["args"] = args
        captured["kwargs"] = kwargs
        mock = MagicMock()
        mock.returncode = 0
        mock.stdout = b""
        mock.stderr = b""
        return mock

    return _run


def test_run_command_strips_sairgent_sidechannel_token(harness_env):
    import main

    captured: dict = {}
    with patch.object(
        main.subprocess,
        "run",
        side_effect=_fake_subprocess_run_capture(captured),
    ):
        with patch.object(main, "_emit_audit_sidechannel"):
            main.run_command("true")

    child_env = captured["kwargs"].get("env")
    assert child_env is not None, "run_command must pass explicit env= to subprocess.run"
    assert "SAIRGENT_SIDECHANNEL_TOKEN" not in child_env
    assert "REGISTRY_DATABASE" not in child_env
    assert "AGENT_MANIFEST_JSON" not in child_env
    assert "OPENAI_API_KEY" not in child_env
    assert "ANTHROPIC_API_KEY" not in child_env


def test_run_command_forwards_whitelisted_vars(harness_env):
    import main

    captured: dict = {}
    with patch.object(
        main.subprocess,
        "run",
        side_effect=_fake_subprocess_run_capture(captured),
    ):
        with patch.object(main, "_emit_audit_sidechannel"):
            main.run_command("true")

    child_env = captured["kwargs"].get("env")
    assert "PATH" in child_env
    assert child_env["PATH"] == "/usr/local/bin:/usr/bin:/bin"
    assert child_env["HOME"] == harness_env.parent.as_posix() or child_env.get("HOME")
    assert child_env.get("LANG") == "en_US.UTF-8"
    assert child_env.get("TZ") == "UTC"


def test_git_helper_strips_sairgent_sidechannel_token(harness_env):
    import main

    captured: dict = {}
    with patch.object(
        main.subprocess,
        "run",
        side_effect=_fake_subprocess_run_capture(captured),
    ):
        main._git(["status"], cwd=harness_env / "workspace")

    child_env = captured["kwargs"].get("env")
    assert child_env is not None, "_git must pass explicit env= to subprocess.run"
    assert "SAIRGENT_SIDECHANNEL_TOKEN" not in child_env
    assert "REGISTRY_DATABASE" not in child_env
    assert "AGENT_MANIFEST_JSON" not in child_env


def test_allowed_child_env_is_frozenset():
    import main

    assert isinstance(main._ALLOWED_CHILD_ENV, frozenset)
    assert "SAIRGENT_SIDECHANNEL_TOKEN" not in main._ALLOWED_CHILD_ENV
    assert "AGENT_MANIFEST_JSON" not in main._ALLOWED_CHILD_ENV
    assert "REGISTRY_DATABASE" not in main._ALLOWED_CHILD_ENV
    assert "PYTHONPATH" not in main._ALLOWED_CHILD_ENV
    assert "LD_PRELOAD" not in main._ALLOWED_CHILD_ENV
    assert "LD_LIBRARY_PATH" not in main._ALLOWED_CHILD_ENV
    assert "PATH" in main._ALLOWED_CHILD_ENV
    assert "HOME" in main._ALLOWED_CHILD_ENV
    assert "AGENT_WORKSPACE" in main._ALLOWED_CHILD_ENV
    assert "TMPDIR" in main._ALLOWED_CHILD_ENV
    assert "TZ" in main._ALLOWED_CHILD_ENV
