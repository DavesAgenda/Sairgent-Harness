---
cadence: dusk
schedule: "Daily, Monday-Friday, 18:04 UTC"
assignee: Perry
version: "1.0"
---

# Dusk Cadence Brief

**Schedule**: Daily, Mon-Fri @ 18:04 UTC
**Assignee**: Perry (COO)
**Purpose**: End-of-day audit, task closeout, state snapshot — wrap the day's work, capture learnings, prepare for next morning, and document overnight watch items

## Prerequisites

**Environment Variables**:
- `SAIRGENT_CONTEXT_DIR` — Path to `00_Context/Memory/` (required)
- `SAIRGENT_ARTIFACTS_DIR` — Path to agent artifacts folder (required)
- `CADENCE_STATE_JSON` — Optional; JSON file tracking cadence state (updated by Heartbeat)

**MCP Tools** (optional, graceful degradation):
- `linear` — Linear workspace queries (issues, comments, status)
- `n8n-email` — Email send (for evening briefing/summary)
- `web-search` — Competitive intelligence or market sweeps

**Specialist Agents**:
- Oliver (Competitive Intelligence) — end-of-day competitive check
- Lois (Research Specialist) — prepare meeting prep dossiers for tomorrow
- Felicity (CTO/Lead Engineer) — engineering closeout if needed

---

## Steps

### Step 1: Read Memory Vault

**Action**: Load current state and decision context.

- Read: `$SAIRGENT_CONTEXT_DIR/state_of_play.md`
  - Note: current priorities, blockers, progress
  
- Read: `$SAIRGENT_CONTEXT_DIR/decisions_log.md`
  - Note: decisions made today (append review)

**Graceful Degradation**: If files missing, note in evening briefing and continue with what you have.

---

### Step 2: Read Today's Pulse Journal Entries

**Action**: Collect all pulse journal entries from today (since 00:00 UTC).

**Extract**:
- All `CADENCE::dawn_complete`, `CADENCE::heartbeat_run` entries
- All `ESCALATION::` entries flagged by Heartbeat or earlier
- Any `DELEGATION::completed` entries (delegations that finished today)
- Any `TASK::` entries (task completions, failures, retries)

**Parse**: Summarize by category (tasks completed, delegations dispatched, escalations handled, blockers).

**Output**: Structured summary with counts and highlights.

**Graceful Degradation**: If no journal available, note "journal unavailable" and continue.

---

### Step 3: Full Linear Audit — All Issues with Activity Today

**Action**: Use `linear` MCP to capture complete activity picture for the day.

**Query**:
- project: "Sairgent"
- updatedAt >= [today 00:00 UTC]
- Include: all issues, status changes, comments, assignee changes

**Parse Results**:
- **New Issues Created**: Count, list titles
- **Issues Closed**: Count, list titles
- **Status Transitions**: Example: "3 moved to Done, 2 moved to Blocked"
- **Comments**: Count, flag if any "HITL feedback" markers
- **Escalations Resolved**: List any "escalation" or "human" labeled issues that were closed today
- **Stale Issues**: Any issues open >48h without progress? Flag for Chair.

**Output**: Comprehensive activity log for the day.

**Graceful Degradation**: If `linear` unavailable, skip to Step 4; note "Linear unavailable" in evening briefing.

---

### Step 4: Close Loops — Check for Tasks That Should Have Been Completed Today

**Action**: Identify tasks that were due today but not yet closed.

**Criteria**:
- Linear issues with `dueDate <= today`
- Status NOT in {Closed, Done, Cancelled}
- Assigned to Perry or flagged "Sairgent" project

**Action**:
- If task is legitimately blocked, update Linear with blocker context
- If task was completed but status not updated, manually close it
- If task needs delegation, queue for tomorrow's Dawn briefing

**Output**: List of tasks due today with status (closed, blocked, deferred).

**Graceful Degradation**: If Linear unavailable, use memory notes to infer due tasks.

---

### Step 5: Compile Daily Stats

**Action**: Synthesize day's work into metrics.

**Stats to Calculate**:
- **Tasks Completed**: Count of issues moved to "Done" today
- **Delegations Made**: Count of work orders dispatched to agents
- **Artifacts Produced**: Count of artifacts created (briefings, analyses, etc.)
- **Escalations Handled**: Count of escalations created/resolved
- **Time Spent on Blockers**: Estimate from pulse journal (qualitative: "1 blocker unresolved, 2 resolved")
- **Chair Interaction**: Count of decisions required from Chair

**Output**: Markdown table or bullet list with all metrics.

**Example**:
```
Tasks Completed: 7
Delegations Made: 4
Artifacts Produced: 3 (morning briefing, afternoon sync, analysis)
Escalations Handled: 1 (resolved)
Blockers: 0 unresolved
Chair Decisions: 1 (project scope approval)
```

---

### Step 6: Delegate to Oliver — End-of-Day Competitive Intelligence Check

**Action**: Dispatch delegation to Oliver for EOD competitive scan.

- **Task**: "Scan end-of-day competitive developments (pricing, announcements, funding, hires). Summarize material changes."
- **Inputs**: Current competitive landscape from state_of_play, known competitors, top threats
- **Expected Output**: Artifact file in `$SAIRGENT_ARTIFACTS_DIR/oliver/` named `eod_competitive_brief_[date].md`
- **Timeout**: 20 minutes. If not complete by Step 9, include "pending" status in evening briefing.

**Tool**: Use delegation command to Oliver with `chat_mode` and artifact path.

---

### Step 7: Delegate to Lois — Prepare Meeting Prep Dossiers for Tomorrow

**Action**: Dispatch delegation to Lois to prepare any dossiers needed for tomorrow's calendar.

- **Task**: "Review tomorrow's calendar (from state_of_play and calendar MCP). Prepare 1-page dossiers for meetings with external parties or decision points. Include: context, attendees, pre-work needed, recommended talking points."
- **Inputs**: Tomorrow's calendar (see Heartbeat for lookahead), context from state_of_play
- **Expected Output**: Artifact files in `$SAIRGENT_ARTIFACTS_DIR/lois/` named `dossier_[meeting_name]_[date].md`
- **Timeout**: 25 minutes. If not complete by Step 9, include "pending" status in evening briefing.

**Tool**: Use delegation command to Lois with `chat_mode` and artifact path.

**Graceful Degradation**: If tomorrow's calendar unavailable, task Lois with "review known upcoming meetings" and let her prioritize.

---

### Step 8: Review Analytics Dashboards (Optional)

**Action**: If analytics dashboard available, review daily KPIs and trends.

**Check**:
- DAU, conversion rate, revenue — compare to yesterday and week average
- Any >10% anomalies? Flag for investigation.
- System uptime and latency — any incidents?

**Output**: Summary of health or "metrics nominal".

**Graceful Degradation**: If dashboard unavailable, note "analytics unavailable" in evening briefing. Heartbeat (Step 5, analytics domain) will have captured trends.

---

### Step 9: Rewrite state_of_play.md with Updated Status

**Action**: Overwrite `state_of_play.md` with today's final status snapshot (do not append; **replace entire file**).

**New Content Should Include**:
- **Last Updated**: Today's date and time (18:04 UTC)
- **Current Phase**: (e.g., Phase 5D)
- **Strategic Focus**: Top OKR or mission for current quarter
- **Today's Accomplishments**: Bullets from Step 5 (tasks completed, delegations, artifacts)
- **Current Blockers**: Unresolved blockers from Linear audit (Step 3)
- **Today's Key Decisions**: Any Chair approvals or pivots made
- **Daily Stats**: From Step 5 (task count, delegation count, etc.)
- **Upcoming Deadlines**: Next 7 days with at-risk items flagged
- **Overnight Watch Items**: Escalations or tasks to monitor during night/early morning
- **Tomorrow's Focus**: Top 3–5 priorities for next business day

**Format**: Use existing template from current state_of_play.md; update all fields.

**Versioning**: Overwrite completely (single source of truth for "current state").

**Confirmation**: Log "state_of_play_rewritten" to pulse journal.

**Graceful Degradation**: If state_of_play.md missing, create from scratch using info from Steps 1–5.

---

### Step 10: Archive Previous state_of_play Version (Optional)

**Action**: If configured, archive yesterday's state_of_play for historical reference.

**Process**:
- Check if `state_of_play_archive/` folder exists
- If yes, move yesterday's state_of_play to `state_of_play_archive/state_of_play_[YYYY-MM-DD].md`
- If no, create archive folder first

**Graceful Degradation**: If archiving fails or not configured, skip and continue. Keep only latest state_of_play.md in main folder.

---

### Step 11: Update project_tasks.md with Progress

**Action**: Refresh project task list based on today's work.

**Review Process**:
- Read: `$SAIRGENT_CONTEXT_DIR/project_tasks.md`
- For each task completed today, mark with `[x]` instead of `[ ]`
- For each task in progress, mark with `[/]` instead of `[ ]`
- For each new task discovered today, add to the list with `[ ]`
- Update the "Last Updated" date at top of file

**New Tasks to Add**:
- Any follow-up work from today's escalations
- Any delegations that need formal task tracking
- Any blockers that need explicit unblocking work

**Format**: Preserve existing table/checklist format; do not restructure.

**Confirmation**: Log "project_tasks_updated" to pulse journal.

---

### Step 12: Write Evening Briefing Artifact

**Action**: Compile day's closure into an "Evening Briefing" artifact.

**Briefing Structure**:
```markdown
# Evening Briefing — [Date]

## Day Summary
- Tasks completed: [count]
- Delegations made: [count]
- Artifacts produced: [count]
- Escalations handled: [count]

## Linear Activity
- New issues: [count]
- Issues closed: [count]
- Status changes: [count with examples]
- Unresolved blockers: [count or "none"]

## Accomplishments Highlighted
- [Bullet 1]
- [Bullet 2]
- [Bullet 3]

## Escalations & Blockers
- [List any unresolved or flagged items]

## Tomorrow's Focus
- [Top 3–5 priorities]
- Hard-block meetings: [count, names]
- Decision checkpoints: [any needed]

## Specialist Updates
- **Oliver (Competitive)**: [pending | brief summary] (link if ready)
- **Lois (Meeting Prep)**: [pending | brief summary] (link if ready)

## Overnight Watch Items
- [Any escalations or tasks to monitor during night]
- [Any pending delegations from today]

## Chair Attention Needed
- [Any items requiring tomorrow morning action]

## Metrics
- [Daily stats table from Step 5]
```

**Output Path**: `$SAIRGENT_ARTIFACTS_DIR/perry/evening_briefing_[YYYY-MM-DD].md`

**Versioning**: If file exists, overwrite (single daily briefing).

---

### Step 13: Send Evening Email to Chair (Optional)

**Action**: Send evening briefing email via `n8n-email` MCP (if available and configured).

**Email Template**:
- **To**: Chair email (from config)
- **Subject**: `🌆 Evening Briefing — [Date]`
- **Body**: Plaintext version of briefing (Steps 1–12)
- **Attachments**: Link to artifact file (or paste inline if small)

**Graceful Degradation**: If `n8n-email` unavailable:
- Note "email not sent, artifact only" in pulse journal
- Briefing still written to artifacts (Step 12 above)

**Confirmation**: Log "evening_email_sent" (or "evening_email_skipped") to pulse journal.

---

### Step 14: Append Pulse Journal Entry — dusk_complete

**Action**: Log completion to the pulse journal.

**Entry Format**:
```
[2026-04-11 18:15] CADENCE::dusk_complete
  linear_audit: new=2, closed=3, status_changes=4, escalations=0
  daily_stats:
    tasks_completed: 7
    delegations_made: 4
    artifacts_produced: 3
    blockers_unresolved: 0
  specialists:
    oliver: pending (15m timeout)
    lois: pending (20m timeout)
  state_of_play: rewritten
  project_tasks: updated
  evening_briefing: written to /artifacts/perry/evening_briefing_2026-04-11.md
  email_sent: yes
  overnight_watch_items: 0
```

**Tool**: Use `append_pulse_journal` with cadence="dusk", status="complete", and detailed summary dict.

---

### Step 15: Log Tomorrow's Priorities and Overnight Watch Items

**Action**: Create a structured entry for tomorrow's focus and any overnight monitoring.

**Entry Format**:
```
[2026-04-11 18:15] CADENCE::dusk_priorities
  tomorrow_focus:
    - Priority 1: [description]
    - Priority 2: [description]
    - Priority 3: [description]
  overnight_watch:
    - Escalation: [description + owner/context]
    - Pending: [delegations from today awaiting completion]
  calendar_preview: 3 hard-block meetings, 2 decision checkpoints
  headwinds: [any blockers anticipated for tomorrow]
  opportunities: [any wins to capitalize on]
```

**Tool**: Use `append_pulse_journal` with cadence="dusk_priorities" and structured data.

**Purpose**: This entry feeds directly into tomorrow's Dawn briefing (Step 2 of Dawn reads overnight watch items).

---

## Graceful Degradation Summary

| Tool/Resource | If Unavailable | Action |
|---|---|---|
| `linear` MCP | Skip Step 3; use pulse journal for activity summary | Note "Linear unavailable" in evening briefing |
| `n8n-email` MCP | Skip email send; artifact still created | Note "Email not sent, artifact only" in pulse |
| Specialist agents (Oliver, Lois) | Mark as "pending" in briefing, do not block | Include "pending" status; retry in next Dawn |
| Analytics dashboard | Skip Step 8; Heartbeat analytics domain will have captured trends | Note "Analytics unavailable" |
| Memory files | Treat as missing; continue with pulse journal only | Note "Memory files missing" and alert Chair |
| state_of_play_archive | Skip Step 10; keep latest version only | Log "archiving unavailable, latest only" |

---

## Output

**Artifacts Created**:
1. `perry/evening_briefing_[YYYY-MM-DD].md` — Full evening briefing
2. `oliver/eod_competitive_brief_[YYYY-MM-DD].md` — Oliver's competitive scan (if completed)
3. `lois/dossier_[meeting_name]_[YYYY-MM-DD].md` — Lois's meeting prep (if completed)

**Files Modified**:
1. `state_of_play.md` — Rewritten with today's final status
2. `project_tasks.md` — Updated with task progress

**Communications**:
- Email to Chair (if `n8n-email` available): Evening briefing email
- Pulse journal: dusk_complete entry + dusk_priorities entry

**State Updates**:
- Updated state_of_play.md with final day snapshot
- Updated project_tasks.md with task progress
- Optional: Archived previous state_of_play to state_of_play_archive/

---

## Notes for Perry

- Dusk runs at **18:04 UTC** daily (Mon-Fri only). No weekend runs unless explicitly triggered.
- Unlike Dawn (which **prepares**) and Heartbeat (which **observes**), Dusk **closes loops and documents**.
- The evening briefing is a **historical record** — Chair can review it to understand what happened during the day.
- **State rewrite** (Step 9) is critical: state_of_play.md is the single source of truth for "where we are now". Overwrite completely, do not append.
- If Oliver or Lois are busy, mark their contributions as "pending" and retry in the next business day's Dawn.
- Any escalations flagged in Heartbeat throughout the day should already have pulse journal entries; Dusk just summarizes them.
- Overnight watch items (Step 15) feed directly into tomorrow's Dawn briefing, so be explicit about what needs monitoring.
