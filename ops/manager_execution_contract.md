# Manager Execution Contract

**Status**: Active engineering contract (CHA-407, partially wired). Applies to all Manager-class agents running multi-step jobs across turns.

## Purpose

Every Manager-class agent must be able to run a Ralph loop: iterative delegate → review → revise → continue → escalate when stuck. This contract defines the shared mental model all managers follow regardless of persona. It is domain-agnostic. It is not Felicity-specific, not Perry-specific — it applies uniformly.

Without a shared contract, managers drift into two failure modes:
1. **Accept-and-finalize prematurely**: single-turn synthesis model, the manager thinks "child delivered something, job must be done" and finalizes the parent SWO before the full scope is addressed.
2. **Reject-and-loop-forever**: the manager never recognizes completion because every turn re-discovers the plan from scratch, so there is no ground truth for "job done".

The Manager Execution Contract closes both failure modes by giving managers:
- A **durable plan artifact** that persists state across turns
- A **three-way decision schema** that separates "deliverable acceptable" from "job complete"
- A **structured escalation path** when revision loops exhaust their retry budget
- A **continuation semantics** so the parent SWO stays alive across multiple manager turns

## Components

### 1. The job-plan artifact (CHA-412, this doc)

On turn 1 of any multi-step job, the manager writes `job_plan.md` to their own `workspace/` directory. The plan is a numbered task list:

```markdown
# Job Plan: Q3 competitive positioning dossier

1. Pricing comparison — status: pending
2. Feature comparison matrix — status: pending
3. Target-customer analysis — status: pending
4. Go-to-market narrative — status: pending
5. Final synthesis and executive summary — status: pending
```

On every subsequent turn, the manager reads `job_plan.md` FIRST via `read_agent_file`, crosses off completed items, updates statuses (`delegated`, `accepted`, `rejected`, `failed`, `escalated`), and decides what to delegate next.

Statuses follow the sub-SWO lifecycle:
- `pending` — not yet delegated
- `delegated` — in flight with a subordinate
- `accepted` — deliverable returned and accepted by the manager
- `rejected` — deliverable returned but rejected; revision in progress
- `failed` — revision ceiling exhausted; escalation recorded (CHA-411)
- `escalated` — flagged to parent manager for intervention

The plan is the source of truth for "what have I done vs what is left". Without it, each turn re-plans from scratch and drift is inevitable.

Tools the manager uses to maintain the plan:
- `create_file("workspace/job_plan.md", ...)` on turn 1 (requires `file_write`, default-granted via CHA-408)
- `read_file("workspace/job_plan.md")` on turns 2+ (requires `file_read`, default-granted)
- `edit_file("workspace/job_plan.md", ...)` to update statuses (requires `file_write`)

### 2. The three-way synthesis decision (CHA-409, CHA-410)

Manager synthesis decisions distinguish quality review from job completion:

| Action | Meaning | Parent SWO status | final_response required |
|---|---|---|---|
| `ACCEPT_AND_COMPLETE` | Deliverable acceptable AND job done | Finalized as COMPLETED | Yes |
| `ACCEPT_AND_CONTINUE` | Deliverable acceptable, more work pending | Stays IN_PROGRESS (pending CHA-421) | No |
| `REJECT_AND_REVISE` | Deliverable not acceptable | Stays IN_PROGRESS; revision loop | No (revision_swos required) |

The legacy `APPROVE_AND_REPLY` is accepted as an alias for `ACCEPT_AND_COMPLETE` during migration.

The manager emits the decision by populating `SynthesisDecision` in their synthesis prompt response. The kernel's `classify_synthesis_action` helper in `orchestrator.rs` normalizes legacy and new action strings to `accept_complete | accept_continue | reject` buckets for dispatch.

### 3. Revision ceiling escalation (CHA-411)

When a manager's revision loop hits `MAX_REVIEW_FAILURES` (currently 3), the kernel:

1. Records an entry in the `escalations` registry table with `swo_id`, `child_agent_id`, `parent_swo_id`, `parent_agent_id`, `attempts`, `reasoning`, `created_at`.
2. Emits a tamper-evident audit event with `TaintLabel::ManagerEscalation` (kernel-authoritative, not a harness self-report).
3. Marks the child SWO as `FAILED`.
4. The parent manager's next synthesis turn should query `list_recent_escalations_for_agent(parent_agent_id)` and distinguish escalated children from ordinary subtask failures.

The parent-side query and prompt injection is tracked as **CHA-422**. For now the escalation is recorded structurally and visible in the registry + audit chain — downstream consumers can opt in on their own timeline.

### 4. Continuation loop (CHA-421, deferred)

The full `ACCEPT_AND_CONTINUE` semantics — where the parent SWO stays IN_PROGRESS across multiple manager turns and the manager re-enters triage with accumulated deliverables as context — is tracked as CHA-421. Today the kernel logs the intent, records `next_step_brief` in the manager review reasoning, and falls through to `ACCEPT_AND_COMPLETE` to prevent SWO dangling. Managers should still emit `ACCEPT_AND_CONTINUE` honestly when the job is not done — the decision is recorded in logs and the audit chain even if the kernel does not yet act on it differently.

## What the contract is not

- **Pattern 1 (parallel git clones with branches)** — specialists do NOT get durable file state on a shared codebase. The job-plan lives in the manager's own workspace, not in a branch managed by specialists. Specialists return code/text as SWO deliverables; only managers hold durable state.
- **A policy for who becomes a manager** — org-class assignment is per-agent state in the registry, set at seeding or via `agent_update_org_class`. The contract applies to anyone with `Manager` class regardless of persona.
- **A retry policy for individual specialists** — the Manager Execution Contract is about the manager's plan. Specialist retry is the manager's decision per sub-SWO (via `REJECT_AND_REVISE` with a revision brief).

## Implementation touchpoints

- `sairgent_harness/main.py` — `_manager_execution_contract_section()` helper injects the prompt fragment into `build_triage_prompt` and `build_synthesis_prompt` for agents with subordinates.
- `sairgent_harness/hsm.py` — `SynthesisDecision` schema with the three action values + `next_step_brief`.
- `sairgent_kernel/src/orchestrator.rs` — `classify_synthesis_action()` dispatch + escalation recording at the revision ceiling.
- `sairgent_kernel/src/registry.rs` — `escalations` table + `record_escalation` + `list_recent_escalations_for_agent`.
- `sairgent_kernel/src/audit.rs` — `TaintLabel::ManagerEscalation` variant.

## Related Linear issues

- CHA-407 — Manager Execution Contract (epic)
- CHA-408 — Default FileRead + FileWrite (prerequisite for writing job_plan.md without explicit opt-in)
- CHA-409 — Shared synthesis prompt split
- CHA-410 — SynthesisDecision schema split
- CHA-411 — Revision ceiling escalation
- CHA-412 — Job-plan artifact convention (this doc)
- CHA-421 — Kernel continuation loop for ACCEPT_AND_CONTINUE
- CHA-422 — Parent manager escalation pickup (prompt + UI)
