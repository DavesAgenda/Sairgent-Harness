---
cadence: heartbeat
schedule: "Hourly, 24/7"
assignee: Perry
version: "1.0"
---

# Heartbeat Cadence Brief

**Schedule**: Hourly, 24/7
**Assignee**: Perry (COO)
**Purpose**: Rotating domain health checks, operational pulse, and escalation detection — runs every hour to keep Sairgent's operational dashboard fresh and catch actionable signals in real-time

## Prerequisites

**Environment Variables**:
- `SAIRGENT_CONTEXT_DIR` — Path to `00_Context/Memory/` (required)
- `SAIRGENT_ARTIFACTS_DIR` — Path to agent artifacts folder (required)
- `CADENCE_STATE_JSON` — Path to JSON file tracking domain check rotations and last-checked timestamps (required)

**MCP Tools** (optional, graceful degradation):
- `linear` — Linear workspace queries (issues, status, comments)
- `web-search` — Competitor, market, and SEO intelligence

**Specialist Agents**:
- Iris (Tech Intelligence) — deep-dive technical checks if domains surface issues
- Oliver (Competitive Intelligence) — competitor analysis if market signals warrant
- Lois (Research Specialist) — analytics/data synthesis if needed
- Various domain-specific agents as needed

---

## Steps

### Step 1: Read Current State from state_of_play.md

**Action**: Load the latest operational snapshot.

- Read: `$SAIRGENT_CONTEXT_DIR/state_of_play.md`
- Extract: current phase, active blockers, priority projects, key metrics
- Note: any "24/7 watch" items requiring hourly checks

**Graceful Degradation**: If file missing, use last known state from memory or start with minimal context.

---

### Step 2: Read Cadence State — Find Due Domains

**Action**: Determine which domain(s) are due for checking this hour.

**Load `CADENCE_STATE_JSON`** (or create if missing):
```json
{
  "last_heartbeat_run": "2026-04-11T06:30:00Z",
  "domains": {
    "crm": {
      "interval_hours": 2,
      "last_checked": "2026-04-11T04:30:00Z",
      "next_due": "2026-04-11T06:30:00Z"
    },
    "content": {
      "interval_hours": 2,
      "last_checked": "2026-04-11T04:45:00Z",
      "next_due": "2026-04-11T06:45:00Z"
    },
    "competitor": {
      "interval_hours": 4,
      "last_checked": "2026-04-11T02:30:00Z",
      "next_due": "2026-04-11T06:30:00Z"
    },
    "analytics": {
      "interval_hours": 6,
      "last_checked": "2026-04-11T00:30:00Z",
      "next_due": "2026-04-11T06:30:00Z"
    },
    "repo": {
      "interval_hours": 4,
      "last_checked": "2026-04-11T02:30:00Z",
      "next_due": "2026-04-11T06:30:00Z"
    },
    "seo": {
      "interval_hours": 24,
      "last_checked": "2026-04-10T06:04:00Z",
      "next_due": "2026-04-11T06:04:00Z"
    },
    "git": {
      "interval_hours": 24,
      "last_checked": "2026-04-10T06:04:00Z",
      "next_due": "2026-04-11T06:04:00Z"
    }
  }
}
```

**Logic**: For current hour, find all domains where `now >= next_due`.

**Output**: List of due domains (e.g., `["crm", "competitor", "analytics", "seo", "git"]`)

**Graceful Degradation**: If JSON missing, check all domains on first run, then establish baseline.

---

### Step 3: Check Linear for Updates Since Last Heartbeat

**Action**: Use `linear` MCP to identify actionable changes.

**Query**:
- project: "Sairgent"
- updatedAt >= [last_heartbeat_run]
- Include: new issues, status changes, comments, assignee changes

**Parse Results**:
- **New Issues**: Count, label severity
- **Status Changes**: Which issues moved (e.g., Todo → In Progress), flag if deadline-adjacent
- **Comments**: Count, flag if "urgent" or "escalation" markers
- **Escalations**: Any issues labeled "Human", "Blocked", or "Urgent"

**Output**: Categorized summary: `{new_count, status_changes, comments_count, escalations}`

**Graceful Degradation**: If `linear` unavailable, skip to Step 4; note "Linear unavailable" in pulse.

---

### Step 4: Categorize Linear Updates

**Action**: Classify updates for downstream action.

**Categories**:
- **Actionable**: New issues, status blockers, HITL feedback → Perry review
- **FYI**: Routine comments, closed tickets → log only
- **Escalation**: High priority, missed deadlines, compliance alerts → flag for immediate attention

**Output**: Markdown list with categories and issue links.

---

### Step 5: Execute Domain Checks for Due Domains

**Action**: Run domain-specific checks for each domain in the "due" list.

#### Domain: CRM (2-hour interval)

**Objective**: Monitor customer relationship health, pipeline, and escalations.

**Checks**:
- Active CRM issues (HubSpot, Salesforce, or internal tracker)
- New inbound leads or partnership inquiries
- Escalated customer requests
- Pipeline velocity (if metrics available)

**Output**: Brief summary (1-2 paragraphs) or "no issues"

**Tool**: Use `web-search` if CRM data unavailable via MCP; otherwise manual review.

#### Domain: Content (2-hour interval)

**Objective**: Monitor content production, publication status, and performance.

**Checks**:
- Published content since last check
- Engagement metrics (views, clicks, shares)
- Content in review or pending approval
- User feedback on content

**Output**: Summary: items published, metrics delta, pending approvals.

**Tool**: Check Linear for "content" label issues; use `web-search` for published URLs if needed.

#### Domain: Competitor (4-hour interval)

**Objective**: Competitive positioning, market moves, and threat assessment.

**Checks**:
- Competitor announcements or releases
- Pricing changes
- New product features
- Market sentiment shifts

**Output**: Brief alert or "no material changes"

**Tool**: `web-search` (news, press releases, social); delegate to Oliver if deep analysis needed.

#### Domain: Analytics (6-hour interval)

**Objective**: KPI health, user metrics, and performance trends.

**Checks**:
- DAU/MAU, conversion funnels
- Performance metrics (latency, uptime)
- Anomalies or unexpected drops
- Revenue/usage trends

**Output**: Summary of metrics, flags for any >20% anomalies.

**Tool**: If dashboard available, query directly; otherwise delegate to Lois for synthesis.

#### Domain: Repo (4-hour interval)

**Objective**: Code health, test status, build pipeline, and deployment readiness.

**Checks**:
- Failing tests or builds (CI/CD pipeline status)
- Security vulnerabilities or code reviews pending >4h
- Deployment blockers
- Open PRs by age and status

**Output**: Summary of health; flag any tests failing >2h or PRs open >24h.

**Tool**: Use `web-search` for public status pages; Linear for internal issues tagged "repo".

#### Domain: SEO (24-hour interval)

**Objective**: Search engine visibility, indexing, and organic traffic.

**Checks**:
- New pages indexed
- Ranking changes (top keywords)
- Search console alerts (crawl errors, coverage issues)
- Organic traffic trends

**Output**: Summary of status and any alerts.

**Tool**: `web-search` (Google Trends, search console data if available); delegate to Oliver if deep analysis needed.

#### Domain: Git (24-hour interval)

**Objective**: Repository activity, contributor health, and release readiness.

**Checks**:
- Commits per day, contributor count
- Open/closed issues rate
- Release branches and version tags
- Any unusual activity (mass deletes, force pushes)

**Output**: Activity summary; flag any unusual patterns.

**Tool**: Git CLI (if running on same machine) or Linear issue activity.

---

### Step 6: Update Cadence State with New Timestamps

**Action**: Upsert `CADENCE_STATE_JSON` with latest check times.

**For each domain checked**, update:
```json
"domain_name": {
  "last_checked": "[ISO timestamp of this check]",
  "next_due": "[ISO timestamp of next scheduled check]"
}
```

**Also update root**:
```json
"last_heartbeat_run": "[ISO timestamp of this heartbeat]"
```

**Write updated JSON** back to `$CADENCE_STATE_JSON`.

---

### Step 7: Delegate to Specialists if Domain Checks Surface Actionable Items

**Action**: If any domain reveals a need for deeper investigation, delegate.

**Examples**:
- **Competitor** domain flags new market threat → delegate to Oliver for 30-min deep dive
- **Repo** domain shows tests failing >2h → delegate to Iris (or Felicity) for root cause
- **Analytics** domain shows >30% drop in DAU → delegate to Lois for root cause synthesis
- **CRM** domain shows high-value customer escalation → notify Chair immediately

**Tool**: Use delegation command with `chat_mode` and focused task scope (limit to 30 min for non-critical).

**Graceful Degradation**: If no actionable items, log "no delegations needed" and continue.

---

### Step 8: Append Pulse Journal Entry with Step Summaries

**Action**: Log the heartbeat execution to the pulse journal.

**Entry Format**:
```
[2026-04-11 07:30] CADENCE::heartbeat_run
  due_domains: crm, competitor, analytics, seo, git (5 checks)
  linear_updates: new=1, status_changes=2, comments=3, escalations=0
  domain_results:
    crm: OK (3 active leads, no escalations)
    content: pending (next due 08:45)
    competitor: OK (no material changes)
    analytics: ALERT (DAU -5%, investigate)
    repo: OK (all tests passing, 2 PRs open <4h)
    seo: pending (next due tomorrow 06:04)
    git: OK (15 commits today, all contributors active)
  delegations: 1 (Lois for analytics anomaly)
  escalations: 0
  state_json_updated: yes
```

**Tool**: Use `append_pulse_journal` with cadence="heartbeat", status="complete", and detailed summary dict.

---

### Step 9: Check for Escalation Conditions

**Action**: Scan for signals requiring immediate Chair or team attention.

**Escalation Criteria**:
- Test failure >2 hours (repo domain)
- Customer escalation (CRM domain)
- DAU/usage drop >20% (analytics domain)
- Competitor critical announcement (competitor domain)
- Security or compliance alert (any domain)
- Stale task (Linear issue open >48h without progress on "blocker" label)
- Missed deadline (task due today not yet closed)

**Output**: List of escalations (may be empty).

**No Action Yet**: Just flag for Step 10. Do not auto-close or auto-delegate.

---

### Step 10: Create Flagged Journal Entry for Next Dawn/Dusk if Escalation Detected

**Action**: If escalations found, create a durable flag for Dawn (morning) or Dusk (evening) to act on.

**Entry Format**:
```
[2026-04-11 07:30] ESCALATION::heartbeat_detected
  severity: high
  issue: Analytics DAU -5% (possible outage or UX regression)
  context: Last normal reading 2026-04-11 01:30; current 07:30 check shows drop
  action_needed: Delegate to Lois for root cause; notify Chair if >15% drop detected by next check
  window: monitor every heartbeat, escalate to Chair by 08:30 if not resolved
```

**Graceful Degradation**: If no escalations, log "no_escalations_detected" and continue.

---

## Graceful Degradation Summary

| Tool/Resource | If Unavailable | Action |
|---|---|---|
| `linear` MCP | Skip Step 3; note "Linear unavailable" in pulse | Continue with domain checks only |
| `web-search` MCP | Use manual data sources or skip intelligence checks | Log "web-search unavailable", defer to specialist agents |
| Specialist agents | All busy; do not queue new delegations | Log "specialists unavailable" and flag for next Dawn/Dusk |
| `CADENCE_STATE_JSON` | Create new file with default intervals on first run | Initialize all domains with default check intervals |
| Memory files | Treat as unavailable; continue with domain checks | Log "memory files unavailable" to pulse journal |

---

## Output

**Artifacts Created** (optional):
- None by default; heartbeat is observational.
- If escalation detected, may create: `escalations_heartbeat_[timestamp].md`

**Communications**:
- None by default; heartbeat is asynchronous.
- If escalation detected: flag entry added to pulse journal for Dawn/Dusk review.

**State Updates**:
- Update `CADENCE_STATE_JSON` with: last_checked timestamps for each domain, next_due for each domain

**Journal Entries**:
- `heartbeat_run` entry with summary
- `escalation_flagged` entry if escalations detected

---

## Notes for Perry

- Heartbeat runs **every hour** (24/7), including weekends and nights.
- Unlike Dawn (morning briefing) and Dusk (daily closeout), Heartbeat is **observational** — it detects signals but does not drive major action.
- Domain intervals are staggered to balance real-time visibility with overhead:
  - **Fast** (2h): CRM, Content — customer-facing, high-churn
  - **Medium** (4h): Competitor, Repo — strategic importance
  - **Slow** (6h): Analytics — trend-based, less time-sensitive
  - **Daily** (24h): SEO, Git — routine operational health
- If a domain check surfaces an escalation, **do not wait for next Dawn/Dusk** — create a flagged pulse entry immediately so Chair sees it.
- Heartbeat delegations should be **short-lived** (15–30 min). Longer investigations should be bundled into formal work orders or tasks.
- If specialist agents are busy, queue the delegation in the pulse journal and re-attempt in the next Heartbeat.
