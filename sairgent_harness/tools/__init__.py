"""Tool Registry — auto-discovers tool modules and resolves capabilities to concrete functions.

Usage from main.py:

    from tools import resolve_tools

    execution_tools, triage_tools = resolve_tools(
        capabilities={"file_read", "file_write", "shell_exec", "git_ops"},
        allowed_tools={"web_search", "write_artifact_file", "list_available_skills", ...},
        can_hire=True,
    )

Each tool module in this package registers its tools by defining a module-level
TOOLS list of ToolSpec objects. The registry discovers all modules at import time,
collects every ToolSpec, and resolve_tools() filters them by capability + allowed_tools.
"""

from __future__ import annotations

import importlib
import pkgutil
from dataclasses import dataclass, field
from typing import Callable, Optional

# Global registry populated by register() calls from tool modules.
_ALL_TOOLS: list[ToolSpec] = []


@dataclass
class ToolSpec:
    """Metadata wrapper for a single tool function.

    Attributes:
        fn:               The callable PydanticAI will use as a tool.
        name:             Unique tool name. Defaults to fn.__name__.
        capability:       Kernel capability slug required (e.g. "file_read", "shell_exec").
                          None means the tool is gated only by allowed_tools.
        allowed_tool_key: The string key used in AGENT_ALLOWED_TOOLS to enable this tool.
                          None means the tool is gated only by capability.
                          At least one of capability or allowed_tool_key must be set.
        exclude_modes:    Modes where this tool must NOT be offered (e.g. {"execute_triage"}).
        always:           If True, this tool is always included regardless of
                          capability/allowed_tools checks. Use sparingly.
    """
    fn: Callable
    name: str = ""
    capability: Optional[str] = None
    allowed_tool_key: Optional[str] = None
    exclude_modes: set[str] = field(default_factory=set)
    always: bool = False

    def __post_init__(self):
        if not self.name:
            self.name = self.fn.__name__


def register(*specs: ToolSpec) -> None:
    """Register one or more ToolSpecs into the global registry.

    Called at module level in each tool file.
    """
    _ALL_TOOLS.extend(specs)


def resolve_tools(
    capabilities: set[str],
    allowed_tools: set[str],
    can_hire: bool = False,
    mode: str = "",
) -> tuple[list[Callable], list[Callable]]:
    """Resolve capabilities + allowed_tools into concrete (execution_tools, triage_tools).

    Returns two lists:
      - execution_tools: all tools the agent can use in execution modes
      - triage_tools: subset of execution_tools minus triage-excluded tools

    The caller passes both lists to the appropriate PydanticAI Agent() constructor.
    """
    execution = []
    triage = []

    for spec in _ALL_TOOLS:
        # Gate: hiring tools need can_hire
        if spec.name == "hire_subordinate_internal" and not can_hire:
            continue

        # Gate: capability check (dark-factory tools)
        if spec.capability is not None:
            if spec.capability not in capabilities:
                continue

        # Gate: allowed_tools check (named tools)
        if spec.allowed_tool_key is not None:
            if not spec.always and spec.allowed_tool_key not in allowed_tools:
                continue

        # Gate: always-on tools bypass the above checks
        # (already handled — if spec.always is True, the capability/allowed_tools
        #  checks are skipped by the `not spec.always` guard above)

        execution.append(spec.fn)

        # Triage gets everything except explicitly excluded tools
        if "execute_triage" not in spec.exclude_modes:
            triage.append(spec.fn)

    return execution, triage


def all_specs() -> list[ToolSpec]:
    """Return a copy of all registered tool specs. Useful for debugging/introspection."""
    return list(_ALL_TOOLS)


# ── Auto-discover all tool modules in this package ──────────────────────────
# Any .py file in tools/ (except __init__ and _common) that calls register()
# at module level gets its tools added to _ALL_TOOLS automatically.

def _autodiscover():
    package_path = __path__
    for importer, modname, ispkg in pkgutil.iter_modules(package_path):
        if modname.startswith("_"):
            continue
        importlib.import_module(f".{modname}", __package__)


_autodiscover()
