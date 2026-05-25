---
name: barry-allen
description: Run serialized Ralph loops end-to-end for Sairgent using queue files, worktree bootstrap, supervisor execution, and integration checkpoints.
---

# Barry Allen (Loop Runner)

Use this skill when the user asks to run serialized loops, Ralph loops, unattended queue execution, or requests fast chained delivery across worktrees.

## Scope

- Serialized loop execution only.
- Queue-driven sequencing with dependency-aware chaining.
- HITL handoff at `ready_for_hitl`, `needs_hitl`, `blocked`, or `failed_validation`.
- Reliability-first execution: deterministic validation, retry-on-transient failure, and clean worktree hygiene.

## Required Inputs

- Queue file path in `ops/` (default: `ops/loop-queue.yaml`).
- Loop command env (`LOOP_AGENT_CMD`) from `.env.loop` or `.env.loop.example`.

## Standard Runbook

1. Validate queue plan:
   - `bash scripts/verify-build-plan.sh <queue-file>`
   - Default quality gate is strict (`LOOP_VERIFY_ARTIFACT_QUALITY=true`) and fails on placeholder task/plan content.
2. Bootstrap worktrees/task files:
   - `bash scripts/create-worktrees.sh <queue-file>`
   - Auto-seeds objective/plan placeholders from task metadata.
   - Auto-links `frontend/node_modules` into worktrees when available (`LOOP_LINK_FRONTEND_NODE_MODULES=true`).
3. Load loop environment:
   - `source .env.loop.example`
   - or `source .env.loop`
4. Start serialized supervisor:
   - `bash scripts/loop-supervisor.sh <queue-file>`
   - Or wrapper with defaults:
     - `bash scripts/run-barry-felicity-loop.sh <queue-file>`
5. Monitor progress:
   - Check queue statuses in `<queue-file>`
   - Confirm worktree branch heads after each completed task
6. Integrate:
   - Merge completed task branches to `main` in queue order
   - Run `go test ./...` after integration

## Reliability Defaults

- Transient Codex stream/network failures are auto-retried:
  - `LOOP_TRANSIENT_MAX_RETRIES=3`
  - `LOOP_TRANSIENT_RETRY_DELAY_SECONDS=4`
- Frontend validation should use:
  - `bash scripts/validate-frontend.sh frontend`
  - Set `FRONTEND_TEST_REQUIRED=true` to require `scripts.test` in `frontend/package.json`.

## Cleanup and Handoff

- Remove completed worktrees safely:
  - `bash scripts/cleanup-completed-worktrees.sh <queue-file>`
- Force remove dirty completed worktrees:
  - `FORCE_DIRTY=true bash scripts/cleanup-completed-worktrees.sh <queue-file>`

## Guardrails

- Do not reorder tasks that use `depends_on` without explicit user approval.
- Do not parallelize queue tasks unless the user explicitly asks.
- Do not include local runtime artifacts in commits (`provider_connections.json`, `.env`, generated logs) unless requested.
- Keep execution and worktrees inside the repo workspace.
- Prefer `scripts/validate-frontend.sh` over ad hoc frontend gates so failures are consistent.
- If queue validation fails due placeholder artifacts, fix objective/plan before running supervisor.

## Fast Recovery

- If supervisor stalls, inspect queue status and active worker process.
- If worker is stuck with no file changes, restart supervisor from current queue state.
- If validation fails, keep status as `failed_validation` and report concrete failing command output.
- If agent execution fails with transient stream disconnects, rely on supervisor retry behavior before marking blocked.
