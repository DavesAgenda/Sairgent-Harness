"""CHA-425 sub-fix tests — artifact dedup + UsageLimits ceiling.

Covers the two mechanical guardrails added to prevent the revision-spiral
failure mode from the calculator-app test (Perry wrote 17 near-duplicate
artifacts in one SWO turn before hitting the PydanticAI request_limit).

- Sub-fix 2: `_agent_usage_limits` honors `SAIRGENT_AGENT_REQUEST_LIMIT`
  env override and clamps invalid values.
- Sub-fix 3: `_check_artifact_dedup` normalizes noisy suffixes and rejects
  after N similar-prefix writes per SWO turn.
"""
from unittest.mock import MagicMock, patch

import pytest


@pytest.fixture(autouse=True)
def reset_dedup_state():
    import main
    main._reset_artifact_dedup()
    yield
    main._reset_artifact_dedup()


# ---------------------------------------------------------------------------
# Sub-fix 2 — UsageLimits helper
# ---------------------------------------------------------------------------

def test_usage_limits_default_is_200(monkeypatch):
    import main
    monkeypatch.delenv("SAIRGENT_AGENT_REQUEST_LIMIT", raising=False)
    limits = main._agent_usage_limits()
    assert limits.request_limit == 200


def test_usage_limits_env_override(monkeypatch):
    import main
    monkeypatch.setenv("SAIRGENT_AGENT_REQUEST_LIMIT", "500")
    limits = main._agent_usage_limits()
    assert limits.request_limit == 500


def test_usage_limits_invalid_falls_back_to_default(monkeypatch):
    import main
    monkeypatch.setenv("SAIRGENT_AGENT_REQUEST_LIMIT", "not-a-number")
    limits = main._agent_usage_limits()
    assert limits.request_limit == 200


def test_usage_limits_zero_or_negative_falls_back(monkeypatch):
    import main
    monkeypatch.setenv("SAIRGENT_AGENT_REQUEST_LIMIT", "0")
    assert main._agent_usage_limits().request_limit == 200
    monkeypatch.setenv("SAIRGENT_AGENT_REQUEST_LIMIT", "-5")
    assert main._agent_usage_limits().request_limit == 200


# ---------------------------------------------------------------------------
# Sub-fix 3 — artifact prefix normalization
# ---------------------------------------------------------------------------

def test_normalize_strips_extension_and_suffix_chains():
    import main
    n = main._normalize_artifact_prefix
    # All of these should collapse to the same base prefix
    base = n("calculator-app-revision-feedback-answers.md")
    assert base == n("calculator-app-revision-feedback-response.md")
    assert base == n("calculator-app-revision-feedback-response-final.md")
    assert base == n("calculator-app-revision-feedback-response-v2.md")
    assert base == n("calculator-app-revision-feedback-answered.md")


def test_normalize_distinct_prefixes_stay_distinct():
    import main
    n = main._normalize_artifact_prefix
    assert n("research-dossier.md") != n("marketing-plan.md")
    assert n("index.html") != n("script.js")


def test_normalize_handles_path_separators():
    import main
    n = main._normalize_artifact_prefix
    assert n("workspace/calculator/index.html") == n("index.html")
    assert n("/tmp/absolute/path/foo.md") == n("foo.md")


def test_normalize_handles_empty_and_edge_cases():
    import main
    n = main._normalize_artifact_prefix
    assert n("") == ""
    assert n(".md") == ""  # extension-only
    assert len(n("a" * 100)) == 30  # truncated to 30 chars


# ---------------------------------------------------------------------------
# Sub-fix 3 — dedup cap enforcement
# ---------------------------------------------------------------------------

def test_dedup_allows_first_cap_writes():
    import main
    # Default cap is 5 — first five should pass
    for i in range(5):
        err = main._check_artifact_dedup(f"calculator-feedback-answer-v{i}.md")
        assert err is None, f"write {i} unexpectedly rejected: {err}"


def test_dedup_rejects_after_cap():
    import main
    # Exhaust the cap
    for i in range(5):
        main._check_artifact_dedup(f"calculator-feedback-answer-v{i}.md")
    # Sixth must be rejected with a clear message
    err = main._check_artifact_dedup("calculator-feedback-answer-v5.md")
    assert err is not None
    assert "CHA-425" in err or "spam" in err.lower() or "rejected" in err.lower()


def test_dedup_rejects_suffix_variants_as_duplicates():
    """The 17-artifact spiral from the calculator test all had the prefix
    'calculator-...-feedback' with -response / -final / -v2 / -answered /
    -for-user tails. Exhaust the cap with the exact thrash from the log
    and assert the next write is rejected.
    """
    import main
    spiral_filenames = [
        "calculator-app-revision-feedback-answers.md",
        "calculator-app-revision-feedback-response.md",
        "calculator-app-revision-feedback-response-final.md",
        "calculator-app-revision-feedback-response-v2.md",
        "calculator-app-revision-feedback-answered.md",
    ]
    for name in spiral_filenames:
        err = main._check_artifact_dedup(name)
        # All 5 should slip under the cap since they share one prefix
        # but do not exceed the limit.
        assert err is None, f"write {name} unexpectedly rejected: {err}"

    next_spiral = "calculator-app-revision-feedback-where-hosted.md"
    err = main._check_artifact_dedup(next_spiral)
    assert err is not None, (
        f"write {next_spiral} should have been rejected after the 5-write cap"
    )


def test_dedup_distinct_prefixes_do_not_interfere():
    import main
    # Exhaust one prefix
    for i in range(5):
        main._check_artifact_dedup(f"calculator-feedback-v{i}.md")
    # A completely different prefix must still be accepted
    err = main._check_artifact_dedup("research-dossier.md")
    assert err is None


def test_dedup_warns_only_once_per_prefix(capsys):
    import main
    # Exhaust the cap
    for i in range(5):
        main._check_artifact_dedup(f"calculator-feedback-v{i}.md")

    # Two rejected writes should produce exactly ONE stderr warning line.
    # Use monkeypatch on _emit_stderr_line to count invocations.
    warnings = []
    with patch.object(main, "_emit_stderr_line", side_effect=lambda line: warnings.append(line) or True):
        main._check_artifact_dedup("calculator-feedback-v5.md")
        main._check_artifact_dedup("calculator-feedback-v6.md")

    spam_warnings = [w for w in warnings if "CHA-425" in w]
    assert len(spam_warnings) == 1, (
        f"expected exactly one CHA-425 warning, got {len(spam_warnings)}"
    )


def test_dedup_reset_clears_state():
    import main
    # Exhaust the cap
    for i in range(5):
        main._check_artifact_dedup(f"calculator-feedback-v{i}.md")
    assert main._check_artifact_dedup("calculator-feedback-v5.md") is not None

    # Reset — subsequent writes on the same prefix should again pass
    main._reset_artifact_dedup()
    assert main._check_artifact_dedup("calculator-feedback-v99.md") is None


# ---------------------------------------------------------------------------
# Sub-fix 3 — integration with write_artifact_file
# ---------------------------------------------------------------------------

def test_write_artifact_file_enforces_dedup(monkeypatch, tmp_path):
    import main
    artifacts_dir = tmp_path / "artifacts"
    artifacts_dir.mkdir()
    monkeypatch.setenv("AGENT_ARTIFACTS", str(artifacts_dir))
    monkeypatch.setenv("SAIRGENT_SIDECHANNEL_TOKEN", "test-token")
    monkeypatch.setenv("AGENT_SWO_ID", "85")

    # First 5 writes should succeed
    for i in range(5):
        result = main.write_artifact_file(f"calc-feedback-v{i}.md", "body")
        assert result.startswith("Successfully"), f"write {i} failed: {result}"

    # 6th should be rejected with the CHA-425 error
    result = main.write_artifact_file("calc-feedback-v5.md", "body")
    assert "rejected" in result.lower() or "CHA-425" in result


def test_write_artifact_file_different_prefix_still_works(monkeypatch, tmp_path):
    import main
    artifacts_dir = tmp_path / "artifacts"
    artifacts_dir.mkdir()
    monkeypatch.setenv("AGENT_ARTIFACTS", str(artifacts_dir))
    monkeypatch.setenv("SAIRGENT_SIDECHANNEL_TOKEN", "test-token")
    monkeypatch.setenv("AGENT_SWO_ID", "85")

    # Exhaust one prefix
    for i in range(5):
        main.write_artifact_file(f"calc-feedback-v{i}.md", "body")

    # Orthogonal artifact must still pass
    result = main.write_artifact_file("market-research.md", "body")
    assert result.startswith("Successfully")
