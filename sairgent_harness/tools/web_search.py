"""Web search tool — Tavily and Exa providers.

Gated by allowed_tools key "web_search".
"""

import json
import os
import urllib.error
import urllib.parse
import urllib.request

from ._common import BlockedToolError, ModelRetry
from . import ToolSpec, register

# Module-level reference to the credential dict in main.py.
# Set by init_web_search() during worker startup.
_tool_api_keys_by_slug: dict = {}


def init_web_search(api_keys_by_slug: dict) -> None:
    """Inject the tool API key dict from main.py at worker startup."""
    global _tool_api_keys_by_slug
    _tool_api_keys_by_slug = api_keys_by_slug


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
    payload = json.dumps({
        "query": query, "search_depth": "advanced",
        "max_results": 5, "include_answer": False,
    }).encode("utf-8")
    request = urllib.request.Request(
        "https://api.tavily.com/search", data=payload,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {api_key}"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=20) as response:
        parsed = json.loads(response.read().decode("utf-8"))
    results = parsed.get("results", [])
    return [
        {"title": item.get("title", ""), "url": item.get("url", ""), "snippet": item.get("content", "")}
        for item in results[:5]
    ]


def _perform_exa_search(query: str, api_key: str) -> list[dict]:
    payload = json.dumps({
        "query": query, "numResults": 5, "useAutoprompt": True,
        "type": "keyword", "highlights": {"numSentences": 2, "highlightsPerUrl": 1},
    }).encode("utf-8")
    request = urllib.request.Request(
        "https://api.exa.ai/search", data=payload,
        headers={"Content-Type": "application/json", "x-api-key": api_key},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=20) as response:
        parsed = json.loads(response.read().decode("utf-8"))
    results = parsed.get("results", [])
    normalized = []
    for item in results[:5]:
        text = item.get("text", "")
        highlights = item.get("highlights") or []
        snippet = highlights[0] if highlights else text[:400]
        normalized.append({
            "title": item.get("title", ""), "url": item.get("url", ""), "snippet": snippet,
        })
    return normalized


def web_search(query: str) -> str:
    """Search the live web via the agent's configured provider.

    Use when you need current facts, recent events, competitor
    intel, or documentation not in your training data. Write
    a SPECIFIC query — generic terms waste a search slot.
      BAD:  "AI agents"
      GOOD: "OpenAI Agents SDK pricing vs Claude Computer Use
             2026"

    Args:
      query: non-empty search string.

    Returns: JSON string with shape:
      {"provider": str, "query": str,
       "results": [{"title": str, "url": str, "snippet": str}]}
    Up to 5 results. On a recoverable error (empty query,
    provider misconfigured, provider unreachable) the LLM is
    asked to retry — adjust the query or proceed without web
    search if the provider is unavailable.
    """
    cleaned = query.strip()
    if not cleaned:
        raise ModelRetry(
            "Web search query cannot be empty. "
            "Pass a specific, focused query string."
        )

    provider_slug = _search_provider_slug()
    provider_status = _search_provider_status()
    api_key = _tool_api_keys_by_slug.get(provider_slug, "")

    if provider_status != "configured" or not provider_slug or not api_key:
        raise ModelRetry(_search_blocked_reason())

    try:
        if provider_slug == "tavily":
            results = _perform_tavily_search(cleaned, api_key)
        elif provider_slug == "exa":
            results = _perform_exa_search(cleaned, api_key)
        else:
            raise ModelRetry(
                f"Assigned search provider '{provider_slug}' is not supported in "
                f"this runtime yet. Proceed without web search for this task."
            )
    except ModelRetry:
        raise
    except urllib.error.HTTPError as exc:
        raise ModelRetry(
            f"Web search provider '{provider_slug}' rejected the request "
            f"({exc.code}). The credential may be invalid — proceed without "
            f"web search, or ask the operator to check Settings."
        ) from exc
    except urllib.error.URLError as exc:
        raise ModelRetry(
            f"Web search provider '{provider_slug}' is unreachable right now. "
            f"Proceed without web search for this task."
        ) from exc
    except Exception as exc:
        raise ModelRetry(
            f"Web search through '{provider_slug}' failed unexpectedly: {exc}. "
            f"Try a simpler query or proceed without web search."
        ) from exc

    if not results:
        return json.dumps({"provider": provider_slug, "query": cleaned, "results": []}, indent=2)

    return json.dumps({"provider": provider_slug, "query": cleaned, "results": results}, indent=2)


# ── Registration ────────────────────────────────────────────────────────────

register(
    ToolSpec(fn=web_search, allowed_tool_key="web_search"),
)
