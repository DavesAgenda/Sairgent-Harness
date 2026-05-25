"""Lock in the error-handling convention for tool modules (CHA-497).

Two exception types, two meanings:

  BlockedToolError  — terminal; agent run aborts with status=BLOCKED.
                      Used for capability denial, config missing, hiring
                      disabled, dedup cap hit.

  ModelRetry        — recoverable; PydanticAI routes the message back to
                      the LLM for another turn. Used for file-not-found,
                      invalid path, empty query, provider misconfigured.

These tests guard against regressions where a recoverable error sneaks
back into BlockedToolError and starts killing agent runs.
"""
import os
import pathlib

import pytest
from pydantic_ai import ModelRetry

from tools._common import BlockedToolError, resolve_safe_path, require_capability
from tools.file_ops import read_file, list_directory
from tools.web_search import web_search


# ── resolve_safe_path: recoverable vs terminal ──────────────────────────────

def test_resolve_safe_path_empty_is_recoverable(tmp_path, monkeypatch):
    monkeypatch.setenv("AGENT_ROOT", str(tmp_path))
    with pytest.raises(ModelRetry) as exc:
        resolve_safe_path("")
    assert "path is required" in str(exc.value).lower()


def test_resolve_safe_path_traversal_is_recoverable(tmp_path, monkeypatch):
    monkeypatch.setenv("AGENT_ROOT", str(tmp_path))
    with pytest.raises(ModelRetry) as exc:
        resolve_safe_path("../../etc/passwd")
    assert "outside the agent workspace" in str(exc.value).lower()


def test_resolve_safe_path_missing_root_is_terminal(monkeypatch):
    monkeypatch.delenv("AGENT_ROOT", raising=False)
    with pytest.raises(BlockedToolError) as exc:
        resolve_safe_path("anywhere")
    assert "agent_root" in str(exc.value).lower()


# ── require_capability: terminal contract preserved ─────────────────────────

def test_require_capability_denial_is_terminal(monkeypatch):
    monkeypatch.setenv("AGENT_MANIFEST_JSON", '{"capabilities": []}')
    with pytest.raises(BlockedToolError) as exc:
        require_capability("file_write")
    assert "not granted" in str(exc.value).lower()


# ── read_file: all recoverable errors now raise ModelRetry ─────────────────

def _grant_file_read(monkeypatch):
    monkeypatch.setenv("AGENT_MANIFEST_JSON", '{"capabilities": ["file_read"]}')


def test_read_file_missing_is_recoverable(tmp_path, monkeypatch):
    monkeypatch.setenv("AGENT_ROOT", str(tmp_path))
    _grant_file_read(monkeypatch)
    with pytest.raises(ModelRetry) as exc:
        read_file("no-such-file.txt")
    assert "not found" in str(exc.value).lower()
    assert "list_directory" in str(exc.value)


def test_read_file_on_directory_is_recoverable(tmp_path, monkeypatch):
    monkeypatch.setenv("AGENT_ROOT", str(tmp_path))
    _grant_file_read(monkeypatch)
    (tmp_path / "sub").mkdir()
    with pytest.raises(ModelRetry) as exc:
        read_file("sub")
    assert "directory" in str(exc.value).lower()


def test_read_file_success_returns_content(tmp_path, monkeypatch):
    monkeypatch.setenv("AGENT_ROOT", str(tmp_path))
    _grant_file_read(monkeypatch)
    (tmp_path / "hello.md").write_text("# hello", encoding="utf-8")
    assert read_file("hello.md") == "# hello"


# ── list_directory: all recoverable errors raise ModelRetry ────────────────

def test_list_directory_missing_is_recoverable(tmp_path, monkeypatch):
    monkeypatch.setenv("AGENT_ROOT", str(tmp_path))
    _grant_file_read(monkeypatch)
    with pytest.raises(ModelRetry) as exc:
        list_directory("no-such-dir")
    assert "not found" in str(exc.value).lower()


def test_list_directory_on_file_is_recoverable(tmp_path, monkeypatch):
    monkeypatch.setenv("AGENT_ROOT", str(tmp_path))
    _grant_file_read(monkeypatch)
    (tmp_path / "file.txt").write_text("hi", encoding="utf-8")
    with pytest.raises(ModelRetry) as exc:
        list_directory("file.txt")
    assert "not a directory" in str(exc.value).lower()


def test_list_directory_success_returns_sorted_list(tmp_path, monkeypatch):
    monkeypatch.setenv("AGENT_ROOT", str(tmp_path))
    _grant_file_read(monkeypatch)
    (tmp_path / "b.txt").write_text("", encoding="utf-8")
    (tmp_path / "a").mkdir()
    entries = list_directory(".")
    assert entries == [
        {"name": "a", "type": "dir"},
        {"name": "b.txt", "type": "file"},
    ]


# ── web_search: recoverable errors raise ModelRetry ────────────────────────

def test_web_search_empty_query_is_recoverable(monkeypatch):
    with pytest.raises(ModelRetry) as exc:
        web_search("   ")
    assert "empty" in str(exc.value).lower()


def test_web_search_missing_provider_is_recoverable(monkeypatch):
    monkeypatch.delenv("AGENT_SEARCH_PROVIDER_SLUG", raising=False)
    monkeypatch.delenv("AGENT_SEARCH_PROVIDER_STATUS", raising=False)
    with pytest.raises(ModelRetry) as exc:
        web_search("something specific")
    # Reason text comes from _search_blocked_reason — should mention
    # provider/credential so the LLM knows what's missing.
    msg = str(exc.value).lower()
    assert "provider" in msg or "credential" in msg
