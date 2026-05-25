# Architecture

This is a tour of the repo's shape, written for someone who has just landed here from the Substack post and is deciding whether to keep reading. It is not a tutorial and not a sales pitch — it is the orientation a new engineer would get on day one.

## Mental model

Sairgent is a **Rust kernel** that holds sole authority over delegation, authorization, persistence, and audit; a set of **Python harnesses** that run as subprocess workers with no ambient privilege; and a **Tauri + React desktop** that is the operator UI. Everything between the desktop and the kernel goes over a single **Runtime Event Bus** contract — bootstrap, subscribe, replay — and nothing else.

The unit of work is an **agent org**: a roster of named agents (Manager / LeadIC / Specialist) defined as markdown skill files under `.agents/skills/`. Work is durable as **Subordinate Work Orders** (SWOs), each a row in SQLite with a state machine, lineage, and an audit trail. Managers delegate, specialists execute, the kernel arbitrates.

```
   .agents/skills/*.md         (the "who")
            │
            ▼
   sairgent_kernel  ──spawn──►  run_worker.sh  ──►  Python harness
       (Rust, SQLite)                                (PydanticAI or Codex CLI)
            │
            │ runtime_bootstrap / runtime-signal / runtime_replay
            ▼
   apps/workspace  (Tauri + React desktop)
```

## Top-level layout

```
sairgent_kernel/            Rust crate. Sole authority for delegation, auth, audit, persistence.
apps/workspace/             Tauri + React desktop shell. The only live UI.
sairgent_harness/           PydanticAI Python worker (chat_mode, execute_triage, format_swo).
sairgent_codex_harness/     Codex CLI Python worker (default execution backend).
packages/chat-core/         Shared TypeScript contracts (RuntimeBootstrap, RuntimeSignal, RuntimeCommand, domain types).
packages/chat-ui/           Shared ChatPanel React component.
.agents/skills/             Markdown agent definitions with YAML frontmatter — the actual "who" of the org.
.agents/workflows/          Workflow definitions (e.g. `task_completion.md`).
.agents/cadences/           Daily rhythm scripts: `dawn.md`, `heartbeat.md`, `dusk.md`.
00_Context/Seeds/           Runtime seed JSON. Provisions the initial agent roster + org config.
ops/runtime_event_bus_v1.md       Canonical engineering rule for kernel ↔ client comms.
ops/manager_execution_contract.md Manager-side delegation contract.
ops/plans/mvp_getting_started.md  Small architectural plan doc.
run_worker.sh               Dispatch shim. Picks harness based on mode or SAIRGENT_WORKER_BACKEND.
tools/                      `sqlite-snapshot.sh` + `sqlite-inspector.html` for live DB inspection.
```

Older surfaces (`apps/desktop/` and similar) may still appear in history; `apps/workspace/` is the one that matters.

## Key kernel modules

Inside [`sairgent_kernel/src/`](sairgent_kernel/src/):

- **`kernel.rs`** — entry point. Wires vault, registry, router, and orchestrator into one process.
- **`registry.rs`** — SQLite persistence for *all* domain state: agents, SWOs, projects, artifacts, approvals, inbox items, skills, tools, audit events. If a fact survives a restart, it lives here.
- **`orchestrator.rs`** — spawns worker subprocesses through `run_worker.sh`. Owns heartbeats, cron, stall detection, and SWO lifecycle transitions.
- **`audit.rs`** — tamper-evident hash-chained audit records, each annotated with a `TaintLabel` classification.
- **`vault.rs`** — AES-256-GCM secret storage. LLM keys and the sidechannel token live here (backed by the OS keychain where available).
- **`protocol.rs`** — normalizes output from both Python harnesses onto a single JSON contract so the orchestrator does not care which backend produced a result.
- **`seed.rs`** — loads a `RuntimeSeedSpec` from `00_Context/Seeds/` to provision the initial org.
- **`router.rs`**, **`manifest.rs`**, **`skills.rs`**, **`tools.rs`**, **`workflow.rs`** — routing, skill manifests, tool registry, workflow compilation.

## The agent org model

Every agent carries an `AgentOrgClass`: **Manager**, **LeadIc**, or **Specialist**. The kernel enforces the delegation policy:

- **Managers must delegate before self-executing.** A small set of self-execution exceptions exists for narrowly scoped meta-work.
- **Lineage loops are blocked.** An agent cannot end up in its own delegation chain.
- **Manager completion gates** on child SWO terminal status: a manager SWO cannot close until its children are accepted or terminally failed.

This is what makes a "manager" structurally different from "an agent that calls other agents" — the kernel refuses the shortcut.

## SWO lifecycle

```
PENDING ──► IN_PROGRESS ──► COMPLETED
                         ├─► FAILED
                         └─► CANCELLED
```

- **Retry-in-place** reopens the ancestor SWO chain rather than forking a new one.
- **Manual close** requires `FAILED` or `CANCELLED` — a stuck `IN_PROGRESS` cannot be silently buried.
- **Approval decisions** (`approve` / `reject` / `revise`) flow through `approval.decide` runtime commands, not direct DB writes.

## The runtime event bus

This was the engineering rule that mattered most, and it is worth reading the spec at [`ops/runtime_event_bus_v1.md`](ops/runtime_event_bus_v1.md) before anything else.

Desktop ↔ kernel communication follows exactly one shape:

1. `runtime_bootstrap` — initial state on mount.
2. Subscribe to the `runtime-signal` Tauri event — live updates.
3. `runtime_replay` on reconnect — gap recovery.

No polling. No route-local refresh logic. No client-specific side channels. Every new piece of runtime state has to enter the desktop through a projection-safe bus signal, or the bus contract itself gets fixed first.

Runtime commands carry a `commandId`. The Tauri backend enforces idempotency through an LRU cache of 10,000 entries; duplicate command IDs are rejected. This is what makes the bus safe to replay.

## Worker dispatch

[`run_worker.sh`](run_worker.sh) is the single dispatch shim. Mode-based routing:

- `chat_mode` and `format_swo` → PydanticAI harness (`sairgent_harness/`).
- Everything else → Codex CLI harness (`sairgent_codex_harness/`).
- `SAIRGENT_WORKER_BACKEND` overrides the default.

Each worker is a plain subprocess with **no ambient privilege**. Authorization decisions stay in Rust; the Python side cannot bypass them.

Each agent has an isolated filesystem at `~/Sairgent_Agents/{name}/context/` and `~/Sairgent_Agents/{name}/artifacts/`. Artifact writes are append-only with auto-versioned `-vN` suffixes. Path traversal is blocked at both the Rust and Python layers — defence in depth is cheap here.

## Where to start reading

If you want to actually read this repo rather than skim it:

1. [`ops/runtime_event_bus_v1.md`](ops/runtime_event_bus_v1.md) — the contract the rest of the system is shaped around.
2. [`sairgent_kernel/src/registry.rs`](sairgent_kernel/src/registry.rs) — the data model. Every domain noun is here.
3. [`sairgent_kernel/src/orchestrator.rs`](sairgent_kernel/src/orchestrator.rs) — the runtime loop: spawning, heartbeats, SWO transitions.
4. [`apps/workspace/src-tauri/src/lib.rs`](apps/workspace/src-tauri/src/lib.rs) — the thinnest possible Tauri command surface over the kernel. Useful as a worked example of "what the desktop is allowed to ask for."

From there, the harnesses and the skill markdown explain themselves.
