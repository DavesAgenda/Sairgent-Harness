---
cadence: dawn
schedule: "Daily, Monday-Friday, 06:04 UTC"
assignee: Perry
version: "1.0"
---

# Dawn Cadence Brief

**Schedule**: Daily, Mon-Fri @ 06:04 UTC
**Assignee**: Perry (COO)
**Purpose**: Morning situational awareness briefing — ingest overnight developments, delegate tech/brand scans, compile morning briefing

## Prerequisites

**Environment Variables**:
- `SAIRGENT_CONTEXT_DIR` — Path to `00_Context/Memory/` (required)
- `SAIRGENT_ARTIFACTS_DIR` — Path to agent artifacts folder (required)
- `CADENCE_STATE_JSON` — Optional; JSON file tracking last-checked timestamps

**MCP Tools** (optional, graceful degradation):
- `linear` — Linear workspace queries (issues, comments, updates)
- `n8n-email` — Email send (for morning briefing delivery)
- `n8n-calendar` — Calendar API (for lookahead)
- `web-search` — News/intelligence sweeps

**Specialist Agents**:
- Iris (Tech Intelligence) — overnight tech scan
- Robin (Brand Sentinel) — overnight reputation check

---

## Steps

### Step 1: Read Memory Vault

**Action**: Load the current state of play and decision log.

- Read: `$SAIRGENT_CONTEXT_DIR/state_of_play.md`
  - Note: last update date, current priorities, blockers
  - Flag: any "overnight watch" items from yesterday's Dusk
  
- Read: `$SAIRGENT_CONTEXT_DIR/decisions_log.md`
  - Note: last 5 decisions (context for today's work)

**Graceful Degradation**: If files missing, note in briefing and continue.

---

### Step 2: Check Entity Mission Context

**Action**: Confirm mission alignment for today.

- Extract from state_of_play: current phase (e.g., Phase 5D, Phase 7), top-level OKR, and strategic focus
- Note any escalation flags or "Chair attention needed" markers from yesterday
- Prepare context for all downstream domain checks

---

### Step 3: Sweep Linear for Overnight Updates

**Action**: Use `linear` MCP tool to capture new/updated issues.

```
Query:
- project: "Sairgent"
- updatedAt >= [yesterday 18:04 UTC] (last Dusk)
- Include: issues, status changes, comments, new work
```

**Parse Results**:
- New issues (status = Todo)
- Status changes (e.g., In Progress → Done, Todo → In Progress)
- Comments on existing issues (especially HITL feedback)
- Escalations (high priority, blocked, or "Human" label)

**Output**: Categorized list: `[new_count, status_changed_count, comments_with_feedback, escalations]`

**Graceful Degradation**: If `linear` MCP unavailable, skip and note in briefing. Continue.

---

### Step 4: Read Recent Linear Issue Comments for HITL Feedback

**Action**: Extract human-in-the-loop feedback from Linear comments.

- Focus on comments from last 24 hours (since yesterday 06:04)
- Filter for comments on issues assigned to Perry or labeled "Perry"
- Extract: approval notes, revision requests, feedback on completed work
- Flag any "blocker" or "urgent" markers

**Output**: Markdown list of feedback items with source issue links

**Graceful Degradation**: If no comments or tool unavailable, note "no HITL feedback" in briefing.

---

### Step 5: Delegate to Iris — Overnight Tech Intelligence Scan

**Action**: Dispatch delegation to Iris for overnight tech intelligence.

- **Task**: "Scan overnight tech developments (CVEs, releases, market moves) relevant to Sairgent tech stack. Summarize in 1-page brief."
- **Inputs**: Current tech focus from state_of_play, top dependencies, competitive threats
- **Expected Output**: Artifact file in `$SAIRGENT_ARTIFACTS_DIR/iris/` named `overnight_tech_brief_[date].md`
- **Timeout**: 15 minutes. If not complete by Step 9, include "pending" status in final briefing.

**Tool**: Use delegation command to Iris with `chat_mode`, context from state_of_play, and artifact path.

---

### Step 6: Delegate to Robin — Overnight Brand/Reputation Check

**Action**: Dispatch delegation to Robin for brand sentiment and reputation.

- **Task**: "Scan overnight brand mentions, social sentiment, and reputation signals (Reddit, HN, Twitter, news). Summarize escalations."
- **Inputs**: Company name, target audience, known competitors, recent product announcements
- **Expected Output**: Artifact file in `$SAIRGENT_ARTIFACTS_DIR/robin/` named `overnight_brand_brief_[date].md`
- **Timeout**: 15 minutes. If not complete by Step 9, include "pending" status in final briefing.

**Tool**: Use delegation command to Robin with `chat_mode` and artifact path.

---

### Step 7: Fetch Inbox Summary (Email via n8n)

**Action**: Pull overnight inbox summary via `n8n-email` MCP (if available).

**Query**:
- Since: yesterday 18:04 UTC
- To: Chair (CEO) or Perry direct reports
- Filter: starred, flagged, or high-priority markers
- Limit: top 10 unread threads by relevance

**Output**: Bullet list of inbox themes (e.g., "3 vendor escalations", "2 partnership inquiries")

**Graceful Degradation**: If `n8n-email` unavailable, note "email summary unavailable" and continue.

---

### Step 8: Fetch Calendar Lookahead — Next 48 Hours

**Action**: Pull 48-hour calendar view via `n8n-calendar` MCP (if available).

**Query**:
- Since: now
- Until: 48 hours from now
- Include: Perry, Chair, key stakeholders
- Focus: meetings with external parties, decision-critical reviews, deadline-driven blocks

**Parse Results**:
- Hard-block meetings (cannot delegate)
- Decision checkpoints (need context prepared)
- Deadline-adjacent tasks (prepare for closeout)

**Output**: Structured lookahead with time, attendees, and pre-work needed

**Graceful Degradation**: If `n8n-calendar` unavailable, note "calendar unavailable" and continue.

---

### Step 9: Compile All Inputs into Structured Morning Briefing

**Action**: Synthesize Steps 1–8 into a single "Morning Briefing" artifact.

**Briefing Structure**:
```markdown
# Morning Briefing — [Date]

## Overnight Summary
- Key developments (Linear, email, brand, tech)
- Escalations requiring Chair attention
- New blockers or opportunities

## Today's Focus
- Current phase and OKR
- High-priority work (from state_of_play)
- Overnight watch items (from yesterday's Dusk)

## Linear Status
- New issues: [count]
- Status changes: [count]
- HITL feedback: [list with issue links]

## Inbox & Calendar
- Top email themes: [list]
- Hard-block meetings: [time, attendees, pre-work]
- Decision checkpoints: [time, context needed]

## Specialist Scans
- **Iris (Tech)**: [pending | brief summary] (link to artifact if ready)
- **Robin (Brand)**: [pending | brief summary] (link to artifact if ready)

## Operational Reminders
- Stale tasks to check: [from Linear query]
- Missed deadlines: [if any]
- Overnight escalations: [if any]

## Chair Attention Needed
- [Escalations requiring human approval or decision]
```

**Output Path**: `$SAIRGENT_ARTIFACTS_DIR/perry/morning_briefing_[date].md`

---

### Step 10: Write Briefing Artifact File

**Action**: Save the compiled briefing to the artifacts folder.

- File: `$SAIRGENT_ARTIFACTS_DIR/perry/morning_briefing_[YYYY-MM-DD].md`
- Format: Markdown with YAML frontmatter (cadence, date, version)
- Versioning: If file exists, overwrite (single daily briefing)

**Confirmation**: Log "briefing_artifact_written" to pulse journal.

---

### Step 11: Send Briefing Email to Chair (Optional)

**Action**: Send morning briefing email via `n8n-email` MCP (if available and configured).

**Email Template**:
- **To**: Chair email (from config)
- **Subject**: `🌅 Morning Briefing — [Date]`
- **Body**: Plaintext version of briefing (Steps 1–9)
- **Attachments**: Link to artifact file (or paste inline if small)

**Graceful Degradation**: If `n8n-email` unavailable:
- Note "email not sent, artifact only" in pulse journal
- Briefing still written to artifacts (Step 10 above)

**Confirmation**: Log "briefing_email_sent" (or "briefing_email_skipped") to pulse journal.

---

### Step 12: Append Pulse Journal Entry — dawn_complete

**Action**: Log completion to the pulse journal.

**Entry Format**:
```
[2026-04-11 06:15] CADENCE::dawn_complete
  Linear sweep: new=3, status_changed=2, feedback_items=1
  Iris scan: pending (15m timeout)
  Robin scan: complete (1-page brief)
  Inbox summary: 5 high-priority threads
  Calendar: 3 hard-block meetings, 2 decision checkpoints
  Briefing written to: /artifacts/perry/morning_briefing_2026-04-11.md
  Email sent: yes
  Chair attention items: 1 escalation (vendor outage)
```

**Tool**: Use `append_pulse_journal` with cadence="dawn", status="complete", and summary dict.

---

### Step 13: Log Escalations Requiring Chair Attention

**Action**: If any escalations found (Steps 3–8), create flagged entry for Chair review.

**Escalation Criteria**:
- New issues labeled "Urgent" or "High"
- Missed deadlines
- Critical blocker in Linear
- Brand/reputation crisis signal from Robin
- Security or compliance alert

**Action if Escalation Found**:
- Create a separate escalation artifact: `escalations_[date].md`
- Include: issue links, context, recommended action, timeline
- Append pulse journal: "escalation_flagged: [description]"

**Graceful Degradation**: If no escalations, log "no_escalations" and continue.

---

## Graceful Degradation Summary

| Tool/Resource | If Unavailable | Action |
|---|---|---|
| `linear` MCP | Use last known state from `state_of_play.md` | Note "Linear unavailable" in briefing |
| `n8n-email` MCP | Skip inbox summary, continue to Step 8 | Note "Email summary unavailable" |
| `n8n-calendar` MCP | Use Perry's manual calendar check | Note "Calendar unavailable" |
| Specialist agents (Iris, Robin) | Mark as "pending", do not block | Include "pending" in briefing, retry in Heartbeat |
| Memory files (`state_of_play`, etc.) | Treat as missing; start fresh | Note "Memory files missing" and alert Chair |

---

## Output

**Artifacts Created**:
1. `perry/morning_briefing_[YYYY-MM-DD].md` — Full morning briefing
2. `iris/overnight_tech_brief_[YYYY-MM-DD].md` — Iris tech scan (if completed)
3. `robin/overnight_brand_brief_[YYYY-MM-DD].md` — Robin brand scan (if completed)
4. `perry/escalations_[YYYY-MM-DD].md` — (if escalations found)

**Communications**:
- Email to Chair (if `n8n-email` available): Morning briefing email
- Pulse journal: dawn_complete entry with summary

**State Updates**:
- Update `CADENCE_STATE_JSON` with: `last_dawn_run: [timestamp]`

---

## Notes for Perry

- This cadence runs at **06:04 UTC** daily (Mon-Fri only). No weekend runs unless explicitly triggered.
- The briefing is **read-only** from the Chair's perspective — Perry drives the day based on this input.
- If Iris or Robin are busy (other delegations in progress), mark their scans as "pending" and re-delegate in the Heartbeat.
- Escalations must be surfaced **immediately** — do not batch until Dusk.
- All artifact links should be absolute paths or artifact URIs so the Chair can navigate directly.
