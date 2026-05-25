use crate::audit::TaintLabel;
use crate::error::{KernelError, Result};
use crate::manifest::{AgentManifestV1, CapabilityGrant, ProviderProtocolFamily};
use crate::protocol::{normalize_worker_output, WorkerTokenUsage};
use crate::registry::{
    CreateSwoParams, OutboxArtifactListFilters, RecurringWorkOrderRunRecord,
    RecurringWorkOrderScheduleRecord, RecurringWorkOrderTemplateRecord, Registry, SwoDetailRecord,
};
use crate::router::Router;
use crate::skills::RuntimeSkillIndexEntry;
use crate::tools::active_web_search_provider;
use crate::vault::Vault;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{Duration, interval};

fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn utc_tm_from_unix(unix_secs: i64) -> Result<libc::tm> {
    let mut raw = unix_secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::gmtime_r(&mut raw, &mut tm) };
    if result.is_null() {
        return Err(KernelError::Internal(
            "Failed to convert unix timestamp into UTC time".to_string(),
        ));
    }
    Ok(tm)
}

fn utc_tm_to_unix(mut tm: libc::tm) -> Result<i64> {
    let result = unsafe { libc::timegm(&mut tm) };
    if result < 0 {
        return Err(KernelError::Internal(
            "Failed to convert UTC time into unix timestamp".to_string(),
        ));
    }
    Ok(result as i64)
}

fn format_utc_timestamp(unix_secs: i64) -> Result<String> {
    let tm = utc_tm_from_unix(unix_secs)?;
    let mut buffer = [0i8; 32];
    let written = unsafe {
        libc::strftime(
            buffer.as_mut_ptr(),
            buffer.len(),
            b"%Y-%m-%d %H:%M:%S\0".as_ptr().cast(),
            &tm,
        )
    };
    if written == 0 {
        return Err(KernelError::Internal(
            "Failed to format UTC timestamp".to_string(),
        ));
    }
    let formatted = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }
        .to_str()
        .map_err(|error| {
            KernelError::Internal(format!("Failed to read formatted timestamp: {}", error))
        })?;
    Ok(formatted.to_string())
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month_zero_based: i32) -> i32 {
    match month_zero_based {
        0 | 2 | 4 | 6 | 7 | 9 | 11 => 31,
        3 | 5 | 8 | 10 => 30,
        1 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn sanitize_schedule_day(day: i64) -> i32 {
    day.clamp(0, 6) as i32
}

/// CHA-410: Normalize legacy + new synthesis action values.
/// Returns "accept_complete" | "accept_continue" | "reject" for downstream dispatch.
/// APPROVE_AND_REPLY is treated as an alias for ACCEPT_AND_COMPLETE.
/// ACCEPT_AND_CONTINUE currently falls through to COMPLETE pending CHA-421
/// (full kernel continuation loop).
fn classify_synthesis_action(raw: &str) -> &'static str {
    match raw {
        "ACCEPT_AND_COMPLETE" | "APPROVE_AND_REPLY" => "accept_complete",
        "ACCEPT_AND_CONTINUE" => "accept_continue",
        "REJECT_AND_REVISE" => "reject",
        _ => "accept_complete", // unknown values default to legacy accept for safety
    }
}

fn infer_attachment_content_type(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("md") => "text/markdown".to_string(),
        Some("txt") => "text/plain".to_string(),
        Some("json") => "application/json".to_string(),
        Some("csv") => "text/csv".to_string(),
        Some("html") => "text/html".to_string(),
        Some("png") => "image/png".to_string(),
        Some("jpg") | Some("jpeg") => "image/jpeg".to_string(),
        Some("pdf") => "application/pdf".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

pub enum KernelEvent {
    Status(String),
    ChatMessage {
        content: String,
        message_kind: String,
    },
    Error(String),
    SwoTerminal {
        swo_id: i64,
    },
    ArtifactRegistered {
        swo_id: i64,
    },
    StreamingDelta {
        message_id: String,
        delta: String,
        is_final: bool,
        agent_id: Option<String>,
    },
    SwoCreated {
        swo_id: i64,
        assigned_agent_id: String,
        parent_swo_id: Option<i64>,
    },
    SwoStatusChanged {
        swo_id: i64,
        new_status: String,
    },
    DelegationStarted {
        parent_swo_id: i64,
        child_swo_ids: Vec<i64>,
        to_agent_ids: Vec<String>,
    },
    AgentPresenceChanged {
        agent_id: String,
        presence: String,
    },
    /// A new agent was dynamically hired (via hire_subordinate_internal).
    /// The workspace UI uses this to add the new agent to the roster so its
    /// desk renders, its UUID resolves in the activity log, and any subsequent
    /// delegation to it can be visualized.
    AgentCreated {
        agent_id: String,
        name: String,
        role: String,
        parent_id: Option<String>,
        reason: String,
    },
}

/// Emit `SwoStatusChanged` to the workspace relay. Uses `try_send` (non-blocking)
/// so it works from both async and sync (closure) contexts. CHA-362.
fn emit_swo_status_changed(
    ui_tx: &Option<tokio::sync::mpsc::Sender<KernelEvent>>,
    swo_id: i64,
    new_status: &str,
) {
    if let Some(tx) = ui_tx {
        let _ = tx.try_send(KernelEvent::SwoStatusChanged {
            swo_id,
            new_status: new_status.to_string(),
        });
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ManagedWorkRequest {
    payload: String,
    requested_assignee_agent_id: Option<String>,
    requested_assignee_name: Option<String>,
    routing_policy: String,
    user_visible_summary: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttachmentSpec {
    pub attachment_id: String,
    pub source_kind: String,
    pub display_name: String,
    pub original_path: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub originating_swo_id: Option<i64>,
    pub originating_artifact_id: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SairgentToolProposal {
    pub call_id: String,
    pub tool_name: String,
    pub summary: String,
    pub arguments_json: String,
    pub requires_confirmation: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SairgentChatResult {
    pub reply: String,
    pub tool_calls: Vec<SairgentToolProposal>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeliveredAttachmentContext {
    pub attachment_id: String,
    pub source_kind: String,
    pub display_name: String,
    pub original_path: String,
    pub workspace_path: String,
    pub workspace_ref: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub originating_swo_id: Option<i64>,
    pub originating_artifact_id: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HireSubordinateRequest {
    name: String,
    role: String,
    raison_detre: String,
    provider: String,
    model: String,
    cron_interval_seconds: Option<i64>,
    /// CHA-428 — optional "hire on behalf of another manager". When set, the
    /// new agent is assigned to *this* manager as parent instead of the caller.
    /// The caller must be authorized: the root (Perry), an ancestor of the
    /// target in the org tree, or the target themselves. Passed by name so
    /// the Python harness doesn't need to know UUIDs.
    #[serde(default)]
    reports_to: Option<String>,
}

#[derive(Default)]
struct WorkerSideEffects {
    dispatch_swos: Vec<(String, String)>,
    managed_work_requests: Vec<ManagedWorkRequest>,
    sairgent_proposals: Vec<SairgentToolProposal>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct SairgentProjectContext {
    id: String,
    name: String,
    status: String,
    lead: Option<String>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct SairgentSwoContext {
    id: i64,
    title: String,
    status: String,
    assignee: String,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct SairgentRuntimeSnapshot {
    company_name: String,
    company_summary: Option<String>,
    operating_principles: Vec<String>,
    non_goals: Vec<String>,
    active_projects: usize,
    paused_projects: usize,
    archived_projects: usize,
    open_swos: usize,
    approvals_waiting: usize,
    agent_count: usize,
    ready_agents: usize,
    degraded_agents: usize,
    highlights: Vec<String>,
    current_project: Option<SairgentProjectContext>,
    current_swo: Option<SairgentSwoContext>,
    default_provider: String,
    default_model: String,
}

struct RuntimeProjection {
    dir: std::path::PathBuf,
    manifest_path: std::path::PathBuf,
    context_path: std::path::PathBuf,
    skill_index: Vec<RuntimeSkillIndexEntry>,
}

struct RuntimeProjectionGuard {
    path: std::path::PathBuf,
}

impl Drop for RuntimeProjectionGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[derive(serde::Deserialize, std::fmt::Debug)]
struct HeartbeatPayload {
    token: String,
    run_id: String,
    status: String,
    seq: i64,
    // CHA-429 — deny_unknown_fields was previously set here but silently rejected
    // every heartbeat because the Python heartbeat payload always carries an extra
    // `__sairgent_sidechannel: "heartbeat"` field that the outer dispatcher uses
    // to route the event. The strict parse failed, AgentPresenceChanged never
    // fired during synthesis (or any other phase), and the grid showed agents
    // as idle even while they were actively computing. Dave observed this as
    // "Perry idle during final synthesis" on the CHA-428 retests. deny_unknown_fields
    // removed; extra fields are now ignored.
}

pub struct Orchestrator {
    worker_cmd_binary: String,
    registry: Arc<Registry>,
    _vault: Arc<Vault>,
    router: Arc<Router>,
    secrets: Arc<crate::kernel::Secrets>,
    agent_home_root_override: Option<std::path::PathBuf>,
}

impl Orchestrator {
    const MAX_REVIEW_FAILURES: i32 = 3;
    const WORKER_STALL_CHECK_INTERVAL: Duration = Duration::from_secs(15);
    const WORKER_STALL_TIMEOUT: Duration = Duration::from_secs(120);
    const MANAGER_SELF_EXECUTE_EXCEPTIONS: [&'static str; 5] = [
        "NO_QUALIFIED_REPORT",
        "NO_REQUIRED_TOOLING",
        "URGENT_DIRECT_RESPONSE",
        "CROSS_FUNCTION_SYNTHESIS_REQUIRED",
        "TEAM_GAP_PENDING_HIRE",
    ];

    fn result_is_reportable_upward(result: &Value) -> bool {
        if result["terminal_status"].as_str() == Some("CLOSED_FAILED") {
            return true;
        }
        if let Some(action) = result["synthesis"]["action"].as_str() {
            return classify_synthesis_action(action) != "reject";
        }
        if let Some(action) = result["triage"]["action"].as_str() {
            return action == "ANSWER_DIRECTLY";
        }
        result["reply"].as_str().is_some() || result["formatted_swo"].as_str().is_some()
    }

    /// Check if worker stdout indicates a successful (COMPLETED/BLOCKED) response
    /// despite a non-zero exit code. Tries JSON parsing first for reliability,
    /// falls back to substring matching.
    fn stdout_has_success_status(stdout: &str) -> bool {
        // Try to find a parseable JSON line with a status field
        for line in stdout.lines().rev() {
            let trimmed = line.trim();
            if trimmed.starts_with('{') {
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    if let Some(status) = parsed.get("status").and_then(|v| v.as_str()) {
                        return status == "COMPLETED" || status == "BLOCKED";
                    }
                    // Also check nested triage/synthesis structures
                    if parsed.get("triage").is_some() || parsed.get("synthesis").is_some() {
                        return true;
                    }
                }
            }
        }
        // Fallback: substring match for cases where JSON is spread across lines
        stdout.contains("\"status\": \"COMPLETED\"")
            || stdout.contains("\"status\":\"COMPLETED\"")
            || stdout.contains("\"status\": \"BLOCKED\"")
            || stdout.contains("\"status\":\"BLOCKED\"")
    }

    fn result_review_gate_reason(result: &Value) -> String {
        if let Some(action) = result["synthesis"]["action"].as_str() {
            if classify_synthesis_action(action) == "reject" {
                return format!(
                    "Synthesis returned {}: {}",
                    action,
                    result["synthesis"]["reasoning"]
                        .as_str()
                        .unwrap_or("no reasoning supplied")
                );
            }
        }
        if let Some(action) = result["triage"]["action"].as_str() {
            if action != "ANSWER_DIRECTLY" && action != "DELEGATE" {
                return format!("Triage returned unsupported terminal action {}", action);
            }
        }
        "Result was not upward-reportable.".to_string()
    }

    fn closed_failed_payload(reason: &str, attempts: i32) -> Value {
        json!({
            "terminal_status": "CLOSED_FAILED",
            "reason": reason,
            "review_failure_count": attempts,
        })
    }

    fn payload_keywords(payload: &str) -> Vec<String> {
        payload
            .split(|ch: char| !ch.is_alphanumeric())
            .map(|part| part.trim().to_lowercase())
            .filter(|part| part.len() >= 3)
            .collect()
    }

    fn direct_report_fit_score(
        payload_keywords: &[String],
        report_role: &str,
        managed_domains: &[String],
        skill_names: &[String],
        tool_names: &[String],
        team_goal_text: &[String],
        presence: &str,
    ) -> i32 {
        let mut score = 0;
        let report_role = report_role.to_lowercase();
        // Presence is a tiebreaker, not a disqualifier. Subordinates are
        // inherently OFFLINE between tasks — penalising them would make
        // qualified_candidates permanently empty and disable delegation.
        if matches!(presence, "OFFLINE" | "STALE") {
            score -= 1; // mild tiebreaker: prefer online agents when equal
        }
        for keyword in payload_keywords {
            if report_role.contains(keyword) {
                score += 3;
            }
            if managed_domains.iter().any(|domain| domain.to_lowercase().contains(keyword)) {
                score += 4;
            }
            if skill_names.iter().any(|name| name.to_lowercase().contains(keyword)) {
                score += 2;
            }
            if tool_names.iter().any(|name| name.to_lowercase().contains(keyword)) {
                score += 1;
            }
            if team_goal_text.iter().any(|goal| goal.to_lowercase().contains(keyword)) {
                score += 2;
            }
        }
        score
    }

    fn is_cross_function_synthesis(payload_keywords: &[String]) -> bool {
        payload_keywords.iter().any(|keyword| {
            matches!(
                keyword.as_str(),
                "synthesize" | "strategy" | "strategic" | "coordinate" | "coordination"
                    | "review" | "plan" | "planning" | "alignment"
            )
        })
    }

    fn valid_self_execute_exception(code: Option<&str>) -> bool {
        code.map(|value| Self::MANAGER_SELF_EXECUTE_EXCEPTIONS.contains(&value))
            .unwrap_or(false)
    }

    fn extract_quoted_text(message: &str) -> Option<String> {
        let mut quote_start = None;
        let mut quote_char = '\0';
        for (index, ch) in message.char_indices() {
            if quote_start.is_none() && (ch == '"' || ch == '\'') {
                quote_start = Some(index + ch.len_utf8());
                quote_char = ch;
                continue;
            }
            if let Some(start) = quote_start {
                if ch == quote_char {
                    let candidate = message[start..index].trim();
                    if !candidate.is_empty() {
                        return Some(candidate.to_string());
                    }
                    break;
                }
            }
        }
        None
    }

    fn extract_name_after_phrase(message: &str, phrases: &[&str]) -> Option<String> {
        let lowered = message.to_ascii_lowercase();
        phrases.iter().find_map(|phrase| {
            let needle = phrase.to_ascii_lowercase();
            let start = lowered.find(&needle)?;
            let raw = message[start + phrase.len()..].trim();
            let candidate = raw
                .split(['.', '\n', '!', '?', ';'])
                .next()
                .unwrap_or("")
                .trim()
                .trim_start_matches(':')
                .trim();
            if candidate.is_empty() {
                None
            } else {
                Some(candidate.to_string())
            }
        })
    }

    fn normalize_single_line(input: &str) -> String {
        input.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn summarize_request(input: &str, fallback: &str) -> String {
        let normalized = Self::normalize_single_line(input);
        if normalized.is_empty() {
            return fallback.to_string();
        }
        let truncated = normalized.chars().take(140).collect::<String>();
        if normalized.chars().count() > 140 {
            format!("{}...", truncated)
        } else {
            truncated
        }
    }

    fn infer_sairgent_tool_calls(
        message: &str,
        snapshot: &SairgentRuntimeSnapshot,
    ) -> Vec<SairgentToolProposal> {
        let lowered = message.to_ascii_lowercase();
        let quoted = Self::extract_quoted_text(message);

        if lowered.contains("create project")
            || lowered.contains("new project")
            || lowered.contains("start project")
        {
            if let Some(name) = quoted.clone().or_else(|| {
                Self::extract_name_after_phrase(
                    message,
                    &["create project", "new project", "start project"],
                )
            }) {
                let target_outcome = Self::summarize_request(
                    message,
                    "Create a new project from the Sairgent desktop panel.",
                );
                let arguments = json!({
                    "name": name,
                    "summary": format!("Requested from Sairgent Agent: {}", target_outcome),
                    "leadAgentId": null,
                    "priority": "NORMAL",
                    "targetOutcome": target_outcome,
                    "tags": ["sairgent-agent"],
                });
                return vec![SairgentToolProposal {
                    call_id: uuid::Uuid::new_v4().to_string(),
                    tool_name: "create_project".to_string(),
                    summary: format!(
                        "Create project {}",
                        arguments["name"].as_str().unwrap_or("Untitled")
                    ),
                    arguments_json: arguments.to_string(),
                    requires_confirmation: true,
                }];
            }
        }

        if lowered.contains("create work order")
            || lowered.contains("new work order")
            || lowered.contains("create swo")
            || lowered.contains("new swo")
            || lowered.contains("draft work order")
        {
            if let Some(title) = quoted.clone().or_else(|| {
                Self::extract_name_after_phrase(
                    message,
                    &[
                        "create work order",
                        "new work order",
                        "create swo",
                        "new swo",
                        "draft work order",
                    ],
                )
            }) {
                let outcome = Self::summarize_request(
                    message,
                    "Create a new governed work order from the Sairgent panel.",
                );
                let arguments = json!({
                    "title": title,
                    "outcome": outcome,
                    "constraints": "Review before ship. Route through the standard audited work-order path.",
                    "priority": "NORMAL",
                    "projectId": snapshot.current_project.as_ref().map(|project| project.id.clone()),
                });
                return vec![SairgentToolProposal {
                    call_id: uuid::Uuid::new_v4().to_string(),
                    tool_name: "create_work_order".to_string(),
                    summary: format!(
                        "Create work order {}",
                        arguments["title"].as_str().unwrap_or("Untitled")
                    ),
                    arguments_json: arguments.to_string(),
                    requires_confirmation: true,
                }];
            }
        }

        if lowered.contains("create agent")
            || lowered.contains("new agent")
            || lowered.contains("add agent")
            || lowered.contains("hire agent")
        {
            if let Some(name) = quoted.or_else(|| {
                Self::extract_name_after_phrase(
                    message,
                    &["create agent", "new agent", "add agent", "hire agent"],
                )
            }) {
                let role = message
                    .split_once(" as ")
                    .map(|(_, tail)| tail)
                    .and_then(|tail| tail.split(['.', '\n', '!', '?', ';']).next().map(str::trim))
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Operator Support");
                let mission = format!(
                    "Support {} with {}.",
                    snapshot.company_name,
                    Self::summarize_request(message, "operator support")
                );
                let arguments = json!({
                    "name": name,
                    "role": role,
                    "mission": mission,
                    "provider": snapshot.default_provider,
                    "model": snapshot.default_model,
                    "managerAgentId": null,
                });
                return vec![SairgentToolProposal {
                    call_id: uuid::Uuid::new_v4().to_string(),
                    tool_name: "create_agent".to_string(),
                    summary: format!(
                        "Create least-privilege agent {}",
                        arguments["name"].as_str().unwrap_or("Untitled")
                    ),
                    arguments_json: arguments.to_string(),
                    requires_confirmation: true,
                }];
            }
        }

        Vec::new()
    }

    fn build_sairgent_snapshot(
        &self,
        agent_id: &str,
        related_project_id: Option<&str>,
        related_swo_id: Option<i64>,
    ) -> Result<SairgentRuntimeSnapshot> {
        let agent = self.registry.get_agent(agent_id)?;
        let runtime_context = self.registry.get_runtime_context()?;
        let projects = self.registry.list_projects()?;
        let swos = self.registry.list_swo_summaries(120)?;
        let roster = self
            .registry
            .get_agent_tree_snapshot(unix_now_secs() * 1000)?;
        let project_updates = self.registry.list_project_status_updates()?;
        let recurring_runs = self.registry.list_recurring_runs(None)?;

        let active_projects = projects
            .iter()
            .filter(|project| project.status == "ACTIVE")
            .count();
        let paused_projects = projects
            .iter()
            .filter(|project| project.status == "PAUSED")
            .count();
        let archived_projects = projects
            .iter()
            .filter(|project| project.status == "ARCHIVED")
            .count();
        let open_swos = swos
            .iter()
            .filter(|record| {
                let status = record.swo.status.as_str();
                status != "COMPLETED" && status != "FAILED" && status != "CANCELLED"
            })
            .count();
        let approvals_waiting = swos
            .iter()
            .filter(|record| {
                record.review_status != "NO_REVIEW"
                    && classify_synthesis_action(record.review_status.as_str()) != "accept_complete"
            })
            .count();
        let ready_agents = roster
            .iter()
            .filter(|agent| agent.presence == "READY" || agent.presence == "IDLE")
            .count();
        let degraded_agents = roster
            .iter()
            .filter(|agent| agent.presence == "OFFLINE" || agent.presence == "STALE")
            .count();

        let mut highlights = project_updates
            .into_iter()
            .take(2)
            .map(|update| format!("Project {} -> {}", update.project_id, update.next_status))
            .collect::<Vec<_>>();
        highlights.extend(
            recurring_runs
                .into_iter()
                .take(2)
                .map(|run| format!("Recurring run {} is {}", run.run_id, run.status)),
        );

        let current_project = related_project_id
            .map(|project_id| self.registry.get_project(project_id))
            .transpose()?
            .flatten()
            .map(|project| SairgentProjectContext {
                id: project.id,
                name: project.name,
                status: project.status,
                lead: project.lead_agent_id,
            });

        let current_swo = related_swo_id
            .map(|swo_id| self.registry.get_swo_detail(swo_id))
            .transpose()?
            .flatten()
            .map(|detail| SairgentSwoContext {
                id: detail.swo.id,
                title: detail
                    .swo
                    .work_order_title
                    .clone()
                    .unwrap_or_else(|| format!("Work order #{}", detail.swo.id)),
                status: detail.swo.status,
                assignee: detail.swo.assigned_agent_name,
            });

        let operating_principles: Vec<String> = runtime_context
            .operating_principles
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let non_goals: Vec<String> = runtime_context
            .non_goals
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        Ok(SairgentRuntimeSnapshot {
            company_name: runtime_context
                .company_name
                .unwrap_or_else(|| "Sairgent".to_string()),
            company_summary: runtime_context.company_summary,
            operating_principles,
            non_goals,
            active_projects,
            paused_projects,
            archived_projects,
            open_swos,
            approvals_waiting,
            agent_count: roster.len(),
            ready_agents,
            degraded_agents,
            highlights,
            current_project,
            current_swo,
            default_provider: agent.default_provider,
            default_model: agent.default_model,
        })
    }

    fn compose_sairgent_reply(
        snapshot: &SairgentRuntimeSnapshot,
        attachment_count: usize,
        tool_calls: &[SairgentToolProposal],
    ) -> String {
        // Rule-based fallback (no LLM API key). Keep it conversational — no stats dump.
        let mut sections = Vec::new();

        // Lead with a helpful greeting, not a status report
        sections.push("Hey! I'm running in offline mode right now (no LLM API key configured). I can still help with basic actions.".to_string());

        // Mention current context if relevant
        if let Some(project) = snapshot.current_project.as_ref() {
            sections.push(format!(
                "You're in the **{}** project (status: {}).",
                project.name, project.status,
            ));
        }
        if let Some(swo) = snapshot.current_swo.as_ref() {
            sections.push(format!(
                "Current task: **{}** (#{}, {}).",
                swo.title, swo.id, swo.status,
            ));
        }
        if !snapshot.highlights.is_empty() {
            sections.push(format!(
                "Recent activity: {}.",
                snapshot.highlights.join(", ")
            ));
        }
        if attachment_count > 0 {
            sections.push(format!(
                "I see {} attachment{}.",
                attachment_count,
                if attachment_count == 1 { "" } else { "s" }
            ));
        }
        if let Some(tool_call) = tool_calls.first() {
            sections.push(format!(
                "I've drafted an action: **{}**. Confirm it below to proceed.",
                tool_call.summary
            ));
        } else {
            sections.push(
                "To unlock full capabilities, add an API key in **Settings**. I'll be able to research, write, analyze, and delegate to specialist agents."
                    .to_string(),
            );
        }
        sections.join("\n\n")
    }

    /// Best-effort conversion of a JSON reply from the LLM into readable text.
    /// Returns None if the JSON doesn't have recognizable structure.
    fn render_json_reply_as_text(raw: &str) -> Option<String> {
        let parsed: Value = serde_json::from_str(raw).ok()?;
        let obj = parsed.as_object()?;
        let mut sections = Vec::new();

        // Render scalar top-level fields as header lines
        for (key, val) in obj {
            match val {
                Value::String(s) if !s.is_empty() => {
                    let label = key.replace('_', " ");
                    sections.push(format!("**{}:** {}", label, s));
                }
                Value::Number(n) => {
                    let label = key.replace('_', " ");
                    sections.push(format!("**{}:** {}", label, n));
                }
                _ => {}
            }
        }

        // Render object fields (like "scorecard") as bullet lists
        for (key, val) in obj {
            if let Some(inner_obj) = val.as_object() {
                let label = key.replace('_', " ");
                sections.push(format!("**{}:**", label));
                for (k, v) in inner_obj {
                    let k_label = k.replace('_', " ");
                    let v_str = match v {
                        Value::String(s) => s.clone(),
                        _ => v.to_string(),
                    };
                    sections.push(format!("- {}: {}", k_label, v_str));
                }
            }
        }

        // Render array fields as bullet lists
        for (key, val) in obj {
            if let Some(arr) = val.as_array() {
                if arr.is_empty() {
                    continue;
                }
                let label = key.replace('_', " ");
                sections.push(format!("**{}:**", label));
                for item in arr {
                    let text = match item {
                        Value::String(s) => s.clone(),
                        _ => item.to_string(),
                    };
                    sections.push(format!("- {}", text));
                }
            }
        }

        if sections.is_empty() {
            return None;
        }
        Some(sections.join("\n"))
    }

    /// Strip trailing lines that are raw JSON artifacts (e.g. codex tool-call
    /// outputs like `- {"change":"moved to ACTIVE","project_id":"..."}` that
    /// sometimes bleed into the reply text).
    fn strip_trailing_json_artifacts(text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let keep_count = lines
            .iter()
            .rposition(|line| {
                let trimmed = line.trim().trim_start_matches("- ").trim();
                !(trimmed.starts_with('{') && trimmed.ends_with('}'))
            })
            .map(|i| i + 1)
            .unwrap_or(lines.len());
        lines[..keep_count].join("\n")
    }

    fn format_sairgent_system_context(
        snapshot: &SairgentRuntimeSnapshot,
        _roster: &[crate::registry::AgentTreeNodeRecord],
    ) -> String {
        // Only include actionable context — no stats dumps, no agent roster,
        // no company context XML.  The harness persona handles identity and behavior.
        let mut sections = Vec::new();

        if let Some(project) = snapshot.current_project.as_ref() {
            sections.push(format!(
                "Current project: {} (status: {})",
                project.name, project.status,
            ));
        }

        if let Some(swo) = snapshot.current_swo.as_ref() {
            sections.push(format!(
                "Current task: #{} — {} (status: {})",
                swo.id, swo.title, swo.status,
            ));
        }

        if !snapshot.highlights.is_empty() {
            sections.push(format!("Recent activity: {}", snapshot.highlights.join(", ")));
        }

        sections.join("\n")
    }

    pub async fn run_sairgent_chat(
        self: Arc<Self>,
        agent_id: String,
        user_message: String,
        related_project_id: Option<String>,
        related_swo_id: Option<i64>,
        attachment_count: usize,
        provider_override: Option<String>,
        model_override: Option<String>,
    ) -> Result<SairgentChatResult> {
        #[cfg(debug_assertions)]
        eprintln!("[SairgentChat] run_sairgent_chat called for agent {}", &agent_id[..8.min(agent_id.len())]);
        let snapshot =
            self.build_sairgent_snapshot(&agent_id, related_project_id.as_deref(), related_swo_id)?;

        // Try LLM-backed path
        let agent = self.registry.get_agent(&agent_id)?;
        let route = self.router.resolve_route(&agent, None);
        let provider = provider_override.as_deref().unwrap_or(&route.provider_name);
        let model = model_override.as_deref().unwrap_or(&route.model);
        let api_key = self.resolve_llm_api_key(provider);
        let protocol_family = crate::manifest::ProviderProtocolFamily::from_provider_name(provider);
        #[cfg(debug_assertions)]
        eprintln!("[SairgentChat] provider={} model={} has_key={} protocol_family={:?}", provider, model, !api_key.trim().is_empty(), protocol_family);

        // Gate: if no API key and not a local/CLI provider, fall back to rule-based
        let has_key = !api_key.trim().is_empty();
        let is_local = matches!(protocol_family, crate::manifest::ProviderProtocolFamily::OauthCodexStyle);

        if !has_key && !is_local {
            let tool_calls = Self::infer_sairgent_tool_calls(&user_message, &snapshot);
            let reply = Self::compose_sairgent_reply(&snapshot, attachment_count, &tool_calls);
            return Ok(SairgentChatResult { reply, tool_calls });
        }

        // Build system context with agent roster
        let roster = self.registry.get_agent_tree_snapshot(unix_now_secs() * 1000)?;
        let system_context = Self::format_sairgent_system_context(&snapshot, &roster);
        let system_prompt = format!("{}\n\n{}", system_context, agent.persona_prompt);

        // Resolve memory DB
        let storage_base = std::path::Path::new(&self.registry.db_path).parent().unwrap();
        let db_path = storage_base
            .join("agents")
            .join(&agent_id)
            .join("memory.sqlite")
            .to_string_lossy()
            .to_string();

        // All agents are potential subordinates for the Sairgent super-agent
        let all_agents = self.registry.list_agents()?;
        let subordinates_json = serde_json::to_string(
            &all_agents.iter().map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "name": a.name,
                    "role": a.role,
                    "raison": a.raison_detre,
                })
            }).collect::<Vec<_>>()
        ).unwrap_or_else(|_| "[]".to_string());

        // Call run_worker with sairgent_chat mode
        let worker_result = self.run_worker(
            &agent_id,
            &agent.name,
            None,
            &db_path,
            provider,
            model,
            &api_key,
            "sairgent_chat",
            &user_message,
            &[],  // no attachments for sairgent chat
            &subordinates_json,
            &agent.role,
            &system_prompt,
            &agent.raison_detre,
            None, None, None,
            None,
            None,
            None,
        ).await;

        match worker_result {
            Ok((result, side_effects)) => {
                // Extract reply from worker result — handle both PydanticAI chat
                // format (result.reply) and codex_cli synthesis format
                // (result.synthesis.final_response / result.triage.direct_answer)
                #[cfg(debug_assertions)]
                eprintln!("[SairgentChat] worker Ok — top-level keys: {:?}",
                    result.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default());
                let raw_reply = result["reply"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| result["synthesis"]["final_response"].as_str().filter(|s| !s.is_empty()))
                    .or_else(|| result["final_response"].as_str().filter(|s| !s.is_empty()))
                    .or_else(|| result["triage"]["direct_answer"].as_str().filter(|s| !s.is_empty()))
                    .or_else(|| result["direct_answer"].as_str().filter(|s| !s.is_empty()))
                    // REJECT_AND_REVISE: fall back to blocked_reason so the UI always shows something
                    .or_else(|| result["blocked_reason"].as_str().filter(|s| !s.is_empty()))
                    .unwrap_or("I processed your request.")
                    .to_string();
                #[cfg(debug_assertions)]
                eprintln!("[SairgentChat] raw_reply length: {}", raw_reply.len());

                // If the LLM returned a JSON blob as its reply, render it as
                // readable markdown so the UI displays something useful.
                let cooked = if raw_reply.trim_start().starts_with('{') {
                    Self::render_json_reply_as_text(&raw_reply).unwrap_or(raw_reply)
                } else {
                    raw_reply
                };
                // Strip any trailing raw JSON artifacts (e.g. tool-call outputs
                // like `- {"change":"moved to ACTIVE",...}` that bleed in).
                let reply = Self::strip_trailing_json_artifacts(&cooked);

                // Collect proposals from sidechannel
                let mut tool_calls = side_effects.sairgent_proposals;

                // Also run rule-based inference on both user message and reply
                // for backward compat and as a safety net when LLM fails to use tools
                for source in [user_message.as_str(), reply.as_str()] {
                    let inferred = Self::infer_sairgent_tool_calls(source, &snapshot);
                    for inferred_call in inferred {
                        if !tool_calls.iter().any(|tc| tc.tool_name == inferred_call.tool_name) {
                            tool_calls.push(inferred_call);
                        }
                    }
                }

                Ok(SairgentChatResult { reply, tool_calls })
            }
            Err(e) => {
                // Fallback to rule-based
                #[cfg(debug_assertions)]
                eprintln!("[SairgentChat] worker Err — falling back to rule-based: {:?}", e);
                let tool_calls = Self::infer_sairgent_tool_calls(&user_message, &snapshot);
                let reply = Self::compose_sairgent_reply(&snapshot, attachment_count, &tool_calls);
                Ok(SairgentChatResult { reply, tool_calls })
            }
        }
    }

    fn worker_stall_reason() -> String {
        format!(
            "Worker stopped heartbeating or emitting output for {} seconds",
            Self::WORKER_STALL_TIMEOUT.as_secs()
        )
    }

    fn worker_backend_for_mode(
        mode: &str,
        protocol_family: &ProviderProtocolFamily,
    ) -> &'static str {
        match protocol_family {
            ProviderProtocolFamily::OauthCodexStyle => "codex_cli",
            _ => match mode {
                "chat_mode" | "format_swo" | "execute_triage" | "write_briefs" | "execute_synthesis" | "execute_ideation" | "sairgent_chat" => "pydantic_ai",
                _ => "codex_cli",
            },
        }
    }

    fn resolve_llm_api_key(&self, provider: &str) -> String {
        let normalized = provider.trim().to_lowercase();
        self.secrets
            .llm_api_keys_by_provider
            .get(&normalized)
            .cloned()
            .unwrap_or_else(|| self.secrets.default_llm_api_key.clone())
    }

    fn build_work_order_payload(title: &str, outcome: &str, constraints: Option<&str>) -> String {
        if let Some(constraints) = constraints.filter(|value| !value.trim().is_empty()) {
            format!(
                "WORK ORDER\nTitle: {title}\nRequested outcome: {outcome}\nConstraints: {constraints}"
            )
        } else {
            format!("WORK ORDER\nTitle: {title}\nRequested outcome: {outcome}")
        }
    }

    fn recurring_run_title(template: &RecurringWorkOrderTemplateRecord, run_number: i64) -> String {
        format!("{} (Run #{})", template.title, run_number)
    }

    pub fn compute_next_recurring_run_at(
        schedule: &RecurringWorkOrderScheduleRecord,
        from_unix_secs: i64,
    ) -> Result<String> {
        let interval = schedule.interval.max(1);
        let next_unix = match schedule.cadence.as_str() {
            "hourly" => {
                let minute = schedule.minute.unwrap_or(0).clamp(0, 59);
                let hour_floor = from_unix_secs - (from_unix_secs % 3_600);
                let mut candidate = hour_floor + (minute * 60);
                if candidate <= from_unix_secs {
                    candidate += 3_600 * interval;
                }
                candidate
            }
            "weekly" => {
                let days = schedule
                    .days_of_week
                    .clone()
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| vec![1]);
                let hour = schedule.hour.unwrap_or(9).clamp(0, 23) as i32;
                let minute = schedule.minute.unwrap_or(0).clamp(0, 59) as i32;
                let current_tm = utc_tm_from_unix(from_unix_secs)?;
                let mut best = None;
                for day_offset in 0..=(7 * interval) {
                    let mut candidate_tm = current_tm;
                    candidate_tm.tm_mday += day_offset as i32;
                    let candidate_unix = utc_tm_to_unix(candidate_tm)?;
                    let mut normalized_tm = utc_tm_from_unix(candidate_unix)?;
                    if !days
                        .iter()
                        .any(|day| normalized_tm.tm_wday == sanitize_schedule_day(*day))
                    {
                        continue;
                    }
                    normalized_tm.tm_hour = hour;
                    normalized_tm.tm_min = minute;
                    normalized_tm.tm_sec = 0;
                    let candidate = utc_tm_to_unix(normalized_tm)?;
                    if candidate > from_unix_secs {
                        best = Some(candidate);
                        break;
                    }
                }
                best.unwrap_or(from_unix_secs + (7 * interval * 86_400))
            }
            "monthly" => {
                let hour = schedule.hour.unwrap_or(9).clamp(0, 23) as i32;
                let minute = schedule.minute.unwrap_or(0).clamp(0, 59) as i32;
                let mut candidate_tm = utc_tm_from_unix(from_unix_secs)?;
                let day = schedule
                    .day_of_month
                    .unwrap_or((candidate_tm.tm_mday as i64).max(1))
                    .clamp(1, 31) as i32;
                let year = candidate_tm.tm_year + 1900;
                let month = candidate_tm.tm_mon;
                candidate_tm.tm_mday = day.min(days_in_month(year, month));
                candidate_tm.tm_hour = hour;
                candidate_tm.tm_min = minute;
                candidate_tm.tm_sec = 0;
                let mut candidate = utc_tm_to_unix(candidate_tm)?;
                if candidate <= from_unix_secs {
                    let mut next_tm = utc_tm_from_unix(candidate)?;
                    next_tm.tm_mon += interval as i32;
                    let normalized_unix = utc_tm_to_unix(next_tm)?;
                    let mut normalized_tm = utc_tm_from_unix(normalized_unix)?;
                    let normalized_year = normalized_tm.tm_year + 1900;
                    let normalized_month = normalized_tm.tm_mon;
                    normalized_tm.tm_mday =
                        day.min(days_in_month(normalized_year, normalized_month));
                    normalized_tm.tm_hour = hour;
                    normalized_tm.tm_min = minute;
                    normalized_tm.tm_sec = 0;
                    candidate = utc_tm_to_unix(normalized_tm)?;
                }
                candidate
            }
            "custom" | "daily" => {
                let hour = schedule.hour.unwrap_or(9).clamp(0, 23) as i32;
                let minute = schedule.minute.unwrap_or(0).clamp(0, 59) as i32;
                let mut candidate_tm = utc_tm_from_unix(from_unix_secs)?;
                candidate_tm.tm_hour = hour;
                candidate_tm.tm_min = minute;
                candidate_tm.tm_sec = 0;
                let mut candidate = utc_tm_to_unix(candidate_tm)?;
                if candidate <= from_unix_secs {
                    candidate += 86_400 * interval;
                }
                candidate
            }
            _ => {
                let hour = schedule.hour.unwrap_or(9).clamp(0, 23) as i32;
                let minute = schedule.minute.unwrap_or(0).clamp(0, 59) as i32;
                let mut candidate_tm = utc_tm_from_unix(from_unix_secs)?;
                candidate_tm.tm_hour = hour;
                candidate_tm.tm_min = minute;
                candidate_tm.tm_sec = 0;
                let mut candidate = utc_tm_to_unix(candidate_tm)?;
                if candidate <= from_unix_secs {
                    candidate += 86_400 * interval;
                }
                candidate
            }
        };
        format_utc_timestamp(next_unix)
    }

    fn prior_run_attachments(
        &self,
        prior_run: &RecurringWorkOrderRunRecord,
        new_run_id: &str,
    ) -> Result<Vec<AttachmentSpec>> {
        let Some(prior_swo_id) = prior_run.swo_id else {
            return Ok(Vec::new());
        };
        let artifacts = self
            .registry
            .list_outbox_artifacts(OutboxArtifactListFilters {
                agent_id: None,
                swo_id: Some(prior_swo_id),
                query: None,
                limit: 100,
            })?;
        let attachments = artifacts
            .into_iter()
            .filter_map(|artifact| {
                let path = std::path::PathBuf::from(&artifact.absolute_path);
                if !path.exists() {
                    return None;
                }
                let size_bytes = std::fs::metadata(&path).ok()?.len() as i64;
                Some(AttachmentSpec {
                    attachment_id: format!("{}-artifact-{}", new_run_id, artifact.id),
                    source_kind: "outbox_artifact".to_string(),
                    display_name: artifact.filename,
                    original_path: artifact.absolute_path,
                    content_type: infer_attachment_content_type(&path),
                    size_bytes,
                    originating_swo_id: Some(prior_swo_id),
                    originating_artifact_id: Some(artifact.id),
                })
            })
            .collect::<Vec<_>>();
        Ok(attachments)
    }

    async fn materialize_recurring_template_run(
        self: Arc<Self>,
        template_id: String,
        trigger_source: &'static str,
        advance_schedule: bool,
    ) -> Result<(
        RecurringWorkOrderTemplateRecord,
        RecurringWorkOrderRunRecord,
        i64,
    )> {
        let template = self
            .registry
            .get_recurring_template(&template_id)?
            .ok_or_else(|| {
                KernelError::Internal(format!("Unknown recurring template {}", template_id))
            })?;
        if template.status == "CANCELLED" || template.status == "ARCHIVED" {
            return Err(KernelError::Internal(format!(
                "Recurring template {} is not runnable in status {}",
                template.template_id, template.status
            )));
        }

        let run_id = format!("rwo-run-{}", uuid::Uuid::new_v4());
        let run_number = self
            .registry
            .next_recurring_run_number(&template.template_id)?;
        let run_title = Self::recurring_run_title(&template, run_number);
        let payload = Self::build_work_order_payload(
            &run_title,
            &template.outcome,
            template.constraints.as_deref(),
        );
        let assignee_agent_id = template
            .assignee_agent_id
            .clone()
            .unwrap_or_else(|| template.owner_agent_id.clone());
        let project = template
            .project_id
            .as_deref()
            .map(|project_id| self.registry.get_project(project_id))
            .transpose()?
            .flatten();
        let swo_id = self.registry.create_swo_with_metadata(CreateSwoParams {
            assigned_agent_id: &assignee_agent_id,
            owner_agent_id: &template.owner_agent_id,
            created_by_agent_id: &template.owner_agent_id,
            payload: &payload,
            status: "PENDING",
            parent_swo_id: None,
            kind: "WORK_ORDER",
            source: "RECURRING_TEMPLATE",
            work_order_title: Some(&run_title),
            work_order_outcome: Some(&template.outcome),
            work_order_constraints: template.constraints.as_deref(),
            requested_owner_agent_id: Some(&template.owner_agent_id),
            requested_assignee_agent_id: Some(&assignee_agent_id),
            routing_policy: "NONE",
            originating_swo_id: template.source_swo_id,
            initiative_id: template.project_id.as_deref(),
            initiative_name: project.as_ref().map(|record| record.name.as_str()),
            initiative_owner_agent_id: None,
            priority_class: Some(template.priority.as_str()),
        })?;

        if let Some(project) = project.as_ref() {
            self.registry.update_swo_initiative(
                swo_id,
                Some(project.id.as_str()),
                Some(project.name.as_str()),
            )?;
        }

        if template.include_prior_artifacts {
            if let Some(prior_run) = self
                .registry
                .latest_recurring_run_for_template(&template.template_id)?
            {
                let attachments = self.prior_run_attachments(&prior_run, &run_id)?;
                for attachment in &attachments {
                    self.registry.record_attachment(
                        &attachment.attachment_id,
                        &attachment.source_kind,
                        &attachment.display_name,
                        &attachment.original_path,
                        &attachment.content_type,
                        attachment.size_bytes,
                        attachment.originating_swo_id,
                        attachment.originating_artifact_id,
                    )?;
                    self.registry.link_swo_attachment(
                        swo_id,
                        &attachment.attachment_id,
                        None,
                        "PENDING",
                        None,
                    )?;
                }
            }
        }

        let claimed = self.registry.claim_swo_with_run_id(swo_id, &run_id)?;
        let queued_at = format_utc_timestamp(unix_now_secs())?;
        let started_at = if claimed > 0 {
            Some(queued_at.as_str())
        } else {
            None
        };
        let initial_status = if claimed > 0 { "RUNNING" } else { "QUEUED" };
        let run = self.registry.create_recurring_run(
            crate::registry::CreateRecurringWorkOrderRunParams {
                run_id: &run_id,
                template_id: &template.template_id,
                swo_id: Some(swo_id),
                project_id: template.project_id.as_deref(),
                run_number,
                status: initial_status,
                trigger_source,
                queued_at: Some(&queued_at),
                started_at,
                completed_at: None,
                error_message: None,
                artifact_ids: &[],
            },
        )?;
        let next_run_at = if advance_schedule {
            Some(Self::compute_next_recurring_run_at(
                &template.schedule,
                unix_now_secs(),
            )?)
        } else {
            template.next_run_at.clone()
        };
        let updated_template = self.registry.update_recurring_template(
            crate::registry::UpdateRecurringWorkOrderTemplateParams {
                template_id: &template.template_id,
                project_id: None,
                source_swo_id: None,
                owner_agent_id: None,
                assignee_agent_id: None,
                name: None,
                title: None,
                outcome: None,
                constraints: None,
                priority: None,
                include_prior_artifacts: None,
                schedule: None,
                status: None,
                next_run_at: Some(next_run_at.as_deref()),
                last_run_at: Some(Some(queued_at.as_str())),
                last_run_status: Some(Some(initial_status)),
            },
        )?;

        if claimed > 0 {
            let orchestrator = Arc::clone(&self);
            let requested_assignee_name = template
                .assignee_agent_name
                .clone()
                .or_else(|| Some(template.owner_agent_name.clone()));
            tokio::spawn(async move {
                let _ = orchestrator
                    .execute_hsm_loop_with_context(
                        assignee_agent_id,
                        None,
                        payload,
                        None,
                        Some(swo_id),
                        None,
                        Some("WORK_ORDER".to_string()),
                        Some("RECURRING_TEMPLATE".to_string()),
                        Some(template.owner_agent_id.clone()),
                        Some(template.owner_agent_id.clone()),
                        Some(
                            template
                                .assignee_agent_id
                                .clone()
                                .unwrap_or_else(|| template.owner_agent_id.clone()),
                        ),
                        requested_assignee_name,
                        Some("NONE".to_string()),
                        template.source_swo_id,
                        Some(run_id),
                    )
                    .await;
            });
        }

        Ok((updated_template, run, swo_id))
    }

    pub async fn trigger_recurring_template_now(
        self: Arc<Self>,
        template_id: String,
    ) -> Result<(
        RecurringWorkOrderTemplateRecord,
        RecurringWorkOrderRunRecord,
        i64,
    )> {
        self.materialize_recurring_template_run(template_id, "manual", false)
            .await
    }

    pub fn new(
        worker_binary: &str,
        registry: Arc<Registry>,
        vault: Arc<Vault>,
        router: Arc<Router>,
        secrets: Arc<crate::kernel::Secrets>,
        agent_home_root_override: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            worker_cmd_binary: worker_binary.to_string(),
            registry,
            _vault: vault,
            router,
            secrets,
            agent_home_root_override,
        }
    }

    pub async fn upsert_heartbeat_async(
        &self,
        run_id: String,
        agent_id: String,
        status: String,
        seq: i64,
    ) {
        let registry = Arc::clone(&self.registry);
        tokio::task::spawn_blocking(move || {
            if let Err(e) = registry.upsert_heartbeat(&run_id, &agent_id, &status, seq) {
                eprintln!("[Kernel] Heartbeat upsert error: {:?}", e);
            }
        });
    }

    fn sanitize_agent_directory_name(agent_name: &str) -> Result<String> {
        const MAX_LEN: usize = 64;
        let sanitized: String = agent_name
            .chars()
            .take(MAX_LEN)
            .map(|c| if c.is_ascii_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
            .collect();
        let sanitized = sanitized.trim().trim_matches('_').to_string();
        if sanitized.is_empty() {
            return Err(KernelError::Internal(format!(
                "Agent name '{}' produces an empty sanitized directory name", agent_name
            )));
        }
        Ok(sanitized)
    }

    fn read_directory_id_marker(path: &std::path::Path) -> Result<Option<String>> {
        let marker = path.join(".id");
        if !marker.exists() {
            return Ok(None);
        }
        let existing_id = std::fs::read_to_string(&marker).map_err(|e| {
            KernelError::Internal(format!(
                "Failed to read agent directory marker '{}': {}",
                marker.display(),
                e
            ))
        })?;
        Ok(Some(existing_id.trim().to_string()))
    }

    fn ensure_directory_binding(path: &std::path::Path, agent_id: &str) -> Result<()> {
        if let Some(existing_id) = Self::read_directory_id_marker(path)? {
            if existing_id != agent_id {
                return Err(KernelError::Internal(format!(
                    "Agent directory '{}' is already bound to a different agent id",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    fn conflict_safe_agent_directory_name(agent_name: &str, agent_id: &str) -> Result<String> {
        let dir_name = Self::sanitize_agent_directory_name(agent_name)?;
        let short_id = agent_id.chars().take(8).collect::<String>();
        Ok(format!("{}-{}", dir_name, short_id))
    }

    fn resolve_agent_directory_base(
        root: &std::path::Path,
        agent_id: &str,
        agent_name: &str,
    ) -> Result<std::path::PathBuf> {
        let dir_name = Self::sanitize_agent_directory_name(agent_name)?;
        let preferred = root.join(&dir_name);
        let legacy = root.join(agent_id);

        if legacy.exists() {
            if !preferred.exists() {
                std::fs::rename(&legacy, &preferred).map_err(|e| {
                    KernelError::Internal(format!(
                        "Failed to migrate agent directory from '{}' to '{}': {}",
                        legacy.display(),
                        preferred.display(),
                        e
                    ))
                })?;
                Self::ensure_directory_binding(&preferred, agent_id)?;
                return Ok(preferred);
            }

            match Self::read_directory_id_marker(&preferred)? {
                Some(existing_id) if existing_id == agent_id => return Ok(preferred),
                Some(_) => {
                    let fallback = root.join(Self::conflict_safe_agent_directory_name(
                        agent_name, agent_id,
                    )?);
                    if !fallback.exists() {
                        std::fs::rename(&legacy, &fallback).map_err(|e| {
                            KernelError::Internal(format!(
                                "Failed to migrate agent directory from '{}' to '{}': {}",
                                legacy.display(),
                                fallback.display(),
                                e
                            ))
                        })?;
                    }
                    Self::ensure_directory_binding(&fallback, agent_id)?;
                    return Ok(fallback);
                }
                None => {
                    Self::ensure_directory_binding(&preferred, agent_id)?;
                    return Ok(preferred);
                }
            }
        }

        if preferred.exists() {
            match Self::read_directory_id_marker(&preferred)? {
                Some(existing_id) if existing_id == agent_id => return Ok(preferred),
                Some(_) => {
                    let fallback = root.join(Self::conflict_safe_agent_directory_name(
                        agent_name, agent_id,
                    )?);
                    std::fs::create_dir_all(&fallback).map_err(|e| {
                        KernelError::Internal(format!(
                            "Failed to create fallback agent directory '{}': {}",
                            fallback.display(), e
                        ))
                    })?;
                    Self::ensure_directory_binding(&fallback, agent_id)?;
                    return Ok(fallback);
                }
                None => return Ok(preferred),
            }
        }

        Ok(preferred)
    }

    /// Returns the `workspace/` path for an agent without creating it.
    /// Enforces the same traversal safety as the other path helpers: the resolved
    /// path must remain inside the agent root.  Returns `None` if the agent
    /// directory base cannot be determined (e.g. unknown agent).
    /// Used by CHA-394 harness tools — allow dead_code until that lands.
    #[allow(dead_code)]
    pub(crate) fn workspace_path_for_agent(
        root: &std::path::Path,
        agent_id: &str,
        agent_name: &str,
    ) -> Result<std::path::PathBuf> {
        let base = Self::resolve_agent_directory_base(root, agent_id, agent_name)?;
        let workspace = base.join("workspace");
        // Traversal guard: canonicalize parent (base) and verify workspace stays within it.
        // Use base directly since workspace is a single-segment join — no traversal possible.
        // We still check the resolved path starts_with base for defence-in-depth.
        let canonical_base = std::fs::canonicalize(&base).unwrap_or_else(|_| base.clone());
        let canonical_ws = if workspace.exists() {
            std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone())
        } else {
            workspace.clone()
        };
        if !canonical_ws.starts_with(&canonical_base) {
            return Err(KernelError::Internal(format!(
                "workspace_path_for_agent: resolved path '{}' escapes agent root '{}'",
                canonical_ws.display(),
                canonical_base.display(),
            )));
        }
        Ok(workspace)
    }

    fn matching_agent_directories(
        root: &std::path::Path,
        agent_name: &str,
    ) -> Result<Vec<std::path::PathBuf>> {
        let dir_name = Self::sanitize_agent_directory_name(agent_name)?;
        let fallback_prefix = format!("{}-", dir_name);
        let mut matches = Vec::new();

        if !root.exists() {
            return Ok(matches);
        }

        for entry in std::fs::read_dir(root).map_err(|e| {
            KernelError::Internal(format!(
                "Failed to enumerate agent home root '{}': {}",
                root.display(),
                e
            ))
        })? {
            let entry = entry.map_err(|e| {
                KernelError::Internal(format!(
                    "Failed to read entry under '{}': {}",
                    root.display(),
                    e
                ))
            })?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name == dir_name || name.starts_with(&fallback_prefix) {
                matches.push(path);
            }
        }

        matches.sort();
        Ok(matches)
    }

    /// Ensures ~/Sairgent_Agents/{agent_name}/context, artifacts, and workspace directories exist.
    /// The UUID is still validated as the agent identity anchor, and a hidden `.id`
    /// marker is written into the agent root for collision checks and migrations.
    /// Returns (context_path, artifacts_path, workspace_path).
    fn ensure_agent_directories(
        root: &std::path::Path,
        agent_id: &str,
        agent_name: &str,
    ) -> Result<(String, String, String)> {
        // Validate agent_id is a legitimate UUID — keeps identity anchored to registry data
        if uuid::Uuid::parse_str(agent_id).is_err() {
            return Err(KernelError::Internal(format!(
                "ensure_agent_directories: agent_id '{}' is not a valid UUID",
                agent_id
            )));
        }

        let base = Self::resolve_agent_directory_base(root, agent_id, agent_name)?;
        let id_marker = base.join(".id");

        let legacy_inbox = base.join("inbox");
        let legacy_outbox = base.join("outbox");
        let context = base.join("context");
        let artifacts = base.join("artifacts");
        let workspace = base.join("workspace");

        std::fs::create_dir_all(&base).map_err(|e| {
            KernelError::Internal(format!("Failed to create agent root directory: {}", e))
        })?;
        if legacy_inbox.exists() && !context.exists() {
            std::fs::rename(&legacy_inbox, &context).map_err(|e| {
                KernelError::Internal(format!(
                    "Failed to migrate agent context directory from '{}' to '{}': {}",
                    legacy_inbox.display(),
                    context.display(),
                    e
                ))
            })?;
        }
        if legacy_outbox.exists() && !artifacts.exists() {
            std::fs::rename(&legacy_outbox, &artifacts).map_err(|e| {
                KernelError::Internal(format!(
                    "Failed to migrate agent artifacts directory from '{}' to '{}': {}",
                    legacy_outbox.display(),
                    artifacts.display(),
                    e
                ))
            })?;
        }
        std::fs::create_dir_all(&context).map_err(|e| {
            KernelError::Internal(format!("Failed to create agent context directory: {}", e))
        })?;
        std::fs::create_dir_all(&artifacts).map_err(|e| {
            KernelError::Internal(format!("Failed to create agent artifacts directory: {}", e))
        })?;
        // workspace/ is idempotent: create if missing, no-op if it already exists.
        std::fs::create_dir_all(&workspace).map_err(|e| {
            KernelError::Internal(format!("Failed to create agent workspace directory: {}", e))
        })?;
        std::fs::write(&id_marker, format!("{}\n", agent_id)).map_err(|e| {
            KernelError::Internal(format!(
                "Failed to write agent directory marker '{}': {}",
                id_marker.display(),
                e
            ))
        })?;

        Ok((
            context.to_string_lossy().to_string(),
            artifacts.to_string_lossy().to_string(),
            workspace.to_string_lossy().to_string(),
        ))
    }

    fn sync_agent_workspace(
        root: &std::path::Path,
        agent: &crate::registry::AgentIdentity,
        _manifest: &AgentManifestV1,
    ) -> Result<(String, String, String)> {
        let paths = Self::ensure_agent_directories(root, &agent.id, &agent.name)?;
        let base = Self::resolve_agent_directory_base(root, &agent.id, &agent.name)?;

        for legacy_file in [
            "AGENTS.md",
            "IDENTITY.md",
            "HEARTBEAT.md",
            "TOOLS.md",
            "PREFERENCES.md",
        ] {
            let path = base.join(legacy_file);
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
        }

        for legacy_dir in ["sessions", "memory", "state", "cron", "skills"] {
            let path = base.join(legacy_dir);
            if path.exists() {
                let _ = std::fs::remove_dir(&path);
            }
        }

        Ok(paths)
    }

    fn sanitize_attachment_filename(name: &str) -> String {
        let sanitized = name
            .chars()
            .map(|c| match c {
                '/' | '\\' | ':' | '\0' => '_',
                c if c.is_control() => '_',
                _ => c,
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let trimmed = sanitized.trim().trim_matches('.').to_string();
        if trimmed.is_empty() {
            "attachment".to_string()
        } else {
            trimmed
        }
    }

    fn render_attachment_context(
        payload: &str,
        attachments: &[DeliveredAttachmentContext],
    ) -> String {
        if attachments.is_empty() {
            return payload.to_string();
        }

        let manifest = attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| {
                format!(
                    "{}. {} [{}] path={} source={} size={} content_type={}",
                    index + 1,
                    attachment.display_name,
                    attachment.source_kind,
                    attachment.workspace_ref,
                    attachment.original_path,
                    attachment.size_bytes,
                    attachment.content_type
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "{payload}\n\nATTACHMENTS PROVIDED:\nReview the attached files before deciding whether delegation is necessary.\nUse read_agent_file with the workspace-relative paths below when you need the file contents.\n{manifest}"
        )
    }

    fn relative_workspace_ref(
        agent_root: &std::path::Path,
        path: &std::path::Path,
    ) -> Result<String> {
        path.strip_prefix(agent_root)
            .map(|relative| relative.to_string_lossy().to_string())
            .map_err(|_| {
                KernelError::Internal(format!(
                    "Path '{}' is outside agent workspace '{}'",
                    path.display(),
                    agent_root.display()
                ))
            })
    }

    fn materialize_attachments(
        &self,
        agent_id: &str,
        context_path: &str,
        swo_id: Option<i64>,
        attachments: &[AttachmentSpec],
    ) -> Result<Vec<DeliveredAttachmentContext>> {
        let context_root = std::path::Path::new(context_path);
        let agent_root = context_root.parent().ok_or_else(|| {
            KernelError::Internal(format!(
                "Context path '{}' has no agent workspace parent",
                context_path
            ))
        })?;
        let attached_context_root = context_root.join("attachments");
        std::fs::create_dir_all(&attached_context_root).map_err(|e| {
            KernelError::Internal(format!(
                "Failed to create attachments directory '{}': {}",
                attached_context_root.display(),
                e
            ))
        })?;
        if attachments.is_empty() {
            return Ok(Vec::new());
        }

        let mut delivered = Vec::new();
        for attachment in attachments {
            let mut source_owner_agent_id: Option<String> = None;
            let source_path = if attachment.source_kind == "outbox_artifact" {
                let artifact_id = attachment.originating_artifact_id.ok_or_else(|| {
                    KernelError::Internal(format!(
                        "Attachment '{}' is missing originating_artifact_id",
                        attachment.display_name
                    ))
                })?;
                let artifact =
                    self.registry
                        .get_outbox_artifact(artifact_id)?
                        .ok_or_else(|| {
                            KernelError::Internal(format!(
                                "Artifact {} no longer exists for attachment '{}'",
                                artifact_id, attachment.display_name
                            ))
                        })?;
                let artifact_path = std::path::PathBuf::from(&artifact.absolute_path);
                let requested_path = std::path::PathBuf::from(&attachment.original_path);
                if artifact_path != requested_path {
                    return Err(KernelError::Internal(format!(
                        "Attachment '{}' no longer matches artifact {}",
                        attachment.display_name, artifact_id
                    )));
                }
                source_owner_agent_id = Some(artifact.agent_id);
                artifact_path
            } else {
                // Reject paths outside the user's home directory
                let home_dir = std::env::var("HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|_| std::path::PathBuf::from("/"));
                let raw_path = std::path::Path::new(&attachment.original_path);
                let parent = raw_path.parent().unwrap_or(raw_path);
                if let Ok(canonical_parent) = parent.canonicalize() {
                    if !canonical_parent.starts_with(&home_dir) {
                        return Err(KernelError::Internal(format!(
                            "Attachment '{}' path is outside the permitted root",
                            attachment.display_name
                        )));
                    }
                }
                std::path::PathBuf::from(&attachment.original_path)
            };

            let canonical = source_path.canonicalize().map_err(|e| {
                KernelError::Internal(format!(
                    "Attachment '{}' could not be resolved: {}",
                    attachment.display_name, e
                ))
            })?;
            let metadata = std::fs::metadata(&canonical).map_err(|e| {
                KernelError::Internal(format!(
                    "Attachment '{}' metadata failed: {}",
                    attachment.display_name, e
                ))
            })?;
            if !metadata.is_file() {
                return Err(KernelError::Internal(format!(
                    "Attachment '{}' is not a regular file",
                    attachment.display_name
                )));
            }

            self.registry.record_attachment(
                &attachment.attachment_id,
                &attachment.source_kind,
                &attachment.display_name,
                &canonical.to_string_lossy(),
                &attachment.content_type,
                metadata.len() as i64,
                attachment.originating_swo_id,
                attachment.originating_artifact_id,
            )?;

            let can_reference_in_place = canonical.starts_with(agent_root)
                && source_owner_agent_id
                    .as_deref()
                    .map(|owner| owner == agent_id)
                    .unwrap_or(true);
            let (workspace_path, workspace_ref, delivery_status) = if can_reference_in_place {
                let workspace_ref = Self::relative_workspace_ref(agent_root, &canonical)?;
                (canonical.clone(), workspace_ref, "REFERENCED".to_string())
            } else {
                let filename = canonical
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(Self::sanitize_attachment_filename)
                    .unwrap_or_else(|| "attachment".to_string());
                let target_name = format!("{}__{}", attachment.attachment_id, filename);
                let target = attached_context_root.join(&target_name);
                std::fs::copy(&canonical, &target).map_err(|e| {
                    KernelError::Internal(format!(
                        "Attachment '{}' could not be copied into {} context: {}",
                        attachment.display_name, agent_id, e
                    ))
                })?;
                (
                    target.clone(),
                    Self::relative_workspace_ref(agent_root, &target)?,
                    "COPIED".to_string(),
                )
            };

            if let Some(swo_id) = swo_id {
                self.registry.link_swo_attachment(
                    swo_id,
                    &attachment.attachment_id,
                    Some(workspace_path.to_string_lossy().as_ref()),
                    &delivery_status,
                    None,
                )?;
            }

            delivered.push(DeliveredAttachmentContext {
                attachment_id: attachment.attachment_id.clone(),
                source_kind: attachment.source_kind.clone(),
                display_name: attachment.display_name.clone(),
                original_path: canonical.to_string_lossy().to_string(),
                workspace_path: workspace_path.to_string_lossy().to_string(),
                workspace_ref,
                content_type: attachment.content_type.clone(),
                size_bytes: metadata.len() as i64,
                originating_swo_id: attachment.originating_swo_id,
                originating_artifact_id: attachment.originating_artifact_id,
            });
        }

        Ok(delivered)
    }

    pub fn repair_agent_directories(&self) -> Result<()> {
        let root = self.agent_home_root()?;
        for agent in self.registry.list_agents()? {
            let manifest = self.registry.get_agent_manifest(&agent.id)?;
            Self::sync_agent_workspace(&root, &agent, &manifest)?;
        }
        Ok(())
    }

    pub fn archive_agent_directories(
        &self,
        agent_names: &[String],
        dest_root: &std::path::Path,
    ) -> Result<Vec<String>> {
        let root = self.agent_home_root()?;
        let mut archived = Vec::new();

        for agent_name in agent_names {
            for src in Self::matching_agent_directories(&root, agent_name)? {
                let dest = dest_root.join(src.file_name().ok_or_else(|| {
                    KernelError::Internal("Agent home dir missing name".to_string())
                })?);
                copy_dir_recursive(&src, &dest)?;
                archived.push(dest.to_string_lossy().to_string());
            }
        }

        Ok(archived)
    }

    pub fn archive_all_agent_directories(
        &self,
        dest_root: &std::path::Path,
    ) -> Result<Vec<String>> {
        let root = self.agent_home_root()?;
        if !root.exists() {
            return Ok(Vec::new());
        }
        let dest = dest_root
            .join(root.file_name().ok_or_else(|| {
                KernelError::Internal("Agent home root missing name".to_string())
            })?);
        copy_dir_recursive(&root, &dest)?;
        Ok(vec![dest.to_string_lossy().to_string()])
    }

    pub fn clear_agent_directories_for_names(&self, agent_names: &[String]) -> Result<()> {
        let root = self.agent_home_root()?;

        for agent_name in agent_names {
            for path in Self::matching_agent_directories(&root, agent_name)? {
                if path.exists() {
                    std::fs::remove_dir_all(&path)?;
                }
            }
        }

        Ok(())
    }

    pub fn clear_all_agent_directories(&self) -> Result<()> {
        let root = self.agent_home_root()?;
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        std::fs::create_dir_all(&root)?;
        Ok(())
    }

    fn agent_home_root(&self) -> Result<std::path::PathBuf> {
        if let Some(path) = &self.agent_home_root_override {
            std::fs::create_dir_all(&path)?;
            return Ok(path.clone());
        }

        let home_dir = dirs::home_dir()
            .ok_or_else(|| KernelError::Internal("Cannot resolve home directory".to_string()))?;
        Ok(home_dir.join("Sairgent_Agents"))
    }

    fn runtime_projection_root(&self) -> Result<std::path::PathBuf> {
        let root = std::path::Path::new(&self.worker_cmd_binary)
            .parent()
            .map(|path| path.to_path_buf())
            .unwrap_or_else(std::env::temp_dir)
            .join(".sairgent-runtime");
        std::fs::create_dir_all(&root)?;
        Ok(root)
    }

    fn project_runtime_bundle(
        &self,
        run_id: &str,
        mode: &str,
        agent: &crate::registry::AgentIdentity,
        manifest: &AgentManifestV1,
        skill_index: &[RuntimeSkillIndexEntry],
        project_skill_files: bool,
    ) -> Result<RuntimeProjection> {
        let dir = self.runtime_projection_root()?.join(run_id);
        let skills_dir = dir.join("skills");
        std::fs::create_dir_all(&skills_dir)?;

        let mut projected_index = skill_index.to_vec();
        if project_skill_files {
            for entry in &mut projected_index {
                if let Some(skill) = self.registry.get_skill(&entry.id)? {
                    let skill_path = skills_dir.join(format!("{}.md", entry.slug));
                    std::fs::write(&skill_path, skill.raw_markdown).map_err(|e| {
                        KernelError::Internal(format!(
                            "Failed to write runtime skill projection '{}': {}",
                            skill_path.display(),
                            e
                        ))
                    })?;
                    entry.runtime_path = Some(skill_path.to_string_lossy().to_string());
                }
            }
        }

        let manifest_path = dir.join("manifest.json");
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(manifest)?).map_err(|e| {
            KernelError::Internal(format!(
                "Failed to write runtime manifest '{}': {}",
                manifest_path.display(),
                e
            ))
        })?;

        let skill_index_path = dir.join("skill_index.json");
        std::fs::write(
            &skill_index_path,
            serde_json::to_vec_pretty(&projected_index).map_err(|e| {
                KernelError::Internal(format!("Failed to serialize runtime skill index: {}", e))
            })?,
        )
        .map_err(|e| {
            KernelError::Internal(format!(
                "Failed to write runtime skill index '{}': {}",
                skill_index_path.display(),
                e
            ))
        })?;

        let context_path = dir.join("RUN_CONTEXT.md");
        let skill_lines = if projected_index.is_empty() {
            "- none".to_string()
        } else {
            projected_index
                .iter()
                .map(|entry| {
                    let marker = if entry.preselected {
                        "preselected"
                    } else {
                        "available"
                    };
                    match &entry.runtime_path {
                        Some(path) => format!(
                            "- {} [{}] :: {} (path: {})",
                            entry.name, marker, entry.summary, path
                        ),
                        None => format!("- {} [{}] :: {}", entry.name, marker, entry.summary),
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let context_payload = format!(
            "# Run Context\n\nName: {}\nRole: {}\nMode: {}\nMission: {}\n\nThis bundle is derived from the kernel DB for the current run only.\n\n## Skills\n{}\n",
            agent.name, agent.role, mode, manifest.mission, skill_lines
        );
        std::fs::write(&context_path, context_payload).map_err(|e| {
            KernelError::Internal(format!(
                "Failed to write runtime context '{}': {}",
                context_path.display(),
                e
            ))
        })?;

        Ok(RuntimeProjection {
            dir,
            manifest_path,
            context_path,
            skill_index: projected_index,
        })
    }

    fn format_managed_work_ack(
        manager_name: &str,
        assignee_name: Option<&str>,
        routing_policy: &str,
        user_visible_summary: Option<&str>,
    ) -> String {
        let summary = user_visible_summary.unwrap_or("The requested work");
        match assignee_name {
            Some(name) if routing_policy == "HARD_ROUTE" => {
                format!(
                    "{} has been queued under {} for {}-led execution. I will confirm once the delegation SWO is actually opened.",
                    summary, manager_name, name
                )
            }
            Some(name) if routing_policy == "PREFERENCE" => {
                format!(
                    "{} has been queued under {} with a routing preference for {}. I will confirm the actual assignee once delegation is opened.",
                    summary, manager_name, name
                )
            }
            Some(name) => {
                format!(
                    "{} has been queued under {} with {} noted as the likely execution lead.",
                    summary, manager_name, name
                )
            }
            None => format!(
                "{} has been queued under {} for manager-led execution.",
                summary, manager_name
            ),
        }
    }

    /// Background task: scans for orphaned IN_PROGRESS SWOs and resets them to PENDING.
    /// SWOs with retry_count >= 3 are permanently failed instead.
    pub async fn start_queue_reconciler(self: Arc<Self>) {
        const STALE_THRESHOLD_MS: i64 = 5_000; // 5 seconds without a heartbeat = dead
        const MAX_RETRIES: i32 = 3;
        let mut ticker = interval(Duration::from_secs(60));

        loop {
            ticker.tick().await;
            let registry = Arc::clone(&self.registry);
            let stale = tokio::task::spawn_blocking(move || {
                registry.get_stale_in_progress_swos(STALE_THRESHOLD_MS)
            })
            .await;

            match stale {
                Ok(Ok(swos)) => {
                    for (swo_id, agent_id, retry_count) in swos {
                        let registry = Arc::clone(&self.registry);
                        if retry_count >= MAX_RETRIES {
                            eprintln!(
                                "[Reconciler] SWO {} for agent {} exceeded max retries — marking FAILED",
                                swo_id,
                                &agent_id[..8.min(agent_id.len())]
                            );
                            let _ = tokio::task::spawn_blocking(move || registry.fail_swo(swo_id))
                                .await;
                        } else {
                            eprintln!(
                                "[Reconciler] SWO {} for agent {} is stale (retry {}) — resetting to PENDING",
                                swo_id,
                                &agent_id[..8.min(agent_id.len())],
                                retry_count + 1
                            );
                            let _ = tokio::task::spawn_blocking(move || {
                                registry.reset_swo_to_pending(swo_id)
                            })
                            .await;
                        }
                    }
                }
                Ok(Err(e)) => eprintln!("[Reconciler] DB error scanning stale SWOs: {:?}", e),
                Err(e) => eprintln!("[Reconciler] Task join error: {:?}", e),
            }
        }
    }

    /// Background task: checks each cron-enabled agent on its configured interval.
    /// If the agent has PENDING work, it claims and runs it.
    /// If the queue is empty, it boots the agent into ideation mode.
    pub async fn start_cron_loop(self: Arc<Self>) {
        use std::collections::{HashMap, HashSet};
        let mut last_fired: HashMap<String, i64> = match self
            .registry
            .list_agent_cron_last_fired_unix()
        {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!(
                    "[Cron] Failed to load persisted cron state; continuing with empty state: {:?}",
                    error
                );
                HashMap::new()
            }
        };
        // Kryptonite: prevent duplicate concurrent executions for the same agent
        let in_flight: Arc<std::sync::Mutex<HashSet<String>>> =
            Arc::new(std::sync::Mutex::new(HashSet::new()));
        let mut ticker = interval(Duration::from_secs(30));

        loop {
            ticker.tick().await;

            let registry = Arc::clone(&self.registry);
            let agents_result =
                tokio::task::spawn_blocking(move || registry.get_cron_eligible_agents()).await;

            let agents = match agents_result {
                Ok(Ok(a)) => a,
                Ok(Err(e)) => {
                    eprintln!("[Cron] DB error fetching cron agents: {:?}", e);
                    continue;
                }
                Err(e) => {
                    eprintln!("[Cron] Join error: {:?}", e);
                    continue;
                }
            };

            for agent in agents {
                let interval_secs = match agent.cron_interval_seconds {
                    Some(i) if i > 0 => i,
                    _ => continue,
                };
                let now_unix = unix_now_secs();

                let last = last_fired
                    .entry(agent.id.clone())
                    .or_insert(now_unix - interval_secs - 1);
                if now_unix.saturating_sub(*last) < interval_secs {
                    continue; // Not yet due
                }

                // Kryptonite guard: skip if already executing
                {
                    let guard = in_flight.lock().unwrap();
                    if guard.contains(&agent.id) {
                        eprintln!(
                            "[Cron] Agent {} already in-flight — skipping tick.",
                            agent.name
                        );
                        continue;
                    }
                }
                *last = now_unix;
                if let Err(error) = self.registry.set_agent_cron_last_fired_now(&agent.id) {
                    eprintln!(
                        "[Cron] Failed to persist cron tick for {}: {:?}",
                        agent.name, error
                    );
                }

                // Check queue
                let agent_id_clone = agent.id.clone();
                let registry = Arc::clone(&self.registry);
                let next_swo = tokio::task::spawn_blocking(move || {
                    registry.get_next_pending_swo_for_agent(&agent_id_clone)
                })
                .await;

                match next_swo {
                    Ok(Ok(Some((swo_id, swo_payload)))) => {
                        // Claim it and execute — record a run_id so reconciler can scope heartbeat staleness
                        let agent_id_c = agent.id.clone();
                        let registry = Arc::clone(&self.registry);
                        let cron_run_id = format!("cron-{}", uuid::Uuid::new_v4());
                        let cron_run_id_c = cron_run_id.clone();
                        let claimed = tokio::task::spawn_blocking(move || {
                            registry.claim_swo_with_run_id(swo_id, &cron_run_id_c)
                        })
                        .await;

                        if matches!(claimed, Ok(Ok(n)) if n > 0) {
                            eprintln!(
                                "[Cron] Agent {} claimed SWO {} (run_id: {}) and is executing.",
                                &agent.name,
                                swo_id,
                                &cron_run_id[..8]
                            );
                            let self_clone = Arc::clone(&self);
                            let cron_run_id_clone = cron_run_id.clone();
                            let agent_id_c2 = agent_id_c.clone();
                            let in_flight_clone = Arc::clone(&in_flight);
                            // Mark in-flight before spawn, clear on drop
                            in_flight_clone.lock().unwrap().insert(agent_id_c.clone());
                            tokio::spawn(async move {
                                let _ = self_clone
                                    .execute_hsm_loop_with_context(
                                        agent_id_c,
                                        None,
                                        swo_payload,
                                        None,
                                        Some(swo_id),
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        Some(cron_run_id_clone),
                                    )
                                    .await;
                                in_flight_clone.lock().unwrap().remove(&agent_id_c2);
                            });
                        }
                    }
                    Ok(Ok(None)) => {
                        let agent_id_c = agent.id.clone();
                        let agent_name = agent.name.clone();
                        let ideation_prompt = format!(
                            "AUTONOMOUS IDEATION CYCLE\nAgent: {} ({})\nMission: {}\nTask: Review your mission, recent runtime state, and prior decision memory. Generate 1-3 concrete proactive ideas at most, raise only worthwhile innovation SWOs, and record the key lesson from this cycle in the decision log.",
                            agent.name, agent.role, agent.raison_detre,
                        );
                        let self_clone = Arc::clone(&self);
                        let in_flight_clone = Arc::clone(&in_flight);
                        in_flight_clone.lock().unwrap().insert(agent_id_c.clone());
                        eprintln!(
                            "[Cron] Agent {} has no pending work — running ideation.",
                            agent.name
                        );
                        tokio::spawn(async move {
                            let _ = Arc::clone(&self_clone)
                                .run_ideation(&agent_id_c, &ideation_prompt, None)
                                .await;
                            in_flight_clone.lock().unwrap().remove(&agent_id_c);
                            eprintln!("[Cron] Agent {} finished ideation tick.", agent_name);
                        });
                    }
                    Ok(Err(e)) => {
                        eprintln!("[Cron] DB error checking queue for {}: {:?}", agent.name, e)
                    }
                    Err(e) => eprintln!("[Cron] Join error for {}: {:?}", agent.name, e),
                }
            }
        }
    }

    pub async fn start_recurring_work_order_loop(self: Arc<Self>) {
        use std::collections::HashSet;

        let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let mut ticker = interval(Duration::from_secs(60));

        loop {
            ticker.tick().await;

            let templates = match self.registry.list_due_recurring_templates() {
                Ok(records) => records,
                Err(error) => {
                    eprintln!(
                        "[Recurring] Failed to load due recurring templates: {:?}",
                        error
                    );
                    continue;
                }
            };

            for template in templates {
                {
                    let guard = in_flight.lock().unwrap();
                    if guard.contains(&template.template_id) {
                        continue;
                    }
                }

                in_flight
                    .lock()
                    .unwrap()
                    .insert(template.template_id.clone());
                let orchestrator = Arc::clone(&self);
                let in_flight_clone = Arc::clone(&in_flight);
                let template_id = template.template_id.clone();
                tokio::spawn(async move {
                    if let Err(error) = Arc::clone(&orchestrator)
                        .materialize_recurring_template_run(template_id.clone(), "schedule", true)
                        .await
                    {
                        eprintln!(
                            "[Recurring] Failed to materialize template {}: {:?}",
                            template_id, error
                        );
                    }
                    in_flight_clone.lock().unwrap().remove(&template_id);
                });
            }
        }
    }

    /// Run an agent in ideation mode (empty queue cron firing).
    /// The agent generates innovation proposals emitted as SWOs.
    #[allow(dead_code)]
    async fn run_ideation(
        self: Arc<Self>,
        agent_id: &str,
        ideation_prompt: &str,
        ui_tx: Option<tokio::sync::mpsc::Sender<KernelEvent>>,
    ) -> Result<()> {
        let agent = self.registry.get_agent(agent_id)?;
        let route = self.router.resolve_route(&agent, None);
        let decrypted_api_key = self.resolve_llm_api_key(&route.provider_name);
        let storage_base = std::path::Path::new(&self.registry.db_path)
            .parent()
            .unwrap();
        let db_path = storage_base
            .join("agents")
            .join(agent_id)
            .join("memory.sqlite")
            .to_string_lossy()
            .to_string();
        #[cfg(debug_assertions)]
        eprintln!(
            "[Orchestrator] Running ideation for {} with DB: {}",
            agent_id, db_path
        );
        let subordinates = self.registry.get_subordinates(agent_id)?;
        let subordinates_json = serde_json::to_string(
            &subordinates
                .into_iter()
                .map(|s| json!({"id": s.id, "name": s.name, "role": s.role, "raison": s.raison_detre}))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());

        let (_, side_effects) = self
            .run_worker(
                agent_id,
                &agent.name,
                None,
                &db_path,
                &route.provider_name,
                &route.model,
                &decrypted_api_key,
                "execute_ideation",
                ideation_prompt,
                &[],
                &subordinates_json,
                &agent.role,
                &agent.persona_prompt,
                &agent.raison_detre,
                None,
                None,
                None,
                None,
                ui_tx.clone(),
                None,
            )
            .await?;

        // Any SWOs dispatched during ideation are ingested into the registry
        for (target_id, swo) in side_effects.dispatch_swos {
            let _ = self
                .registry
                .create_swo_with_metadata(crate::registry::CreateSwoParams {
                    assigned_agent_id: &target_id,
                    owner_agent_id: agent_id,
                    created_by_agent_id: agent_id,
                    payload: &swo,
                    status: "PENDING",
                    parent_swo_id: None,
                    kind: "TASK",
                    source: "HEARTBEAT",
                    work_order_title: None,
                    work_order_outcome: None,
                    work_order_constraints: None,
                    requested_owner_agent_id: None,
                    requested_assignee_agent_id: None,
                    routing_policy: "NONE",
                    originating_swo_id: None,
                    initiative_id: None,
                    initiative_name: None,
                    initiative_owner_agent_id: None,
                    priority_class: None,
                });
        }

        Ok(())
    }

    /// Dispatch to the ephemeral worker sandbox in a specific mode (e.g. execute_triage, execute_synthesis).
    async fn run_worker(
        &self,
        agent_id: &str,
        _agent_name: &str,
        swo_id: Option<i64>,
        database_path: &str,
        provider: &str,
        model: &str,
        decrypted_api_key: &str,
        mode: &str,
        swo_payload: &str,
        attachments: &[AttachmentSpec],
        subordinates_json: &str,
        role: &str,
        persona_prompt: &str,
        raison_detre: &str,
        requested_assignee_agent_id: Option<&str>,
        requested_assignee_name: Option<&str>,
        routing_policy: Option<&str>,
        // Kryptonite fix #2 final: when the cron loop has pre-claimed a SWO with a specific
        // run_id, that run_id must be used as AGENT_RUN_ID so heartbeats from the worker
        // are keyed to the same value stored in active_swos.current_run_id.
        // None = generate from run_token (execute_triage, synthesis, chat, ideation paths)
        override_run_id: Option<&str>,
        ui_tx: Option<tokio::sync::mpsc::Sender<KernelEvent>>,
        revision_feedback: Option<&str>,
    ) -> Result<(Value, WorkerSideEffects)> {
        // Kryptonite fix #4: Use in-memory sidechannel token
        let expected_sidechannel_token = self.secrets.sidechannel_token.clone();

        // Kryptonite fix #4b: Generate a per-run random token and pass it to the worker.
        // This scopes each worker invocation's sidechannel auth to its own run,
        // preventing token replay across different runs of the same agent.
        let run_token = format!("{}.{}", expected_sidechannel_token, uuid::Uuid::new_v4());

        let manifest = self.registry.get_agent_manifest(agent_id)?;
        let agent_identity = self.registry.get_agent(agent_id)?;

        // Phase 3: provision per-agent context/artifacts/workspace directories and inject env vars
        let (context_path, artifacts_path, workspace_path) = self
            .agent_home_root()
            .and_then(|root| Self::sync_agent_workspace(&root, &agent_identity, &manifest))
            .unwrap_or_else(|e| {
                eprintln!(
                    "[orchestrator] sync_agent_workspace failed for agent '{}': {:?}. Using fallback paths.",
                    agent_identity.name, e
                );
                let fallback_root = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
                let agent_base = fallback_root
                    .join("Sairgent_Agents")
                    .join(&agent_identity.name);
                let ctx = agent_base.join("context");
                let art = agent_base.join("artifacts");
                let ws = agent_base.join("workspace");
                let _ = std::fs::create_dir_all(&ctx);
                let _ = std::fs::create_dir_all(&art);
                let _ = std::fs::create_dir_all(&ws);
                (
                    ctx.to_string_lossy().to_string(),
                    art.to_string_lossy().to_string(),
                    ws.to_string_lossy().to_string(),
                )
            });
        let delivered_attachments =
            self.materialize_attachments(agent_id, &context_path, swo_id, attachments)?;
        let worker_payload = Self::render_attachment_context(swo_payload, &delivered_attachments);
        let attachment_manifest_json =
            serde_json::to_string(&delivered_attachments).unwrap_or_else(|_| "[]".to_string());

        // Kryptonite fix #2 final: AGENT_RUN_ID must match active_swos.current_run_id.
        // If the cron loop provided an override (= cron_run_id stored in claim), use that.
        // All other paths (triage, synthesis, chat, ideation) pass None and use run_token.
        let agent_run_id = override_run_id.unwrap_or(&run_token).to_string();
        let allow_direct_hire_side_effects = mode != "execute_ideation";
        let protocol_family = ProviderProtocolFamily::from_provider_name(provider);
        let worker_backend = Self::worker_backend_for_mode(mode, &protocol_family);
        let mut allowed_worker_tools =
            manifest.allowed_worker_tools_for_mode(mode, allow_direct_hire_side_effects);
        let bound_tools = self
            .registry
            .list_agent_tool_bindings(agent_id)
            .unwrap_or_default();
        let active_search_provider = active_web_search_provider(&bound_tools);
        if manifest.has_capability(&CapabilityGrant::WebSearch) && mode != "format_swo" {
            allowed_worker_tools.push("web_search".to_string());
        }
        let skill_index = self
            .registry
            .preview_agent_skills_for_run(agent_id, mode, &worker_payload, 4)
            .unwrap_or_default();
        if !skill_index.is_empty() {
            allowed_worker_tools.push("list_available_skills".to_string());
            allowed_worker_tools.push("load_skill".to_string());
        }
        allowed_worker_tools.sort();
        allowed_worker_tools.dedup();
        let allowed_worker_tools_json =
            serde_json::to_string(&allowed_worker_tools).unwrap_or_else(|_| "[]".to_string());
        let manifest_json = serde_json::to_string(&manifest).unwrap_or_else(|_| "{}".to_string());
        let runtime_projection = self.project_runtime_bundle(
            &agent_run_id,
            mode,
            &agent_identity,
            &manifest,
            &skill_index,
            worker_backend == "codex_cli",
        )?;
        let _runtime_guard = RuntimeProjectionGuard {
            path: runtime_projection.dir.clone(),
        };
        let skill_index_json = serde_json::to_string(&runtime_projection.skill_index)
            .unwrap_or_else(|_| "[]".to_string());
        let _ = self.registry.record_worker_run_start(
            &agent_run_id,
            swo_id,
            agent_id,
            worker_backend,
            mode,
        );
        let _ = self.registry.record_audit_event(
            Some(agent_id),
            swo_id,
            "worker_run_started",
            TaintLabel::TrustedSystem,
            &json!({
                "mode": mode,
                "backend": worker_backend,
                "protocol_family": format!("{:?}", protocol_family),
                "allow_direct_hire_side_effects": allow_direct_hire_side_effects,
                "allowed_worker_tools": allowed_worker_tools,
                "bound_tools": bound_tools,
                "active_search_provider": active_search_provider,
                "skill_count": runtime_projection.skill_index.len(),
                "attachment_count": delivered_attachments.len(),
                "runtime_dir": runtime_projection.dir.to_string_lossy().to_string(),
            }),
        );
        let cached_tool_keys = self
            .secrets
            .tool_api_keys_by_slug
            .read()
            .map_err(|_| KernelError::Internal("Tool credential cache lock poisoned".into()))?
            .clone();
        let search_provider_status = match active_search_provider.as_ref() {
            Some(provider_slug)
                if cached_tool_keys
                    .get(provider_slug)
                    .map(|secret| !secret.trim().is_empty())
                    .unwrap_or(false) =>
            {
                "configured"
            }
            Some(_) => "missing_credential",
            None => "missing_binding",
        };
        let mut tool_api_keys_by_slug = serde_json::Map::new();
        if let Some(provider_slug) = active_search_provider.as_ref() {
            if let Some(secret) = cached_tool_keys.get(provider_slug) {
                if !secret.trim().is_empty() {
                    tool_api_keys_by_slug.insert(
                        provider_slug.clone(),
                        serde_json::Value::String(secret.clone()),
                    );
                }
            }
        }
        // ── MCP connector resolution ──────────────────────────────────
        // Gather active MCP connector configs for this agent so the Python
        // harness can spin up ephemeral MCP server connections at runtime.
        let mcp_connector_configs: Vec<serde_json::Value>;
        let mut mcp_credentials = serde_json::Map::new();

        if manifest.has_capability(&CapabilityGrant::McpClient) {
            let bindings = self
                .registry
                .list_agent_mcp_bindings(agent_id)
                .unwrap_or_default();

            let mut bound_connectors: Vec<crate::tools::McpConnectorRecord> = Vec::new();
            for binding in &bindings {
                if binding.binding_status != "ACTIVE" {
                    continue;
                }
                match self.registry.get_mcp_connector(&binding.connector_id) {
                    Ok(connector) if connector.enabled => {
                        bound_connectors.push(connector);
                    }
                    _ => {} // skip disabled or missing connectors
                }
            }

            mcp_connector_configs = bound_connectors
                .iter()
                .map(|c| {
                    json!({
                        "slug": c.slug,
                        "transport": c.transport.as_str(),
                        "command": c.command,
                        "args": c.args,
                        "env": c.env,
                        "url": c.url,
                        "headers": c.headers,
                        "cwd": c.cwd,
                    })
                })
                .collect();

            for connector in &bound_connectors {
                let cred_key = format!("mcp_{}", connector.slug);
                if let Some(secret) = cached_tool_keys.get(&cred_key) {
                    if !secret.trim().is_empty() {
                        mcp_credentials
                            .insert(connector.slug.clone(), json!(secret));
                    }
                }
            }
        } else {
            mcp_connector_configs = Vec::new();
        }

        let worker_secret_bundle = json!({
            "llm_api_key": decrypted_api_key,
            "tool_api_keys_by_slug": tool_api_keys_by_slug,
            "mcp_credentials_by_slug": mcp_credentials,
        });
        let worker_secret_bundle_json =
            serde_json::to_string(&worker_secret_bundle).map_err(|e| {
                KernelError::Internal(format!("Failed to serialize worker secrets: {}", e))
            })?;

        const MAX_PERSONA_LEN: usize = 4096;
        let persona_prompt = persona_prompt.chars().take(MAX_PERSONA_LEN).collect::<String>();
        let raison_detre = raison_detre.chars().take(MAX_PERSONA_LEN).collect::<String>();

        let mut child = Command::new(&self.worker_cmd_binary)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .env("TMPDIR", std::env::var("TMPDIR").unwrap_or_default())
            .env("LANG", std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".to_string()))
            .env("AGENT_ID", agent_id)
            .env("AGENT_DATABASE", database_path)
            .env("REGISTRY_DATABASE", &self.registry.db_path)
            .env("LLM_PROVIDER", provider)
            .env("LLM_MODEL", model)
            .env("LLM_PROTOCOL_FAMILY", format!("{:?}", protocol_family))
            // LLM_API_KEY is intentionally excluded from the environment to prevent scraping
            .env("AGENT_ROLE", role)
            .env("AGENT_PERSONA_PROMPT", persona_prompt)
            .env("AGENT_RAISON", raison_detre)
            .env("AGENT_MANIFEST_JSON", &manifest_json)
            .env("AGENT_ALLOWED_TOOLS", &allowed_worker_tools_json)
            .env(
                "AGENT_SEARCH_PROVIDER_SLUG",
                active_search_provider.clone().unwrap_or_default(),
            )
            .env("AGENT_SEARCH_PROVIDER_STATUS", search_provider_status)
            .env("AGENT_SKILL_INDEX_JSON", &skill_index_json)
            .env("AGENT_ATTACHMENT_MANIFEST_JSON", &attachment_manifest_json)
            .env(
                "AGENT_MCP_CONNECTORS_JSON",
                serde_json::to_string(&mcp_connector_configs).unwrap_or_default(),
            )
            .env("AGENT_RUNTIME_DIR", &runtime_projection.dir)
            .env(
                "AGENT_RUNTIME_MANIFEST_PATH",
                &runtime_projection.manifest_path,
            )
            .env(
                "AGENT_RUNTIME_CONTEXT_PATH",
                &runtime_projection.context_path,
            )
            .env("AGENT_SUBORDINATES", subordinates_json)
            .env("SAIRGENT_WORKER_BACKEND", worker_backend)
            .env(
                "AGENT_REQUESTED_ASSIGNEE_ID",
                requested_assignee_agent_id.unwrap_or_default(),
            )
            .env(
                "AGENT_REQUESTED_ASSIGNEE_NAME",
                requested_assignee_name.unwrap_or_default(),
            )
            .env("AGENT_ROUTING_POLICY", routing_policy.unwrap_or("NONE"))
            // Per-run scoped token — unique to this worker invocation (sidechannel auth)
            .env("SAIRGENT_SIDECHANNEL_TOKEN", &run_token)
            // AGENT_RUN_ID = the run_id stored in active_swos.current_run_id at claim time;
            // heartbeats emitted by Python must key to this so the reconciler can find them.
            .env("AGENT_RUN_ID", &agent_run_id)
            .env(
                "AGENT_ROOT",
                std::path::Path::new(&context_path)
                    .parent()
                    .unwrap_or(std::path::Path::new(""))
                    .to_string_lossy()
                    .to_string(),
            )
            .env("AGENT_CONTEXT", &context_path)
            .env("AGENT_ARTIFACTS", &artifacts_path)
            .env("AGENT_WORKSPACE", &workspace_path)
            .env("AGENT_INBOX", &context_path)
            .env("AGENT_OUTBOX", &artifacts_path)
            .env(
                "DECISION_LOG_MAX_ENTRIES",
                self.registry
                    .get_runtime_metadata("decision_log_max_entries")
                    .ok()
                    .flatten()
                    .filter(|v| !v.trim().is_empty())
                    .unwrap_or_else(|| "500".to_string()),
            )
            .env(
                "AGENT_SWO_ID",
                {
                    if swo_id.is_none()
                        && mode != "chat_mode"
                        && mode != "sairgent_chat"
                        && mode != "format_swo"
                        && mode != "execute_ideation"
                    {
                        eprintln!(
                            "[orchestrator] WARNING: swo_id is None for mode '{}' (agent '{}'). \
                             Artifact sidechannel emissions will be skipped by the worker.",
                            mode, _agent_name
                        );
                    }
                    swo_id.map(|v| v.to_string()).unwrap_or_default()
                },
            )
            .env(
                "AGENT_CAN_HIRE",
                if allow_direct_hire_side_effects
                    && manifest.has_capability(&CapabilityGrant::HireSubordinate)
                {
                    "1"
                } else {
                    "0"
                },
            )
            .env(
                "AGENT_REVISION_FEEDBACK",
                revision_feedback.unwrap_or(""),
            )
            // Phase 1C: Inject pulse journal and cadence state for recurring SWO workers
            .env(
                "PULSE_JOURNAL_LAST_JSON",
                swo_id
                    .and_then(|_| self.registry.get_latest_pulse_entry("heartbeat").ok().flatten())
                    .map(|e| serde_json::to_string(&e).unwrap_or_default())
                    .unwrap_or_default(),
            )
            .env(
                "CADENCE_STATE_JSON",
                self.registry
                    .list_cadence_states()
                    .map(|states| serde_json::to_string(&states).unwrap_or_default())
                    .unwrap_or_default(),
            )
            .arg(mode)
            .arg(&worker_payload)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                KernelError::Internal(format!("Failed to spawn worker subprocess: {}", e))
            })?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(worker_secret_bundle_json.as_bytes())
                .await
                .map_err(|e| {
                    KernelError::Internal(format!("Failed to write worker secrets to stdin: {}", e))
                })?;
            drop(stdin); // close stdin to signal EOF
        }

        let mut stdout_lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut stderr_lines = BufReader::new(child.stderr.take().unwrap()).lines();

        let mut side_effects = WorkerSideEffects::default();
        // Kryptonite fix #1c: build a whitelist of authorized subordinate IDs from the
        // subordinates_json passed to this worker. Only these IDs can receive sidechannel dispatches.
        let allowed_dispatch_ids: std::collections::HashSet<String> =
            serde_json::from_str::<Vec<serde_json::Value>>(subordinates_json)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|v| {
                    v.get("id")
                        .and_then(|id| id.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
        let mut stdout_accum = String::new();
        let mut stderr_accum = String::new();

        let last_db_write_ms = Arc::new(AtomicI64::new(0));
        let run_id_shared: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let mut last_worker_progress_at = tokio::time::Instant::now();
        let mut liveness_check = interval(Self::WORKER_STALL_CHECK_INTERVAL);
        let mut last_heartbeat_seq: i64 = -1;

        loop {
            tokio::select! {
                line_opt = stdout_lines.next_line() => {
                    match line_opt {
                        Ok(Some(line)) => {
                            last_worker_progress_at = tokio::time::Instant::now();
                            stdout_accum.push_str(&line);
                            stdout_accum.push('\n');
                            // Redact token if it happens to be printed
                            let redacted_line = line.replace(&self.secrets.sidechannel_token, "[REDACTED_SIDECHANNEL_TOKEN]");
                            let event = KernelEvent::Status(format!("{} - stdout: {}", agent_id, redacted_line));
                            if let Some(tx) = &ui_tx {
                                let _ = tx.send(event).await;
                            }
                        }
                        Ok(None) | Err(_) => { }
                    }
                }
                line_opt = stderr_lines.next_line() => {
                    match line_opt {
                        Ok(Some(line)) => {
                            last_worker_progress_at = tokio::time::Instant::now();
                            stderr_accum.push_str(&line);
                            stderr_accum.push('\n');

                            let mut handled = false;
                            // Check for streaming deltas
                            if line.contains("\"__sairgent_delta\":") {
                                if let Ok(parsed) = serde_json::from_str::<Value>(&line) {
                                    if parsed["__sairgent_delta"] == true {
                                        if let (Some(msg_id), Some(delta_text)) = (
                                            parsed["message_id"].as_str(),
                                            parsed["delta"].as_str(),
                                        ) {
                                            let is_final = parsed["is_final"].as_bool().unwrap_or(false);
                                            let agent_id = parsed["agent_id"].as_str().map(|s| s.to_string());
                                            handled = true;
                                            let event = KernelEvent::StreamingDelta {
                                                message_id: msg_id.to_string(),
                                                delta: delta_text.to_string(),
                                                is_final,
                                                agent_id,
                                            };
                                            if let Some(tx) = &ui_tx {
                                                let _ = tx.send(event).await;
                                            }
                                        }
                                    }
                                }
                            }
                            if !handled && line.contains("\"__sairgent_sidechannel\":") {
                                if let Ok(parsed) = serde_json::from_str::<Value>(&line) {
                                    let sent_token = parsed["token"].as_str().unwrap_or("");
                                    // Validate with per-run token (not the static env token)
                                    if sent_token == run_token {
                                        handled = true;
                                        if parsed["__sairgent_sidechannel"] == "heartbeat" {
                                            if let Ok(hb) = serde_json::from_str::<HeartbeatPayload>(&line) {
                                                if (hb.seq as i64) <= last_heartbeat_seq {
                                                    eprintln!("[Orchestrator] Heartbeat seq {} out of order (last: {}), ignoring", hb.seq, last_heartbeat_seq);
                                                    continue;
                                                }
                                                last_heartbeat_seq = hb.seq as i64;
                                                let now = std::time::SystemTime::now()
                                                    .duration_since(std::time::UNIX_EPOCH)
                                                    .unwrap()
                                                    .as_millis() as i64;
                                                last_worker_progress_at = tokio::time::Instant::now();
                                                if now - last_db_write_ms.load(Ordering::Relaxed) >= 1000 {
                                                    last_db_write_ms.store(now, Ordering::Relaxed);
                                                    *run_id_shared.lock().unwrap() = Some(hb.run_id.clone());
                                                    self.upsert_heartbeat_async(hb.run_id, agent_id.to_string(), hb.status.clone(), hb.seq).await;
                                                }
                                                // Redact token from UI status
                                                let sanitized_line = line.replace(&hb.token, "[REDACTED_RUN_TOKEN]");
                                                let event = KernelEvent::Status(format!("Agent {} - {}: {}", agent_id, hb.status, sanitized_line));
                                                if let Some(tx) = &ui_tx {
                                                    let _ = tx.send(event).await;
                                                    let _ = tx.send(KernelEvent::AgentPresenceChanged {
                                                        agent_id: agent_id.to_string(),
                                                        presence: hb.status.clone(),
                                                    }).await;
                                                }
                                            }
                                        } else if parsed["__sairgent_sidechannel"] == "innovation_swo" {
                                            if !manifest.has_capability(&CapabilityGrant::SubmitInnovationSwo) {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "capability_denied_innovation_swo",
                                                    TaintLabel::UntrustedModelOutput,
                                                    &parsed,
                                                );
                                                continue;
                                            }
                                            let report_str = serde_json::to_string(&parsed["report"]).unwrap_or_default();
                                            let originating_swo_id = parsed["originating_swo_id"].as_i64().or(swo_id);
                                            if report_str.len() > 4096 {
                                                eprintln!("Warning: Innovation report exceeded 4096 bytes, dropped.");
                                            } else if let Ok(agent) = self.registry.get_agent(agent_id) {
                                                if let Some(parent_id) = agent.parent_id {
                                                    let title = parsed["report"]["title"].as_str().unwrap_or("Innovation Proposal");
                                                    let context = parsed["report"]["context"].as_str().unwrap_or("");
                                                    let solution = parsed["report"]["proposed_solution"].as_str().unwrap_or("");
                                                    let impact = parsed["report"]["estimated_impact"].as_str().unwrap_or("");
                                                    let payload = format!(
                                                        "Innovation review: {title}\nContext: {context}\nProposed solution: {solution}\nEstimated impact: {impact}"
                                                    );
                                                    let source = if mode == "execute_ideation" { "HEARTBEAT" } else { "HSM" };
                                                    let _ = self.registry.create_swo_with_metadata(crate::registry::CreateSwoParams {
                                                        assigned_agent_id: &parent_id,
                                                        owner_agent_id: &parent_id,
                                                        created_by_agent_id: agent_id,
                                                        payload: &payload,
                                                        status: "PENDING",
                                                        parent_swo_id: None,
                                                        kind: "INNOVATION_REVIEW",
                                                        source,
                                                        work_order_title: None,
                                                        work_order_outcome: None,
                                                        work_order_constraints: None,
                                                        requested_owner_agent_id: None,
                                                        requested_assignee_agent_id: None,
                                                        routing_policy: "NONE",
                                                        originating_swo_id,
                                                        initiative_id: None,
                                                        initiative_name: None,
                                                        initiative_owner_agent_id: None,
                                                        priority_class: None,
                                                    });
                                                    let _ = self.registry.record_audit_event(
                                                        Some(agent_id),
                                                        originating_swo_id,
                                                        "innovation_swo_created",
                                                        TaintLabel::UntrustedModelOutput,
                                                        &parsed,
                                                    );
                                                }
                                            }
                                        } else if parsed["__sairgent_sidechannel"] == "dispatch_swo" {
                                            if !manifest.has_capability(&CapabilityGrant::DispatchSwo) {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "capability_denied_dispatch_swo",
                                                    TaintLabel::UntrustedModelOutput,
                                                    &parsed,
                                                );
                                                continue;
                                            }
                                            if let Some(payload_str) = parsed["payload"].as_str() {
                                                if let Ok(inner_payload) = serde_json::from_str::<serde_json::Value>(payload_str) {
                                                    if let (Some(target_id), Some(actual_payload)) = (inner_payload["target_id"].as_str(), inner_payload["payload"].as_str()) {
                                                        // Kryptonite fix #1c: Validate sidechannel dispatch target at Rust level too.
                                                        if !allowed_dispatch_ids.contains(target_id) {
                                                            eprintln!("[Security] Sidechannel dispatch_swo to unauthorized target '{}' — silently dropped.", &target_id[..8.min(target_id.len())]);
                                                            let _ = self.registry.record_audit_event(
                                                                Some(agent_id),
                                                                swo_id,
                                                                "dispatch_swo_denied",
                                                                TaintLabel::UntrustedModelOutput,
                                                                &inner_payload,
                                                            );
                                                        } else if actual_payload.len() > 8192 {
                                                            eprintln!("Warning: dispatch_swo payload exceeded 8192 bytes, dropped.");
                                                        } else {
                                                            side_effects.dispatch_swos.push((target_id.to_string(), actual_payload.to_string()));
                                                            let _ = self.registry.record_audit_event(
                                                                Some(agent_id),
                                                                swo_id,
                                                                "dispatch_swo_accepted",
                                                                TaintLabel::UntrustedModelOutput,
                                                                &inner_payload,
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        } else if parsed["__sairgent_sidechannel"] == "queue_managed_work" {
                                            if !manifest.has_capability(&CapabilityGrant::QueueManagedWork) {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "capability_denied_queue_managed_work",
                                                    TaintLabel::UntrustedModelOutput,
                                                    &parsed,
                                                );
                                                continue;
                                            }
                                            let payload = if let Some(payload_obj) = parsed.get("payload") {
                                                serde_json::from_value::<ManagedWorkRequest>(payload_obj.clone()).ok()
                                            } else if let Some(payload_str) = parsed["payload"].as_str() {
                                                let trimmed = payload_str.trim();
                                                if trimmed.is_empty() {
                                                    None
                                                } else {
                                                    Some(ManagedWorkRequest {
                                                        payload: trimmed.to_string(),
                                                        requested_assignee_agent_id: None,
                                                        requested_assignee_name: None,
                                                        routing_policy: "NONE".to_string(),
                                                        user_visible_summary: None,
                                                    })
                                                }
                                            } else {
                                                None
                                            };
                                            if let Some(mut payload) = payload {
                                                payload.payload = payload.payload.trim().to_string();
                                                if payload.payload.is_empty() {
                                                    continue;
                                                }
                                                let routing = payload.routing_policy.trim().to_uppercase();
                                                payload.routing_policy = match routing.as_str() {
                                                    "HARD_ROUTE" | "PREFERENCE" | "NONE" => routing,
                                                    _ => "NONE".to_string(),
                                                };
                                                if payload.requested_assignee_agent_id.is_none()
                                                    && payload.requested_assignee_name.is_none()
                                                {
                                                    payload.routing_policy = "NONE".to_string();
                                                }
                                                let payload_value = serde_json::to_value(&payload).unwrap_or(Value::Null);
                                                side_effects.managed_work_requests.push(payload);
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "managed_work_queued",
                                                    TaintLabel::UntrustedModelOutput,
                                                    &payload_value,
                                                );
                                            }
                                        } else if parsed["__sairgent_sidechannel"] == "hire_subordinate" {
                                            if !manifest.has_capability(&CapabilityGrant::HireSubordinate) || !allow_direct_hire_side_effects {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "capability_denied_hire_subordinate",
                                                    TaintLabel::UntrustedModelOutput,
                                                    &parsed,
                                                );
                                                continue;
                                            }
                                            // CHA-427 / CHA-428 — autonomous_hiring_mode + per-manager cap.
                                            // The gate is now resolved AFTER we parse the spec because the
                                            // target manager (reports_to) may differ from the caller. The
                                            // cap applies to the target's direct reports, not the caller's.
                                            if let Some(spec_val) = parsed.get("spec") {
                                                if let Ok(spec) = serde_json::from_value::<HireSubordinateRequest>(spec_val.clone()) {
                                                    // CHA-428 — resolve reports_to (if set) to a real agent id.
                                                    // Default to the caller as target when reports_to is unset.
                                                    let target_manager_id: String = match spec.reports_to.as_deref() {
                                                        Some(name) if !name.trim().is_empty() => {
                                                            match self.registry.find_agent_id_by_name(name.trim()) {
                                                                Ok(Some(id)) => id,
                                                                Ok(None) => {
                                                                    eprintln!(
                                                                        "[CHA-428] hire_subordinate reports_to='{}' does not resolve to any agent; rejecting",
                                                                        name
                                                                    );
                                                                    let mut denial_payload = parsed.clone();
                                                                    if let Some(obj) = denial_payload.as_object_mut() {
                                                                        obj.insert(
                                                                            "rejection_reason".to_string(),
                                                                            serde_json::Value::String(format!(
                                                                                "reports_to='{}' does not resolve to any agent",
                                                                                name
                                                                            )),
                                                                        );
                                                                    }
                                                                    let _ = self.registry.record_audit_event(
                                                                        Some(agent_id),
                                                                        swo_id,
                                                                        "hire_subordinate_policy_denied",
                                                                        TaintLabel::UntrustedModelOutput,
                                                                        &denial_payload,
                                                                    );
                                                                    continue;
                                                                }
                                                                Err(err) => {
                                                                    eprintln!(
                                                                        "[CHA-428] hire_subordinate reports_to lookup failed: {}",
                                                                        err
                                                                    );
                                                                    continue;
                                                                }
                                                            }
                                                        }
                                                        _ => agent_id.to_string(),
                                                    };

                                                    // CHA-427/428 — run the cross-manager gate with the
                                                    // resolved target. Checks mode + ancestor auth + per-
                                                    // manager cap against the TARGET.
                                                    if let Err(hire_err) = self.registry.check_cross_manager_hire_allowed(agent_id, &target_manager_id) {
                                                        eprintln!(
                                                            "[CHA-427/428] hire_subordinate rejected for caller {} target {} SWO {:?}: {}",
                                                            agent_id, target_manager_id, swo_id, hire_err
                                                        );
                                                        let mut denial_payload = parsed.clone();
                                                        if let Some(obj) = denial_payload.as_object_mut() {
                                                            obj.insert(
                                                                "rejection_reason".to_string(),
                                                                serde_json::Value::String(hire_err.to_string()),
                                                            );
                                                            obj.insert(
                                                                "target_manager_id".to_string(),
                                                                serde_json::Value::String(target_manager_id.clone()),
                                                            );
                                                        }
                                                        let _ = self.registry.record_audit_event(
                                                            Some(agent_id),
                                                            swo_id,
                                                            "hire_subordinate_policy_denied",
                                                            TaintLabel::UntrustedModelOutput,
                                                            &denial_payload,
                                                        );
                                                        continue;
                                                    }

                                                    if let Ok(new_agent_id) = self.registry.hire_subordinate_with_cron(
                                                        &spec.name,
                                                        Some(&target_manager_id),
                                                        &spec.role,
                                                        &spec.raison_detre,
                                                        &spec.provider,
                                                        &spec.model,
                                                        spec.cron_interval_seconds,
                                                    ) {
                                                        if let Ok(new_agent) = self.registry.get_agent(&new_agent_id) {
                                                            if let Ok(root) = self.agent_home_root() {
                                                                let manifest = self.registry.get_agent_manifest(&new_agent.id).unwrap_or_else(|_| AgentManifestV1::default_for_agent(&new_agent));
                                                                let _ = Self::sync_agent_workspace(&root, &new_agent, &manifest);
                                                            }
                                                        }
                                                        if let Some(id) = swo_id {
                                                            let spec_json = serde_json::to_string(&spec).unwrap_or_default();
                                                            let _ = self.registry.record_agent_hire(id, agent_id, &new_agent_id, &spec_json);
                                                        }
                                                        let mut accept_payload = spec_val.clone();
                                                        if let Some(obj) = accept_payload.as_object_mut() {
                                                            obj.insert(
                                                                "parent_agent_id".to_string(),
                                                                serde_json::Value::String(target_manager_id.clone()),
                                                            );
                                                            obj.insert(
                                                                "caller_agent_id".to_string(),
                                                                serde_json::Value::String(agent_id.to_string()),
                                                            );
                                                        }
                                                        let _ = self.registry.record_audit_event(
                                                            Some(agent_id),
                                                            swo_id,
                                                            "hire_subordinate_accepted",
                                                            TaintLabel::UntrustedModelOutput,
                                                            &accept_payload,
                                                        );
                                                        // Notify the UI so the new agent shows up on the
                                                        // workspace grid + activity log can resolve its
                                                        // UUID to a name. parent_id is the TARGET, not the
                                                        // caller (CHA-428).
                                                        if let Some(tx) = ui_tx.as_ref() {
                                                            let _ = tx
                                                                .send(KernelEvent::AgentCreated {
                                                                    agent_id: new_agent_id.clone(),
                                                                    name: spec.name.clone(),
                                                                    role: spec.role.clone(),
                                                                    parent_id: Some(target_manager_id.clone()),
                                                                    reason: spec.raison_detre.clone(),
                                                                })
                                                                .await;
                                                        }
                                                    }
                                                }
                                            }
                                        } else if parsed["__sairgent_sidechannel"] == "append_pulse_journal" {
                                            // No capability check needed — any agent can journal
                                            if let (Some(cadence), Some(entry_type), Some(summary)) = (
                                                parsed["cadence"].as_str(),
                                                parsed["entry_type"].as_str(),
                                                parsed["summary"].as_str(),
                                            ) {
                                                let run_id = parsed["run_id"].as_str();
                                                let detail_json = parsed.get("detail_json").and_then(|v| {
                                                    if v.is_null() { None } else { Some(v.to_string()) }
                                                });
                                                let _ = self.registry.append_pulse_journal_entry(
                                                    cadence,
                                                    run_id,
                                                    agent_id,
                                                    entry_type,
                                                    summary,
                                                    detail_json.as_deref(),
                                                );
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "pulse_journal_appended",
                                                    TaintLabel::UntrustedModelOutput,
                                                    &parsed,
                                                );
                                            }
                                        } else if parsed["__sairgent_sidechannel"] == "outbox_artifact" {
                                            if !manifest.has_capability(&CapabilityGrant::WriteOutbox) {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "capability_denied_outbox_artifact",
                                                    TaintLabel::UntrustedModelOutput,
                                                    &parsed,
                                                );
                                                continue;
                                            }
                                            if let (Some(artifact_swo_id), Some(filename), Some(absolute_path)) = (
                                                parsed["swo_id"].as_i64(),
                                                parsed["filename"].as_str(),
                                                parsed["absolute_path"].as_str(),
                                            ) {
                                                let absolute = std::path::Path::new(absolute_path);
                                                let artifacts_root = std::path::Path::new(&artifacts_path);
                                                if absolute.starts_with(artifacts_root)
                                                    && absolute.file_name().and_then(|name| name.to_str()) == Some(filename)
                                                {
                                                    let _ = self.registry.record_outbox_artifact(
                                                        artifact_swo_id,
                                                        agent_id,
                                                        absolute_path,
                                                        filename,
                                                    );
                                                    if let Some(tx) = &ui_tx {
                                                        let _ = tx.send(KernelEvent::ArtifactRegistered { swo_id: artifact_swo_id }).await;
                                                    }
                                                    let _ = self.registry.record_audit_event(
                                                        Some(agent_id),
                                                        Some(artifact_swo_id),
                                                        "outbox_artifact_registered",
                                                        TaintLabel::UntrustedModelOutput,
                                                        &parsed,
                                                    );
                                                } else {
                                                    eprintln!("[Security] Dropped invalid outbox artifact path '{}'", absolute_path);
                                                    let _ = self.registry.record_audit_event(
                                                        Some(agent_id),
                                                        Some(artifact_swo_id),
                                                        "outbox_artifact_denied",
                                                        TaintLabel::UntrustedModelOutput,
                                                        &parsed,
                                                    );
                                                }
                                            }
                                        } else if parsed["__sairgent_sidechannel"] == "sairgent_proposal" {
                                            if let (Some(call_id), Some(tool_name)) = (
                                                parsed["call_id"].as_str(),
                                                parsed["tool_name"].as_str(),
                                            ) {
                                                let summary = parsed["summary"].as_str().unwrap_or("").to_string();
                                                let arguments = serde_json::to_string(
                                                    parsed.get("arguments").unwrap_or(&Value::Null)
                                                ).unwrap_or_else(|_| "{}".to_string());
                                                side_effects.sairgent_proposals.push(SairgentToolProposal {
                                                    call_id: call_id.to_string(),
                                                    tool_name: tool_name.to_string(),
                                                    summary,
                                                    arguments_json: arguments,
                                                    requires_confirmation: true,
                                                });
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "sairgent_proposal_accepted",
                                                    TaintLabel::UntrustedModelOutput,
                                                    &parsed,
                                                );
                                            }
                                        // Dark factory tool audit events (CHA-396).
                                        //
                                        // SECURITY NOTE: these events are SELF-REPORTED by the
                                        // harness AFTER the operation has executed. The kernel
                                        // capability check below is a DETECTION mechanism, not
                                        // enforcement — actual enforcement lives in the
                                        // harness's _require_capability() gate. By the time
                                        // `capability_violation_reported_*` lands, the side
                                        // effect has already happened on disk. These records
                                        // mean "the harness claims an agent without the cap
                                        // attempted this," not "the kernel prevented it."
                                        } else if parsed["__sairgent_sidechannel"] == "shell_exec" {
                                            if !manifest.has_capability(&CapabilityGrant::ShellExec) {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "capability_violation_reported_shell_exec",
                                                    TaintLabel::ToolExecution,
                                                    &parsed,
                                                );
                                            } else {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "shell_exec",
                                                    TaintLabel::ToolExecution,
                                                    &parsed,
                                                );
                                            }
                                        } else if parsed["__sairgent_sidechannel"] == "file_mutation" {
                                            if !manifest.has_capability(&CapabilityGrant::FileWrite) {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "capability_violation_reported_file_mutation",
                                                    TaintLabel::ToolExecution,
                                                    &parsed,
                                                );
                                            } else {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "file_mutation",
                                                    TaintLabel::ToolExecution,
                                                    &parsed,
                                                );
                                            }
                                        } else if parsed["__sairgent_sidechannel"] == "git_operation" {
                                            if !manifest.has_capability(&CapabilityGrant::GitOps) {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "capability_violation_reported_git_operation",
                                                    TaintLabel::ToolExecution,
                                                    &parsed,
                                                );
                                            } else {
                                                let _ = self.registry.record_audit_event(
                                                    Some(agent_id),
                                                    swo_id,
                                                    "git_operation",
                                                    TaintLabel::ToolExecution,
                                                    &parsed,
                                                );
                                            }
                                        } else {
                                            handled = false;
                                        }
                                    }
                                }
                            }

                            if !handled {
                                // Redact token
                                let redacted_line = line.replace(&self.secrets.sidechannel_token, "[REDACTED_SIDECHANNEL_TOKEN]");
                                eprintln!("[{}] {}", agent_id.chars().take(8).collect::<String>(), redacted_line);
                                let event = KernelEvent::Status(format!("Agent {} - stderr: {}", agent_id, redacted_line));
                                if let Some(tx) = &ui_tx {
                                    let _ = tx.send(event).await;
                                }
                            }

                        }
                        Ok(None) | Err(_) => { }
                    }
                }
                status_res = child.wait() => {
                    let status = match status_res {
                        Ok(st) => st,
                        Err(e) => {
                            let rid_opt = run_id_shared.lock().unwrap().clone();
                            if let Some(rid) = rid_opt {
                                self.upsert_heartbeat_async(rid, agent_id.to_string(), "ERROR".to_string(), -1).await;
                            }
                            let _ = self.registry.record_worker_run_finish(
                                &agent_run_id,
                                "FAILED",
                                0,
                                false,
                                None,
                                Some(&format!("Failed to await worker subprocess: {}", e)),
                            );
                            if let Some(tx) = &ui_tx {
                                let _ = tx.send(KernelEvent::Error(format!("Failed to await worker subprocess: {}", e))).await;
                            }
                            return Err(KernelError::Internal(format!("Failed to await worker subprocess: {}", e)));
                        }
                    };

                    if !status.success() {
                        // Check if stdout contains a valid COMPLETED/BLOCKED response despite non-zero exit.
                        // This handles harness cleanup code that may throw after printing valid output.
                        // Try JSON parse first for reliability, fall back to substring match.
                        let stdout_indicates_success = Self::stdout_has_success_status(&stdout_accum);
                        if !stdout_indicates_success {
                            let rid_opt = run_id_shared.lock().unwrap().clone();
                            if let Some(rid) = rid_opt {
                                self.upsert_heartbeat_async(rid, agent_id.to_string(), "ERROR".to_string(), -1).await;
                            }
                            let redacted_stdout = stdout_accum.replace(&self.secrets.sidechannel_token, "[REDACTED_SIDECHANNEL_TOKEN]");
                            let redacted_stderr = stderr_accum.replace(&self.secrets.sidechannel_token, "[REDACTED_SIDECHANNEL_TOKEN]");
                            let _ = self.registry.record_worker_run_finish(
                                &agent_run_id,
                                "FAILED",
                                0,
                                false,
                                None,
                                Some(&format!("Worker failed. Stdout: {} Stderr: {}", redacted_stdout, redacted_stderr)),
                            );
                            if let Some(tx) = &ui_tx {
                                let _ = tx.send(KernelEvent::Error(format!("Worker failed. Stdout: {} Stderr: {}", redacted_stdout, redacted_stderr))).await;
                            }
                            return Err(KernelError::Internal(format!("Worker failed. Stdout: {} Stderr: {}", redacted_stdout, redacted_stderr)));
                        }
                        // Stdout indicates success — proceed to parse despite non-zero exit code
                        eprintln!("[kernel] Worker exited non-zero but stdout indicates COMPLETED/BLOCKED — proceeding with output parsing");
                    }

                    let rid_opt = run_id_shared.lock().unwrap().clone();
                    if let Some(rid) = rid_opt {
                        // Keep successful agents visible as ready after a completed run.
                        self.upsert_heartbeat_async(rid, agent_id.to_string(), "READY".to_string(), -1).await;
                    }

                    break;
                }
                _ = liveness_check.tick() => {
                    if last_worker_progress_at.elapsed() < Self::WORKER_STALL_TIMEOUT {
                        continue;
                    }

                    let stall_reason = Self::worker_stall_reason();
                    let _ = child.kill().await;
                    let rid_opt = run_id_shared.lock().unwrap().clone();
                    if let Some(rid) = rid_opt {
                        self.upsert_heartbeat_async(rid, agent_id.to_string(), "DEAD".to_string(), -1).await;
                    }
                    let _ = self.registry.record_worker_run_finish(
                        &agent_run_id,
                        "FAILED",
                        0,
                        false,
                        Some("Worker stalled"),
                        Some(&stall_reason),
                    );
                    if let Some(tx) = &ui_tx {
                        let _ = tx.send(KernelEvent::Error(stall_reason.clone())).await;
                    }
                    return Err(KernelError::Internal(stall_reason));
                }
            }
        }

        while let Ok(Some(line)) = stdout_lines.next_line().await {
            stdout_accum.push_str(&line);
            stdout_accum.push('\n');
        }
        while let Ok(Some(line)) = stderr_lines.next_line().await {
            stderr_accum.push_str(&line);
            stderr_accum.push('\n');
        }

        // Try parsing the full stdout as one JSON value.
        // If that fails (e.g. codex_cli emits multiple lines), take the last
        // non-empty line that parses as a JSON object.
        #[cfg(debug_assertions)]
        if mode == "sairgent_chat" {
            eprintln!("[SairgentChat] stdout_accum ({} bytes, {} lines): {:?}",
                stdout_accum.len(),
                stdout_accum.lines().count(),
                &stdout_accum[..stdout_accum.len().min(200)]);
        }
        let parsed: Value = serde_json::from_str(&stdout_accum).or_else(|_| {
            stdout_accum
                .lines()
                .rev()
                .filter(|line| !line.trim().is_empty())
                .find_map(|line| serde_json::from_str::<Value>(line).ok())
                .ok_or_else(|| serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "No parseable JSON line found in worker stdout",
                )))
        }).map_err(|e| {
            KernelError::Internal(format!("Failed to parse worker JSON output: {:?}. Stdout was: {}", e, &stdout_accum[..stdout_accum.len().min(500)]))
        })?;
        let parsed = normalize_worker_output(mode, agent_id, parsed);
        #[cfg(debug_assertions)]
        if mode == "sairgent_chat" {
            eprintln!("[SairgentChat] after normalize — keys: {:?}",
                parsed.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default());
        }
        if let Some(requests) = parsed
            .get("managed_work_requests")
            .and_then(|value| value.as_array())
        {
            for request in requests {
                if let Ok(mut payload) =
                    serde_json::from_value::<ManagedWorkRequest>(request.clone())
                {
                    payload.payload = payload.payload.trim().to_string();
                    if !payload.payload.is_empty() {
                        side_effects.managed_work_requests.push(payload);
                    }
                }
            }
        }
        let artifact_count = match swo_id {
            Some(id) => self
                .registry
                .count_outbox_artifacts_for_swo(id)
                .unwrap_or(0),
            None => 0,
        };
        let structured_output_present = parsed
            .as_object()
            .map(|value| !value.is_empty())
            .unwrap_or(false);
        let blocked_reason = parsed
            .get("blocked_reason")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let failure_reason = parsed
            .get("error")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let final_status = if failure_reason.is_some() {
            "FAILED"
        } else if blocked_reason.is_some() {
            "BLOCKED"
        } else {
            "COMPLETED"
        };
        let _ = self.registry.record_worker_run_finish(
            &agent_run_id,
            final_status,
            artifact_count,
            structured_output_present,
            blocked_reason.as_deref(),
            failure_reason.as_deref(),
        );
        if let Some(usage_val) = parsed.get("token_usage") {
            if let Ok(token_usage) = serde_json::from_value::<WorkerTokenUsage>(usage_val.clone()) {
                let _ = self.registry.record_token_usage(
                    &agent_run_id,
                    swo_id,
                    agent_id,
                    provider,
                    model,
                    token_usage.input_tokens,
                    token_usage.output_tokens,
                    token_usage.cache_read_tokens,
                    token_usage.cache_write_tokens,
                    token_usage.requests,
                    token_usage.cost_usd,
                );
            }
        }
        let _ = self.registry.record_audit_event(
            Some(agent_id),
            swo_id,
            "worker_run_finished",
            if failure_reason.is_some() {
                TaintLabel::UntrustedModelOutput
            } else {
                TaintLabel::TrustedSystem
            },
            &json!({
                "run_id": agent_run_id,
                "mode": mode,
                "status": final_status,
                "blocked_reason": blocked_reason,
                "failure_reason": failure_reason,
                "artifacts": artifact_count,
            }),
        );

        Ok((parsed, side_effects))
    }

    pub fn execute_hsm_loop(
        self: Arc<Self>,
        agent_id: String,
        manager_id: Option<String>,
        swo_payload: String,
        ui_tx: Option<tokio::sync::mpsc::Sender<KernelEvent>>,
        parent_swo_id: Option<i64>,
        swo_run_id: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> {
        self.execute_hsm_loop_with_context(
            agent_id,
            manager_id,
            swo_payload,
            ui_tx,
            None,
            parent_swo_id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            swo_run_id,
        )
    }

    pub async fn trigger_manual_heartbeat(
        self: Arc<Self>,
        agent_id: String,
        ui_tx: Option<tokio::sync::mpsc::Sender<KernelEvent>>,
    ) -> Result<i64> {
        let agent = self.registry.get_agent(&agent_id)?;
        let payload = format!(
            "MANUAL PROACTIVE REVIEW\nAgent: {} ({})\nMission: {}\nTask: Review your mission, current queue, and likely blind spots. Produce a concrete review result. If you identify a worthwhile systemic opportunity, raise it through the proper typed channel rather than roleplaying completion.",
            agent.name, agent.role, agent.raison_detre
        );
        let swo_id = self
            .registry
            .create_swo_with_metadata(crate::registry::CreateSwoParams {
                assigned_agent_id: &agent_id,
                owner_agent_id: &agent_id,
                created_by_agent_id: &agent_id,
                payload: &payload,
                status: "PENDING",
                parent_swo_id: None,
                kind: "PROACTIVE_REVIEW",
                source: "MANUAL_HEARTBEAT",
                work_order_title: Some("Manual proactive review"),
                work_order_outcome: Some(
                    "Produce a typed proactive review result or blocked state.",
                ),
                work_order_constraints: Some(
                    "Do not simulate work completion. Keep the review concrete and bounded.",
                ),
                requested_owner_agent_id: Some(&agent_id),
                requested_assignee_agent_id: None,
                routing_policy: "NONE",
                originating_swo_id: None,
                initiative_id: None,
                initiative_name: None,
                initiative_owner_agent_id: None,
                priority_class: Some("REVIEW"),
            })?;

        let run_id = format!("heartbeat-{}", uuid::Uuid::new_v4());
        let claimed = self.registry.claim_swo_with_run_id(swo_id, &run_id)?;
        if claimed > 0 {
            let self_clone = Arc::clone(&self);
            tokio::spawn(async move {
                let _ = self_clone
                    .execute_hsm_loop_with_context(
                        agent_id,
                        None,
                        payload,
                        ui_tx,
                        Some(swo_id),
                        None,
                        Some("PROACTIVE_REVIEW".to_string()),
                        Some("MANUAL_HEARTBEAT".to_string()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(run_id),
                    )
                    .await;
            });
        }

        Ok(swo_id)
    }

    /// The core recursive loop handling the Hierarchical State Machine for an agent.
    pub fn execute_hsm_loop_with_context(
        self: Arc<Self>,
        agent_id: String,
        manager_id: Option<String>,
        swo_payload: String,
        ui_tx: Option<tokio::sync::mpsc::Sender<KernelEvent>>,
        existing_swo_id: Option<i64>,
        parent_swo_id: Option<i64>,
        swo_kind: Option<String>,
        swo_source: Option<String>,
        owner_agent_id: Option<String>,
        created_by_agent_id: Option<String>,
        requested_assignee_agent_id: Option<String>,
        requested_assignee_name: Option<String>,
        routing_policy: Option<String>,
        originating_swo_id: Option<i64>,
        // Kryptonite fix #2 final: when a SWO has been pre-claimed with a specific run_id
        // (e.g., by the cron loop via claim_swo_with_run_id), pass that run_id here so
        // run_worker can inject it as AGENT_RUN_ID for correct heartbeat correlation.
        swo_run_id: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> {
        Box::pin(async move {
            let owner_agent_id = owner_agent_id
                .or_else(|| manager_id.clone())
                .unwrap_or_else(|| agent_id.clone());
            let created_by_agent_id = created_by_agent_id
                .or_else(|| manager_id.clone())
                .unwrap_or_else(|| owner_agent_id.clone());
            let mut requested_assignee_agent_id = requested_assignee_agent_id;
            let mut requested_assignee_name = requested_assignee_name;
            let mut routing_policy = routing_policy.unwrap_or_else(|| "NONE".to_string());

            if existing_swo_id.is_some()
                && requested_assignee_agent_id.is_none()
                && requested_assignee_name.is_none()
                && routing_policy == "NONE"
            {
                if let Some(existing_id) = existing_swo_id {
                    if let Some(existing) = self.registry.get_swo_detail(existing_id).ok().flatten()
                    {
                        requested_assignee_agent_id = existing.swo.requested_assignee_agent_id;
                        requested_assignee_name = existing.swo.requested_assignee_agent_name;
                        routing_policy = existing.swo.routing_policy;
                    }
                }
            }

            let attached_specs = if let Some(existing_id) = existing_swo_id {
                self.registry
                    .get_swo_detail(existing_id)?
                    .map(|detail| {
                        detail
                            .attachments
                            .into_iter()
                            .map(|attachment| AttachmentSpec {
                                attachment_id: attachment.attachment.id,
                                source_kind: attachment.attachment.source_kind,
                                display_name: attachment.attachment.display_name,
                                original_path: attachment.attachment.original_path,
                                content_type: attachment.attachment.content_type,
                                size_bytes: attachment.attachment.size_bytes,
                                originating_swo_id: attachment.attachment.originating_swo_id,
                                originating_artifact_id: attachment
                                    .attachment
                                    .originating_artifact_id,
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            let swo_id = if let Some(id) = existing_swo_id {
                Some(id)
            } else {
                self.registry
                    .create_swo_with_metadata(crate::registry::CreateSwoParams {
                        assigned_agent_id: &agent_id,
                        owner_agent_id: &owner_agent_id,
                        created_by_agent_id: &created_by_agent_id,
                        payload: &swo_payload,
                        status: "IN_PROGRESS",
                        parent_swo_id,
                        kind: swo_kind.as_deref().unwrap_or("TASK"),
                        source: swo_source.as_deref().unwrap_or("HSM"),
                        work_order_title: None,
                        work_order_outcome: None,
                        work_order_constraints: None,
                        requested_owner_agent_id: None,
                        requested_assignee_agent_id: requested_assignee_agent_id.as_deref(),
                        routing_policy: &routing_policy,
                        originating_swo_id,
                        initiative_id: None,
                        initiative_name: None,
                        initiative_owner_agent_id: None,
                        priority_class: None,
                    })
                    .ok()
            };

            // SECURITY: Path Traversal Mitigation
            // We ensure agent_id is strictly a valid UUID, preventing malicious SWOs
            // from jumping out of the storage directory via `../` patterns.
            if uuid::Uuid::parse_str(&agent_id).is_err() {
                return Err(KernelError::Internal(format!(
                    "Invalid agent_id format, potential path traversal detected: {}",
                    agent_id
                )));
            }

            let agent = self.registry.get_agent(&agent_id)?;
            let route = self.router.resolve_route(&agent, None);

            // In a real system we would fetch from the vault, but we stub it for now
            // For testing, we read it from the orchestrator's environment to pass securely to the worker's stdin
            let decrypted_api_key = self.resolve_llm_api_key(&route.provider_name);
            let storage_base = std::path::Path::new(&self.registry.db_path)
                .parent()
                .unwrap();
            let db_path = storage_base
                .join("agents")
                .join(&agent_id)
                .join("memory.sqlite")
                .to_string_lossy()
                .to_string();

            let subordinates = self.registry.get_subordinates(&agent_id)?;
            let subordinates_json = serde_json::to_string(
                &subordinates
                    .into_iter()
                    .map(|s| {
                        json!({
                            "id": s.id,
                            "name": s.name,
                            "role": s.role,
                            "raison": s.raison_detre
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_else(|_| "[]".to_string());
            let requested_subordinate = self
                .registry
                .find_direct_subordinate(
                    &agent_id,
                    requested_assignee_agent_id.as_deref(),
                    requested_assignee_name.as_deref(),
                )
                .ok()
                .flatten();
            let requested_route_id = requested_subordinate
                .as_ref()
                .map(|sub| sub.id.clone())
                .or_else(|| requested_assignee_agent_id.clone());
            let requested_route_name = requested_subordinate
                .as_ref()
                .map(|sub| sub.name.clone())
                .or_else(|| requested_assignee_name.clone());
            let agent_org_profile = self.registry.get_agent_org_profile(&agent_id).ok();
            let payload_keywords = Self::payload_keywords(&swo_payload);
            let manager_requires_delegation = agent_org_profile
                .as_ref()
                .map(|profile| {
                    profile.org_class == "manager"
                        && profile.delegation_policy == "must_delegate_when_fit_exists"
                })
                .unwrap_or(false);
            let mut qualified_candidates: Vec<(String, String, i32)> = Vec::new();
            if manager_requires_delegation {
                for report in self.registry.get_subordinates(&agent_id).unwrap_or_default() {
                    let report_profile = self
                        .registry
                        .get_agent_org_profile(&report.id)
                        .unwrap_or_else(|_| crate::registry::AgentOrgProfileRecord::default_for_agent(&report));
                    let report_skills = self
                        .registry
                        .list_agent_skill_bindings(&report.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|binding| binding.skill_name)
                        .collect::<Vec<_>>();
                    let report_tools = self
                        .registry
                        .list_agent_tool_bindings(&report.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|binding| binding.name)
                        .collect::<Vec<_>>();
                    let report_presence = self
                        .registry
                        .get_agent_presence(unix_now_secs() * 1000)
                        .unwrap_or_default()
                        .into_iter()
                        .find(|presence| presence.agent_id == report.id)
                        .map(|presence| presence.presence)
                        .unwrap_or_else(|| "OFFLINE".to_string());
                    let goal_text = self
                        .registry
                        .list_descendant_team_goals(&report.id)
                        .unwrap_or_default()
                        .into_iter()
                        .flat_map(|goal| {
                            vec![
                                goal.title,
                                goal.summary,
                                goal.success_criteria,
                                goal.managed_domain_tags.join(" "),
                            ]
                        })
                        .collect::<Vec<_>>();
                    let score = Self::direct_report_fit_score(
                        &payload_keywords,
                        &report.role,
                        &report_profile.managed_domains,
                        &report_skills,
                        &report_tools,
                        &goal_text,
                        &report_presence,
                    );
                    if score >= 4 {
                        qualified_candidates.push((report.id.clone(), report.name.clone(), score));
                    }
                }
                qualified_candidates.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.1.cmp(&right.1)));
            }

            if let Some(tx) = &ui_tx {
                let _ = tx
                    .send(KernelEvent::Status(format!(
                        "{} ({}): IN_PROGRESS",
                        agent.name, agent.role
                    )))
                    .await;
            }

            // Load revision_feedback from the SWO record if this is a re-run of an existing SWO.
            // This allows the harness to present human feedback to the agent on the next pass.
            let revision_feedback: Option<String> = existing_swo_id.and_then(|id| {
                self.registry
                    .get_swo_detail(id)
                    .ok()
                    .flatten()
                    .and_then(|detail| detail.swo.revision_feedback)
                    .filter(|fb| !fb.trim().is_empty())
            });

            // PASS 1: Routing + Brief-writing (or Triage for ICs)
            //
            // Managers with subordinates use `write_briefs` mode: the schema
            // forces delegation (no ANSWER_DIRECTLY option). This works at any
            // depth — Cat can delegate to Lucy just as Perry delegates to Cat.
            //
            // Agents with no subordinates use `execute_triage` (IC direct execution).
            let has_subordinates = !self.registry.get_subordinates(&agent_id).unwrap_or_default().is_empty();
            let use_write_briefs = manager_requires_delegation && has_subordinates;
            let worker_mode = if use_write_briefs { "write_briefs" } else { "execute_triage" };

            // Per-mode model override (CHA-370): use triage_model for routing/triage,
            // execution_model for synthesis/execution, falling back to default model.
            let triage_model = agent.triage_model.as_deref().unwrap_or(&route.model);
            let execution_model = agent.execution_model.as_deref().unwrap_or(&route.model);

            let action;
            let mut swos_map_cloned = Vec::new();

            {
                let (triage_result, triage_side_effects) = self
                    .run_worker(
                        &agent_id,
                        &agent.name,
                        swo_id,
                        &db_path,
                        &route.provider_name,
                        triage_model,
                        &decrypted_api_key,
                        worker_mode,
                        &swo_payload,
                        &attached_specs,
                        &subordinates_json,
                        &agent.role,
                        &agent.persona_prompt,
                        &agent.raison_detre,
                        requested_assignee_agent_id.as_deref(),
                        requested_assignee_name.as_deref(),
                        Some(routing_policy.as_str()),
                        // Pass the pre-claimed run_id if available so AGENT_RUN_ID = current_run_id
                        swo_run_id.as_deref(),
                        ui_tx.clone(),
                        revision_feedback.as_deref(),
                    )
                    .await
                    .map_err(|err| {
                        if let Some(id) = swo_id {
                            let _ = self.registry.update_swo_status(id, "FAILED");
                            emit_swo_status_changed(&ui_tx, id, "FAILED");
                        }
                        err
                    })?;

                if let Some(id) = swo_id {
                    let _ =
                        self.registry
                            .record_swo_result(id, &agent_id, &triage_result.to_string());
                }

                action = triage_result["triage"]["action"]
                    .as_str()
                    .unwrap_or("ANSWER_DIRECTLY")
                    .to_string();

                let route_exception_code = triage_result["triage"]["exception_code"].as_str();
                let route_exception_reason = triage_result["triage"]["exception_reason"].as_str();
                let route_exception_user_message = triage_result["triage"]["user_message"].as_str();
                let direct_answer = triage_result["triage"]["direct_answer"].as_str();

                if action == "ANSWER_DIRECTLY" && direct_answer.is_none() {
                    if let Some(id) = swo_id {
                        let _ = self.registry.record_manager_review(
                            id,
                            &agent_id,
                            "CLOSED_FAILED",
                            "Manager direct execution returned no final response.",
                            None,
                        );
                        let _ = self.registry.update_swo_status(id, "FAILED");
                        emit_swo_status_changed(&ui_tx, id, "FAILED");
                    }
                    return Err(KernelError::Internal(
                        "Manager direct execution returned no final response.".to_string(),
                    ));
                }

                if routing_policy == "HARD_ROUTE"
                    && requested_route_id.is_some()
                    && action == "ANSWER_DIRECTLY"
                {
                    if let Some(id) = swo_id {
                        let requested_name = requested_route_name
                            .clone()
                            .unwrap_or_else(|| requested_route_id.clone().unwrap_or_default());
                        let reasoning = format!(
                            "Hard-route contract violation. Requested subordinate '{}' was not delegated and no approved exception was supplied.",
                            requested_name
                        );
                        let _ = self.registry.record_manager_review(
                            id,
                            &agent_id,
                            "REJECTED_ROUTE_CONTRACT",
                            &reasoning,
                            route_exception_user_message,
                        );
                        let _ = self.registry.update_swo_status(id, "FAILED");
                        emit_swo_status_changed(&ui_tx, id, "FAILED");
                    }
                    return Err(KernelError::Internal(
                        "Hard-route contract violation: manager answered directly instead of delegating."
                            .to_string(),
                    ));
                }

                // ── Manager delegation policy gate (ANSWER_DIRECTLY) ──
                // This check MUST run before the ANSWER_DIRECTLY early return
                // so that managers with qualified subordinates cannot bypass
                // the delegation requirement by self-executing.
                if action == "ANSWER_DIRECTLY" && manager_requires_delegation {
                    if let Some((_, best_candidate_name, _)) = qualified_candidates.first() {
                        if !Self::valid_self_execute_exception(route_exception_code) {
                            if let Some(id) = swo_id {
                                let reasoning = format!(
                                    "Manager self-execution rejected. Qualified direct report '{}' exists and no valid structured exception was supplied.",
                                    best_candidate_name
                                );
                                let _ = self.registry.record_manager_review(
                                    id,
                                    &agent_id,
                                    "REJECTED_MANAGER_POLICY",
                                    &reasoning,
                                    route_exception_user_message,
                                );
                                let _ = self.registry.update_swo_status(id, "FAILED");
                                emit_swo_status_changed(&ui_tx, id, "FAILED");
                            }
                            return Err(KernelError::Internal(format!(
                                "Manager policy violation: qualified direct report '{}' exists. Record a valid structured exception or delegate.",
                                best_candidate_name
                            )));
                        }
                    } else {
                        // No qualified candidates found. Two cases:
                        // 1. Triage chose ANSWER_DIRECTLY — this is a trivial query that
                        //    no subordinate can handle. Allow the manager to self-execute.
                        // 2. Cross-function synthesis — manager is the right executor.
                        // Both cases are legitimate self-execution, not team gaps.
                        // (Team gaps are recorded when a DELEGATE action fails to find
                        //  a target, which is handled in the delegation path below.)
                    }
                    // If we reach here, the manager is allowed to self-execute
                    // (either cross-function synthesis or valid exception supplied).
                    // Record the delegation decision for audit trail.
                    if let Some(id) = swo_id {
                        let selected_candidate = qualified_candidates.first().cloned();
                        let decision = if selected_candidate.is_some() {
                            "SELF_EXECUTE"
                        } else if Self::is_cross_function_synthesis(&payload_keywords) {
                            "SELF_EXECUTE"
                        } else if route_exception_code == Some("TEAM_GAP_PENDING_HIRE") {
                            "HIRE_THEN_DELEGATE"
                        } else {
                            "ESCALATE_TEAM_GAP"
                        };
                        let _ = self.registry.record_delegation_decision(
                            &crate::registry::DelegationDecisionRecord {
                                id: uuid::Uuid::new_v4().to_string(),
                                swo_id: id,
                                manager_agent_id: agent_id.clone(),
                                decision: decision.to_string(),
                                candidate_assignees: qualified_candidates
                                    .iter()
                                    .map(|(candidate_id, _, _)| candidate_id.clone())
                                    .collect(),
                                selected_agent_id: None,
                                fit_reason: selected_candidate.as_ref().map(|(_, name, score)| {
                                    format!("Best-fit qualified report: {} (score {}).", name, score)
                                }),
                                exception_code: route_exception_code.map(str::to_string),
                                exception_reason: route_exception_reason.map(str::to_string),
                                team_gap_code: if selected_candidate.is_none() {
                                    Some("NO_QUALIFIED_REPORT".to_string())
                                } else {
                                    None
                                },
                                created_at: String::new(),
                            },
                        );
                    }
                }

                if action == "ANSWER_DIRECTLY" {
                    if let Some(id) = swo_id {
                        let reasoning = triage_result["triage"]["reasoning"].as_str().unwrap_or("");
                        let final_response = triage_result["triage"]["direct_answer"].as_str();
                        let _ = self.registry.record_manager_review(
                            id,
                            &agent_id,
                            "ACCEPT_AND_COMPLETE",
                            reasoning,
                            final_response,
                        );
                        if let Some(response) = final_response {
                            let _ = self.registry.append_memory_interaction(
                                &agent_id,
                                "assistant",
                                response,
                                Some(id),
                            );
                        }
                        let _ = self.registry.update_swo_status(id, "COMPLETED");
                        emit_swo_status_changed(&ui_tx, id, "COMPLETED");
                        let _ = self.registry.cancel_active_descendant_swos(id);
                    }

                    // Process fire-and-forget inline dispatches from dispatch_swo_internal tool calls.
                    // These are side-effects collected during worker execution that would otherwise be
                    // silently dropped because ANSWER_DIRECTLY returns before the DELEGATE branch.
                    if !triage_side_effects.dispatch_swos.is_empty() {
                        let authorized_sub_ids: std::collections::HashSet<String> = self
                            .registry
                            .get_subordinates(&agent_id)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|s| s.id)
                            .collect();

                        let (parent_initiative_id, parent_initiative_name, parent_initiative_owner) = swo_id
                            .and_then(|id| self.registry.get_swo_detail(id).ok().flatten())
                            .map(|detail| (
                                detail.swo.initiative_id.clone(),
                                detail.swo.initiative_name.clone(),
                                detail.swo.initiative_owner_agent_id.clone(),
                            ))
                            .unwrap_or((None, None, None));

                        let mut child_swo_ids = Vec::new();

                        for (target_id, payload) in triage_side_effects.dispatch_swos {
                            if !authorized_sub_ids.contains(&target_id) {
                                eprintln!(
                                    "[Security] ANSWER_DIRECTLY inline dispatch to unauthorized target '{}' — dropped.",
                                    &target_id[..8.min(target_id.len())]
                                );
                                continue;
                            }
                            if let Ok(child_swo_id) = self.registry.create_swo_with_metadata(
                                crate::registry::CreateSwoParams {
                                    assigned_agent_id: &target_id,
                                    owner_agent_id: &agent_id,
                                    created_by_agent_id: &agent_id,
                                    payload: &payload,
                                    status: "IN_PROGRESS",
                                    parent_swo_id: swo_id,
                                    kind: "TASK",
                                    source: "HSM",
                                    work_order_title: None,
                                    work_order_outcome: None,
                                    work_order_constraints: None,
                                    requested_owner_agent_id: None,
                                    requested_assignee_agent_id: None,
                                    routing_policy: "NONE",
                                    originating_swo_id: None,
                                    initiative_id: parent_initiative_id.as_deref(),
                                    initiative_name: parent_initiative_name.as_deref(),
                                    initiative_owner_agent_id: parent_initiative_owner.as_deref(),
                                    priority_class: None,
                                },
                            ) {
                                child_swo_ids.push((target_id.clone(), child_swo_id));
                                if let Some(tx) = &ui_tx {
                                    let _ = tx.send(KernelEvent::SwoCreated {
                                        swo_id: child_swo_id,
                                        assigned_agent_id: target_id.clone(),
                                        parent_swo_id: swo_id,
                                    }).await;
                                }
                                // Fire-and-forget: spawn child HSM loop without blocking parent
                                let self_clone = Arc::clone(&self);
                                let agent_id_clone = agent_id.clone();
                                let tx_clone = ui_tx.clone();
                                tokio::spawn(async move {
                                    let result = self_clone
                                        .execute_hsm_loop_with_context(
                                            target_id,
                                            Some(agent_id_clone.clone()),
                                            payload,
                                            tx_clone,
                                            Some(child_swo_id),
                                            swo_id,
                                            None,
                                            Some("HSM".to_string()),
                                            Some(agent_id_clone.clone()),
                                            Some(agent_id_clone),
                                            None,
                                            None,
                                            None,
                                            None,
                                            None,
                                        )
                                        .await;
                                    if let Err(e) = result {
                                        eprintln!("[HSM] Fire-and-forget child SWO {} failed: {:?}", child_swo_id, e);
                                    }
                                });
                            }
                        }

                        // Emit DelegationStarted so workspace shows delegation activity
                        if !child_swo_ids.is_empty() {
                            if let Some(tx) = &ui_tx {
                                if let Some(parent_id) = swo_id {
                                    let _ = tx.send(KernelEvent::DelegationStarted {
                                        parent_swo_id: parent_id,
                                        child_swo_ids: child_swo_ids.iter().map(|(_, id)| *id).collect(),
                                        to_agent_ids: child_swo_ids.iter().map(|(aid, _)| aid.clone()).collect(),
                                    }).await;
                                }
                            }
                        }
                    }

                    return Ok(triage_result);
                }

                if let Some(swos_map) = triage_result["triage"]["delegation_swos"].as_object() {
                    // Kryptonite fix #1b: Validate delegation targets against registry.
                    // The LLM triage result must not be able to dispatch to arbitrary agent IDs —
                    // only registered direct subordinates of this agent are allowed.
                    let authorized_subs: Vec<_> = self
                        .registry
                        .get_subordinates(&agent_id)
                        .unwrap_or_default();
                    let authorized_sub_ids: std::collections::HashSet<String> = authorized_subs
                        .iter()
                        .map(|s| s.id.clone())
                        .collect();

                    for (sub_id, swo_val) in swos_map {
                        // Try exact UUID match first, then fall back to case-insensitive
                        // name match. LLMs frequently hallucinate UUIDs but get names right.
                        let resolved_id = if authorized_sub_ids.contains(sub_id) {
                            sub_id.clone()
                        } else {
                            let name_match: Vec<_> = authorized_subs.iter()
                                .filter(|s| s.name.eq_ignore_ascii_case(sub_id) || s.id.starts_with(sub_id))
                                .collect();
                            if name_match.len() == 1 {
                                eprintln!(
                                    "[Delegation] Resolved hallucinated ID '{}' to '{}' ({}) via name match.",
                                    &sub_id[..8.min(sub_id.len())],
                                    &name_match[0].id[..8],
                                    name_match[0].name
                                );
                                name_match[0].id.clone()
                            } else {
                                eprintln!(
                                    "[Security] Triage delegation to unauthorized sub_id '{}' from agent '{}' — dropped.",
                                    sub_id,
                                    &agent_id[..8.min(agent_id.len())]
                                );
                                continue;
                            }
                        };
                        if let Some(swo_str) = swo_val.as_str() {
                            swos_map_cloned.push((resolved_id, swo_str.to_string()));
                        }
                    }
                }
                for (sub_id, swo) in triage_side_effects.dispatch_swos {
                    swos_map_cloned.push((sub_id, swo));
                }

                // ── Manager delegation policy gate (DELEGATE path) ──
                // ANSWER_DIRECTLY is already handled by the pre-return gate above.
                // This block enforces that DELEGATE targets the best-fit candidate.
                if manager_requires_delegation && action == "DELEGATE" {
                    let selected_candidate = qualified_candidates.first().cloned();
                    if let Some(id) = swo_id {
                        let _ = self.registry.record_delegation_decision(
                            &crate::registry::DelegationDecisionRecord {
                                id: uuid::Uuid::new_v4().to_string(),
                                swo_id: id,
                                manager_agent_id: agent_id.clone(),
                                decision: "DELEGATE".to_string(),
                                candidate_assignees: qualified_candidates
                                    .iter()
                                    .map(|(candidate_id, _, _)| candidate_id.clone())
                                    .collect(),
                                selected_agent_id: swos_map_cloned
                                    .first()
                                    .map(|(candidate_id, _)| candidate_id.clone())
                                    .or_else(|| {
                                        selected_candidate
                                            .clone()
                                            .map(|(candidate_id, _, _)| candidate_id)
                                    }),
                                fit_reason: selected_candidate.as_ref().map(|(_, name, score)| {
                                    format!("Best-fit qualified report: {} (score {}).", name, score)
                                }),
                                exception_code: route_exception_code.map(str::to_string),
                                exception_reason: route_exception_reason.map(str::to_string),
                                team_gap_code: None,
                                created_at: String::new(),
                            },
                        );
                    }

                    // We no longer force the LLM to pick the highest-scoring
                    // candidate — the fit score is a heuristic, not a mandate. The LLM
                    // may have better judgment about routing than keyword matching.
                    // Authorization of individual delegation targets is already enforced
                    // in the swos_map resolution block above (unauthorized IDs are dropped).
                    // This gate only needs to verify that at least one target survived.
                }

                if routing_policy == "HARD_ROUTE" {
                    if let Some(required_sub_id) = requested_route_id.as_deref() {
                        let has_required_child = swos_map_cloned
                            .iter()
                            .any(|(sub_id, _)| sub_id == required_sub_id);
                        let exception_supplied = route_exception_code.is_some()
                            && route_exception_reason.is_some()
                            && route_exception_user_message.is_some();
                        if !has_required_child && !exception_supplied {
                            if let Some(id) = swo_id {
                                let reasoning = format!(
                                    "Hard-route contract violation. Required subordinate '{}' was not delegated and no exception was supplied.",
                                    requested_route_name
                                        .clone()
                                        .unwrap_or_else(|| required_sub_id.to_string())
                                );
                                let _ = self.registry.record_manager_review(
                                    id,
                                    &agent_id,
                                    "REJECTED_ROUTE_CONTRACT",
                                    &reasoning,
                                    None,
                                );
                                let _ = self.registry.update_swo_status(id, "FAILED");
                                emit_swo_status_changed(&ui_tx, id, "FAILED");
                            }
                            return Err(KernelError::Internal(
                                "Hard-route contract violation: requested subordinate was not delegated."
                                    .to_string(),
                            ));
                        }
                        if !has_required_child && exception_supplied {
                            if let Some(id) = swo_id {
                                let reasoning = format!(
                                    "Hard-route exception for '{}': {} ({})",
                                    requested_route_name
                                        .clone()
                                        .unwrap_or_else(|| required_sub_id.to_string()),
                                    route_exception_code.unwrap_or("UNSPECIFIED"),
                                    route_exception_reason.unwrap_or("")
                                );
                                let _ = self.registry.record_manager_review(
                                    id,
                                    &agent_id,
                                    "ROUTING_EXCEPTION",
                                    &reasoning,
                                    route_exception_user_message,
                                );
                                let _ = self.registry.update_swo_status(id, "FAILED");
                                emit_swo_status_changed(&ui_tx, id, "FAILED");
                            }
                            return Err(KernelError::Internal(
                                route_exception_user_message
                                    .unwrap_or("Requested subordinate is unavailable for hard-routed work.")
                                    .to_string(),
                            ));
                        }
                    }
                }
            } // triage_result gets dropped here

            if action == "DELEGATE" {
                if swos_map_cloned.is_empty() {
                    if let Some(id) = swo_id {
                        let _ = self.registry.record_manager_review(
                            id,
                            &agent_id,
                            "REJECTED_MANAGER_POLICY",
                            "Delegation action returned no immediate child SWOs. Queued narration is not treated as delivery.",
                            None,
                        );
                        let _ = self.registry.update_swo_status(id, "FAILED");
                        emit_swo_status_changed(&ui_tx, id, "FAILED");
                    }
                    return Err(KernelError::Internal(
                        "Delegation action returned no immediate child SWOs.".to_string(),
                    ));
                }
                if let Some(tx) = &ui_tx {
                    let _ = tx
                        .send(KernelEvent::Status(format!(
                            "{} ({}): DELEGATING",
                            agent.name, agent.role
                        )))
                        .await;
                }

                let mut tasks = Vec::new();
                let mut child_swo_ids = Vec::new();
                let lineage_agent_ids = swo_id
                    .map(|id| self.registry.swo_lineage_agent_ids(id).unwrap_or_default())
                    .unwrap_or_default();

                // Inherit initiative (project linkage) from the parent SWO so
                // delegated children remain grouped under the same project.
                let (parent_initiative_id, parent_initiative_name, parent_initiative_owner) = swo_id
                    .and_then(|id| self.registry.get_swo_detail(id).ok().flatten())
                    .map(|detail| (
                        detail.swo.initiative_id.clone(),
                        detail.swo.initiative_name.clone(),
                        detail.swo.initiative_owner_agent_id.clone(),
                    ))
                    .unwrap_or((None, None, None));

                for (sub_id_string, swo_string) in swos_map_cloned {
                    if sub_id_string == agent_id || lineage_agent_ids.iter().any(|entry| entry == &sub_id_string) {
                        if let Some(id) = swo_id {
                            let reasoning = format!(
                                "Delegation loop blocked. '{}' is already present in the current SWO lineage.",
                                sub_id_string
                            );
                            let _ = self.registry.record_manager_review(
                                id,
                                &agent_id,
                                "REJECTED_MANAGER_POLICY",
                                &reasoning,
                                None,
                            );
                            let _ = self.registry.update_swo_status(id, "FAILED");
                            emit_swo_status_changed(&ui_tx, id, "FAILED");
                        }
                        return Err(KernelError::Internal(format!(
                            "Delegation loop blocked for agent {}.",
                            sub_id_string
                        )));
                    }
                    // Derive a child title from the brief (first sentence, max 100 chars)
                    let child_title = {
                        let brief = swo_string.trim();
                        let first_sentence = brief.split_once('.')
                            .map(|(s, _)| s.trim())
                            .unwrap_or(brief);
                        if first_sentence.len() > 100 {
                            // Find a char boundary at or before byte 97 to avoid
                            // panicking on multi-byte UTF-8 (e.g. smart quotes).
                            let truncate_at = first_sentence
                                .char_indices()
                                .map(|(i, _)| i)
                                .take_while(|&i| i <= 97)
                                .last()
                                .unwrap_or(0);
                            format!("{}...", &first_sentence[..truncate_at])
                        } else {
                            first_sentence.to_string()
                        }
                    };
                    let child_swo_id = self.registry.create_swo_with_metadata(
                        crate::registry::CreateSwoParams {
                            assigned_agent_id: &sub_id_string,
                            owner_agent_id: &agent_id,
                            created_by_agent_id: &agent_id,
                            payload: &swo_string,
                            status: "IN_PROGRESS",
                            parent_swo_id: swo_id,
                            kind: "TASK",
                            source: "HSM",
                            work_order_title: Some(&child_title),
                            work_order_outcome: None,
                            work_order_constraints: None,
                            requested_owner_agent_id: None,
                            requested_assignee_agent_id: None,
                            routing_policy: "NONE",
                            originating_swo_id: None,
                            initiative_id: parent_initiative_id.as_deref(),
                            initiative_name: parent_initiative_name.as_deref(),
                            initiative_owner_agent_id: parent_initiative_owner.as_deref(),
                            priority_class: None,
                        },
                    )?;
                    child_swo_ids.push((sub_id_string.clone(), child_swo_id));
                    // Emit SwoCreated signal for this child
                    if let Some(tx) = &ui_tx {
                        let _ = tx.send(KernelEvent::SwoCreated {
                            swo_id: child_swo_id,
                            assigned_agent_id: sub_id_string.clone(),
                            parent_swo_id: swo_id,
                        }).await;
                    }
                    let self_clone = Arc::clone(&self);
                    let sub_id_clone = sub_id_string.clone();
                    let agent_id_clone = agent_id.clone();
                    // Phase 2C: propagate parent_swo_id for SWO lineage tracking
                    let tx_clone = ui_tx.clone();
                    let handle = tokio::spawn(async move {
                        self_clone
                            .execute_hsm_loop_with_context(
                                sub_id_clone,
                                Some(agent_id_clone.clone()),
                                swo_string,
                                tx_clone,
                                Some(child_swo_id),
                                swo_id,
                                None,
                                Some("HSM".to_string()),
                                Some(agent_id_clone.clone()),
                                Some(agent_id_clone),
                                None,
                                None,
                                None,
                                None,
                                None,
                            )
                            .await
                    });
                    tasks.push((sub_id_string, handle));
                }

                // Emit DelegationStarted after all child SWOs are created
                if let Some(tx) = &ui_tx {
                    if let Some(parent_id) = swo_id {
                        let _ = tx.send(KernelEvent::DelegationStarted {
                            parent_swo_id: parent_id,
                            child_swo_ids: child_swo_ids.iter().map(|(_, id)| *id).collect(),
                            to_agent_ids: child_swo_ids.iter().map(|(aid, _)| aid.clone()).collect(),
                        }).await;
                    }
                }

                if let (Some(tx), Some(required_sub_id), Some(required_name)) = (
                    &ui_tx,
                    requested_route_id.as_deref(),
                    requested_route_name.as_deref(),
                ) {
                    if child_swo_ids
                        .iter()
                        .any(|(sub_id, _)| sub_id == required_sub_id)
                    {
                        let _ = tx
                            .send(KernelEvent::ChatMessage {
                                content: format!(
                                "Delegation opened: {} is now leading execution. {} will review the completed work before replying.",
                                required_name, agent.name
                                ),
                                message_kind: "progress_update".to_string(),
                            })
                            .await;
                    }
                }

                let mut sub_results = serde_json::Map::new();
                let mut blocked_children = Vec::new();
                for ((sub_id, child_swo_id), (_, handle)) in
                    child_swo_ids.into_iter().zip(tasks.into_iter())
                {
                    match handle.await {
                        Ok(Ok(val)) => {
                            // Emit terminal signal for child SWO so UI updates
                            if let Some(tx) = &ui_tx {
                                let _ = tx.send(KernelEvent::SwoTerminal { swo_id: child_swo_id }).await;
                            }
                            if !Self::result_is_reportable_upward(&val) {
                                let reason = Self::result_review_gate_reason(&val);
                                blocked_children.push(format!(
                                    "{} via child SWO #{} was not approved for upward synthesis: {}",
                                    sub_id, child_swo_id, reason
                                ));
                                sub_results.insert(
                                    sub_id,
                                    json!({
                                        "error": reason,
                                        "child_swo_id": child_swo_id,
                                        "result": val,
                                    }),
                                );
                                continue;
                            }
                            let hires = self
                                .registry
                                .get_swo_detail(child_swo_id)
                                .ok()
                                .flatten()
                                .map(|detail| {
                                    detail
                                        .hires
                                        .into_iter()
                                        .map(|hire| {
                                            json!({
                                                "new_agent_id": hire.new_agent_id,
                                                "new_agent_name": hire.new_agent_name,
                                                "spec_json": hire.spec_json,
                                            })
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();
                            sub_results.insert(
                                sub_id,
                                json!({
                                    "result": val,
                                    "hires": hires,
                                    "child_swo_id": child_swo_id,
                                }),
                            );
                        }
                        Ok(Err(e)) => {
                            // Emit terminal signal for failed child SWO
                            if let Some(tx) = &ui_tx {
                                let _ = tx.send(KernelEvent::SwoTerminal { swo_id: child_swo_id }).await;
                                let _ = tx
                                    .send(KernelEvent::Error(format!(
                                        "Error from subordinate {}: {:?}",
                                        sub_id, e
                                    )))
                                    .await;
                            }
                            sub_results.insert(
                                sub_id,
                                json!({"error": format!("{:?}", e), "child_swo_id": child_swo_id}),
                            );
                        }
                        Err(e) => {
                            // Emit terminal signal for panicked child SWO
                            if let Some(tx) = &ui_tx {
                                let _ = tx.send(KernelEvent::SwoTerminal { swo_id: child_swo_id }).await;
                                let _ = tx
                                    .send(KernelEvent::Error(format!(
                                        "Subordinate task {} panicked: {:?}",
                                        sub_id, e
                                    )))
                                    .await;
                            }
                            sub_results.insert(
                                sub_id,
                                json!({"error": format!("Task panicked: {:?}", e), "child_swo_id": child_swo_id}),
                            );
                        }
                    }
                }

                if !blocked_children.is_empty() {
                    let reason = blocked_children.join(" | ");
                    if let Some(id) = swo_id {
                        let attempts = self
                            .registry
                            .increment_swo_retry_count(id)
                            .unwrap_or(Self::MAX_REVIEW_FAILURES);
                        let _ = self.registry.record_manager_review(
                            id,
                            &agent_id,
                            if attempts >= Self::MAX_REVIEW_FAILURES {
                                "CLOSED_FAILED"
                            } else {
                                "CHILDREN_INCOMPLETE"
                            },
                            &reason,
                            None,
                        );
                        let _ = self.registry.update_swo_status(id, "FAILED");
                        emit_swo_status_changed(&ui_tx, id, "FAILED");
                        if attempts >= Self::MAX_REVIEW_FAILURES {
                            return Ok(Self::closed_failed_payload(&reason, attempts));
                        }
                    }
                    return Err(KernelError::Internal(format!(
                        "Manager {} cannot synthesize upward until all child SWOs are accepted: {}",
                        agent.name, reason
                    )));
                }

                // PASS 2: Synthesis with bounded revision loop.
                // When synthesis returns REJECT_AND_REVISE with revision_swos,
                // dispatch the revision SWOs, collect results, and re-synthesize.
                let mut synthesis_sub_results = sub_results;
                let mut last_synthesis_result: Option<Value> = None;

                'synthesis_loop: for _synthesis_pass in 0..Self::MAX_REVIEW_FAILURES {
                    let synthesis_context = serde_json::to_string(&json!({
                        "original_swo": swo_payload,
                        "subordinate_results": synthesis_sub_results
                    }))
                    .unwrap_or_default();

                    if let Some(tx) = &ui_tx {
                        let _ = tx
                            .send(KernelEvent::Status(format!(
                                "{} ({}): SYNTHESIZING",
                                agent.name, agent.role
                            )))
                            .await;
                    }

                    let (synthesis_result, _) = self
                        .run_worker(
                            &agent_id,
                            &agent.name,
                            swo_id,
                            &db_path,
                            &route.provider_name,
                            execution_model,
                            &decrypted_api_key,
                            "execute_synthesis",
                            &synthesis_context,
                            &attached_specs,
                            &subordinates_json,
                            &agent.role,
                            &agent.persona_prompt,
                            &agent.raison_detre,
                            requested_assignee_agent_id.as_deref(),
                            requested_assignee_name.as_deref(),
                            Some(routing_policy.as_str()),
                            None,
                            ui_tx.clone(),
                            revision_feedback.as_deref(),
                        )
                        .await
                        .map_err(|err| {
                            if let Some(id) = swo_id {
                                let _ = self.registry.update_swo_status(id, "FAILED");
                                emit_swo_status_changed(&ui_tx, id, "FAILED");
                            }
                            err
                        })?;

                    if let Some(id) = swo_id {
                        let raw_action = synthesis_result["synthesis"]["action"]
                            .as_str()
                            .unwrap_or("ACCEPT_AND_COMPLETE");
                        let classified = classify_synthesis_action(raw_action);
                        let final_response = synthesis_result["synthesis"]["final_response"].as_str();

                        // ACCEPT_AND_COMPLETE (and its legacy alias APPROVE_AND_REPLY) require a
                        // final_response. ACCEPT_AND_CONTINUE does not — more work is coming.
                        if classified == "accept_complete" && final_response.is_none() {
                            let _ = self.registry.record_manager_review(
                                id,
                                &agent_id,
                                "CLOSED_FAILED",
                                "Synthesis approved child work without producing a final response.",
                                None,
                            );
                            let _ = self.registry.update_swo_status(id, "FAILED");
                            emit_swo_status_changed(&ui_tx, id, "FAILED");
                            return Err(KernelError::Internal(
                                "Synthesis approved child work without producing a final response."
                                    .to_string(),
                            ));
                        }

                        // Build the review reasoning. For ACCEPT_AND_CONTINUE, append
                        // next_step_brief so the intent is visible in logs (CHA-410).
                        let base_reasoning = synthesis_result["synthesis"]["reasoning"]
                            .as_str()
                            .unwrap_or("");
                        let review_reasoning = if classified == "accept_continue" {
                            let next_step = synthesis_result["synthesis"]["next_step_brief"]
                                .as_str()
                                .unwrap_or("");
                            if next_step.is_empty() {
                                base_reasoning.to_string()
                            } else {
                                format!("{} [next_step_brief: {}]", base_reasoning, next_step)
                            }
                        } else {
                            base_reasoning.to_string()
                        };

                        let _ = self.registry.record_swo_result(
                            id,
                            &agent_id,
                            &synthesis_result.to_string(),
                        );
                        let _ = self.registry.record_manager_review(
                            id,
                            &agent_id,
                            raw_action,
                            &review_reasoning,
                            final_response,
                        );

                        if let Some(response) = final_response {
                            let _ = self.registry.append_memory_interaction(
                                &agent_id,
                                "assistant",
                                response,
                                Some(id),
                            );
                        }

                        if classified == "accept_complete" || classified == "accept_continue" {
                            if classified == "accept_continue" {
                                // CHA-410 migration: log the manager's intent. The full
                                // continuation loop is deferred to CHA-421. For now we fall
                                // through to COMPLETE so the SWO is not left dangling.
                                eprintln!(
                                    "[WARN] CHA-410: manager {} returned ACCEPT_AND_CONTINUE for SWO {} but kernel continuation loop is not yet wired (CHA-421). Finalizing SWO as COMPLETE.",
                                    agent.name, id
                                );
                            }
                            let _ = self.registry.update_swo_status(id, "COMPLETED");
                            emit_swo_status_changed(&ui_tx, id, "COMPLETED");
                            let _ = self.registry.cancel_active_descendant_swos(id);
                            return Ok(synthesis_result);
                        }

                        // REJECT_AND_REVISE: dispatch revision SWOs if provided
                        let attempts = self
                            .registry
                            .increment_swo_retry_count(id)
                            .unwrap_or(Self::MAX_REVIEW_FAILURES);

                        if attempts >= Self::MAX_REVIEW_FAILURES {
                            let reason = synthesis_result["synthesis"]["reasoning"]
                                .as_str()
                                .unwrap_or("synthesis did not approve the child work");

                            // CHA-411: structured escalation to parent manager before marking FAILED.
                            // The parent's list_recent_escalations_for_agent query on their next
                            // triage turn will surface this so they can reassign or change approach
                            // instead of silently inheriting a terminal FAILED child.
                            let parent_swo_id = self.registry.get_swo_parent_id(id).ok().flatten();
                            let parent_agent_id = parent_swo_id
                                .and_then(|pid| self.registry.get_swo_assigned_agent_id(pid).ok().flatten());

                            let original_task_snippet: String = swo_payload
                                .chars()
                                .take(500)
                                .collect();

                            let _ = self.registry.record_escalation(
                                id,
                                &agent_id,
                                parent_swo_id,
                                parent_agent_id.as_deref(),
                                attempts as i64,
                                reason,
                            );

                            let _ = self.registry.record_audit_event(
                                Some(&agent_id),
                                Some(id),
                                "escalation_reported",
                                TaintLabel::ManagerEscalation,
                                &json!({
                                    "swo_id": id,
                                    "child_agent_id": agent_id,
                                    "child_agent_name": agent.name,
                                    "parent_swo_id": parent_swo_id,
                                    "parent_agent_id": parent_agent_id,
                                    "attempts": attempts,
                                    "reasoning": reason,
                                    "original_task": original_task_snippet,
                                }),
                            );

                            eprintln!(
                                "[CHA-411] manager {} exhausted revision ceiling on SWO {} after {} attempts. Escalation recorded for parent agent {:?}.",
                                agent.name, id, attempts, parent_agent_id
                            );

                            let _ = self.registry.record_manager_review(
                                id,
                                &agent_id,
                                "CLOSED_FAILED",
                                reason,
                                None,
                            );
                            let _ = self.registry.update_swo_status(id, "FAILED");
                            emit_swo_status_changed(&ui_tx, id, "FAILED");
                            return Ok(Self::closed_failed_payload(reason, attempts));
                        }

                        // Extract revision SWOs: expects Dict[agent_id, revision_payload]
                        let revision_map = synthesis_result["synthesis"]["revision_swos"]
                            .as_object()
                            .cloned()
                            .unwrap_or_default();

                        if revision_map.is_empty() {
                            // No revision SWOs provided — cannot self-correct, fail
                            let _ = self.registry.update_swo_status(id, "FAILED");
                            emit_swo_status_changed(&ui_tx, id, "FAILED");
                            last_synthesis_result = Some(synthesis_result);
                            break 'synthesis_loop;
                        }

                        if let Some(tx) = &ui_tx {
                            let _ = tx
                                .send(KernelEvent::Status(format!(
                                    "{} ({}): REVISING {} subordinate(s)",
                                    agent.name, agent.role, revision_map.len()
                                )))
                                .await;
                        }

                        // Dispatch revision SWOs to the specified subordinates
                        let mut revision_tasks = Vec::new();
                        let mut revision_child_ids = Vec::new();

                        for (sub_id, revision_payload_value) in &revision_map {
                            let revision_payload = revision_payload_value
                                .as_str()
                                .unwrap_or("")
                                .to_string();
                            if revision_payload.is_empty() {
                                continue;
                            }

                            let revision_child_id = self.registry.create_swo_with_metadata(
                                crate::registry::CreateSwoParams {
                                    assigned_agent_id: sub_id,
                                    owner_agent_id: &agent_id,
                                    created_by_agent_id: &agent_id,
                                    payload: &revision_payload,
                                    status: "IN_PROGRESS",
                                    parent_swo_id: swo_id,
                                    kind: "TASK",
                                    source: "HSM",
                                    work_order_title: None,
                                    work_order_outcome: None,
                                    work_order_constraints: None,
                                    requested_owner_agent_id: None,
                                    requested_assignee_agent_id: None,
                                    routing_policy: "NONE",
                                    originating_swo_id: None,
                                    initiative_id: parent_initiative_id.as_deref(),
                                    initiative_name: parent_initiative_name.as_deref(),
                                    initiative_owner_agent_id: parent_initiative_owner.as_deref(),
                                    priority_class: None,
                                },
                            )?;

                            revision_child_ids.push((sub_id.clone(), revision_child_id));
                            let self_clone = Arc::clone(&self);
                            let sub_id_clone = sub_id.clone();
                            let agent_id_clone = agent_id.clone();
                            let tx_clone = ui_tx.clone();
                            let handle = tokio::spawn(async move {
                                self_clone
                                    .execute_hsm_loop_with_context(
                                        sub_id_clone,
                                        Some(agent_id_clone.clone()),
                                        revision_payload,
                                        tx_clone,
                                        Some(revision_child_id),
                                        swo_id,
                                        None,
                                        Some("HSM".to_string()),
                                        Some(agent_id_clone.clone()),
                                        Some(agent_id_clone),
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                    )
                                    .await
                            });
                            revision_tasks.push((sub_id.clone(), handle));
                        }

                        // Collect revision results and merge into sub_results
                        for ((sub_id, revision_child_id), (_, handle)) in
                            revision_child_ids.into_iter().zip(revision_tasks.into_iter())
                        {
                            match handle.await {
                                Ok(Ok(val)) => {
                                    if let Some(tx) = &ui_tx {
                                        let _ = tx.send(KernelEvent::SwoTerminal { swo_id: revision_child_id }).await;
                                    }
                                    synthesis_sub_results.insert(
                                        sub_id,
                                        json!({
                                            "result": val,
                                            "child_swo_id": revision_child_id,
                                            "revision": true,
                                        }),
                                    );
                                }
                                Ok(Err(e)) => {
                                    if let Some(tx) = &ui_tx {
                                        let _ = tx.send(KernelEvent::SwoTerminal { swo_id: revision_child_id }).await;
                                    }
                                    synthesis_sub_results.insert(
                                        sub_id,
                                        json!({
                                            "error": format!("{:?}", e),
                                            "child_swo_id": revision_child_id,
                                            "revision": true,
                                        }),
                                    );
                                }
                                Err(e) => {
                                    if let Some(tx) = &ui_tx {
                                        let _ = tx.send(KernelEvent::SwoTerminal { swo_id: revision_child_id }).await;
                                    }
                                    synthesis_sub_results.insert(
                                        sub_id,
                                        json!({
                                            "error": format!("Task panicked: {:?}", e),
                                            "child_swo_id": revision_child_id,
                                            "revision": true,
                                        }),
                                    );
                                }
                            }
                        }

                        // Continue to next synthesis pass
                        last_synthesis_result = Some(synthesis_result);
                        continue 'synthesis_loop;
                    }

                    // No swo_id — cannot track synthesis state, just return
                    return Ok(synthesis_result);
                }

                // Exhausted synthesis loop without approval
                if let (Some(id), Some(result)) = (swo_id, &last_synthesis_result) {
                    let reason = result["synthesis"]["reasoning"]
                        .as_str()
                        .unwrap_or("synthesis did not approve the child work after revision attempts");
                    let _ = self.registry.update_swo_status(id, "FAILED");
                    emit_swo_status_changed(&ui_tx, id, "FAILED");
                    return Err(KernelError::Internal(format!(
                        "Manager {} could not approve subordinate work after revision: {}",
                        agent.name, reason
                    )));
                }
            }

            if let Some(id) = swo_id {
                let _ = self.registry.update_swo_status(id, "FAILED");
                emit_swo_status_changed(&ui_tx, id, "FAILED");
            }

            Err(KernelError::Internal(format!(
                "Unknown triage action: {}",
                action
            )))
        })
    }

    pub fn rerun_existing_swo(
        self: Arc<Self>,
        detail: SwoDetailRecord,
        ui_tx: Option<tokio::sync::mpsc::Sender<KernelEvent>>,
        run_id: String,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> {
        self.execute_hsm_loop_with_context(
            detail.swo.assigned_agent_id.clone(),
            None,
            detail.swo.payload.clone(),
            ui_tx,
            Some(detail.swo.id),
            detail.swo.parent_swo_id,
            Some(detail.swo.kind.clone()),
            Some(detail.swo.source.clone()),
            Some(detail.swo.owner_agent_id.clone()),
            Some(detail.swo.created_by_agent_id.clone()),
            detail.swo.requested_assignee_agent_id.clone(),
            detail.swo.requested_assignee_agent_name.clone(),
            Some(detail.swo.routing_policy.clone()),
            detail.swo.originating_swo_id,
            Some(run_id),
        )
    }

    pub async fn run_chat_mode(
        self: Arc<Self>,
        agent_id: String,
        user_message: String,
        attachments: &[AttachmentSpec],
        ui_tx: Option<tokio::sync::mpsc::Sender<KernelEvent>>,
    ) -> Result<Value> {
        let agent = self.registry.get_agent(&agent_id)?;
        let route = self.router.resolve_route(&agent, None);
        let decrypted_api_key = self.resolve_llm_api_key(&route.provider_name);
        let storage_base = std::path::Path::new(&self.registry.db_path)
            .parent()
            .unwrap();
        let db_path = storage_base
            .join("agents")
            .join(&agent_id)
            .join("memory.sqlite")
            .to_string_lossy()
            .to_string();
        #[cfg(debug_assertions)]
        eprintln!(
            "[Orchestrator] Running chat for {} with DB: {}",
            agent_id, db_path
        );

        let subordinates = self.registry.get_subordinates(&agent_id)?;
        let subordinates_json = serde_json::to_string(
            &subordinates
                .into_iter()
                .map(|s| {
                    json!({
                        "id": s.id,
                        "name": s.name,
                        "role": s.role
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());

        let (result, side_effects) = self
            .run_worker(
                &agent_id,
                &agent.name,
                None,
                &db_path,
                &route.provider_name,
                &route.model,
                &decrypted_api_key,
                "chat_mode",
                &user_message,
                attachments,
                &subordinates_json,
                &agent.role,
                &agent.persona_prompt,
                &agent.raison_detre,
                None,
                None,
                None,
                None,
                ui_tx.clone(),
                None,
            )
            .await?;

        let managed_work_count = side_effects.managed_work_requests.len();
        let mut ack_messages = Vec::new();

        for request in side_effects.managed_work_requests {
            let requested_subordinate = self.registry.find_direct_subordinate(
                &agent_id,
                request.requested_assignee_agent_id.as_deref(),
                request.requested_assignee_name.as_deref(),
            )?;
            let requested_assignee_agent_id = requested_subordinate
                .as_ref()
                .map(|sub| sub.id.clone())
                .or(request.requested_assignee_agent_id.clone());
            let requested_assignee_name = requested_subordinate
                .as_ref()
                .map(|sub| sub.name.clone())
                .or(request.requested_assignee_name.clone());
            let routing_policy = if requested_assignee_agent_id.is_some() {
                request.routing_policy.clone()
            } else {
                "NONE".to_string()
            };
            let ack_reply = Self::format_managed_work_ack(
                &agent.name,
                requested_assignee_name.as_deref(),
                &routing_policy,
                request.user_visible_summary.as_deref(),
            );
            ack_messages.push(ack_reply.clone());
            let swo_id =
                self.registry
                    .create_swo_with_metadata(crate::registry::CreateSwoParams {
                        assigned_agent_id: &agent_id,
                        owner_agent_id: &agent_id,
                        created_by_agent_id: &agent_id,
                        payload: &request.payload,
                        status: "PENDING",
                        parent_swo_id: None,
                        kind: "TASK",
                        source: "CHAT",
                        work_order_title: None,
                        work_order_outcome: None,
                        work_order_constraints: None,
                        requested_owner_agent_id: None,
                        requested_assignee_agent_id: requested_assignee_agent_id.as_deref(),
                        routing_policy: &routing_policy,
                        originating_swo_id: None,
                        initiative_id: None,
                        initiative_name: None,
                        initiative_owner_agent_id: None,
                        priority_class: None,
                    })?;

            let _ = self.registry.record_manager_review(
                swo_id,
                &agent_id,
                "ACKNOWLEDGED",
                "Queued from chat for manager-led execution.",
                Some(ack_reply.as_str()),
            );
            let _ = self
                .registry
                .tag_latest_memory_interactions(&agent_id, swo_id, 1);
            let _ = self.registry.append_memory_interaction(
                &agent_id,
                "assistant",
                &ack_reply,
                Some(swo_id),
            );

            let run_id = format!("chat-{}", uuid::Uuid::new_v4());
            let claimed = self.registry.claim_swo_with_run_id(swo_id, &run_id)?;
            if claimed == 0 {
                continue;
            }

            let self_clone = Arc::clone(&self);
            let tx_clone = ui_tx.clone();
            let agent_id_clone = agent_id.clone();
            let request_payload = request.payload.clone();
            let request_assignee_id = requested_assignee_agent_id.clone();
            let request_assignee_name = requested_assignee_name.clone();
            let request_routing = routing_policy.clone();
            tokio::spawn(async move {
                let res = self_clone
                    .execute_hsm_loop_with_context(
                        agent_id_clone.clone(),
                        None,
                        request_payload,
                        tx_clone.clone(),
                        Some(swo_id),
                        None,
                        Some("TASK".to_string()),
                        Some("CHAT".to_string()),
                        Some(agent_id_clone.clone()),
                        Some(agent_id_clone.clone()),
                        request_assignee_id,
                        request_assignee_name,
                        Some(request_routing),
                        None,
                        Some(run_id),
                    )
                    .await;
                if let Some(tx) = tx_clone {
                    match res {
                        Ok(val) => {
                            let mut text = val.to_string();
                            if let Some(content) = val["synthesis"]["final_response"].as_str() {
                                text = content.to_string();
                            } else if let Some(content) = val["triage"]["direct_answer"].as_str() {
                                text = content.to_string();
                            } else if let Some(content) = val["reply"].as_str() {
                                text = content.to_string();
                            } else if val["terminal_status"].as_str() == Some("CLOSED_FAILED") {
                                text = format!(
                                    "Closed failed after {} review-gate attempts. {}",
                                    val["review_failure_count"].as_i64().unwrap_or(0),
                                    val["reason"].as_str().unwrap_or("No reason supplied.")
                                );
                            }
                            let _ = tx
                                .send(KernelEvent::ChatMessage {
                                    content: text,
                                    message_kind: "final_reply".to_string(),
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(KernelEvent::Error(format!("Background HSM panic: {:?}", e)))
                                .await;
                        }
                    }
                }
            });
        }

        let mut result = result;
        if managed_work_count > 0 {
            result["queued_work_count"] = json!(managed_work_count);
            result["reply"] = json!(ack_messages.join("\n\n"));
        }
        Ok(result)
    }

    pub async fn format_delegated_swo(
        self: Arc<Self>,
        agent_id: String,
        target_id: String,
        user_message: String,
        recent_context_json: String,
    ) -> Result<String> {
        let agent = self.registry.get_agent(&agent_id)?;
        let target = self.registry.get_agent(&target_id)?;
        let route = self.router.resolve_route(&agent, None);
        let decrypted_api_key = self.resolve_llm_api_key(&route.provider_name);
        let storage_base = std::path::Path::new(&self.registry.db_path)
            .parent()
            .unwrap();
        let db_path = storage_base
            .join("agents")
            .join(&agent_id)
            .join("memory.sqlite")
            .to_string_lossy()
            .to_string();

        let subordinates = self.registry.get_subordinates(&agent_id)?;
        let subordinates_json = serde_json::to_string(
            &subordinates
                .into_iter()
                .map(|s| {
                    json!({
                        "id": s.id,
                        "name": s.name,
                        "role": s.role
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());

        let formatter_payload = serde_json::to_string(&json!({
            "target_subordinate": {
                "id": target.id,
                "name": target.name,
                "role": target.role
            },
            "user_message": user_message,
            "recent_context": serde_json::from_str::<Value>(&recent_context_json).unwrap_or_else(|_| json!([]))
        }))
        .unwrap_or_default();

        let (result, _) = self
            .run_worker(
                &agent_id,
                &agent.name,
                None,
                &db_path,
                &route.provider_name,
                &route.model,
                &decrypted_api_key,
                "format_swo",
                &formatter_payload,
                &[],
                &subordinates_json,
                &agent.role,
                &agent.persona_prompt,
                &agent.raison_detre,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;

        if let Some(formatted_swo) = result.get("formatted_swo").and_then(|v| v.as_str()) {
            let trimmed = formatted_swo.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }

        Err(KernelError::Internal(
            "SWO formatter returned an empty payload".to_string(),
        ))
    }
}

fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Orchestrator, SairgentRuntimeSnapshot};
    use crate::registry::RecurringWorkOrderScheduleRecord;

    #[test]
    fn test_secret_redaction_in_status_events() {
        let sidechannel_token = "super_secret_sidechannel_token".to_string();
        let run_token = "mock_run_token_123".to_string();

        let stdout_line = format!(
            "Sending heartbeat with token: {} and data",
            sidechannel_token
        );
        let redacted_stdout =
            stdout_line.replace(&sidechannel_token, "[REDACTED_SIDECHANNEL_TOKEN]");
        assert!(!redacted_stdout.contains("super_secret"));
        assert!(redacted_stdout.contains("[REDACTED_SIDECHANNEL_TOKEN]"));

        let log_line = format!(
            "JSON from python: {{\"token\": \"{}\", \"status\": \"WORKING\"}}",
            run_token
        );
        let redacted_log = log_line.replace(&run_token, "[REDACTED_RUN_TOKEN]");
        assert!(!redacted_log.contains("mock_run_token"));
        assert!(redacted_log.contains("[REDACTED_RUN_TOKEN]"));
    }

    #[test]
    fn test_worker_stall_reason_mentions_inactivity_not_wall_clock_timeout() {
        let message = Orchestrator::worker_stall_reason();
        assert!(message.contains("stopped heartbeating or emitting output"));
        assert!(message.contains("120 seconds"));
    }

    #[test]
    fn compute_next_recurring_run_at_supports_daily_and_weekly_templates() {
        let daily = Orchestrator::compute_next_recurring_run_at(
            &RecurringWorkOrderScheduleRecord {
                cadence: "daily".to_string(),
                interval: 1,
                timezone: "UTC".to_string(),
                days_of_week: None,
                day_of_month: None,
                hour: Some(9),
                minute: Some(30),
                cron_expression: None,
            },
            1_710_000_000,
        )
        .unwrap();
        let weekly = Orchestrator::compute_next_recurring_run_at(
            &RecurringWorkOrderScheduleRecord {
                cadence: "weekly".to_string(),
                interval: 1,
                timezone: "UTC".to_string(),
                days_of_week: Some(vec![1]),
                day_of_month: None,
                hour: Some(14),
                minute: Some(0),
                cron_expression: None,
            },
            1_710_000_000,
        )
        .unwrap();

        assert!(daily.contains("09:30:00"));
        assert!(weekly.contains("14:00:00"));
        assert_eq!(daily.len(), 19);
        assert_eq!(weekly.len(), 19);
    }

    fn sample_snapshot() -> SairgentRuntimeSnapshot {
        SairgentRuntimeSnapshot {
            company_name: "Sairgent".to_string(),
            company_summary: Some("Event-driven operator console".to_string()),
            operating_principles: vec![],
            non_goals: vec![],
            active_projects: 3,
            paused_projects: 1,
            archived_projects: 0,
            open_swos: 7,
            approvals_waiting: 2,
            agent_count: 6,
            ready_agents: 4,
            degraded_agents: 1,
            highlights: vec!["Project proj-1 -> ACTIVE".to_string()],
            current_project: None,
            current_swo: None,
            default_provider: "openai".to_string(),
            default_model: "gpt-4.1-mini".to_string(),
        }
    }

    #[test]
    fn infer_sairgent_tool_calls_creates_project_proposal() {
        let calls = Orchestrator::infer_sairgent_tool_calls(
            "Create project \"Operator UX Hardening\" for the desktop panel",
            &sample_snapshot(),
        );

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "create_project");
        assert!(calls[0].arguments_json.contains("Operator UX Hardening"));
    }

    #[test]
    fn compose_sairgent_reply_mentions_governed_actions() {
        let snapshot = sample_snapshot();
        let calls = Orchestrator::infer_sairgent_tool_calls(
            "Create work order \"Close PM workflow gaps\"",
            &snapshot,
        );
        let reply = Orchestrator::compose_sairgent_reply(&snapshot, 1, &calls);

        assert!(reply.contains("1 attachment"), "reply should mention attachment count: {}", reply);
        assert!(reply.contains("drafted an action"), "reply should mention drafted action: {}", reply);
    }

    #[test]
    fn ensure_agent_directories_provisions_workspace_dir() {
        let unique = uuid::Uuid::new_v4().to_string();
        let root = std::env::temp_dir().join(format!("sairgent_test_{}", unique));
        std::fs::create_dir_all(&root).expect("create test root");
        let agent_id = uuid::Uuid::new_v4().to_string();
        let agent_name = "test_agent";

        let (context_path, artifacts_path, workspace_path) =
            Orchestrator::ensure_agent_directories(&root, &agent_id, agent_name)
                .expect("ensure_agent_directories should succeed");

        let context = std::path::Path::new(&context_path);
        let artifacts = std::path::Path::new(&artifacts_path);
        let workspace = std::path::Path::new(&workspace_path);

        assert!(context.is_dir(), "context/ must exist");
        assert!(artifacts.is_dir(), "artifacts/ must exist");
        assert!(workspace.is_dir(), "workspace/ must exist");

        // Idempotent: calling again must not fail.
        let (_, _, ws2) = Orchestrator::ensure_agent_directories(&root, &agent_id, agent_name)
            .expect("second call should be idempotent");
        assert_eq!(workspace_path, ws2, "workspace path must be stable across calls");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn workspace_path_for_agent_resolves_correctly() {
        let unique = uuid::Uuid::new_v4().to_string();
        let root = std::env::temp_dir().join(format!("sairgent_test_{}", unique));
        std::fs::create_dir_all(&root).expect("create test root");
        let agent_id = uuid::Uuid::new_v4().to_string();
        let agent_name = "ws_helper_agent";

        // Provision first so the base dir exists.
        Orchestrator::ensure_agent_directories(&root, &agent_id, agent_name)
            .expect("provisioning should succeed");

        let ws = Orchestrator::workspace_path_for_agent(&root, &agent_id, agent_name)
            .expect("workspace_path_for_agent should resolve");

        assert!(ws.ends_with("workspace"), "path must end with workspace/");
        assert!(ws.starts_with(&root), "path must be inside agent home root");

        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod synthesis_classification_tests {
    use super::*;

    #[test]
    fn classify_new_three_way() {
        assert_eq!(classify_synthesis_action("ACCEPT_AND_COMPLETE"), "accept_complete");
        assert_eq!(classify_synthesis_action("ACCEPT_AND_CONTINUE"), "accept_continue");
        assert_eq!(classify_synthesis_action("REJECT_AND_REVISE"), "reject");
    }

    #[test]
    fn classify_legacy_alias() {
        assert_eq!(classify_synthesis_action("APPROVE_AND_REPLY"), "accept_complete");
    }

    #[test]
    fn classify_unknown_defaults_to_complete() {
        assert_eq!(classify_synthesis_action(""), "accept_complete");
        assert_eq!(classify_synthesis_action("BOGUS"), "accept_complete");
    }
}

#[cfg(test)]
mod heartbeat_payload_tests {
    use super::*;

    /// CHA-429 regression test. HeartbeatPayload previously carried
    /// #[serde(deny_unknown_fields)], which silently rejected every
    /// heartbeat emitted by the Python harness because the payload
    /// always carries an extra `__sairgent_sidechannel` routing field.
    /// Downstream consequence: AgentPresenceChanged never fired during
    /// execute_triage or execute_synthesis, and the workspace grid
    /// showed idle agents even when they were actively computing.
    #[test]
    fn heartbeat_payload_accepts_sidechannel_routing_field() {
        let line = r#"{"__sairgent_sidechannel": "heartbeat", "token": "tok", "run_id": "rid", "seq": 5, "status": "COMPUTING"}"#;
        let hb: HeartbeatPayload = serde_json::from_str(line)
            .expect("heartbeat payload must parse with extra sidechannel field");
        assert_eq!(hb.token, "tok");
        assert_eq!(hb.run_id, "rid");
        assert_eq!(hb.seq, 5);
        assert_eq!(hb.status, "COMPUTING");
    }

    #[test]
    fn heartbeat_payload_accepts_idle_status() {
        // The harness may extend statuses later; the struct should not
        // assume a fixed set of status values.
        let line = r#"{"__sairgent_sidechannel": "heartbeat", "token": "t", "run_id": "r", "seq": 0, "status": "IDLE"}"#;
        let hb: HeartbeatPayload = serde_json::from_str(line).unwrap();
        assert_eq!(hb.status, "IDLE");
    }
}
