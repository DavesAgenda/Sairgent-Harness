# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Sairgent is a multi-agent AI orchestration platform. It consists of:
- A **Rust kernel** (`sairgent_kernel/`) that is the sole authority for delegation, manifests, authorization, workflow compilation, and audit state.
- A **Tauri + React desktop app** (`apps/desktop/`) that is the primary operator UI.
- **Shared TypeScript contracts** (`packages/chat-core/`) consumed by the desktop.
- **Python agent harnesses** (`sairgent_harness/`, `sairgent_codex_harness/`) executed as subprocesses by the kernel orchestrator.
- **Agent skill definitions** (`.agents/skills/`) as markdown files with YAML frontmatter.

The desktop communicates with the kernel exclusively via the **Runtime Event Bus** (`ops/runtime_event_bus_v1.md`): bootstrap (`runtime_bootstrap`) → subscribe (`runtime-signal`) → replay (`runtime_replay`). This is a hard engineering rule: do not add polling, route-local refresh logic, or client-specific side channels.

## Commands

### Desktop App (Tauri + React)

```bash
# From apps/desktop/
bun run dev          # Vite dev server (browser mode, uses mock API)
bun run build        # TypeScript check + Vite build
bun run lint         # ESLint
bun run tauri dev    # Launch full Tauri app (requires Rust build)
bun run tauri build  # Build production Tauri bundle
```

### Rust Kernel (sairgent_kernel/)

```bash
# From sairgent_kernel/
cargo build
cargo check          # Fast type/borrow check without full build
cargo test           # Run all kernel tests
cargo test <name>    # Run a single test by name
cargo run            # Run the kernel CLI loop (requires storage/ dir)
```

### Tauri Backend (apps/desktop/src-tauri/)

```bash
# From apps/desktop/src-tauri/
cargo check          # Check Tauri backend
cargo build
```

### TypeScript Packages

```bash
# From repo root (PNPM/Bun workspace)
bun install          # Install all workspace deps
# From packages/chat-core/
tsc --noEmit         # Type-check without emitting
```

### Python Harnesses

```bash
# From sairgent_harness/
python3 -m venv venv && source venv/bin/activate
pip install -r requirements.txt
# From repo root:
./run_worker.sh <mode>   # Dispatched by kernel orchestrator
```

### Tools

```bash
tools/sqlite-snapshot.sh   # Snapshot the live SQLite DB
tools/sqlite-inspector.html  # Browser-based DB inspector
```

## Architecture

### Monorepo Layout

```
sairgent_kernel/          Rust crate: kernel, registry (SQLite), orchestrator, vault, audit
apps/desktop/
  src/
    desktop/              Runtime adapter, types, selectors, format helpers, useDesktopApp hook
    ui/                   Route surfaces (Overview, Inbox, Projects, WorkOrders, Agents, Artifacts, Settings)
    App.tsx               Shell with nav + route rendering
    platform.ts           Tauri vs. browser detection
    mockDesktopApi.ts     Browser dev mock (no Tauri)
  src-tauri/src/
    lib.rs                All Tauri commands + AppState; wraps sairgent_kernel
packages/
  chat-core/src/          Shared TypeScript types (RuntimeBootstrap, RuntimeSignal, RuntimeCommand, all domain types)
  chat-ui/src/            Shared ChatPanel component
sairgent_harness/         PydanticAI worker harness (chat_mode, execute_triage, etc.)
sairgent_codex_harness/   Codex CLI worker harness
.agents/skills/           Markdown skill definitions (perry, lex, lois, felicity, etc.)
.agents/workflows/        Workflow definitions (e.g. task_completion.md)
00_Context/Memory/        Living project memory: state_of_play.md, decisions_log.md, project_tasks.md
00_Context/Seeds/         Runtime seed JSON (agent roster + org config)
ops/                      Engineering specs and sprint logs
storage/                  Live SQLite DB (kernel_registry.sqlite) + agent file trees
```

### Key Kernel Modules

- **`registry.rs`** — SQLite persistence layer. All domain state: agents, SWOs, projects, artifacts, approvals, inbox items, skills, tools, audit events.
- **`orchestrator.rs`** — Spawns worker subprocesses via `run_worker.sh`. Manages heartbeats, cron, stall detection, and SWO lifecycle.
- **`kernel.rs`** — Entry point that wires vault + registry + router + orchestrator.
- **`vault.rs`** — AES-256-GCM encrypted secret storage.
- **`audit.rs`** — Tamper-evident hash-chained audit event records with taint labels.
- **`protocol.rs`** — Normalizes worker output from both Python harnesses onto the same JSON contract.
- **`seed.rs`** — Loads `RuntimeSeedSpec` JSON to provision the initial agent org from `00_Context/Seeds/`.

### Desktop State Management

The entire desktop state is a single large hook: `useDesktopApp` (`apps/desktop/src/desktop/useDesktopApp.ts`). It owns:
- Bootstrap on mount via `runtime_bootstrap` Tauri command
- Live signal subscription via `runtime-signal` Tauri event
- Replay on reconnect via `runtime_replay`
- All `UiCommandIntent` dispatch through `desktopAdapter` (`adapter.ts`)

Domain slices (`RuntimeSlice`, `AgentSlice`, `QueueSlice`, `ProjectSlice`, `InboxSlice`, etc.) are defined in `types.ts`. Format helpers live in `format.ts`; derived/computed selectors in `selectors.ts`.

### Agent Org Model

Agents have an `AgentOrgClass`: `Manager`, `LeadIc`, or `Specialist`. The kernel enforces delegation policy:
- Managers must delegate before self-executing (unless a self-execution exception applies).
- Lineage loops are blocked.
- Manager completion gates require child results accepted or terminally failed.

### SWO (Subordinate Work Order) Lifecycle

`PENDING → IN_PROGRESS → COMPLETED | FAILED | CANCELLED`. Retry-in-place reopens ancestor SWO chains. Manual close requires `FAILED` or `CANCELLED` status. Approval decisions (`approve/reject/revise`) come through `approval.decide` runtime commands.

### Worker Dispatch

`run_worker.sh` selects the Python harness based on mode or `SAIRGENT_WORKER_BACKEND` env var. `chat_mode`/`format_swo` → PydanticAI harness; default → Codex CLI harness.

### Idempotency

Runtime commands carry `commandId`. The Tauri backend enforces idempotency via an LRU cache (10,000 capacity). Duplicate command IDs are rejected with an error.

### Agent File System

Each agent has isolated file access at `~/Sairgent_Agents/{name}/context/` and `artifacts/`. Artifact writes are append-only with auto-versioned `-vN` suffixes. Path traversal is enforced at both Rust and Python layers.

## Memory / Documentation Workflow

After completing a sprint or major milestone, call the **clark** skill (`.agents/skills/clark/SKILL.md`) to sync:
- `00_Context/Memory/decisions_log.md` — Append new decisions.
- `00_Context/Memory/state_of_play.md` — Update current status.
- `00_Context/Memory/project_tasks.md` — Mark progress.
- Move sprint files to `ops/sprints/`.

Sprint specs live in `ops/sprints/`. The current active engineering rule is `ops/runtime_event_bus_v1.md`.

## Cross-Platform Development (Windows 11 / WSL)

Primary development is transitioning to a Windows 11 machine using WSL2. Key notes:

### Environment Setup (WSL2 / Ubuntu)

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Bun
curl -fsSL https://bun.sh/install | bash

# Python
sudo apt install python3 python3-venv python3-pip

# Tauri system deps (Ubuntu/Debian)
# NOTE: libayatana-appindicator3-dev is required for system tray (CHA-365)
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  libgtk-3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev
```

### WSL vs Native Windows Builds

- **Kernel dev, tests, harness work**: Run inside WSL. All `cargo check`, `cargo test`, `bun run dev` (browser mock) work natively in WSL.
- **Tauri desktop builds**: WSL2 on Win11 supports GUI apps, but Tauri rendering depends on a working display server. If windows don't render, check `$DISPLAY` and WebView2 availability.
- **Windows .exe/.msi distribution**: Tauri produces native `.exe` and `.msi` installers, but these must be built from the **Windows side** (not WSL). Install Rust, Bun/Node, and Visual Studio Build Tools (C++ workload) on Windows natively. Run `bun run tauri build` from PowerShell/cmd.
- **Cross-compilation from WSL to Windows is not supported** for Tauri apps due to native GUI dependencies.

### Keyring / Vault Differences

- **macOS**: Uses OS Keychain via `com.sairgent.deck.v2` service.
- **Windows (native Tauri build)**: Tauri's keyring crate uses Windows Credential Manager — same API, no code changes needed.
- **WSL/Linux**: No native keyring. Requires `libsecret` + `gnome-keyring` (or `secret-tool`) for the vault to work. Alternatively, fall back to environment variable injection for LLM API keys during WSL-only development.

### Performance

- **Keep the repo inside the WSL filesystem** (`~/Developer/Sairgent`), NOT on `/mnt/c/`. The Windows mount is dramatically slower for git, cargo, and node operations.
- For native Windows Tauri builds, clone a separate copy on the Windows filesystem (e.g., `C:\Dev\sairgent`).

### Agent File System

- On WSL, `~/Sairgent_Agents/{name}/` maps to the WSL home directory.
- On native Windows builds, it maps to `C:\Users\{user}\Sairgent_Agents\{name}\`.
- Path separator handling is already abstracted through Rust's `std::path` — no changes needed.

## Security Notes

- LLM API keys and the sidechannel token are stored in the OS Keychain (macOS). The Tauri keyring service is `com.sairgent.deck.v2`.
- The sidechannel token is scoped per-run. Monotonic heartbeat sequence enforcement prevents stale replay.
- Rust is the sole authorization authority. Python harnesses cannot bypass Rust-level authz.
- Audit events use tamper-evident hash chaining with `TaintLabel` classification.
