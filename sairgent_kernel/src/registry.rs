use crate::audit::{compute_chain_hash, AuditEventRecord, TaintLabel};
use crate::error::{KernelError, Result};
use crate::manifest::{AgentManifestV1, ProviderConfigV1, ProviderProtocolFamily};
use crate::seed::{AgentInteractionCount, RuntimeArchiveCounts, RuntimeContext};
use crate::skills::{
    build_runtime_skill_index, normalize_skill_metadata, slugify_name, AgentSkillBindingRecord,
    RuntimeSkillIndexEntry, SkillMetadataV1, SkillRecord, SkillUpsertRequest, SkillVersionRecord,
};
use crate::tools::{
    active_web_search_provider, built_in_tool_catalog, find_built_in_tool,
    mcp_validation, required_capability_slug, AgentMcpBindingRecord, AgentToolBindingRecord,
    CliToolRecord, CliToolUpsertRequest, McpConnectorRecord, McpConnectorUpsertRequest, McpTransport,
};
use crate::workflow::WorkflowRun;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

const COMPUTING_FRESH_MS: i64 = 4_000;
const READY_FRESH_MS: i64 = 30_000;
const OFFLINE_AFTER_MS: i64 = 90_000;
const DEFAULT_MAX_AGENTS: usize = 50;
// CHA-427 / CHA-428 — default cap on direct reports per manager.
// Runtime-metadata overridable via `max_direct_reports_per_manager`.
// Prevents one rogue manager from hoarding the org-wide `max_agents` budget.
// CHA-428 bump: original cap of 8 was too tight — Perry's default seeded team
// already sits at 8+ (Cat, Felicity, Lex, Lois, Iris, Robin, Oliver, Lucy,
// plus any dynamic hires), so the legitimate "hire 5 dev agents for the team"
// request blew the cap on the first attempt. 16 gives reasonable headroom
// without abandoning the sprawl guardrail.
const DEFAULT_MAX_DIRECT_REPORTS_PER_MANAGER: usize = 16;

#[derive(Clone, Debug)]
pub struct AgentIdentity {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub persona_prompt: String,
    pub raison_detre: String,
    pub default_provider: String,
    pub default_model: String,
    pub cron_interval_seconds: Option<i64>,
    pub triage_model: Option<String>,
    pub execution_model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentOrgClass {
    Manager,
    LeadIc,
    Specialist,
}

impl AgentOrgClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manager => "manager",
            Self::LeadIc => "lead_ic",
            Self::Specialist => "specialist",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "manager" => Self::Manager,
            "lead_ic" => Self::LeadIc,
            _ => Self::Specialist,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationPolicyValue {
    MustDelegateWhenFitExists,
    MayDelegate,
    MayNotDelegate,
}

impl DelegationPolicyValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MustDelegateWhenFitExists => "must_delegate_when_fit_exists",
            Self::MayDelegate => "may_delegate",
            Self::MayNotDelegate => "may_not_delegate",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "may_delegate" => Self::MayDelegate,
            "may_not_delegate" => Self::MayNotDelegate,
            _ => Self::MustDelegateWhenFitExists,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPolicyValue {
    SynthesizeOnly,
    DirectAllowed,
}

impl ReviewPolicyValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SynthesizeOnly => "synthesize_only",
            Self::DirectAllowed => "direct_allowed",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "direct_allowed" => Self::DirectAllowed,
            _ => Self::SynthesizeOnly,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationDecisionValue {
    Delegate,
    SelfExecute,
    HireThenDelegate,
    EscalateTeamGap,
}

impl DelegationDecisionValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Delegate => "DELEGATE",
            Self::SelfExecute => "SELF_EXECUTE",
            Self::HireThenDelegate => "HIRE_THEN_DELEGATE",
            Self::EscalateTeamGap => "ESCALATE_TEAM_GAP",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TeamGoalStatusValue {
    Active,
    Paused,
    Archived,
}

impl TeamGoalStatusValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Paused => "PAUSED",
            Self::Archived => "ARCHIVED",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentOrgProfileRecord {
    pub agent_id: String,
    pub org_class: String,
    pub team_goal_ids: Vec<String>,
    pub delegation_policy: String,
    pub review_policy: String,
    pub managed_domains: Vec<String>,
    pub quality_rubric: String,
    pub max_delegation_depth: i64,
    pub max_parallel_delegates: i64,
    pub manager_can_hire: bool,
    pub manager_can_restructure: bool,
    pub updated_at: String,
}

impl AgentOrgProfileRecord {
    pub fn default_for_agent(agent: &AgentIdentity) -> Self {
        let role_lower = agent.role.to_lowercase();
        let inferred_manager = agent.parent_id.is_none()
            || matches!(agent.name.as_str(), "Perry")
            || role_lower.contains("chief")
            || role_lower.contains("manager")
            // Catch C-suite abbreviations (CMO, CTO, CFO, COO, CRO, CIO, CISO)
            || matches!(role_lower.as_str(), "cmo" | "cto" | "cfo" | "coo" | "cro" | "cio" | "ciso");
        let org_class = if inferred_manager {
            AgentOrgClass::Manager
        } else {
            AgentOrgClass::Specialist
        };
        let managed_domains = vec![agent.role.to_lowercase()];
        Self {
            agent_id: agent.id.clone(),
            org_class: org_class.as_str().to_string(),
            team_goal_ids: Vec::new(),
            delegation_policy: if org_class == AgentOrgClass::Manager {
                DelegationPolicyValue::MustDelegateWhenFitExists
                    .as_str()
                    .to_string()
            } else {
                DelegationPolicyValue::MayDelegate.as_str().to_string()
            },
            review_policy: if org_class == AgentOrgClass::Manager {
                ReviewPolicyValue::SynthesizeOnly.as_str().to_string()
            } else {
                ReviewPolicyValue::DirectAllowed.as_str().to_string()
            },
            managed_domains,
            quality_rubric: format!(
                "Deliver work aligned to {}'s mission without role drift.",
                agent.name
            ),
            max_delegation_depth: 3,
            max_parallel_delegates: 3,
            manager_can_hire: org_class == AgentOrgClass::Manager,
            manager_can_restructure: org_class == AgentOrgClass::Manager,
            updated_at: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeamGoalRecord {
    pub goal_id: String,
    pub team_owner_agent_id: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub priority: String,
    pub success_criteria: String,
    pub managed_domain_tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DelegationDecisionRecord {
    pub id: String,
    pub swo_id: i64,
    pub manager_agent_id: String,
    pub decision: String,
    pub candidate_assignees: Vec<String>,
    pub selected_agent_id: Option<String>,
    pub fit_reason: Option<String>,
    pub exception_code: Option<String>,
    pub exception_reason: Option<String>,
    pub team_gap_code: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeamGapRecord {
    pub id: String,
    pub swo_id: i64,
    pub manager_agent_id: String,
    pub gap_code: String,
    pub summary: String,
    pub recommended_action: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct ActiveSwoRecord {
    pub id: i64,
    pub assigned_agent_id: String,
    pub assigned_agent_name: String,
    pub owner_agent_id: String,
    pub owner_agent_name: String,
    pub created_by_agent_id: String,
    pub created_by_agent_name: String,
    pub status: String,
    pub payload: String,
    pub kind: String,
    pub source: String,
    pub work_order_title: Option<String>,
    pub work_order_outcome: Option<String>,
    pub work_order_constraints: Option<String>,
    pub requested_owner_agent_id: Option<String>,
    pub requested_owner_agent_name: Option<String>,
    pub requested_assignee_agent_id: Option<String>,
    pub requested_assignee_agent_name: Option<String>,
    pub routing_policy: String,
    pub parent_swo_id: Option<i64>,
    pub originating_swo_id: Option<i64>,
    pub initiative_id: Option<String>,
    pub initiative_name: Option<String>,
    pub initiative_owner_agent_id: Option<String>,
    pub initiative_owner_agent_name: Option<String>,
    pub priority_class: Option<String>,
    pub created_at: String,
    pub retry_count: i32,
    pub revision_feedback: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SwoResultRecord {
    pub id: i64,
    pub swo_id: i64,
    pub producer_agent_id: String,
    pub producer_agent_name: String,
    pub result_json: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct ManagerReviewRecord {
    pub id: i64,
    pub swo_id: i64,
    pub reviewer_agent_id: String,
    pub reviewer_agent_name: String,
    pub action: String,
    pub reasoning: String,
    pub final_response: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub status: String,
    pub priority: String,
    pub lead_agent_id: Option<String>,
    pub target_outcome: String,
    pub tags: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct ProjectStatusUpdateRecord {
    pub project_id: String,
    pub previous_status: Option<String>,
    pub next_status: String,
    pub reason: Option<String>,
    pub updated_by: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurringWorkOrderScheduleRecord {
    pub cadence: String,
    pub interval: i64,
    pub timezone: String,
    pub days_of_week: Option<Vec<i64>>,
    pub day_of_month: Option<i64>,
    pub hour: Option<i64>,
    pub minute: Option<i64>,
    pub cron_expression: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RecurringWorkOrderTemplateRecord {
    pub template_id: String,
    pub project_id: Option<String>,
    pub source_swo_id: Option<i64>,
    pub owner_agent_id: String,
    pub owner_agent_name: String,
    pub assignee_agent_id: Option<String>,
    pub assignee_agent_name: Option<String>,
    pub name: String,
    pub title: String,
    pub outcome: String,
    pub constraints: Option<String>,
    pub priority: String,
    pub include_prior_artifacts: bool,
    pub schedule: RecurringWorkOrderScheduleRecord,
    pub status: String,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_run_status: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct RecurringWorkOrderRunRecord {
    pub run_id: String,
    pub template_id: String,
    pub swo_id: Option<i64>,
    pub project_id: Option<String>,
    pub run_number: i64,
    pub status: String,
    pub trigger_source: String,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub error_message: Option<String>,
    pub artifact_ids: Vec<i64>,
}

pub struct CreateRecurringWorkOrderTemplateParams<'a> {
    pub template_id: &'a str,
    pub project_id: Option<&'a str>,
    pub source_swo_id: Option<i64>,
    pub owner_agent_id: &'a str,
    pub assignee_agent_id: Option<&'a str>,
    pub name: &'a str,
    pub title: &'a str,
    pub outcome: &'a str,
    pub constraints: Option<&'a str>,
    pub priority: &'a str,
    pub include_prior_artifacts: bool,
    pub schedule: &'a RecurringWorkOrderScheduleRecord,
    pub status: &'a str,
    pub next_run_at: Option<&'a str>,
    pub last_run_at: Option<&'a str>,
    pub last_run_status: Option<&'a str>,
}

pub struct UpdateRecurringWorkOrderTemplateParams<'a> {
    pub template_id: &'a str,
    pub project_id: Option<Option<&'a str>>,
    pub source_swo_id: Option<Option<i64>>,
    pub owner_agent_id: Option<&'a str>,
    pub assignee_agent_id: Option<Option<&'a str>>,
    pub name: Option<&'a str>,
    pub title: Option<&'a str>,
    pub outcome: Option<&'a str>,
    pub constraints: Option<Option<&'a str>>,
    pub priority: Option<&'a str>,
    pub include_prior_artifacts: Option<bool>,
    pub schedule: Option<&'a RecurringWorkOrderScheduleRecord>,
    pub status: Option<&'a str>,
    pub next_run_at: Option<Option<&'a str>>,
    pub last_run_at: Option<Option<&'a str>>,
    pub last_run_status: Option<Option<&'a str>>,
}

pub struct CreateRecurringWorkOrderRunParams<'a> {
    pub run_id: &'a str,
    pub template_id: &'a str,
    pub swo_id: Option<i64>,
    pub project_id: Option<&'a str>,
    pub run_number: i64,
    pub status: &'a str,
    pub trigger_source: &'a str,
    pub queued_at: Option<&'a str>,
    pub started_at: Option<&'a str>,
    pub completed_at: Option<&'a str>,
    pub error_message: Option<&'a str>,
    pub artifact_ids: &'a [i64],
}

pub struct UpdateRecurringWorkOrderRunParams<'a> {
    pub run_id: &'a str,
    pub swo_id: Option<Option<i64>>,
    pub status: Option<&'a str>,
    pub started_at: Option<Option<&'a str>>,
    pub completed_at: Option<Option<&'a str>>,
    pub error_message: Option<Option<&'a str>>,
    pub artifact_ids: Option<&'a [i64]>,
}

#[derive(Clone, Debug)]
pub struct OutboxArtifactRecord {
    pub id: i64,
    pub swo_id: i64,
    pub agent_id: String,
    pub agent_name: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub parent_swo_id: Option<i64>,
    pub source_work_order_title: Option<String>,
    pub source_work_order_outcome: Option<String>,
    pub source_status: Option<String>,
    pub absolute_path: String,
    pub filename: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct AttachmentRecord {
    pub id: String,
    pub source_kind: String,
    pub display_name: String,
    pub original_path: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub originating_swo_id: Option<i64>,
    pub originating_artifact_id: Option<i64>,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct DeliveredAttachmentRecord {
    pub attachment: AttachmentRecord,
    pub swo_id: i64,
    pub inbox_path: Option<String>,
    pub delivery_status: String,
    pub delivery_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct InboxItemRecord {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub priority: String,
    pub title: String,
    pub summary: String,
    pub created_at: String,
    pub updated_at: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub swo_id: Option<i64>,
    pub artifact_id: Option<i64>,
    pub agent_id: Option<String>,
    pub resolved_at: Option<String>,
    pub resolution: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InboxAttentionSummaryRecord {
    pub open_inbox_items: i64,
    pub open_approval_items: i64,
    pub open_deliverable_items: i64,
    pub open_blocked_items: i64,
}

pub struct UpsertInboxItemParams<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub status: &'a str,
    pub priority: &'a str,
    pub title: &'a str,
    pub summary: &'a str,
    pub project_id: Option<&'a str>,
    pub project_name: Option<&'a str>,
    pub swo_id: Option<i64>,
    pub artifact_id: Option<i64>,
    pub agent_id: Option<&'a str>,
}

#[derive(Clone, Debug)]
pub struct AgentHireRecord {
    pub id: i64,
    pub swo_id: i64,
    pub manager_agent_id: String,
    pub manager_agent_name: String,
    pub new_agent_id: String,
    pub new_agent_name: String,
    pub spec_json: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct InteractionExcerpt {
    pub agent_id: String,
    pub agent_name: String,
    pub interaction_id: i64,
    pub timestamp: String,
    pub role: String,
    pub mode: String,
    pub interaction_kind: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct WorkerRunRecord {
    pub id: i64,
    pub run_id: String,
    pub swo_id: Option<i64>,
    pub agent_id: String,
    pub agent_name: String,
    pub backend: String,
    pub mode: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub artifact_count: i64,
    pub structured_output_present: bool,
    pub blocked_reason: Option<String>,
    pub failure_reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WorkflowRunRecord {
    pub id: i64,
    pub template_id: String,
    pub template_name: String,
    pub entry_agent_id: String,
    pub entry_agent_name: String,
    pub status: String,
    pub compiled_json: String,
    pub root_swo_id: Option<i64>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct TokenUsageRecord {
    pub id: i64,
    pub run_id: String,
    pub swo_id: Option<i64>,
    pub agent_id: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub requests: i64,
    pub cost_usd: Option<f64>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentTokenTotals {
    pub agent_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: Option<f64>,
    pub run_count: i64,
}

#[derive(Clone, Debug, Default)]
pub struct OutboxArtifactListFilters {
    pub agent_id: Option<String>,
    pub swo_id: Option<i64>,
    pub query: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug)]
pub struct ProjectOutputRecord {
    pub id: String,
    pub output_kind: String,
    pub artifact_id: Option<i64>,
    pub result_id: Option<i64>,
    pub swo_id: i64,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub agent_id: String,
    pub agent_name: String,
    pub display_name: String,
    pub created_at: String,
    pub absolute_path: Option<String>,
    pub preview_text: Option<String>,
    pub source_work_order_title: Option<String>,
    pub source_work_order_outcome: Option<String>,
    pub source_status: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProjectActivityItemRecord {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub actor_type: String,
    pub timestamp: String,
    pub title: String,
    pub summary: String,
    pub detail: Option<String>,
    pub status: Option<String>,
    pub swo_id: Option<i64>,
    pub artifact_id: Option<i64>,
    pub related_agent_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProjectWorkspaceRecord {
    pub project: ProjectRecord,
    pub swos: Vec<ActiveSwoRecord>,
    pub status_updates: Vec<ProjectStatusUpdateRecord>,
    pub activity: Vec<ProjectActivityItemRecord>,
    pub outputs: Vec<ProjectOutputRecord>,
}

#[derive(Clone, Debug)]
pub struct AgentFileRecord {
    pub id: String,
    pub agent_id: String,
    pub kind: String,
    pub source_kind: String,
    pub display_name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub created_at: String,
    pub swo_id: Option<i64>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub artifact_id: Option<i64>,
    pub attachment_id: Option<String>,
    pub workspace_path: Option<String>,
    pub absolute_path: Option<String>,
    pub delivery_status: Option<String>,
    pub source_work_order_title: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AgentHistoryEventRecord {
    pub id: String,
    pub agent_id: String,
    pub kind: String,
    pub timestamp: String,
    pub title: String,
    pub summary: String,
    pub detail: Option<String>,
    pub status: Option<String>,
    pub swo_id: Option<i64>,
    pub artifact_id: Option<i64>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SwoDetailRecord {
    pub swo: ActiveSwoRecord,
    pub delegation_status: String,
    pub delegation_debug: DelegationDebugRecord,
    pub attachments: Vec<DeliveredAttachmentRecord>,
    pub results: Vec<SwoResultRecord>,
    pub reviews: Vec<ManagerReviewRecord>,
    pub artifacts: Vec<OutboxArtifactRecord>,
    pub hires: Vec<AgentHireRecord>,
    pub child_swos: Vec<ActiveSwoRecord>,
    pub linked_swos: Vec<ActiveSwoRecord>,
    pub interactions: Vec<InteractionExcerpt>,
    pub worker_runs: Vec<WorkerRunRecord>,
    pub execution_lineage: ExecutionLineageRecord,
}

#[derive(Clone, Debug)]
pub struct AgentSummaryRecord {
    pub id: String,
    pub name: String,
    pub role: String,
}

#[derive(Clone, Debug)]
pub struct AgentPresenceRecord {
    pub agent_id: String,
    pub raw_status: Option<String>,
    pub presence: String,
    pub last_seen_unix_ms: Option<i64>,
    pub last_seen_age_ms: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct HeartbeatEventRecord {
    pub run_id: String,
    pub status: String,
    pub last_seen_unix_ms: i64,
    pub last_seen_age_ms: i64,
    pub seq: i64,
}

#[derive(Clone, Debug)]
pub struct DirectReportSummaryRecord {
    pub id: String,
    pub name: String,
    pub role: String,
    pub cron_enabled: bool,
    pub presence: String,
    pub last_seen_unix_ms: Option<i64>,
    pub last_seen_age_ms: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct AgentTreeNodeRecord {
    pub id: String,
    pub name: String,
    pub role: String,
    pub manager: Option<AgentSummaryRecord>,
    pub org_profile: AgentOrgProfileRecord,
    pub depth: usize,
    pub is_direct_report: bool,
    pub direct_report_count: usize,
    pub descendant_count: usize,
    pub cron_enabled: bool,
    pub presence: String,
    pub last_seen_unix_ms: Option<i64>,
    pub last_seen_age_ms: Option<i64>,
    pub last_cron_fired_at: Option<String>,
    pub children: Vec<AgentTreeNodeRecord>,
    pub default_provider: String,
    pub model: String,
    pub triage_model: Option<String>,
    pub execution_model: Option<String>,
    pub raison_detre: String,
    pub persona_prompt: String,
}

#[derive(Clone, Debug)]
pub struct AgentSwoSummaryRecord {
    pub swo: ActiveSwoRecord,
    pub actual_child_assignees: Vec<String>,
    pub child_swo_count: usize,
    pub review_status: String,
    pub mismatch_flags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PulseJournalEntry {
    pub id: i64,
    pub cadence: String,
    pub run_id: Option<String>,
    pub agent_id: String,
    pub entry_type: String,
    pub summary: String,
    pub detail_json: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CadenceStateRecord {
    pub domain: String,
    pub check_interval_hours: i64,
    pub last_checked_at: Option<String>,
    pub last_run_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// View struct for a single escalation row — returned by `list_recent_escalations_for_agent`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EscalationRecord {
    pub id: String,
    pub swo_id: i64,
    pub child_agent_id: String,
    pub parent_swo_id: Option<i64>,
    pub parent_agent_id: Option<String>,
    pub attempts: i64,
    pub reasoning: String,
    pub created_at: i64,
}

#[derive(Clone, Debug)]
pub struct HireDebugRecord {
    pub id: i64,
    pub swo_id: i64,
    pub manager_agent_id: String,
    pub manager_agent_name: String,
    pub new_agent_id: String,
    pub new_agent_name: String,
    pub spec_json: String,
    pub created_at: String,
    pub parent_matches_manager: bool,
    pub actual_parent_agent_id: Option<String>,
    pub actual_parent_agent_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DelegationDebugRecord {
    pub requested_assignee_agent_id: Option<String>,
    pub requested_assignee_agent_name: Option<String>,
    pub routing_policy: String,
    pub actual_child_assignees: Vec<String>,
    pub child_swo_count: usize,
    pub review_status: String,
    pub mismatch_flags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ExecutionLineageRecord {
    pub root_swo: Option<ActiveSwoRecord>,
    pub parent_swo: Option<ActiveSwoRecord>,
    pub child_swos: Vec<ActiveSwoRecord>,
    pub linked_swos: Vec<ActiveSwoRecord>,
    pub hires: Vec<HireDebugRecord>,
}

#[derive(Clone, Debug)]
pub struct AgentDetailRecord {
    pub id: String,
    pub name: String,
    pub role: String,
    pub manager: Option<AgentSummaryRecord>,
    pub org_profile: AgentOrgProfileRecord,
    pub team_goals: Vec<TeamGoalRecord>,
    pub delegation_decisions: Vec<DelegationDecisionRecord>,
    pub team_gaps: Vec<TeamGapRecord>,
    pub direct_reports: Vec<DirectReportSummaryRecord>,
    pub persona_prompt: String,
    pub raison_detre: String,
    pub provider: String,
    pub model: String,
    pub cron_interval_seconds: Option<i64>,
    pub presence: String,
    pub last_seen_unix_ms: Option<i64>,
    pub last_seen_age_ms: Option<i64>,
    pub last_cron_fired_at: Option<String>,
    pub heartbeat_timeline: Vec<HeartbeatEventRecord>,
    pub assigned_swos: Vec<AgentSwoSummaryRecord>,
    pub owned_swos: Vec<AgentSwoSummaryRecord>,
    pub created_swos: Vec<AgentSwoSummaryRecord>,
    pub recent_hires: Vec<HireDebugRecord>,
    pub interactions: Vec<InteractionExcerpt>,
    pub manifest: AgentManifestV1,
    pub bound_skills: Vec<AgentSkillBindingRecord>,
    pub bound_tools: Vec<AgentToolBindingRecord>,
    pub bound_mcp_connectors: Vec<AgentMcpBindingRecord>,
    pub external_channel_bindings: Vec<ExternalChannelBindingRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionLogEntryRecord {
    pub entry_id: String,
    pub agent_id: String,
    pub mode: String,
    pub summary: String,
    pub rationale: String,
    pub outcome: String,
    pub confidence: Option<f64>,
    pub self_note: Option<String>,
    pub linked_swo_id: Option<i64>,
    pub linked_run_id: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalChannelBindingRecord {
    pub agent_id: String,
    pub channel: String,
    pub enabled: bool,
    pub allowed_chat_id: Option<String>,
    pub allowed_user_id: Option<String>,
    pub has_route_token: bool,
    pub has_secret_token: bool,
    pub last_inbound_at: Option<String>,
    pub last_delivery_at: Option<String>,
    pub last_delivery_status: String,
    pub last_delivery_detail: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalChannelBindingSecretRecord {
    pub binding: ExternalChannelBindingRecord,
    pub route_token: Option<String>,
    pub secret_token: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalChatSessionRecord {
    pub session_id: String,
    pub agent_id: String,
    pub channel: String,
    pub external_chat_id: String,
    pub external_user_id: Option<String>,
    pub conversation_id: String,
    pub last_inbound_message_id: Option<String>,
    pub last_inbound_at: Option<String>,
    pub last_delivery_status: String,
    pub last_delivery_detail: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalChannelDeliveryEventRecord {
    pub id: i64,
    pub agent_id: String,
    pub channel: String,
    pub session_id: Option<String>,
    pub direction: String,
    pub status: String,
    pub detail: String,
    pub external_chat_id: Option<String>,
    pub external_user_id: Option<String>,
    pub external_message_id: Option<String>,
    pub created_at: String,
}

pub struct UpsertExternalChannelBindingParams<'a> {
    pub agent_id: &'a str,
    pub channel: &'a str,
    pub enabled: bool,
    pub allowed_chat_id: Option<&'a str>,
    pub allowed_user_id: Option<&'a str>,
    pub route_token: Option<&'a str>,
    pub secret_token: Option<&'a str>,
}

pub struct TouchExternalChatSessionParams<'a> {
    pub agent_id: &'a str,
    pub channel: &'a str,
    pub external_chat_id: &'a str,
    pub external_user_id: Option<&'a str>,
    pub conversation_id: &'a str,
    pub last_inbound_message_id: Option<&'a str>,
}

pub struct RecordExternalChannelDeliveryEventParams<'a> {
    pub agent_id: &'a str,
    pub channel: &'a str,
    pub session_id: Option<&'a str>,
    pub direction: &'a str,
    pub status: &'a str,
    pub detail: &'a str,
    pub external_chat_id: Option<&'a str>,
    pub external_user_id: Option<&'a str>,
    pub external_message_id: Option<&'a str>,
}

pub struct CreateSwoParams<'a> {
    pub assigned_agent_id: &'a str,
    pub owner_agent_id: &'a str,
    pub created_by_agent_id: &'a str,
    pub payload: &'a str,
    pub status: &'a str,
    pub parent_swo_id: Option<i64>,
    pub kind: &'a str,
    pub source: &'a str,
    pub work_order_title: Option<&'a str>,
    pub work_order_outcome: Option<&'a str>,
    pub work_order_constraints: Option<&'a str>,
    pub requested_owner_agent_id: Option<&'a str>,
    pub requested_assignee_agent_id: Option<&'a str>,
    pub routing_policy: &'a str,
    pub originating_swo_id: Option<i64>,
    pub initiative_id: Option<&'a str>,
    pub initiative_name: Option<&'a str>,
    pub initiative_owner_agent_id: Option<&'a str>,
    pub priority_class: Option<&'a str>,
}

pub struct Registry {
    conn: Mutex<Connection>,
    pub db_path: String,
}

impl Registry {
    fn replace_team_goal_assignments(
        conn: &Connection,
        agent_id: &str,
        goal_ids: &[String],
    ) -> Result<()> {
        conn.execute(
            "DELETE FROM agent_team_goal_assignments WHERE agent_id = ?1",
            params![agent_id],
        )
        .map_err(KernelError::Database)?;
        for goal_id in goal_ids {
            conn.execute(
                "INSERT OR IGNORE INTO agent_team_goal_assignments (agent_id, goal_id)
                 VALUES (?1, ?2)",
                params![agent_id, goal_id],
            )
            .map_err(KernelError::Database)?;
        }
        Ok(())
    }

    fn provision_agent_storage(&self, agent_id: &str) -> Result<()> {
        let db_dir = self.storage_base_path()?.join("agents").join(agent_id);
        std::fs::create_dir_all(&db_dir).map_err(|e| {
            KernelError::Internal(format!("Failed to create agent storage dir: {}", e))
        })?;

        let db_path = db_dir.join("memory.sqlite");
        let agent_conn = Connection::open(&db_path).map_err(KernelError::Database)?;

        agent_conn
            .execute_batch(
                "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
            )
            .map_err(KernelError::Database)?;

        Self::ensure_agent_memory_schema(&agent_conn)?;
        Ok(())
    }

    fn insert_agent_identity(
        &self,
        id: &str,
        name: &str,
        parent_id: Option<&str>,
        role: &str,
        persona_prompt: &str,
        raison_detre: &str,
        provider: &str,
        model: &str,
        cron_interval_seconds: Option<i64>,
        triage_model: Option<&str>,
        execution_model: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // CHA-159: Defense-in-depth duplicate name check before INSERT.
        // The unique index on agents(name) is the authoritative guard, but
        // this gives callers a clear domain error instead of a raw UNIQUE
        // constraint violation.
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agents WHERE name = ?1)",
                params![name],
                |row| row.get(0),
            )
            .map_err(KernelError::Database)?;
        if exists {
            return Err(KernelError::Internal(format!(
                "Agent with name '{}' already exists",
                name
            )));
        }
        conn.execute(
            "INSERT INTO agents (id, name, parent_id, role, persona_prompt, raison_detre, default_provider, default_model, cron_interval_seconds, triage_model, execution_model)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                name,
                parent_id,
                role,
                persona_prompt,
                raison_detre,
                provider,
                model,
                cron_interval_seconds,
                triage_model,
                execution_model
            ],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    fn count_agents(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))
            .map_err(KernelError::Database)?;
        Ok(count.max(0) as usize)
    }

    fn max_agents_limit(&self) -> Result<usize> {
        let configured = self.get_runtime_metadata("max_agents")?;
        let parsed = configured
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0);
        Ok(parsed.unwrap_or(DEFAULT_MAX_AGENTS))
    }

    /// CHA-427 — per-manager direct-reports cap. Reads the
    /// `max_direct_reports_per_manager` runtime metadata key, falls back to
    /// `DEFAULT_MAX_DIRECT_REPORTS_PER_MANAGER`. Used by
    /// `manager_can_hire_more_subordinates` to gate hire_subordinate_internal
    /// calls before they consume the org-wide `max_agents` budget.
    pub fn max_direct_reports_per_manager(&self) -> Result<usize> {
        let configured = self.get_runtime_metadata("max_direct_reports_per_manager")?;
        let parsed = configured
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0);
        Ok(parsed.unwrap_or(DEFAULT_MAX_DIRECT_REPORTS_PER_MANAGER))
    }

    /// CHA-428 — walk up the org tree from `descendant_id` looking for
    /// `ancestor_id`. Returns true if `ancestor_id` appears as any transitive
    /// parent of `descendant_id` (including `descendant_id == ancestor_id`).
    /// Used to authorize "hire for another manager" — the caller must be an
    /// ancestor of the target manager.
    ///
    /// Bounded to 64 steps of depth to prevent runaway lookups on corrupt data.
    pub fn is_ancestor_of(&self, ancestor_id: &str, descendant_id: &str) -> Result<bool> {
        if ancestor_id == descendant_id {
            return Ok(true);
        }
        let conn = self.conn.lock().unwrap();
        let mut current: Option<String> = Some(descendant_id.to_string());
        for _ in 0..64 {
            let Some(id) = current.clone() else {
                return Ok(false);
            };
            let parent: Option<Option<String>> = conn
                .query_row(
                    "SELECT parent_id FROM agents WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .ok()
                .map(Some)
                .unwrap_or(None);
            match parent {
                Some(Some(p)) => {
                    if p == ancestor_id {
                        return Ok(true);
                    }
                    current = Some(p);
                }
                Some(None) => return Ok(false),
                None => return Ok(false),
            }
        }
        Ok(false)
    }

    /// CHA-428 — authorize and gate a "hire on behalf of another manager"
    /// request. `caller_id` is the agent that emitted the hire_subordinate
    /// sidechannel event; `target_manager_id` is where the new hire will
    /// actually report. Returns `Ok(())` if the hire should proceed, or
    /// `Err(KernelError::Internal(reason))` with a human-readable rejection.
    ///
    /// Three checks, all must pass:
    /// 1. Caller is permitted to hire at all (autonomous_hiring_mode)
    /// 2. Caller is authorized to place under the target:
    ///    - Caller is the root (parent_id is None — e.g., Perry)
    ///    - Caller is an ancestor of the target
    ///    - Caller is the target (normal "hire under me" case)
    /// 3. Target manager's direct-reports count is below the per-manager cap
    ///
    /// When `target_manager_id == caller_id`, this is equivalent to the
    /// single-arg `check_autonomous_hire_allowed` — kept as a shim.
    pub fn check_cross_manager_hire_allowed(
        &self,
        caller_id: &str,
        target_manager_id: &str,
    ) -> Result<()> {
        let mode = self
            .get_runtime_metadata("autonomous_hiring_mode")?
            .unwrap_or_else(|| "ANY_MANAGER".to_string());
        let mode_trimmed = mode.trim().to_uppercase();

        let caller = self.get_agent(caller_id)?;

        // Step 1 — mode check on the CALLER
        match mode_trimmed.as_str() {
            "PERRY_ONLY" => {
                if caller.name != "Perry" {
                    return Err(KernelError::Internal(format!(
                        "autonomous_hiring_mode=PERRY_ONLY: caller '{}' is not permitted to hire. \
                         Only Perry may call hire_subordinate_internal under this policy.",
                        caller.name
                    )));
                }
            }
            "ANY_MANAGER" => {
                let caller_profile = self.get_agent_org_profile(caller_id)?;
                if caller_profile.org_class != "manager" {
                    return Err(KernelError::Internal(format!(
                        "autonomous_hiring_mode=ANY_MANAGER: caller '{}' (org_class={}) is not a Manager.",
                        caller.name, caller_profile.org_class
                    )));
                }
                if !caller_profile.manager_can_hire {
                    return Err(KernelError::Internal(format!(
                        "autonomous_hiring_mode=ANY_MANAGER: manager '{}' has manager_can_hire disabled.",
                        caller.name
                    )));
                }
            }
            "OPEN" => {}
            other => {
                return Err(KernelError::Internal(format!(
                    "unknown autonomous_hiring_mode '{}' in runtime metadata. \
                     Valid values: PERRY_ONLY, ANY_MANAGER, OPEN.",
                    other
                )));
            }
        }

        // Step 2 — authorization: is the caller allowed to place under target?
        if caller_id != target_manager_id {
            let target_agent = self.get_agent(target_manager_id)?;
            // Root (no parent) can place anywhere
            let caller_is_root = caller.parent_id.is_none();
            if !caller_is_root {
                // Otherwise caller must be an ancestor of the target
                if !self.is_ancestor_of(caller_id, target_manager_id)? {
                    return Err(KernelError::Internal(format!(
                        "caller '{}' is not authorized to place hires under '{}'. \
                         Cross-manager hires require the caller to be the root or an \
                         ancestor of the target in the org tree.",
                        caller.name, target_agent.name
                    )));
                }
            }
            // Target must itself be a Manager (can't place hires under a specialist)
            let target_profile = self.get_agent_org_profile(target_manager_id)?;
            if target_profile.org_class != "manager" {
                return Err(KernelError::Internal(format!(
                    "target '{}' (org_class={}) is not a Manager and cannot receive direct reports.",
                    target_agent.name, target_profile.org_class
                )));
            }
        }

        // Step 3 — per-manager cap is on the TARGET, not the caller
        let cap = self.max_direct_reports_per_manager()?;
        let current = self.count_direct_reports(target_manager_id)?;
        if current >= cap {
            let target_agent = self.get_agent(target_manager_id)?;
            return Err(KernelError::Internal(format!(
                "target manager '{}' has {} direct reports, at or above the per-manager cap of {}. \
                 Raise max_direct_reports_per_manager via runtime metadata or restructure an \
                 existing report before hiring more.",
                target_agent.name, current, cap
            )));
        }

        Ok(())
    }

    /// Count direct reports for the given manager agent. Used by CHA-427
    /// hire gate to enforce per-manager sprawl limits.
    pub fn count_direct_reports(&self, manager_id: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agents WHERE parent_id = ?1",
                params![manager_id],
                |row| row.get(0),
            )
            .map_err(KernelError::Database)?;
        Ok(count.max(0) as usize)
    }

    /// CHA-427 — resolve whether *agent_id* is permitted to hire a new
    /// subordinate under the current `autonomous_hiring_mode` policy AND
    /// under the per-manager direct-reports cap. Returns `Ok(())` if the
    /// hire should proceed, or `Err(KernelError::Internal(reason))` with
    /// a human-readable rejection string.
    ///
    /// Modes:
    /// - `PERRY_ONLY` (legacy): only the agent named "Perry" can hire
    /// - `ANY_MANAGER` (default for new seeds): any agent whose org_class
    ///   is Manager can hire
    /// - `OPEN`: any agent with the HireSubordinate capability can hire
    ///
    /// The org-wide `max_agents` limit is still enforced downstream by
    /// `create_agent`; this check adds a *per-manager* ceiling on top.
    pub fn check_autonomous_hire_allowed(&self, agent_id: &str) -> Result<()> {
        let mode = self
            .get_runtime_metadata("autonomous_hiring_mode")?
            .unwrap_or_else(|| "ANY_MANAGER".to_string());
        let mode_trimmed = mode.trim().to_uppercase();

        let agent = self.get_agent(agent_id)?;

        match mode_trimmed.as_str() {
            "PERRY_ONLY" => {
                if agent.name != "Perry" {
                    return Err(KernelError::Internal(format!(
                        "autonomous_hiring_mode=PERRY_ONLY: agent '{}' is not permitted to hire. \
                         Only Perry may call hire_subordinate_internal under this policy.",
                        agent.name
                    )));
                }
            }
            "ANY_MANAGER" => {
                let profile = self.get_agent_org_profile(agent_id)?;
                if profile.org_class != "manager" {
                    return Err(KernelError::Internal(format!(
                        "autonomous_hiring_mode=ANY_MANAGER: agent '{}' (org_class={}) is not a Manager. \
                         Only Manager-class agents may call hire_subordinate_internal under this policy.",
                        agent.name, profile.org_class
                    )));
                }
                if !profile.manager_can_hire {
                    return Err(KernelError::Internal(format!(
                        "autonomous_hiring_mode=ANY_MANAGER: manager '{}' has manager_can_hire disabled in their org profile. \
                         Re-enable via the org policy settings or raise with the operator.",
                        agent.name
                    )));
                }
            }
            "OPEN" => {
                // No mode-level restriction — capability gate in orchestrator already applied
            }
            other => {
                return Err(KernelError::Internal(format!(
                    "unknown autonomous_hiring_mode '{}' in runtime metadata. \
                     Valid values: PERRY_ONLY, ANY_MANAGER, OPEN.",
                    other
                )));
            }
        }

        // Per-manager direct-reports cap applies in every mode.
        let cap = self.max_direct_reports_per_manager()?;
        let current = self.count_direct_reports(agent_id)?;
        if current >= cap {
            return Err(KernelError::Internal(format!(
                "manager '{}' has {} direct reports, at or above the per-manager cap of {}. \
                 Raise max_direct_reports_per_manager via runtime metadata or restructure \
                 an existing report before hiring more.",
                agent.name, current, cap
            )));
        }

        Ok(())
    }

    fn decision_log_max_entries(&self) -> Result<usize> {
        let configured = self.get_runtime_metadata("decision_log_max_entries")?;
        let parsed = configured
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0);
        Ok(parsed.unwrap_or(500))
    }

    fn agent_identity_from_row(row: &Row<'_>) -> rusqlite::Result<AgentIdentity> {
        Ok(AgentIdentity {
            id: row.get(0)?,
            name: row.get(1)?,
            parent_id: row.get(2)?,
            role: row.get(3)?,
            persona_prompt: row.get(4)?,
            raison_detre: row.get(5)?,
            default_provider: row.get(6)?,
            default_model: row.get(7)?,
            cron_interval_seconds: row.get(8)?,
            triage_model: row.get(9)?,
            execution_model: row.get(10)?,
        })
    }

    fn active_swo_from_row(row: &Row<'_>) -> rusqlite::Result<ActiveSwoRecord> {
        Ok(ActiveSwoRecord {
            id: row.get(0)?,
            assigned_agent_id: row.get(1)?,
            assigned_agent_name: row.get(2)?,
            owner_agent_id: row.get(3)?,
            owner_agent_name: row.get(4)?,
            created_by_agent_id: row.get(5)?,
            created_by_agent_name: row.get(6)?,
            status: row.get(7)?,
            payload: row.get(8)?,
            kind: row.get(9)?,
            source: row.get(10)?,
            work_order_title: row.get(11)?,
            work_order_outcome: row.get(12)?,
            work_order_constraints: row.get(13)?,
            requested_owner_agent_id: row.get(14)?,
            requested_owner_agent_name: row.get(15)?,
            requested_assignee_agent_id: row.get(16)?,
            requested_assignee_agent_name: row.get(17)?,
            routing_policy: row.get(18)?,
            parent_swo_id: row.get(19)?,
            originating_swo_id: row.get(20)?,
            initiative_id: row.get(21)?,
            initiative_name: row.get(22)?,
            initiative_owner_agent_id: row.get(23)?,
            initiative_owner_agent_name: row.get(24)?,
            priority_class: row.get(25)?,
            created_at: row.get(26)?,
            retry_count: row.get(27)?,
            revision_feedback: row.get(28).ok(),
        })
    }

    fn active_swo_select_sql(where_clause: &str) -> String {
        format!(
            "
            SELECT
                s.id,
                s.assigned_agent_id,
                COALESCE(a_assigned.name || ' (' || a_assigned.role || ')', s.assigned_agent_id),
                COALESCE(s.owner_agent_id, s.manager_agent_id),
                COALESCE(a_owner.name || ' (' || a_owner.role || ')', COALESCE(s.owner_agent_id, s.manager_agent_id)),
                COALESCE(s.created_by_agent_id, s.manager_agent_id),
                COALESCE(a_creator.name || ' (' || a_creator.role || ')', COALESCE(s.created_by_agent_id, s.manager_agent_id)),
                s.status,
                s.swo_payload,
                COALESCE(s.kind, 'TASK'),
                COALESCE(s.source, 'HSM'),
                s.work_order_title,
                s.work_order_outcome,
                s.work_order_constraints,
                s.requested_owner_agent_id,
                COALESCE(a_requested_owner.name || ' (' || a_requested_owner.role || ')', s.requested_owner_agent_id),
                s.requested_assignee_agent_id,
                COALESCE(a_requested.name || ' (' || a_requested.role || ')', s.requested_assignee_agent_id),
                COALESCE(s.routing_policy, 'NONE'),
                s.parent_swo_id,
                s.originating_swo_id,
                s.initiative_id,
                s.initiative_name,
                s.initiative_owner_agent_id,
                COALESCE(a_initiative_owner.name || ' (' || a_initiative_owner.role || ')', s.initiative_owner_agent_id),
                COALESCE(s.priority_class, 'CORE'),
                s.created_at,
                COALESCE(s.retry_count, 0),
                s.revision_feedback
            FROM active_swos s
            LEFT JOIN agents a_assigned ON a_assigned.id = s.assigned_agent_id
            LEFT JOIN agents a_owner ON a_owner.id = COALESCE(s.owner_agent_id, s.manager_agent_id)
            LEFT JOIN agents a_creator ON a_creator.id = COALESCE(s.created_by_agent_id, s.manager_agent_id)
            LEFT JOIN agents a_requested_owner ON a_requested_owner.id = s.requested_owner_agent_id
            LEFT JOIN agents a_requested ON a_requested.id = s.requested_assignee_agent_id
            LEFT JOIN agents a_initiative_owner ON a_initiative_owner.id = s.initiative_owner_agent_id
            {}
            ",
            where_clause
        )
    }

    fn project_from_row(row: &Row<'_>) -> rusqlite::Result<ProjectRecord> {
        Ok(ProjectRecord {
            id: row.get(0)?,
            name: row.get(1)?,
            summary: row.get(2).unwrap_or_default(),
            status: row.get(3)?,
            priority: row.get(4)?,
            lead_agent_id: row.get(5)?,
            target_outcome: row.get(6).unwrap_or_default(),
            tags: row.get(7).unwrap_or_default(),
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }

    fn outbox_artifact_from_row(row: &Row<'_>) -> rusqlite::Result<OutboxArtifactRecord> {
        Ok(OutboxArtifactRecord {
            id: row.get(0)?,
            swo_id: row.get(1)?,
            agent_id: row.get(2)?,
            agent_name: row.get(3)?,
            project_id: row.get(4)?,
            project_name: row.get(5)?,
            parent_swo_id: row.get(6)?,
            source_work_order_title: row.get(7)?,
            source_work_order_outcome: row.get(8)?,
            source_status: row.get(9)?,
            absolute_path: row.get(10)?,
            filename: row.get(11)?,
            created_at: row.get(12)?,
        })
    }

    fn inbox_item_from_row(row: &Row<'_>) -> rusqlite::Result<InboxItemRecord> {
        Ok(InboxItemRecord {
            id: row.get(0)?,
            kind: row.get(1)?,
            status: row.get(2)?,
            priority: row.get(3)?,
            title: row.get(4)?,
            summary: row.get(5)?,
            project_id: row.get(6)?,
            project_name: row.get(7)?,
            swo_id: row.get(8)?,
            artifact_id: row.get(9)?,
            agent_id: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
            resolved_at: row.get(13)?,
            resolution: row.get(14)?,
        })
    }

    fn recurring_template_from_row(
        row: &Row<'_>,
    ) -> rusqlite::Result<RecurringWorkOrderTemplateRecord> {
        let schedule_json: String = row.get(10)?;
        let schedule = serde_json::from_str::<RecurringWorkOrderScheduleRecord>(&schedule_json)
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        Ok(RecurringWorkOrderTemplateRecord {
            template_id: row.get(0)?,
            project_id: row.get(1)?,
            source_swo_id: row.get(2)?,
            owner_agent_id: row.get(3)?,
            owner_agent_name: row.get(4)?,
            assignee_agent_id: row.get(5)?,
            assignee_agent_name: row.get(6)?,
            name: row.get(7)?,
            title: row.get(8)?,
            outcome: row.get(9)?,
            schedule,
            constraints: row.get(11)?,
            priority: row.get(12)?,
            include_prior_artifacts: row.get::<_, i64>(13)? != 0,
            status: row.get(14)?,
            next_run_at: row.get(15)?,
            last_run_at: row.get(16)?,
            last_run_status: row.get(17)?,
            created_at: row.get(18)?,
            updated_at: row.get(19)?,
        })
    }

    fn recurring_template_select_sql(where_clause: &str) -> String {
        format!(
            "
            SELECT
                t.template_id,
                t.project_id,
                t.source_swo_id,
                t.owner_agent_id,
                COALESCE(owner.name || ' (' || owner.role || ')', t.owner_agent_id),
                t.assignee_agent_id,
                COALESCE(assignee.name || ' (' || assignee.role || ')', t.assignee_agent_id),
                t.name,
                t.title,
                t.outcome,
                t.schedule_json,
                t.constraints,
                t.priority,
                t.include_prior_artifacts,
                t.status,
                t.next_run_at,
                t.last_run_at,
                t.last_run_status,
                t.created_at,
                t.updated_at
            FROM rwo_templates t
            LEFT JOIN agents owner ON owner.id = t.owner_agent_id
            LEFT JOIN agents assignee ON assignee.id = t.assignee_agent_id
            {}
            ",
            where_clause
        )
    }

    fn recurring_run_from_row(row: &Row<'_>) -> rusqlite::Result<RecurringWorkOrderRunRecord> {
        let artifact_ids_json: String = row.get(10)?;
        let artifact_ids = serde_json::from_str::<Vec<i64>>(&artifact_ids_json).unwrap_or_default();
        Ok(RecurringWorkOrderRunRecord {
            run_id: row.get(0)?,
            template_id: row.get(1)?,
            swo_id: row.get(2)?,
            project_id: row.get(3)?,
            run_number: row.get(4)?,
            status: row.get(5)?,
            trigger_source: row.get(6)?,
            queued_at: row.get(7)?,
            started_at: row.get(8)?,
            completed_at: row.get(9)?,
            error_message: row.get(11)?,
            artifact_ids,
        })
    }

    fn current_timestamp(conn: &Connection) -> Result<String> {
        conn.query_row("SELECT CURRENT_TIMESTAMP", [], |row| row.get(0))
            .map_err(KernelError::Database)
    }

    fn normalize_rwo_status_from_swo(status: &str) -> String {
        match status {
            "PENDING" => "QUEUED".to_string(),
            "IN_PROGRESS" => "RUNNING".to_string(),
            "COMPLETED" => "COMPLETED".to_string(),
            "CANCELLED" => "CANCELLED".to_string(),
            "FAILED" | "BLOCKED" => "FAILED".to_string(),
            _ => "QUEUED".to_string(),
        }
    }

    fn storage_base_path(&self) -> Result<PathBuf> {
        Path::new(&self.db_path)
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                KernelError::Internal("Failed to resolve registry storage base".to_string())
            })
    }

    pub fn agent_memory_db_path(&self, agent_id: &str) -> Result<PathBuf> {
        Ok(self
            .storage_base_path()?
            .join("agents")
            .join(agent_id)
            .join("memory.sqlite"))
    }

    fn ensure_agent_memory_schema(conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS interactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                swo_id INTEGER,
                mode TEXT NOT NULL DEFAULT 'legacy',
                run_id TEXT,
                interaction_kind TEXT NOT NULL DEFAULT 'message'
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        let _ = conn.execute("ALTER TABLE interactions ADD COLUMN swo_id INTEGER", []);
        let _ = conn.execute(
            "ALTER TABLE interactions ADD COLUMN mode TEXT NOT NULL DEFAULT 'legacy'",
            [],
        );
        let _ = conn.execute("ALTER TABLE interactions ADD COLUMN run_id TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE interactions ADD COLUMN interaction_kind TEXT NOT NULL DEFAULT 'message'",
            [],
        );
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_interactions_swo_id ON interactions(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_interactions_mode ON interactions(mode)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_interactions_run_id ON interactions(run_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS decision_log (
                entry_id TEXT PRIMARY KEY,
                mode TEXT NOT NULL,
                summary TEXT NOT NULL,
                rationale TEXT NOT NULL,
                outcome TEXT NOT NULL,
                confidence REAL,
                self_note TEXT,
                linked_swo_id INTEGER,
                linked_run_id TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_decision_log_created_at ON decision_log(created_at DESC)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_decision_log_mode ON decision_log(mode)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_decision_log_linked_swo_id ON decision_log(linked_swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path).map_err(KernelError::Database)?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(KernelError::Database)?;

        let registry = Self {
            conn: Mutex::new(conn),
            db_path: db_path.to_string(),
        };
        registry.init_schema()?;

        Ok(registry)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                parent_id TEXT,
                role TEXT NOT NULL,
                persona_prompt TEXT,
                raison_detre TEXT NOT NULL,
                default_provider TEXT NOT NULL,
                default_model TEXT NOT NULL,
                bot_token TEXT UNIQUE,
                FOREIGN KEY (parent_id) REFERENCES agents (id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS active_swos (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                assigned_agent_id TEXT NOT NULL,
                manager_agent_id TEXT NOT NULL,
                owner_agent_id TEXT,
                created_by_agent_id TEXT,
                swo_payload TEXT NOT NULL,
                status TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'TASK',
                source TEXT NOT NULL DEFAULT 'HSM',
                work_order_title TEXT,
                work_order_outcome TEXT,
                work_order_constraints TEXT,
                requested_owner_agent_id TEXT,
                requested_assignee_agent_id TEXT,
                routing_policy TEXT NOT NULL DEFAULT 'NONE',
                parent_swo_id INTEGER,
                originating_swo_id INTEGER,
                initiative_id TEXT,
                initiative_name TEXT,
                initiative_owner_agent_id TEXT,
                priority_class TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                retry_count INTEGER NOT NULL DEFAULT 0,
                revision_feedback TEXT,
                FOREIGN KEY (parent_swo_id) REFERENCES active_swos (id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS agent_heartbeats (
                run_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                status TEXT NOT NULL,
                last_seen_unix_ms INTEGER NOT NULL,
                seq INTEGER NOT NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS agent_cron_last_fired (
                agent_id TEXT PRIMARY KEY,
                last_fired_at TEXT NOT NULL,
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agent_heartbeats_agent_id ON agent_heartbeats(agent_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agent_cron_last_fired_at ON agent_cron_last_fired(last_fired_at DESC)",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_active_swos_agent_status ON active_swos(assigned_agent_id, status)",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS swo_results (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                swo_id INTEGER NOT NULL,
                producer_agent_id TEXT NOT NULL,
                result_json TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS manager_reviews (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                swo_id INTEGER NOT NULL,
                reviewer_agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                reasoning TEXT NOT NULL,
                final_response TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS outbox_artifacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                swo_id INTEGER NOT NULL,
                agent_id TEXT NOT NULL,
                absolute_path TEXT NOT NULL,
                filename TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS inbox_items (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                priority TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                project_id TEXT,
                project_name TEXT,
                swo_id INTEGER,
                artifact_id INTEGER,
                agent_id TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                resolved_at DATETIME,
                resolution TEXT,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE SET NULL,
                FOREIGN KEY (artifact_id) REFERENCES outbox_artifacts(id) ON DELETE SET NULL,
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS attachments (
                id TEXT PRIMARY KEY,
                source_kind TEXT NOT NULL,
                display_name TEXT NOT NULL,
                original_path TEXT NOT NULL,
                content_type TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                originating_swo_id INTEGER,
                originating_artifact_id INTEGER,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (originating_swo_id) REFERENCES active_swos(id) ON DELETE SET NULL,
                FOREIGN KEY (originating_artifact_id) REFERENCES outbox_artifacts(id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS message_attachments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL,
                message_ref TEXT NOT NULL,
                attachment_id TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (attachment_id) REFERENCES attachments(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS swo_attachments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                swo_id INTEGER NOT NULL,
                attachment_id TEXT NOT NULL,
                inbox_path TEXT,
                delivery_status TEXT NOT NULL DEFAULT 'PENDING',
                delivery_error TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE CASCADE,
                FOREIGN KEY (attachment_id) REFERENCES attachments(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS agent_hires (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                swo_id INTEGER NOT NULL,
                manager_agent_id TEXT NOT NULL,
                new_agent_id TEXT NOT NULL,
                spec_json TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_swo_results_swo_id ON swo_results(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_manager_reviews_swo_id ON manager_reviews(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_outbox_artifacts_swo_id ON outbox_artifacts(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_inbox_items_status_updated ON inbox_items(status, updated_at DESC)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_inbox_items_kind_status ON inbox_items(kind, status)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_inbox_items_swo_id ON inbox_items(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_message_attachments_message_ref ON message_attachments(agent_id, message_ref)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_swo_attachments_swo_id ON swo_attachments(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_swo_attachments_unique ON swo_attachments(swo_id, attachment_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agent_hires_swo_id ON agent_hires(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS runtime_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS worker_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL UNIQUE,
                swo_id INTEGER,
                agent_id TEXT NOT NULL,
                backend TEXT NOT NULL,
                mode TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                finished_at DATETIME,
                artifact_count INTEGER NOT NULL DEFAULT 0,
                structured_output_present INTEGER NOT NULL DEFAULT 0,
                blocked_reason TEXT,
                failure_reason TEXT,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_worker_runs_swo_id ON worker_runs(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_worker_runs_agent_id ON worker_runs(agent_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS token_usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL,
                swo_id INTEGER,
                agent_id TEXT NOT NULL,
                provider TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                requests INTEGER NOT NULL DEFAULT 0,
                cost_usd REAL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_token_usage_run_id ON token_usage(run_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_token_usage_agent_id ON token_usage(agent_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_token_usage_swo_id ON token_usage(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS agent_manifests (
                agent_id TEXT PRIMARY KEY,
                manifest_json TEXT NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS agent_org_profiles (
                agent_id TEXT PRIMARY KEY,
                org_class TEXT NOT NULL,
                delegation_policy TEXT NOT NULL,
                review_policy TEXT NOT NULL,
                managed_domains_json TEXT NOT NULL DEFAULT '[]',
                quality_rubric TEXT NOT NULL DEFAULT '',
                max_delegation_depth INTEGER NOT NULL DEFAULT 3,
                max_parallel_delegates INTEGER NOT NULL DEFAULT 3,
                manager_can_hire INTEGER NOT NULL DEFAULT 0,
                manager_can_restructure INTEGER NOT NULL DEFAULT 0,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS team_goals (
                goal_id TEXT PRIMARY KEY,
                team_owner_agent_id TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'ACTIVE',
                priority TEXT NOT NULL DEFAULT 'NORMAL',
                success_criteria TEXT NOT NULL DEFAULT '',
                managed_domain_tags_json TEXT NOT NULL DEFAULT '[]',
                archived_at DATETIME,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (team_owner_agent_id) REFERENCES agents(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS agent_team_goal_assignments (
                agent_id TEXT NOT NULL,
                goal_id TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (agent_id, goal_id),
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
                FOREIGN KEY (goal_id) REFERENCES team_goals(goal_id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS delegation_decisions (
                id TEXT PRIMARY KEY,
                swo_id INTEGER NOT NULL,
                manager_agent_id TEXT NOT NULL,
                decision TEXT NOT NULL,
                candidate_assignees_json TEXT NOT NULL DEFAULT '[]',
                selected_agent_id TEXT,
                fit_reason TEXT,
                exception_code TEXT,
                exception_reason TEXT,
                team_gap_code TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE CASCADE,
                FOREIGN KEY (manager_agent_id) REFERENCES agents(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS team_gaps (
                id TEXT PRIMARY KEY,
                swo_id INTEGER NOT NULL,
                manager_agent_id TEXT NOT NULL,
                gap_code TEXT NOT NULL,
                summary TEXT NOT NULL,
                recommended_action TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE CASCADE,
                FOREIGN KEY (manager_agent_id) REFERENCES agents(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS workflow_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                template_id TEXT NOT NULL,
                template_name TEXT NOT NULL,
                entry_agent_id TEXT NOT NULL,
                status TEXT NOT NULL,
                compiled_json TEXT NOT NULL,
                root_swo_id INTEGER,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (entry_agent_id) REFERENCES agents(id) ON DELETE CASCADE,
                FOREIGN KEY (root_swo_id) REFERENCES active_swos(id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_workflow_runs_entry_agent_id ON workflow_runs(entry_agent_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT,
                swo_id INTEGER,
                event_kind TEXT NOT NULL,
                taint_label TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                previous_chain_hash TEXT,
                chain_hash TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE SET NULL,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_audit_events_agent_id ON audit_events(agent_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_audit_events_swo_id ON audit_events(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS external_channel_bindings (
                agent_id TEXT NOT NULL,
                channel TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 0,
                allowed_chat_id TEXT,
                allowed_user_id TEXT,
                secret_token TEXT,
                last_inbound_at DATETIME,
                last_delivery_at DATETIME,
                last_delivery_status TEXT NOT NULL DEFAULT 'idle',
                last_delivery_detail TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (agent_id, channel),
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_external_channel_bindings_channel
             ON external_channel_bindings(channel, enabled)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS external_chat_sessions (
                session_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                channel TEXT NOT NULL,
                external_chat_id TEXT NOT NULL,
                external_user_id TEXT NOT NULL DEFAULT '',
                conversation_id TEXT NOT NULL,
                last_inbound_message_id TEXT,
                last_inbound_at DATETIME,
                last_delivery_status TEXT NOT NULL DEFAULT 'idle',
                last_delivery_detail TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(agent_id, channel, external_chat_id, external_user_id),
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_external_chat_sessions_agent_channel
             ON external_chat_sessions(agent_id, channel, updated_at DESC)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS external_message_receipts (
                channel TEXT NOT NULL,
                external_chat_id TEXT NOT NULL,
                external_message_id TEXT NOT NULL,
                processed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (channel, external_chat_id, external_message_id)
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS external_channel_delivery_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL,
                channel TEXT NOT NULL,
                session_id TEXT,
                direction TEXT NOT NULL,
                status TEXT NOT NULL,
                detail TEXT NOT NULL,
                external_chat_id TEXT,
                external_user_id TEXT,
                external_message_id TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
                FOREIGN KEY (session_id) REFERENCES external_chat_sessions(session_id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_external_channel_delivery_events_created_at
             ON external_channel_delivery_events(created_at DESC)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS skills (
                id TEXT PRIMARY KEY,
                slug TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                source_uri TEXT,
                owner_agent_id TEXT,
                current_version INTEGER NOT NULL DEFAULT 1,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (owner_agent_id) REFERENCES agents(id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS skill_versions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                skill_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                raw_markdown TEXT NOT NULL,
                metadata_json TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(skill_id, version),
                FOREIGN KEY (skill_id) REFERENCES skills(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_skill_versions_skill_id ON skill_versions(skill_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS agent_skill_bindings (
                agent_id TEXT NOT NULL,
                skill_id TEXT NOT NULL,
                binding_status TEXT NOT NULL DEFAULT 'ACTIVE',
                priority INTEGER NOT NULL DEFAULT 100,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (agent_id, skill_id),
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
                FOREIGN KEY (skill_id) REFERENCES skills(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agent_skill_bindings_agent_id ON agent_skill_bindings(agent_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS agent_tool_bindings (
                agent_id TEXT NOT NULL,
                tool_slug TEXT NOT NULL,
                binding_status TEXT NOT NULL DEFAULT 'ACTIVE',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (agent_id, tool_slug),
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agent_tool_bindings_agent_id ON agent_tool_bindings(agent_id)",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS mcp_connectors (
                id TEXT PRIMARY KEY,
                slug TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                transport TEXT NOT NULL CHECK(transport IN ('stdio', 'sse')),
                command TEXT,
                args_json TEXT,
                env_json TEXT,
                url TEXT,
                headers_json TEXT,
                cwd TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_mcp_connectors_slug ON mcp_connectors(slug)",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS agent_mcp_bindings (
                agent_id TEXT NOT NULL,
                connector_id TEXT NOT NULL,
                binding_status TEXT NOT NULL DEFAULT 'ACTIVE',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (agent_id, connector_id),
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
                FOREIGN KEY (connector_id) REFERENCES mcp_connectors(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agent_mcp_bindings_agent_id ON agent_mcp_bindings(agent_id)",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS cli_tools (
                id TEXT PRIMARY KEY,
                slug TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                summary TEXT,
                command TEXT NOT NULL,
                args_json TEXT,
                env_json TEXT,
                cwd TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_cli_tools_slug ON cli_tools(slug)",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                summary TEXT,
                status TEXT NOT NULL DEFAULT 'ACTIVE',
                priority TEXT NOT NULL DEFAULT 'NORMAL',
                lead_agent_id TEXT,
                target_outcome TEXT,
                tags TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (lead_agent_id) REFERENCES agents(id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS project_status_updates (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id TEXT NOT NULL,
                previous_status TEXT,
                next_status TEXT NOT NULL,
                reason TEXT,
                updated_by TEXT NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS rwo_templates (
                template_id TEXT PRIMARY KEY,
                project_id TEXT,
                source_swo_id INTEGER,
                owner_agent_id TEXT NOT NULL,
                assignee_agent_id TEXT,
                name TEXT NOT NULL,
                title TEXT NOT NULL,
                outcome TEXT NOT NULL,
                constraints TEXT,
                priority TEXT NOT NULL,
                include_prior_artifacts INTEGER NOT NULL DEFAULT 0,
                schedule_json TEXT NOT NULL,
                status TEXT NOT NULL,
                next_run_at TEXT,
                last_run_at TEXT,
                last_run_status TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE SET NULL,
                FOREIGN KEY (source_swo_id) REFERENCES active_swos(id) ON DELETE SET NULL,
                FOREIGN KEY (owner_agent_id) REFERENCES agents(id) ON DELETE CASCADE,
                FOREIGN KEY (assignee_agent_id) REFERENCES agents(id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_rwo_templates_status_next_run
             ON rwo_templates(status, next_run_at)",
            [],
        )
        .map_err(KernelError::Database)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS rwo_runs (
                run_id TEXT PRIMARY KEY,
                template_id TEXT NOT NULL,
                swo_id INTEGER,
                project_id TEXT,
                run_number INTEGER NOT NULL,
                status TEXT NOT NULL,
                trigger_source TEXT NOT NULL,
                queued_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                started_at TEXT,
                completed_at TEXT,
                error_message TEXT,
                artifact_ids_json TEXT NOT NULL DEFAULT '[]',
                FOREIGN KEY (template_id) REFERENCES rwo_templates(template_id) ON DELETE CASCADE,
                FOREIGN KEY (swo_id) REFERENCES active_swos(id) ON DELETE SET NULL,
                FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE SET NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_rwo_runs_template_id
             ON rwo_runs(template_id, run_number DESC)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_project_status_updates_project_id ON project_status_updates(project_id, updated_at DESC)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS pulse_journal (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                cadence TEXT NOT NULL,
                run_id TEXT,
                agent_id TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                summary TEXT NOT NULL,
                detail_json TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pulse_journal_cadence_created
             ON pulse_journal(cadence, created_at DESC)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS cadence_state (
                domain TEXT PRIMARY KEY,
                check_interval_hours INTEGER NOT NULL,
                last_checked_at DATETIME,
                last_run_id TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .map_err(KernelError::Database)?;

        // CHA-411: escalation records — structured parent-manager signal when revision ceiling fires.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS escalations (
                id TEXT PRIMARY KEY,
                swo_id INTEGER NOT NULL,
                child_agent_id TEXT NOT NULL,
                parent_swo_id INTEGER,
                parent_agent_id TEXT,
                attempts INTEGER NOT NULL,
                reasoning TEXT NOT NULL,
                created_at INTEGER NOT NULL
            )",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_escalations_parent ON escalations(parent_agent_id, created_at)",
            [],
        )
        .map_err(KernelError::Database)?;

        // Safe migrations — ignore errors if columns already exist
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN bot_token TEXT UNIQUE", []);
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN persona_prompt TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE agents ADD COLUMN cron_interval_seconds INTEGER",
            [],
        );
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN triage_model TEXT", []);
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN execution_model TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN parent_swo_id INTEGER",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute("ALTER TABLE active_swos ADD COLUMN owner_agent_id TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN created_by_agent_id TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN kind TEXT NOT NULL DEFAULT 'TASK'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN source TEXT NOT NULL DEFAULT 'HSM'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN work_order_title TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN work_order_outcome TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN work_order_constraints TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN requested_owner_agent_id TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN requested_assignee_agent_id TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN routing_policy TEXT NOT NULL DEFAULT 'NONE'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN originating_swo_id INTEGER",
            [],
        );
        let _ = conn.execute("ALTER TABLE active_swos ADD COLUMN initiative_id TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN initiative_name TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN initiative_owner_agent_id TEXT",
            [],
        );
        let _ = conn.execute("ALTER TABLE active_swos ADD COLUMN priority_class TEXT", []);
        // Kryptonite finding #3: track the run_id that claimed this SWO so heartbeat staleness
        // is scoped per-execution, not per-agent (prevents healthy sibling runs from masking dead ones).
        let _ = conn.execute("ALTER TABLE active_swos ADD COLUMN current_run_id TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE active_swos ADD COLUMN revision_feedback TEXT",
            [],
        );
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_team_goals_owner ON team_goals(team_owner_agent_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agent_team_goal_assignments_agent ON agent_team_goal_assignments(agent_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_delegation_decisions_swo_id ON delegation_decisions(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_team_gaps_swo_id ON team_gaps(swo_id)",
            [],
        )
        .map_err(KernelError::Database)?;
        let _ = conn.execute(
            "UPDATE active_swos
                 SET owner_agent_id = COALESCE(owner_agent_id, manager_agent_id),
                     created_by_agent_id = COALESCE(created_by_agent_id, manager_agent_id),
                     kind = COALESCE(kind, 'TASK'),
                     source = COALESCE(source, 'HSM'),
                     routing_policy = COALESCE(routing_policy, 'NONE'),
                     priority_class = COALESCE(priority_class, 'CORE')
             WHERE owner_agent_id IS NULL
                OR created_by_agent_id IS NULL
                OR kind IS NULL
                OR source IS NULL
                OR routing_policy IS NULL
                OR priority_class IS NULL",
            [],
        );
        let _ = conn.execute(
            "UPDATE agents SET persona_prompt = COALESCE(persona_prompt, raison_detre) WHERE persona_prompt IS NULL",
            [],
        );

        // CHA-159: Enforce agent name uniqueness at the DB level.
        // CREATE UNIQUE INDEX is idempotent with IF NOT EXISTS and will fail
        // on existing databases only if duplicates already exist (they don't).
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_agents_name_unique ON agents(name)",
            [],
        )
        .map_err(KernelError::Database)?;

        Ok(())
    }

    pub fn hire_subordinate(
        &self,
        name: &str,
        parent_id: Option<&str>,
        role: &str,
        raison_detre: &str,
        provider: &str,
        model: &str,
    ) -> Result<String> {
        self.hire_subordinate_with_cron(name, parent_id, role, raison_detre, provider, model, None)
    }

    pub fn hire_subordinate_with_cron(
        &self,
        name: &str,
        parent_id: Option<&str>,
        role: &str,
        raison_detre: &str,
        provider: &str,
        model: &str,
        cron_interval_seconds: Option<i64>,
    ) -> Result<String> {
        self.hire_subordinate_with_profile_and_cron(
            name,
            parent_id,
            role,
            raison_detre,
            raison_detre,
            provider,
            model,
            cron_interval_seconds,
            None,
            None,
        )
    }

    pub fn hire_subordinate_with_profile_and_cron(
        &self,
        name: &str,
        parent_id: Option<&str>,
        role: &str,
        persona_prompt: &str,
        raison_detre: &str,
        provider: &str,
        model: &str,
        cron_interval_seconds: Option<i64>,
        triage_model: Option<&str>,
        execution_model: Option<&str>,
    ) -> Result<String> {
        // Idempotent check by name. CHA-426: if the agent already exists,
        // refresh its persona/role/raison from the seed so seed edits take
        // effect on kernel restart instead of being frozen at first-boot.
        {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id FROM agents WHERE name = ?1")
                .map_err(KernelError::Database)?;
            let mut rows = stmt.query(params![name]).map_err(KernelError::Database)?;
            if let Some(row) = rows.next().map_err(KernelError::Database)? {
                let existing_id: String = row.get(0).map_err(KernelError::Database)?;
                drop(rows);
                drop(stmt);
                conn.execute(
                    "UPDATE agents SET role = ?2, persona_prompt = ?3, raison_detre = ?4 WHERE id = ?1",
                    params![existing_id, role, persona_prompt, raison_detre],
                )
                .map_err(KernelError::Database)?;
                return Ok(existing_id);
            }
        }

        let id = Uuid::new_v4().to_string();
        self.provision_agent_storage(&id)?;
        self.insert_agent_identity(
            &id,
            name,
            parent_id,
            role,
            persona_prompt,
            raison_detre,
            provider,
            model,
            cron_interval_seconds,
            triage_model,
            execution_model,
        )?;

        let agent = self.get_agent(&id)?;
        self.upsert_agent_manifest(&AgentManifestV1::default_for_agent(&agent))?;
        self.upsert_agent_org_profile(&AgentOrgProfileRecord::default_for_agent(&agent))?;

        Ok(id)
    }

    pub fn create_agent(
        &self,
        name: &str,
        parent_id: Option<&str>,
        role: &str,
        persona_prompt: &str,
        raison_detre: &str,
        provider: &str,
        model: &str,
    ) -> Result<String> {
        if let Some(manager_id) = parent_id {
            let _ = self.get_agent(manager_id)?;
        }

        let limit = self.max_agents_limit()?;
        let current = self.count_agents()?;
        if current >= limit {
            return Err(KernelError::AgentCapExceeded(format!(
                "Refusing to create agent '{}': {} existing agents meets configured limit of {}.",
                name, current, limit
            )));
        }

        {
            let conn = self.conn.lock().unwrap();
            let duplicate: Option<String> = conn
                .query_row(
                    "SELECT id FROM agents WHERE lower(name) = lower(?1)",
                    params![name],
                    |row| row.get(0),
                )
                .optional()
                .map_err(KernelError::Database)?;
            if duplicate.is_some() {
                return Err(KernelError::Internal(format!(
                    "Agent '{}' already exists.",
                    name
                )));
            }
        }

        let id = Uuid::new_v4().to_string();
        self.provision_agent_storage(&id)?;
        self.insert_agent_identity(
            &id,
            name,
            parent_id,
            role,
            persona_prompt,
            raison_detre,
            provider,
            model,
            None,
            None,
            None,
        )?;

        let agent = self.get_agent(&id)?;
        self.upsert_agent_manifest(&AgentManifestV1::least_privilege_for_agent(&agent))?;
        self.upsert_agent_org_profile(&AgentOrgProfileRecord::default_for_agent(&agent))?;

        Ok(id)
    }

    pub fn upsert_agent_manifest(&self, manifest: &AgentManifestV1) -> Result<()> {
        let agent_id = manifest
            .agent_id
            .as_deref()
            .ok_or_else(|| KernelError::Internal("Agent manifest missing agent_id".to_string()))?;
        let manifest_json = serde_json::to_string_pretty(manifest).map_err(|e| {
            KernelError::Internal(format!("Failed to serialize agent manifest: {}", e))
        })?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_manifests (agent_id, manifest_json, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(agent_id) DO UPDATE SET
                 manifest_json = excluded.manifest_json,
                 updated_at = CURRENT_TIMESTAMP",
            params![agent_id, manifest_json],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn update_agent_manifest_profile(&self, manifest: &AgentManifestV1) -> Result<()> {
        let agent_id = manifest
            .agent_id
            .as_deref()
            .ok_or_else(|| KernelError::Internal("Agent manifest missing agent_id".to_string()))?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agents
             SET name = ?1,
                 role = ?2,
                 persona_prompt = ?3,
                 raison_detre = ?4,
                 default_provider = ?5,
                 default_model = ?6,
                 cron_interval_seconds = ?7,
                 triage_model = ?8,
                 execution_model = ?9
             WHERE id = ?10",
            params![
                manifest.name.trim(),
                manifest.role.trim(),
                manifest.persona_prompt.trim(),
                manifest.mission.trim(),
                manifest.provider.provider_name.trim(),
                manifest.provider.model.trim(),
                manifest.schedule.cron_interval_seconds,
                manifest.provider.triage_model,
                manifest.provider.execution_model,
                agent_id,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);
        self.upsert_agent_manifest(manifest)
    }

    /// Bulk-update provider and model for multiple agents in a single transaction.
    /// Updates both the `agents` table columns and the `agent_manifests` JSON blobs.
    /// Returns the count of successfully updated agents.
    pub fn update_agent_models_bulk(
        &self,
        agent_ids: &[String],
        provider: &str,
        model: &str,
    ) -> Result<usize> {
        if agent_ids.is_empty() {
            return Ok(0);
        }
        let provider = provider.trim();
        let model = model.trim();
        if provider.is_empty() || model.is_empty() {
            return Err(KernelError::Internal(
                "Provider and model must be non-empty".to_string(),
            ));
        }
        let protocol_family = ProviderProtocolFamily::from_provider_name(provider);

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(KernelError::Database)?;
        let mut updated = 0usize;

        for agent_id in agent_ids {
            // 1. Update the agents table row
            let rows_changed = tx
                .execute(
                    "UPDATE agents SET default_provider = ?1, default_model = ?2 WHERE id = ?3",
                    params![provider, model, agent_id],
                )
                .map_err(KernelError::Database)?;
            if rows_changed == 0 {
                continue; // agent not found, skip
            }

            // 2. Update the agent_manifests JSON blob (if it exists)
            let manifest_json: Option<String> = tx
                .query_row(
                    "SELECT manifest_json FROM agent_manifests WHERE agent_id = ?1",
                    params![agent_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(KernelError::Database)?;

            if let Some(json_str) = manifest_json {
                let mut manifest: AgentManifestV1 =
                    serde_json::from_str(&json_str).map_err(|e| {
                        KernelError::Internal(format!(
                            "Failed to deserialize manifest for {}: {}",
                            agent_id, e
                        ))
                    })?;
                manifest.provider = ProviderConfigV1 {
                    provider_name: provider.to_string(),
                    model: model.to_string(),
                    protocol_family: protocol_family.clone(),
                    triage_model: manifest.provider.triage_model.clone(),
                    execution_model: manifest.provider.execution_model.clone(),
                };
                let updated_json = serde_json::to_string_pretty(&manifest).map_err(|e| {
                    KernelError::Internal(format!(
                        "Failed to serialize manifest for {}: {}",
                        agent_id, e
                    ))
                })?;
                tx.execute(
                    "UPDATE agent_manifests SET manifest_json = ?1, updated_at = CURRENT_TIMESTAMP WHERE agent_id = ?2",
                    params![updated_json, agent_id],
                )
                .map_err(KernelError::Database)?;
            }

            updated += 1;
        }

        tx.commit().map_err(KernelError::Database)?;
        Ok(updated)
    }

    /// Update individual agent identity fields. Only non-None fields are updated.
    pub fn update_agent_identity(
        &self,
        agent_id: &str,
        role: Option<&str>,
        raison_detre: Option<&str>,
        persona_prompt: Option<&str>,
        default_provider: Option<&str>,
        default_model: Option<&str>,
        triage_model: Option<Option<&str>>,
        execution_model: Option<Option<&str>>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let mut sets = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(v) = role {
            sets.push(format!("role = ?{}", values.len() + 1));
            values.push(Box::new(v.to_string()));
        }
        if let Some(v) = raison_detre {
            sets.push(format!("raison_detre = ?{}", values.len() + 1));
            values.push(Box::new(v.to_string()));
        }
        if let Some(v) = persona_prompt {
            sets.push(format!("persona_prompt = ?{}", values.len() + 1));
            values.push(Box::new(v.to_string()));
        }
        if let Some(v) = default_provider {
            sets.push(format!("default_provider = ?{}", values.len() + 1));
            values.push(Box::new(v.to_string()));
        }
        if let Some(v) = default_model {
            sets.push(format!("default_model = ?{}", values.len() + 1));
            values.push(Box::new(v.to_string()));
        }
        if let Some(v) = triage_model {
            sets.push(format!("triage_model = ?{}", values.len() + 1));
            values.push(Box::new(v.map(|s| s.to_string())));
        }
        if let Some(v) = execution_model {
            sets.push(format!("execution_model = ?{}", values.len() + 1));
            values.push(Box::new(v.map(|s| s.to_string())));
        }

        if sets.is_empty() {
            return Ok(());
        }

        let idx = values.len() + 1;
        let sql = format!("UPDATE agents SET {} WHERE id = ?{}", sets.join(", "), idx);
        values.push(Box::new(agent_id.to_string()));

        let params: Vec<&dyn rusqlite::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        conn.execute(&sql, params.as_slice())
            .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn get_agent_manifest(&self, agent_id: &str) -> Result<AgentManifestV1> {
        let manifest_json: Option<String> = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT manifest_json FROM agent_manifests WHERE agent_id = ?1",
                params![agent_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(KernelError::Database)?
        };

        if let Some(manifest_json) = manifest_json {
            return serde_json::from_str(&manifest_json).map_err(|e| {
                KernelError::Internal(format!("Failed to deserialize agent manifest: {}", e))
            });
        }

        let agent = self.get_agent(agent_id)?;
        let manifest = AgentManifestV1::default_for_agent(&agent);
        self.upsert_agent_manifest(&manifest)?;
        Ok(manifest)
    }

    fn parse_string_list(value: String) -> Vec<String> {
        serde_json::from_str::<Vec<String>>(&value).unwrap_or_default()
    }

    fn serialize_string_list(values: &[String]) -> Result<String> {
        serde_json::to_string(values)
            .map_err(|e| KernelError::Internal(format!("Failed to serialize string list: {}", e)))
    }

    pub fn upsert_agent_org_profile(
        &self,
        profile: &AgentOrgProfileRecord,
    ) -> Result<AgentOrgProfileRecord> {
        let managed_domains_json = Self::serialize_string_list(&profile.managed_domains)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_org_profiles (
                agent_id, org_class, delegation_policy, review_policy, managed_domains_json,
                quality_rubric, max_delegation_depth, max_parallel_delegates,
                manager_can_hire, manager_can_restructure, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, CURRENT_TIMESTAMP)
            ON CONFLICT(agent_id) DO UPDATE SET
                org_class = excluded.org_class,
                delegation_policy = excluded.delegation_policy,
                review_policy = excluded.review_policy,
                managed_domains_json = excluded.managed_domains_json,
                quality_rubric = excluded.quality_rubric,
                max_delegation_depth = excluded.max_delegation_depth,
                max_parallel_delegates = excluded.max_parallel_delegates,
                manager_can_hire = excluded.manager_can_hire,
                manager_can_restructure = excluded.manager_can_restructure,
                updated_at = CURRENT_TIMESTAMP",
            params![
                profile.agent_id,
                profile.org_class,
                profile.delegation_policy,
                profile.review_policy,
                managed_domains_json,
                profile.quality_rubric,
                profile.max_delegation_depth,
                profile.max_parallel_delegates,
                if profile.manager_can_hire { 1 } else { 0 },
                if profile.manager_can_restructure {
                    1
                } else {
                    0
                },
            ],
        )
        .map_err(KernelError::Database)?;
        Self::replace_team_goal_assignments(&conn, &profile.agent_id, &profile.team_goal_ids)?;
        drop(conn);
        self.get_agent_org_profile(&profile.agent_id)
    }

    pub fn get_agent_org_profile(&self, agent_id: &str) -> Result<AgentOrgProfileRecord> {
        let row_data = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT agent_id, org_class, delegation_policy, review_policy, managed_domains_json,
                        quality_rubric, max_delegation_depth, max_parallel_delegates,
                        manager_can_hire, manager_can_restructure, updated_at
                 FROM agent_org_profiles
                 WHERE agent_id = ?1",
                params![agent_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, String>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(KernelError::Database)?
        };

        if let Some((
            profile_agent_id,
            org_class,
            delegation_policy,
            review_policy,
            managed_domains_json,
            quality_rubric,
            max_delegation_depth,
            max_parallel_delegates,
            manager_can_hire,
            manager_can_restructure,
            updated_at,
        )) = row_data
        {
            let team_goal_ids = self.list_team_goal_ids_for_agent(&profile_agent_id)?;
            return Ok(AgentOrgProfileRecord {
                agent_id: profile_agent_id,
                org_class,
                team_goal_ids,
                delegation_policy,
                review_policy,
                managed_domains: Self::parse_string_list(managed_domains_json),
                quality_rubric,
                max_delegation_depth,
                max_parallel_delegates,
                manager_can_hire: manager_can_hire != 0,
                manager_can_restructure: manager_can_restructure != 0,
                updated_at,
            });
        }

        let agent = self.get_agent(agent_id)?;
        let profile = AgentOrgProfileRecord::default_for_agent(&agent);
        self.upsert_agent_org_profile(&profile)
    }

    pub fn list_team_goal_ids_for_agent(&self, agent_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT goal_id
                 FROM agent_team_goal_assignments
                 WHERE agent_id = ?1
                 ORDER BY created_at ASC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![agent_id], |row| row.get::<_, String>(0))
            .map_err(KernelError::Database)?;
        let mut goal_ids = Vec::new();
        for row in rows {
            goal_ids.push(row.map_err(KernelError::Database)?);
        }
        Ok(goal_ids)
    }

    pub fn list_team_goals_for_agent(&self, agent_id: &str) -> Result<Vec<TeamGoalRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT g.goal_id, g.team_owner_agent_id, g.title, g.summary, g.status, g.priority,
                        g.success_criteria, g.managed_domain_tags_json, g.created_at, g.updated_at, g.archived_at
                 FROM team_goals g
                 INNER JOIN agent_team_goal_assignments a ON a.goal_id = g.goal_id
                 WHERE a.agent_id = ?1
                 ORDER BY g.created_at ASC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok(TeamGoalRecord {
                    goal_id: row.get(0)?,
                    team_owner_agent_id: row.get(1)?,
                    title: row.get(2)?,
                    summary: row.get(3)?,
                    status: row.get(4)?,
                    priority: row.get(5)?,
                    success_criteria: row.get(6)?,
                    managed_domain_tags: Self::parse_string_list(row.get(7)?),
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    archived_at: row.get(10)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut goals = Vec::new();
        for row in rows {
            goals.push(row.map_err(KernelError::Database)?);
        }
        Ok(goals)
    }

    pub fn list_descendant_team_goals(&self, agent_id: &str) -> Result<Vec<TeamGoalRecord>> {
        let mut goals = self.list_team_goals_for_agent(agent_id)?;
        let mut current = self.get_agent(agent_id)?;
        while let Some(parent_id) = current.parent_id.clone() {
            let parent_goals = self.list_team_goals_for_agent(&parent_id)?;
            for goal in parent_goals {
                if !goals
                    .iter()
                    .any(|existing| existing.goal_id == goal.goal_id)
                {
                    goals.push(goal);
                }
            }
            current = self.get_agent(&parent_id)?;
        }
        Ok(goals)
    }

    pub fn upsert_team_goal(&self, goal: &TeamGoalRecord) -> Result<TeamGoalRecord> {
        let tags_json = Self::serialize_string_list(&goal.managed_domain_tags)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO team_goals (
                goal_id, team_owner_agent_id, title, summary, status, priority, success_criteria,
                managed_domain_tags_json, archived_at, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            ON CONFLICT(goal_id) DO UPDATE SET
                team_owner_agent_id = excluded.team_owner_agent_id,
                title = excluded.title,
                summary = excluded.summary,
                status = excluded.status,
                priority = excluded.priority,
                success_criteria = excluded.success_criteria,
                managed_domain_tags_json = excluded.managed_domain_tags_json,
                archived_at = excluded.archived_at,
                updated_at = CURRENT_TIMESTAMP",
            params![
                goal.goal_id,
                goal.team_owner_agent_id,
                goal.title,
                goal.summary,
                goal.status,
                goal.priority,
                goal.success_criteria,
                tags_json,
                goal.archived_at,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);
        self.assign_team_goal(&goal.team_owner_agent_id, &goal.goal_id)?;
        self.get_team_goal(&goal.goal_id)
    }

    pub fn get_team_goal(&self, goal_id: &str) -> Result<TeamGoalRecord> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT goal_id, team_owner_agent_id, title, summary, status, priority,
                    success_criteria, managed_domain_tags_json, created_at, updated_at, archived_at
             FROM team_goals
             WHERE goal_id = ?1",
            params![goal_id],
            |row| {
                Ok(TeamGoalRecord {
                    goal_id: row.get(0)?,
                    team_owner_agent_id: row.get(1)?,
                    title: row.get(2)?,
                    summary: row.get(3)?,
                    status: row.get(4)?,
                    priority: row.get(5)?,
                    success_criteria: row.get(6)?,
                    managed_domain_tags: Self::parse_string_list(row.get(7)?),
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    archived_at: row.get(10)?,
                })
            },
        )
        .map_err(KernelError::Database)
    }

    pub fn assign_team_goal(&self, agent_id: &str, goal_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO agent_team_goal_assignments (agent_id, goal_id)
             VALUES (?1, ?2)",
            params![agent_id, goal_id],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn archive_team_goal(&self, goal_id: &str) -> Result<TeamGoalRecord> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE team_goals
             SET status = 'ARCHIVED',
                 archived_at = CURRENT_TIMESTAMP,
                 updated_at = CURRENT_TIMESTAMP
             WHERE goal_id = ?1",
            params![goal_id],
        )
        .map_err(KernelError::Database)?;
        drop(conn);
        self.get_team_goal(goal_id)
    }

    pub fn set_agent_reporting_line(
        &self,
        agent_id: &str,
        manager_agent_id: Option<&str>,
    ) -> Result<()> {
        if let Some(manager_agent_id) = manager_agent_id {
            let _ = self.get_agent(manager_agent_id)?;
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agents SET parent_id = ?1 WHERE id = ?2",
            params![manager_agent_id, agent_id],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn record_delegation_decision(
        &self,
        record: &DelegationDecisionRecord,
    ) -> Result<DelegationDecisionRecord> {
        let candidates_json = Self::serialize_string_list(&record.candidate_assignees)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO delegation_decisions (
                id, swo_id, manager_agent_id, decision, candidate_assignees_json,
                selected_agent_id, fit_reason, exception_code, exception_reason, team_gap_code
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.id,
                record.swo_id,
                record.manager_agent_id,
                record.decision,
                candidates_json,
                record.selected_agent_id,
                record.fit_reason,
                record.exception_code,
                record.exception_reason,
                record.team_gap_code,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);
        self.list_delegation_decisions_for_swo(record.swo_id)?
            .into_iter()
            .find(|entry| entry.id == record.id)
            .ok_or_else(|| {
                KernelError::Internal("Failed to reload delegation decision".to_string())
            })
    }

    pub fn list_delegation_decisions_for_swo(
        &self,
        swo_id: i64,
    ) -> Result<Vec<DelegationDecisionRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, swo_id, manager_agent_id, decision, candidate_assignees_json,
                        selected_agent_id, fit_reason, exception_code, exception_reason, team_gap_code, created_at
                 FROM delegation_decisions
                 WHERE swo_id = ?1
                 ORDER BY created_at DESC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![swo_id], |row| {
                Ok(DelegationDecisionRecord {
                    id: row.get(0)?,
                    swo_id: row.get(1)?,
                    manager_agent_id: row.get(2)?,
                    decision: row.get(3)?,
                    candidate_assignees: Self::parse_string_list(row.get(4)?),
                    selected_agent_id: row.get(5)?,
                    fit_reason: row.get(6)?,
                    exception_code: row.get(7)?,
                    exception_reason: row.get(8)?,
                    team_gap_code: row.get(9)?,
                    created_at: row.get(10)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn list_recent_delegation_decisions_for_manager(
        &self,
        manager_agent_id: &str,
        limit: usize,
    ) -> Result<Vec<DelegationDecisionRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, swo_id, manager_agent_id, decision, candidate_assignees_json,
                        selected_agent_id, fit_reason, exception_code, exception_reason, team_gap_code, created_at
                 FROM delegation_decisions
                 WHERE manager_agent_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(
                params![manager_agent_id, limit.clamp(1, 100) as i64],
                |row| {
                    Ok(DelegationDecisionRecord {
                        id: row.get(0)?,
                        swo_id: row.get(1)?,
                        manager_agent_id: row.get(2)?,
                        decision: row.get(3)?,
                        candidate_assignees: Self::parse_string_list(row.get(4)?),
                        selected_agent_id: row.get(5)?,
                        fit_reason: row.get(6)?,
                        exception_code: row.get(7)?,
                        exception_reason: row.get(8)?,
                        team_gap_code: row.get(9)?,
                        created_at: row.get(10)?,
                    })
                },
            )
            .map_err(KernelError::Database)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn record_team_gap(&self, record: &TeamGapRecord) -> Result<TeamGapRecord> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO team_gaps (id, swo_id, manager_agent_id, gap_code, summary, recommended_action)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.id,
                record.swo_id,
                record.manager_agent_id,
                record.gap_code,
                record.summary,
                record.recommended_action,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);
        self.list_team_gaps_for_swo(record.swo_id)?
            .into_iter()
            .find(|entry| entry.id == record.id)
            .ok_or_else(|| KernelError::Internal("Failed to reload team gap".to_string()))
    }

    pub fn list_team_gaps_for_swo(&self, swo_id: i64) -> Result<Vec<TeamGapRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, swo_id, manager_agent_id, gap_code, summary, recommended_action, created_at
                 FROM team_gaps
                 WHERE swo_id = ?1
                 ORDER BY created_at DESC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![swo_id], |row| {
                Ok(TeamGapRecord {
                    id: row.get(0)?,
                    swo_id: row.get(1)?,
                    manager_agent_id: row.get(2)?,
                    gap_code: row.get(3)?,
                    summary: row.get(4)?,
                    recommended_action: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn list_recent_team_gaps_for_manager(
        &self,
        manager_agent_id: &str,
        limit: usize,
    ) -> Result<Vec<TeamGapRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, swo_id, manager_agent_id, gap_code, summary, recommended_action, created_at
                 FROM team_gaps
                 WHERE manager_agent_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(
                params![manager_agent_id, limit.clamp(1, 100) as i64],
                |row| {
                    Ok(TeamGapRecord {
                        id: row.get(0)?,
                        swo_id: row.get(1)?,
                        manager_agent_id: row.get(2)?,
                        gap_code: row.get(3)?,
                        summary: row.get(4)?,
                        recommended_action: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                },
            )
            .map_err(KernelError::Database)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn save_skill(&self, request: &SkillUpsertRequest) -> Result<SkillRecord> {
        let metadata = normalize_skill_metadata(request);
        let metadata_json = serde_json::to_string_pretty(&metadata).map_err(|e| {
            KernelError::Internal(format!("Failed to serialize skill metadata: {}", e))
        })?;
        let conn = self.conn.lock().unwrap();
        let skill_id = request
            .skill_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let base_slug = slugify_name(&request.name);
        let slug = Self::allocate_skill_slug(&conn, &base_slug, request.skill_id.as_deref())?;

        let existing: Option<(i64, String, String)> = conn
            .query_row(
                "SELECT current_version, created_at, updated_at FROM skills WHERE id = ?1",
                params![skill_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(KernelError::Database)?;

        let next_version = existing
            .as_ref()
            .map(|(version, _, _)| version + 1)
            .unwrap_or(1);
        conn.execute(
            "INSERT INTO skill_versions (skill_id, version, raw_markdown, metadata_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![skill_id, next_version, request.raw_markdown, metadata_json,],
        )
        .map_err(KernelError::Database)?;

        if existing.is_some() {
            conn.execute(
                "UPDATE skills
                 SET slug = ?1,
                     name = ?2,
                     source_uri = ?3,
                     owner_agent_id = ?4,
                     current_version = ?5,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?6",
                params![
                    slug,
                    request.name.trim(),
                    request
                        .source_uri
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty()),
                    request.owner_agent_id.as_deref(),
                    next_version,
                    skill_id,
                ],
            )
            .map_err(KernelError::Database)?;
        } else {
            conn.execute(
                "INSERT INTO skills (id, slug, name, source_uri, owner_agent_id, current_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    skill_id,
                    slug,
                    request.name.trim(),
                    request
                        .source_uri
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty()),
                    request.owner_agent_id.as_deref(),
                    next_version,
                ],
            )
            .map_err(KernelError::Database)?;
        }

        Self::load_skill_record(&conn, &skill_id)
    }

    pub fn list_skills(&self, limit: usize) -> Result<Vec<SkillRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    s.id,
                    s.slug,
                    s.name,
                    s.source_uri,
                    s.owner_agent_id,
                    s.current_version,
                    s.created_at,
                    s.updated_at,
                    sv.metadata_json
                FROM skills s
                JOIN skill_versions sv
                  ON sv.skill_id = s.id AND sv.version = s.current_version
                ORDER BY s.updated_at DESC, s.name ASC
                LIMIT ?1
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 500) as i64], |row| {
                let metadata_json: String = row.get(8)?;
                let metadata: SkillMetadataV1 =
                    serde_json::from_str(&metadata_json).unwrap_or_default();
                Ok(SkillRecord {
                    id: row.get(0)?,
                    slug: row.get(1)?,
                    name: row.get(2)?,
                    summary: metadata.summary,
                    tags: metadata.tags,
                    trigger_hints: metadata.trigger_hints,
                    source_uri: row.get(3)?,
                    owner_agent_id: row.get(4)?,
                    current_version: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut skills = Vec::new();
        for row in rows {
            skills.push(row.map_err(KernelError::Database)?);
        }
        Ok(skills)
    }

    pub fn get_skill(&self, skill_id: &str) -> Result<Option<SkillVersionRecord>> {
        let conn = self.conn.lock().unwrap();
        Self::load_skill_version(&conn, skill_id)
    }

    pub fn bind_skill_to_agent(&self, agent_id: &str, skill_id: &str, priority: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id FROM agents WHERE id = ?1",
            params![agent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(KernelError::Database)?
        .ok_or_else(|| KernelError::Internal(format!("Unknown agent id {}", agent_id)))?;
        conn.query_row(
            "SELECT id FROM skills WHERE id = ?1",
            params![skill_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(KernelError::Database)?
        .ok_or_else(|| KernelError::Internal(format!("Unknown skill id {}", skill_id)))?;
        conn.execute(
            "INSERT INTO agent_skill_bindings (agent_id, skill_id, binding_status, priority)
             VALUES (?1, ?2, 'ACTIVE', ?3)
             ON CONFLICT(agent_id, skill_id) DO UPDATE SET
                 binding_status = 'ACTIVE',
                 priority = excluded.priority",
            params![agent_id, skill_id, priority],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn unbind_skill_from_agent(&self, agent_id: &str, skill_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM agent_skill_bindings WHERE agent_id = ?1 AND skill_id = ?2",
            params![agent_id, skill_id],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn list_agent_skill_bindings(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentSkillBindingRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    b.agent_id,
                    b.skill_id,
                    s.name,
                    s.slug,
                    s.source_uri,
                    s.current_version,
                    b.priority,
                    b.binding_status,
                    sv.metadata_json
                FROM agent_skill_bindings b
                JOIN skills s ON s.id = b.skill_id
                JOIN skill_versions sv
                  ON sv.skill_id = s.id AND sv.version = s.current_version
                WHERE b.agent_id = ?1
                ORDER BY b.priority ASC, s.name ASC
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                let metadata_json: String = row.get(8)?;
                let metadata: SkillMetadataV1 =
                    serde_json::from_str(&metadata_json).unwrap_or_default();
                Ok(AgentSkillBindingRecord {
                    agent_id: row.get(0)?,
                    skill_id: row.get(1)?,
                    skill_name: row.get(2)?,
                    skill_slug: row.get(3)?,
                    summary: metadata.summary,
                    tags: metadata.tags,
                    trigger_hints: metadata.trigger_hints,
                    source_uri: row.get(4)?,
                    current_version: row.get(5)?,
                    priority: row.get(6)?,
                    binding_status: row.get(7)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut bindings = Vec::new();
        for row in rows {
            bindings.push(row.map_err(KernelError::Database)?);
        }
        Ok(bindings)
    }

    pub fn bind_tool_to_agent(&self, agent_id: &str, tool_slug: &str) -> Result<()> {
        let tool = find_built_in_tool(tool_slug)
            .ok_or_else(|| KernelError::Internal(format!("Unsupported tool slug {}", tool_slug)))?;
        if !tool.assignable {
            return Err(KernelError::Internal(format!(
                "Tool {} is not assignable",
                tool_slug
            )));
        }

        let manifest = self.get_agent_manifest(agent_id)?;
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id FROM agents WHERE id = ?1",
            params![agent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(KernelError::Database)?
        .ok_or_else(|| KernelError::Internal(format!("Unknown agent id {}", agent_id)))?;
        if !manifest.has_capability(&tool.required_capability) {
            return Err(KernelError::Internal(format!(
                "Agent {} lacks required capability {}",
                agent_id,
                required_capability_slug(&tool.required_capability)
            )));
        }

        let conflicting_slugs = built_in_tool_catalog()
            .iter()
            .filter(|candidate| candidate.tool_kind == tool.tool_kind)
            .map(|candidate| candidate.slug.to_string())
            .collect::<Vec<_>>();
        if !conflicting_slugs.is_empty() {
            let placeholders = conflicting_slugs
                .iter()
                .enumerate()
                .map(|(index, _)| format!("?{}", index + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let mut values = vec![agent_id.to_string()];
            values.extend(conflicting_slugs);
            conn.execute(
                &format!(
                    "DELETE FROM agent_tool_bindings WHERE agent_id = ?1 AND tool_slug IN ({})",
                    placeholders
                ),
                rusqlite::params_from_iter(values),
            )
            .map_err(KernelError::Database)?;
        }

        conn.execute(
            "INSERT INTO agent_tool_bindings (agent_id, tool_slug, binding_status)
             VALUES (?1, ?2, 'ACTIVE')
             ON CONFLICT(agent_id, tool_slug) DO UPDATE SET
                 binding_status = 'ACTIVE'",
            params![agent_id, tool_slug],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn unbind_tool_from_agent(&self, agent_id: &str, tool_slug: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM agent_tool_bindings WHERE agent_id = ?1 AND tool_slug = ?2",
            params![agent_id, tool_slug],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn list_agent_tool_bindings(&self, agent_id: &str) -> Result<Vec<AgentToolBindingRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    b.agent_id,
                    b.tool_slug,
                    b.binding_status
                FROM agent_tool_bindings b
                WHERE b.agent_id = ?1
                ORDER BY b.created_at ASC, b.tool_slug ASC
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                let tool_slug: String = row.get(1)?;
                let tool = find_built_in_tool(&tool_slug).ok_or_else(|| {
                    rusqlite::Error::InvalidColumnType(
                        1,
                        "tool_slug".to_string(),
                        rusqlite::types::Type::Text,
                    )
                })?;
                Ok(AgentToolBindingRecord {
                    agent_id: row.get(0)?,
                    tool_slug: tool_slug.clone(),
                    name: tool.name.to_string(),
                    summary: tool.summary.to_string(),
                    tool_kind: tool.tool_kind.to_string(),
                    provider_slug: tool.provider_slug.to_string(),
                    required_capability: required_capability_slug(&tool.required_capability),
                    binding_status: row.get(2)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut bindings = Vec::new();
        for row in rows {
            bindings.push(row.map_err(KernelError::Database)?);
        }
        Ok(bindings)
    }

    pub fn active_web_search_provider_for_agent(&self, agent_id: &str) -> Result<Option<String>> {
        let bindings = self.list_agent_tool_bindings(agent_id)?;
        Ok(active_web_search_provider(&bindings))
    }

    // -----------------------------------------------------------------------
    // MCP Connector CRUD
    // -----------------------------------------------------------------------

    pub fn upsert_mcp_connector(
        &self,
        req: &McpConnectorUpsertRequest,
    ) -> Result<McpConnectorRecord> {
        mcp_validation::validate_mcp_connector(req).map_err(|msg| {
            KernelError::Internal(format!("MCP connector validation failed: {}", msg))
        })?;

        let transport = McpTransport::from_str(&req.transport).ok_or_else(|| {
            KernelError::Internal(format!("Invalid transport '{}'", req.transport))
        })?;

        let id = req.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
        let summary = req.summary.clone().unwrap_or_default();
        let enabled: bool = req.enabled.unwrap_or(true);

        let args_json = req
            .args
            .as_ref()
            .map(|args| serde_json::to_string(args))
            .transpose()
            .map_err(|e| KernelError::Internal(format!("Failed to serialize args: {}", e)))?;

        let env_json = req
            .env
            .as_ref()
            .map(|env| serde_json::to_string(env))
            .transpose()
            .map_err(|e| KernelError::Internal(format!("Failed to serialize env: {}", e)))?;

        let headers_json = req
            .headers
            .as_ref()
            .map(|headers| serde_json::to_string(headers))
            .transpose()
            .map_err(|e| {
                KernelError::Internal(format!("Failed to serialize headers: {}", e))
            })?;

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO mcp_connectors (id, slug, name, summary, transport, command, args_json, env_json, url, headers_json, cwd, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, CURRENT_TIMESTAMP)
             ON CONFLICT(id) DO UPDATE SET
                 slug = excluded.slug,
                 name = excluded.name,
                 summary = excluded.summary,
                 transport = excluded.transport,
                 command = excluded.command,
                 args_json = excluded.args_json,
                 env_json = excluded.env_json,
                 url = excluded.url,
                 headers_json = excluded.headers_json,
                 cwd = excluded.cwd,
                 enabled = excluded.enabled,
                 updated_at = CURRENT_TIMESTAMP",
            params![
                id,
                req.slug,
                req.name,
                summary,
                transport.as_str(),
                req.command,
                args_json,
                env_json,
                req.url,
                headers_json,
                req.cwd,
                enabled as i32,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);

        let is_update = req.id.is_some();
        self.record_audit_event(
            None,
            None,
            if is_update {
                "mcp_connector.updated"
            } else {
                "mcp_connector.created"
            },
            TaintLabel::TrustedOperator,
            &serde_json::json!({
                "connector_id": id,
                "slug": req.slug,
                "name": req.name,
                "transport": transport.as_str(),
            }),
        )?;

        self.get_mcp_connector(&id)
    }

    pub fn delete_mcp_connector(&self, connector_id: &str) -> Result<()> {
        let record = self.get_mcp_connector(connector_id)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM mcp_connectors WHERE id = ?1",
            params![connector_id],
        )
        .map_err(KernelError::Database)?;
        drop(conn);

        self.record_audit_event(
            None,
            None,
            "mcp_connector.deleted",
            TaintLabel::TrustedOperator,
            &serde_json::json!({
                "connector_id": connector_id,
                "slug": record.slug,
            }),
        )?;
        Ok(())
    }

    pub fn list_mcp_connectors(&self) -> Result<Vec<McpConnectorRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, slug, name, summary, transport, command, args_json, env_json, url, headers_json, cwd, enabled, created_at, updated_at
                 FROM mcp_connectors
                 ORDER BY name ASC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map([], |row| Self::mcp_connector_from_row(row))
            .map_err(KernelError::Database)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn get_mcp_connector(&self, connector_id: &str) -> Result<McpConnectorRecord> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, slug, name, summary, transport, command, args_json, env_json, url, headers_json, cwd, enabled, created_at, updated_at
             FROM mcp_connectors
             WHERE id = ?1",
            params![connector_id],
            |row| Self::mcp_connector_from_row(row),
        )
        .optional()
        .map_err(KernelError::Database)?
        .ok_or_else(|| {
            KernelError::Internal(format!("MCP connector '{}' not found", connector_id))
        })
    }

    fn mcp_connector_from_row(row: &Row<'_>) -> rusqlite::Result<McpConnectorRecord> {
        let transport_str: String = row.get(4)?;
        let transport = McpTransport::from_str(&transport_str).unwrap_or(McpTransport::Stdio);

        let args_json: Option<String> = row.get(6)?;
        let args: Option<Vec<String>> = args_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok());

        let env_json: Option<String> = row.get(7)?;
        let env: Option<HashMap<String, String>> = env_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok());

        let headers_json: Option<String> = row.get(9)?;
        let headers: Option<HashMap<String, String>> = headers_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok());

        let enabled_int: i32 = row.get(11)?;

        Ok(McpConnectorRecord {
            id: row.get(0)?,
            slug: row.get(1)?,
            name: row.get(2)?,
            summary: row.get(3)?,
            transport,
            command: row.get(5)?,
            args,
            env,
            url: row.get(8)?,
            headers,
            cwd: row.get(10)?,
            enabled: enabled_int != 0,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
        })
    }

    pub fn bind_mcp_connector_to_agent(
        &self,
        agent_id: &str,
        connector_id: &str,
    ) -> Result<()> {
        // Validate agent exists
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id FROM agents WHERE id = ?1",
            params![agent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(KernelError::Database)?
        .ok_or_else(|| KernelError::Internal(format!("Unknown agent id {}", agent_id)))?;
        drop(conn);

        // Validate agent has McpClient capability
        let manifest = self.get_agent_manifest(agent_id)?;
        if !manifest.has_capability(&crate::manifest::CapabilityGrant::McpClient) {
            return Err(KernelError::Internal(format!(
                "Agent {} lacks McpClient capability",
                agent_id
            )));
        }

        // Validate connector exists and is enabled
        let connector = self.get_mcp_connector(connector_id)?;
        if !connector.enabled {
            return Err(KernelError::Internal(format!(
                "MCP connector '{}' is disabled",
                connector.slug
            )));
        }

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_mcp_bindings (agent_id, connector_id, binding_status)
             VALUES (?1, ?2, 'ACTIVE')
             ON CONFLICT(agent_id, connector_id) DO UPDATE SET
                 binding_status = 'ACTIVE'",
            params![agent_id, connector_id],
        )
        .map_err(KernelError::Database)?;
        drop(conn);

        self.record_audit_event(
            Some(agent_id),
            None,
            "mcp_connector.bound",
            TaintLabel::TrustedOperator,
            &serde_json::json!({
                "agent_id": agent_id,
                "connector_id": connector_id,
                "connector_slug": connector.slug,
            }),
        )?;
        Ok(())
    }

    pub fn unbind_mcp_connector_from_agent(
        &self,
        agent_id: &str,
        connector_id: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM agent_mcp_bindings WHERE agent_id = ?1 AND connector_id = ?2",
            params![agent_id, connector_id],
        )
        .map_err(KernelError::Database)?;
        drop(conn);

        self.record_audit_event(
            Some(agent_id),
            None,
            "mcp_connector.unbound",
            TaintLabel::TrustedOperator,
            &serde_json::json!({
                "agent_id": agent_id,
                "connector_id": connector_id,
            }),
        )?;
        Ok(())
    }

    pub fn list_agent_mcp_bindings(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentMcpBindingRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT
                    b.agent_id,
                    b.connector_id,
                    c.slug,
                    c.name,
                    c.transport,
                    b.binding_status
                 FROM agent_mcp_bindings b
                 JOIN mcp_connectors c ON c.id = b.connector_id
                 WHERE b.agent_id = ?1
                 ORDER BY c.name ASC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok(AgentMcpBindingRecord {
                    agent_id: row.get(0)?,
                    connector_id: row.get(1)?,
                    connector_slug: row.get(2)?,
                    connector_name: row.get(3)?,
                    transport: row.get(4)?,
                    binding_status: row.get(5)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut bindings = Vec::new();
        for row in rows {
            bindings.push(row.map_err(KernelError::Database)?);
        }
        Ok(bindings)
    }

    // -----------------------------------------------------------------------
    // CLI Tool CRUD
    // -----------------------------------------------------------------------

    pub fn upsert_cli_tool(&self, req: &CliToolUpsertRequest) -> Result<CliToolRecord> {
        if req.slug.is_empty() {
            return Err(KernelError::Internal("CLI tool slug must not be empty".to_string()));
        }
        if req.name.is_empty() {
            return Err(KernelError::Internal("CLI tool name must not be empty".to_string()));
        }
        if req.command.is_empty() {
            return Err(KernelError::Internal("CLI tool command must not be empty".to_string()));
        }
        // Validate slug format: [a-z0-9_-]+
        if !req.slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
            return Err(KernelError::Internal(
                "CLI tool slug must match [a-z0-9_-]+".to_string(),
            ));
        }

        let id = req.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
        let enabled = req.enabled.unwrap_or(true);

        let args_json = req
            .args
            .as_ref()
            .map(|args| serde_json::to_string(args))
            .transpose()
            .map_err(|e| KernelError::Internal(format!("Failed to serialize args: {}", e)))?;

        let env_json = req
            .env
            .as_ref()
            .map(|env| serde_json::to_string(env))
            .transpose()
            .map_err(|e| KernelError::Internal(format!("Failed to serialize env: {}", e)))?;

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cli_tools (id, slug, name, summary, command, args_json, env_json, cwd, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CURRENT_TIMESTAMP)
             ON CONFLICT(id) DO UPDATE SET
                 slug = excluded.slug,
                 name = excluded.name,
                 summary = excluded.summary,
                 command = excluded.command,
                 args_json = excluded.args_json,
                 env_json = excluded.env_json,
                 cwd = excluded.cwd,
                 enabled = excluded.enabled,
                 updated_at = CURRENT_TIMESTAMP",
            params![
                id,
                req.slug,
                req.name,
                req.summary,
                req.command,
                args_json,
                env_json,
                req.cwd,
                enabled as i32,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);

        let is_update = req.id.is_some();
        self.record_audit_event(
            None,
            None,
            if is_update { "cli_tool.updated" } else { "cli_tool.created" },
            TaintLabel::TrustedOperator,
            &serde_json::json!({
                "tool_id": id,
                "slug": req.slug,
                "name": req.name,
            }),
        )?;

        self.get_cli_tool(&id)
    }

    pub fn delete_cli_tool(&self, tool_id: &str) -> Result<()> {
        let record = self.get_cli_tool(tool_id)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM cli_tools WHERE id = ?1",
            params![tool_id],
        )
        .map_err(KernelError::Database)?;
        drop(conn);

        self.record_audit_event(
            None,
            None,
            "cli_tool.deleted",
            TaintLabel::TrustedOperator,
            &serde_json::json!({
                "tool_id": tool_id,
                "slug": record.slug,
            }),
        )?;
        Ok(())
    }

    pub fn list_cli_tools(&self) -> Result<Vec<CliToolRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, slug, name, summary, command, args_json, env_json, cwd, enabled, created_at, updated_at
                 FROM cli_tools
                 ORDER BY name ASC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map([], |row| Self::cli_tool_from_row(row))
            .map_err(KernelError::Database)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn get_cli_tool(&self, tool_id: &str) -> Result<CliToolRecord> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, slug, name, summary, command, args_json, env_json, cwd, enabled, created_at, updated_at
             FROM cli_tools
             WHERE id = ?1",
            params![tool_id],
            |row| Self::cli_tool_from_row(row),
        )
        .optional()
        .map_err(KernelError::Database)?
        .ok_or_else(|| KernelError::Internal(format!("CLI tool '{}' not found", tool_id)))
    }

    fn cli_tool_from_row(row: &Row<'_>) -> rusqlite::Result<CliToolRecord> {
        let enabled_int: i32 = row.get(8)?;
        Ok(CliToolRecord {
            id: row.get(0)?,
            slug: row.get(1)?,
            name: row.get(2)?,
            summary: row.get(3)?,
            command: row.get(4)?,
            args_json: row.get(5)?,
            env_json: row.get(6)?,
            cwd: row.get(7)?,
            enabled: enabled_int != 0,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }

    pub fn preview_agent_skills_for_run(
        &self,
        agent_id: &str,
        mode: &str,
        payload: &str,
        preselect_limit: usize,
    ) -> Result<Vec<RuntimeSkillIndexEntry>> {
        let agent = self.get_agent(agent_id)?;
        let bindings = self.list_agent_skill_bindings(agent_id)?;
        Ok(build_runtime_skill_index(
            &bindings,
            &agent.role,
            mode,
            payload,
            preselect_limit,
        ))
    }

    pub fn record_workflow_run(
        &self,
        workflow_run: &WorkflowRun,
        status: &str,
        root_swo_id: Option<i64>,
    ) -> Result<i64> {
        let compiled_json = serde_json::to_string_pretty(workflow_run).map_err(|e| {
            KernelError::Internal(format!("Failed to serialize workflow run: {}", e))
        })?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO workflow_runs (template_id, template_name, entry_agent_id, status, compiled_json, root_swo_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                workflow_run.template_id,
                workflow_run.template_name,
                workflow_run.entry_agent_id,
                status,
                compiled_json,
                root_swo_id
            ],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    fn allocate_skill_slug(
        conn: &Connection,
        preferred_slug: &str,
        existing_skill_id: Option<&str>,
    ) -> Result<String> {
        let base = if preferred_slug.trim().is_empty() {
            "skill".to_string()
        } else {
            preferred_slug.trim().to_string()
        };
        for index in 0..10_000 {
            let candidate = if index == 0 {
                base.clone()
            } else {
                format!("{base}-{index}")
            };
            let owner: Option<String> = conn
                .query_row(
                    "SELECT id FROM skills WHERE slug = ?1",
                    params![candidate],
                    |row| row.get(0),
                )
                .optional()
                .map_err(KernelError::Database)?;
            match owner {
                Some(owner_id) if Some(owner_id.as_str()) != existing_skill_id => continue,
                _ => return Ok(candidate),
            }
        }
        Err(KernelError::Internal(
            "Unable to allocate unique skill slug".to_string(),
        ))
    }

    fn load_skill_version(conn: &Connection, skill_id: &str) -> Result<Option<SkillVersionRecord>> {
        conn.query_row(
            "
            SELECT sv.id, sv.skill_id, sv.version, sv.raw_markdown, sv.metadata_json, sv.created_at
            FROM skill_versions sv
            JOIN skills s ON s.id = sv.skill_id AND s.current_version = sv.version
            WHERE s.id = ?1
            ",
            params![skill_id],
            |row| {
                let metadata_json: String = row.get(4)?;
                let metadata: SkillMetadataV1 =
                    serde_json::from_str(&metadata_json).unwrap_or_default();
                Ok(SkillVersionRecord {
                    id: row.get(0)?,
                    skill_id: row.get(1)?,
                    version: row.get(2)?,
                    raw_markdown: row.get(3)?,
                    metadata,
                    created_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(KernelError::Database)
    }

    fn load_skill_record(conn: &Connection, skill_id: &str) -> Result<SkillRecord> {
        conn.query_row(
            "
            SELECT
                s.id,
                s.slug,
                s.name,
                s.source_uri,
                s.owner_agent_id,
                s.current_version,
                s.created_at,
                s.updated_at,
                sv.metadata_json
            FROM skills s
            JOIN skill_versions sv
              ON sv.skill_id = s.id AND sv.version = s.current_version
            WHERE s.id = ?1
            ",
            params![skill_id],
            |row| {
                let metadata_json: String = row.get(8)?;
                let metadata: SkillMetadataV1 =
                    serde_json::from_str(&metadata_json).unwrap_or_default();
                Ok(SkillRecord {
                    id: row.get(0)?,
                    slug: row.get(1)?,
                    name: row.get(2)?,
                    summary: metadata.summary,
                    tags: metadata.tags,
                    trigger_hints: metadata.trigger_hints,
                    source_uri: row.get(3)?,
                    owner_agent_id: row.get(4)?,
                    current_version: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            },
        )
        .map_err(KernelError::Database)
    }

    pub fn list_workflow_runs(&self, limit: usize) -> Result<Vec<WorkflowRunRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    wr.id,
                    wr.template_id,
                    wr.template_name,
                    wr.entry_agent_id,
                    COALESCE(a.name || ' (' || a.role || ')', wr.entry_agent_id),
                    wr.status,
                    wr.compiled_json,
                    wr.root_swo_id,
                    wr.created_at
                FROM workflow_runs wr
                LEFT JOIN agents a ON a.id = wr.entry_agent_id
                ORDER BY wr.id DESC
                LIMIT ?1
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 200) as i64], |row| {
                Ok(WorkflowRunRecord {
                    id: row.get(0)?,
                    template_id: row.get(1)?,
                    template_name: row.get(2)?,
                    entry_agent_id: row.get(3)?,
                    entry_agent_name: row.get(4)?,
                    status: row.get(5)?,
                    compiled_json: row.get(6)?,
                    root_swo_id: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn record_audit_event(
        &self,
        agent_id: Option<&str>,
        swo_id: Option<i64>,
        event_kind: &str,
        taint_label: TaintLabel,
        payload: &Value,
    ) -> Result<i64> {
        let payload_json = serde_json::to_string(payload).map_err(|e| {
            KernelError::Internal(format!("Failed to serialize audit payload: {}", e))
        })?;
        let conn = self.conn.lock().unwrap();
        let previous_chain_hash: Option<String> = conn
            .query_row(
                "SELECT chain_hash FROM audit_events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(KernelError::Database)?;
        let chain_hash = compute_chain_hash(
            previous_chain_hash.as_deref(),
            event_kind,
            &taint_label,
            payload,
        );
        conn.execute(
            "INSERT INTO audit_events (agent_id, swo_id, event_kind, taint_label, payload_json, previous_chain_hash, chain_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                agent_id,
                swo_id,
                event_kind,
                serde_json::to_string(&taint_label)
                    .map_err(|e| KernelError::Internal(format!("Failed to serialize taint label: {}", e)))?,
                payload_json,
                previous_chain_hash,
                chain_hash
            ],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_audit_events(&self, limit: usize) -> Result<Vec<AuditEventRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT id, agent_id, swo_id, event_kind, taint_label, payload_json, previous_chain_hash, chain_hash, created_at
                FROM audit_events
                ORDER BY id DESC
                LIMIT ?1
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 500) as i64], |row| {
                let taint_label_json: String = row.get(4)?;
                Ok(AuditEventRecord {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    swo_id: row.get(2)?,
                    event_kind: row.get(3)?,
                    taint_label: serde_json::from_str(&taint_label_json)
                        .unwrap_or(TaintLabel::TrustedSystem),
                    payload_json: row.get(5)?,
                    previous_chain_hash: row.get(6)?,
                    chain_hash: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(KernelError::Database)?);
        }
        Ok(events)
    }

    pub fn record_agent_hire(
        &self,
        swo_id: i64,
        manager_agent_id: &str,
        new_agent_id: &str,
        spec_json: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_hires (swo_id, manager_agent_id, new_agent_id, spec_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![swo_id, manager_agent_id, new_agent_id, spec_json],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn create_project(
        &self,
        id: &str,
        name: &str,
        summary: Option<&str>,
        status: &str,
        priority: &str,
        lead_agent_id: Option<&str>,
        target_outcome: Option<&str>,
        tags: Option<&str>,
        updated_by: &str,
    ) -> Result<ProjectRecord> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(KernelError::Database)?;
        tx.execute(
            "INSERT INTO projects (id, name, summary, status, priority, lead_agent_id, target_outcome, tags)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, name, summary, status, priority, lead_agent_id, target_outcome, tags],
        )
        .map_err(KernelError::Database)?;
        tx.execute(
            "INSERT INTO project_status_updates (project_id, previous_status, next_status, reason, updated_by)
             VALUES (?1, NULL, ?2, ?3, ?4)",
            params![id, status, "Project created.", updated_by],
        )
        .map_err(KernelError::Database)?;
        tx.commit().map_err(KernelError::Database)?;
        drop(conn);

        self.get_project(id)?.ok_or_else(|| {
            KernelError::Internal(format!("Created project {} could not be reloaded", id))
        })
    }

    pub fn get_project(&self, project_id: &str) -> Result<Option<ProjectRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, summary, status, priority, lead_agent_id, target_outcome, tags, created_at, updated_at
             FROM projects
             WHERE id = ?1",
            params![project_id],
            Self::project_from_row,
        )
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, summary, status, priority, lead_agent_id, target_outcome, tags, created_at, updated_at
                 FROM projects
                 ORDER BY created_at DESC",
            )
            .map_err(KernelError::Database)?;

        let rows = stmt
            .query_map([], Self::project_from_row)
            .map_err(KernelError::Database)?;

        let mut projects = Vec::new();
        for row in rows {
            projects.push(row.map_err(KernelError::Database)?);
        }
        Ok(projects)
    }

    pub fn list_project_status_updates(&self) -> Result<Vec<ProjectStatusUpdateRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT project_id, previous_status, next_status, reason, updated_by, updated_at
                 FROM project_status_updates
                 ORDER BY updated_at DESC, id DESC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ProjectStatusUpdateRecord {
                    project_id: row.get(0)?,
                    previous_status: row.get(1)?,
                    next_status: row.get(2)?,
                    reason: row.get(3)?,
                    updated_by: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut updates = Vec::new();
        for row in rows {
            updates.push(row.map_err(KernelError::Database)?);
        }
        Ok(updates)
    }

    pub fn update_project_status(
        &self,
        project_id: &str,
        next_status: &str,
        reason: Option<&str>,
        updated_by: &str,
    ) -> Result<ProjectRecord> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(KernelError::Database)?;
        let previous_status = tx
            .query_row(
                "SELECT status FROM projects WHERE id = ?1",
                params![project_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(KernelError::Database)?
            .ok_or_else(|| KernelError::Internal(format!("Unknown project {}", project_id)))?;

        tx.execute(
            "UPDATE projects
             SET status = ?1,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?2",
            params![next_status, project_id],
        )
        .map_err(KernelError::Database)?;
        tx.execute(
            "INSERT INTO project_status_updates (project_id, previous_status, next_status, reason, updated_by)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project_id, previous_status, next_status, reason, updated_by],
        )
        .map_err(KernelError::Database)?;
        tx.commit().map_err(KernelError::Database)?;
        drop(conn);

        self.get_project(project_id)?.ok_or_else(|| {
            KernelError::Internal(format!(
                "Updated project {} could not be reloaded",
                project_id
            ))
        })
    }

    fn load_project_swos(conn: &Connection, project_id: &str) -> Result<Vec<ActiveSwoRecord>> {
        let sql = Self::active_swo_select_sql(
            "WHERE s.initiative_id = ?1 ORDER BY s.created_at DESC, s.id DESC",
        );
        let mut stmt = conn.prepare(&sql).map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![project_id], Self::active_swo_from_row)
            .map_err(KernelError::Database)?;
        let mut swos = Vec::new();
        for row in rows {
            swos.push(row.map_err(KernelError::Database)?);
        }
        Ok(swos)
    }

    pub fn list_project_output_records(
        &self,
        project_id: &str,
        limit: usize,
    ) -> Result<Vec<ProjectOutputRecord>> {
        let safe_limit = limit.clamp(1, 200);
        let conn = self.conn.lock().unwrap();
        let mut artifact_stmt = conn
            .prepare(
                "
                SELECT
                    a.id,
                    a.swo_id,
                    a.agent_id,
                    COALESCE(agent.name || ' (' || agent.role || ')', a.agent_id),
                    s.initiative_id,
                    s.initiative_name,
                    s.parent_swo_id,
                    s.work_order_title,
                    s.work_order_outcome,
                    s.status,
                    a.absolute_path,
                    a.filename,
                    a.created_at
                FROM outbox_artifacts a
                JOIN active_swos s ON s.id = a.swo_id
                LEFT JOIN agents agent ON agent.id = a.agent_id
                WHERE s.initiative_id = ?1
                ORDER BY a.created_at DESC, a.id DESC
                LIMIT ?2
                ",
            )
            .map_err(KernelError::Database)?;
        let artifact_rows = artifact_stmt
            .query_map(
                params![project_id, safe_limit as i64],
                Self::outbox_artifact_from_row,
            )
            .map_err(KernelError::Database)?;
        let mut outputs = Vec::new();
        for row in artifact_rows {
            let artifact = row.map_err(KernelError::Database)?;
            outputs.push(ProjectOutputRecord {
                id: format!("project-output-artifact-{}", artifact.id),
                output_kind: "artifact".to_string(),
                artifact_id: Some(artifact.id),
                result_id: None,
                swo_id: artifact.swo_id,
                project_id: artifact.project_id.clone(),
                project_name: artifact.project_name.clone(),
                agent_id: artifact.agent_id.clone(),
                agent_name: artifact.agent_name.clone(),
                display_name: artifact.filename.clone(),
                created_at: artifact.created_at.clone(),
                absolute_path: Some(artifact.absolute_path.clone()),
                preview_text: None,
                source_work_order_title: artifact.source_work_order_title.clone(),
                source_work_order_outcome: artifact.source_work_order_outcome.clone(),
                source_status: artifact.source_status.clone(),
            });
        }

        let mut result_stmt = conn
            .prepare(
                "
                SELECT
                    r.id,
                    r.swo_id,
                    r.producer_agent_id,
                    COALESCE(a.name || ' (' || a.role || ')', r.producer_agent_id),
                    s.initiative_id,
                    s.initiative_name,
                    s.work_order_title,
                    s.work_order_outcome,
                    s.status,
                    r.result_json,
                    r.created_at
                FROM swo_results r
                JOIN active_swos s ON s.id = r.swo_id
                LEFT JOIN agents a ON a.id = r.producer_agent_id
                WHERE s.initiative_id = ?1
                ORDER BY r.created_at DESC, r.id DESC
                LIMIT ?2
                ",
            )
            .map_err(KernelError::Database)?;
        let result_rows = result_stmt
            .query_map(params![project_id, safe_limit as i64], |row| {
                Ok(ProjectOutputRecord {
                    id: format!("project-output-result-{}", row.get::<_, i64>(0)?),
                    output_kind: "result".to_string(),
                    artifact_id: None,
                    result_id: Some(row.get(0)?),
                    swo_id: row.get(1)?,
                    project_id: row.get(4)?,
                    project_name: row.get(5)?,
                    agent_id: row.get(2)?,
                    agent_name: row.get(3)?,
                    display_name: format!("Structured result for SWO #{}", row.get::<_, i64>(1)?),
                    created_at: row.get(10)?,
                    absolute_path: None,
                    preview_text: Some(row.get(9)?),
                    source_work_order_title: row.get(6)?,
                    source_work_order_outcome: row.get(7)?,
                    source_status: row.get(8)?,
                })
            })
            .map_err(KernelError::Database)?;
        for row in result_rows {
            outputs.push(row.map_err(KernelError::Database)?);
        }

        outputs.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        if outputs.len() > safe_limit {
            outputs.truncate(safe_limit);
        }
        Ok(outputs)
    }

    pub fn list_audit_events_for_agent(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<AuditEventRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT id, agent_id, swo_id, event_kind, taint_label, payload_json, previous_chain_hash, chain_hash, created_at
                FROM audit_events
                WHERE agent_id = ?1
                ORDER BY id DESC
                LIMIT ?2
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![agent_id, limit.clamp(1, 500) as i64], |row| {
                let taint_label_json: String = row.get(4)?;
                Ok(AuditEventRecord {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    swo_id: row.get(2)?,
                    event_kind: row.get(3)?,
                    taint_label: serde_json::from_str(&taint_label_json)
                        .unwrap_or(TaintLabel::TrustedSystem),
                    payload_json: row.get(5)?,
                    previous_chain_hash: row.get(6)?,
                    chain_hash: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(KernelError::Database)?);
        }
        Ok(events)
    }

    pub fn list_audit_events_for_project(
        &self,
        project_id: &str,
        limit: usize,
    ) -> Result<Vec<AuditEventRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT e.id, e.agent_id, e.swo_id, e.event_kind, e.taint_label, e.payload_json, e.previous_chain_hash, e.chain_hash, e.created_at
                FROM audit_events e
                JOIN active_swos s ON s.id = e.swo_id
                WHERE s.initiative_id = ?1
                ORDER BY e.id DESC
                LIMIT ?2
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![project_id, limit.clamp(1, 500) as i64], |row| {
                let taint_label_json: String = row.get(4)?;
                Ok(AuditEventRecord {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    swo_id: row.get(2)?,
                    event_kind: row.get(3)?,
                    taint_label: serde_json::from_str(&taint_label_json)
                        .unwrap_or(TaintLabel::TrustedSystem),
                    payload_json: row.get(5)?,
                    previous_chain_hash: row.get(6)?,
                    chain_hash: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(KernelError::Database)?);
        }
        Ok(events)
    }

    pub fn get_project_workspace(
        &self,
        project_id: &str,
    ) -> Result<Option<ProjectWorkspaceRecord>> {
        let Some(project) = self.get_project(project_id)? else {
            return Ok(None);
        };

        let swos = {
            let conn = self.conn.lock().unwrap();
            Self::load_project_swos(&conn, project_id)?
        };
        let status_updates = self
            .list_project_status_updates()?
            .into_iter()
            .filter(|update| update.project_id == project_id)
            .collect::<Vec<_>>();
        let outputs = self.list_project_output_records(project_id, 100)?;
        let audit_events = self.list_audit_events_for_project(project_id, 50)?;

        let mut activity = Vec::new();
        for update in &status_updates {
            activity.push(ProjectActivityItemRecord {
                id: format!("project-status-{}-{}", update.project_id, update.updated_at),
                project_id: update.project_id.clone(),
                kind: "project_status".to_string(),
                actor_id: None,
                actor_name: Some(update.updated_by.clone()),
                actor_type: "operator".to_string(),
                timestamp: update.updated_at.clone(),
                title: format!("Project {}", update.next_status),
                summary: update
                    .reason
                    .clone()
                    .unwrap_or_else(|| format!("Project moved to {}.", update.next_status)),
                detail: None,
                status: Some(update.next_status.clone()),
                swo_id: None,
                artifact_id: None,
                related_agent_id: None,
            });
        }
        for swo in &swos {
            activity.push(ProjectActivityItemRecord {
                id: format!("project-swo-{}", swo.id),
                project_id: project_id.to_string(),
                kind: "swo".to_string(),
                actor_id: Some(swo.assigned_agent_id.clone()),
                actor_name: Some(swo.assigned_agent_name.clone()),
                actor_type: "agent".to_string(),
                timestamp: swo.created_at.clone(),
                title: swo
                    .work_order_title
                    .clone()
                    .unwrap_or_else(|| format!("Work order #{}", swo.id)),
                summary: swo
                    .work_order_outcome
                    .clone()
                    .unwrap_or_else(|| swo.payload.clone()),
                detail: swo.work_order_constraints.clone(),
                status: Some(swo.status.clone()),
                swo_id: Some(swo.id),
                artifact_id: None,
                related_agent_id: Some(swo.assigned_agent_id.clone()),
            });
        }
        for output in &outputs {
            activity.push(ProjectActivityItemRecord {
                id: format!("project-output-{}", output.id),
                project_id: project_id.to_string(),
                kind: output.output_kind.clone(),
                actor_id: Some(output.agent_id.clone()),
                actor_name: Some(output.agent_name.clone()),
                actor_type: if output.output_kind == "result" {
                    "agent".to_string()
                } else {
                    "artifact".to_string()
                },
                timestamp: output.created_at.clone(),
                title: output.display_name.clone(),
                summary: output
                    .source_work_order_title
                    .clone()
                    .unwrap_or_else(|| format!("Output from SWO #{}", output.swo_id)),
                detail: output.absolute_path.clone().or_else(|| {
                    output
                        .preview_text
                        .as_ref()
                        .map(|value| value.chars().take(280).collect())
                }),
                status: output.source_status.clone(),
                swo_id: Some(output.swo_id),
                artifact_id: output.artifact_id,
                related_agent_id: Some(output.agent_id.clone()),
            });
        }
        for event in audit_events {
            activity.push(ProjectActivityItemRecord {
                id: format!("project-audit-{}", event.id),
                project_id: project_id.to_string(),
                kind: "audit".to_string(),
                actor_id: event.agent_id.clone(),
                actor_name: event.agent_id.clone(),
                actor_type: "system".to_string(),
                timestamp: event.created_at.clone(),
                title: event.event_kind.clone(),
                summary: event.payload_json.clone(),
                detail: Some(event.chain_hash.clone()),
                status: None,
                swo_id: event.swo_id,
                artifact_id: None,
                related_agent_id: event.agent_id.clone(),
            });
        }
        activity.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));

        Ok(Some(ProjectWorkspaceRecord {
            project,
            swos,
            status_updates,
            activity,
            outputs,
        }))
    }

    pub fn list_agent_files(&self, agent_id: &str, limit: usize) -> Result<Vec<AgentFileRecord>> {
        let safe_limit = limit.clamp(1, 200);
        let conn = self.conn.lock().unwrap();
        let mut records = Vec::new();

        let mut input_stmt = conn
            .prepare(
                "
                SELECT
                    a.id,
                    sa.swo_id,
                    s.initiative_id,
                    s.initiative_name,
                    s.work_order_title,
                    a.display_name,
                    a.content_type,
                    a.size_bytes,
                    a.created_at,
                    sa.inbox_path,
                    a.original_path,
                    sa.delivery_status
                FROM swo_attachments sa
                JOIN attachments a ON a.id = sa.attachment_id
                JOIN active_swos s ON s.id = sa.swo_id
                WHERE s.assigned_agent_id = ?1
                ORDER BY a.created_at DESC, a.id DESC
                LIMIT ?2
                ",
            )
            .map_err(KernelError::Database)?;
        let input_rows = input_stmt
            .query_map(params![agent_id, safe_limit as i64], |row| {
                Ok(AgentFileRecord {
                    id: format!("attachment-{}", row.get::<_, String>(0)?),
                    agent_id: agent_id.to_string(),
                    kind: "input".to_string(),
                    source_kind: "attachment".to_string(),
                    display_name: row.get(5)?,
                    content_type: row.get(6)?,
                    size_bytes: row.get(7)?,
                    created_at: row.get(8)?,
                    swo_id: row.get(1)?,
                    project_id: row.get(2)?,
                    project_name: row.get(3)?,
                    artifact_id: None,
                    attachment_id: Some(row.get(0)?),
                    workspace_path: row.get(9)?,
                    absolute_path: row.get(10)?,
                    delivery_status: row.get(11)?,
                    source_work_order_title: row.get(4)?,
                })
            })
            .map_err(KernelError::Database)?;
        for row in input_rows {
            records.push(row.map_err(KernelError::Database)?);
        }

        let mut output_stmt = conn
            .prepare(
                "
                SELECT
                    a.id,
                    a.swo_id,
                    s.initiative_id,
                    s.initiative_name,
                    s.work_order_title,
                    a.filename,
                    a.absolute_path,
                    a.created_at
                FROM outbox_artifacts a
                JOIN active_swos s ON s.id = a.swo_id
                WHERE a.agent_id = ?1
                ORDER BY a.created_at DESC, a.id DESC
                LIMIT ?2
                ",
            )
            .map_err(KernelError::Database)?;
        let output_rows = output_stmt
            .query_map(params![agent_id, safe_limit as i64], |row| {
                let absolute_path: String = row.get(6)?;
                let size_bytes = std::fs::metadata(&absolute_path)
                    .map(|meta| meta.len() as i64)
                    .unwrap_or(0);
                let content_type = match Path::new(&absolute_path)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.to_ascii_lowercase())
                    .as_deref()
                {
                    Some("md") => "text/markdown".to_string(),
                    Some("json") => "application/json".to_string(),
                    Some("txt") => "text/plain".to_string(),
                    Some("png") => "image/png".to_string(),
                    Some("jpg") | Some("jpeg") => "image/jpeg".to_string(),
                    _ => "application/octet-stream".to_string(),
                };
                Ok(AgentFileRecord {
                    id: format!("artifact-{}", row.get::<_, i64>(0)?),
                    agent_id: agent_id.to_string(),
                    kind: "output".to_string(),
                    source_kind: "artifact".to_string(),
                    display_name: row.get(5)?,
                    content_type,
                    size_bytes,
                    created_at: row.get(7)?,
                    swo_id: row.get(1)?,
                    project_id: row.get(2)?,
                    project_name: row.get(3)?,
                    artifact_id: Some(row.get(0)?),
                    attachment_id: None,
                    workspace_path: None,
                    absolute_path: Some(absolute_path),
                    delivery_status: None,
                    source_work_order_title: row.get(4)?,
                })
            })
            .map_err(KernelError::Database)?;
        for row in output_rows {
            records.push(row.map_err(KernelError::Database)?);
        }

        records.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        records.truncate(safe_limit);
        Ok(records)
    }

    pub fn list_agent_history(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<AgentHistoryEventRecord>> {
        let safe_limit = limit.clamp(1, 200);
        let mut events = Vec::new();
        {
            let conn = self.conn.lock().unwrap();
            let sql = Self::active_swo_select_sql(
                "WHERE s.assigned_agent_id = ?1 OR COALESCE(s.owner_agent_id, s.manager_agent_id) = ?1 OR COALESCE(s.created_by_agent_id, s.manager_agent_id) = ?1 ORDER BY s.created_at DESC, s.id DESC LIMIT ?2",
            );
            let mut swo_stmt = conn.prepare(&sql).map_err(KernelError::Database)?;
            let swo_rows = swo_stmt
                .query_map(
                    params![agent_id, safe_limit as i64],
                    Self::active_swo_from_row,
                )
                .map_err(KernelError::Database)?;
            for row in swo_rows {
                let swo = row.map_err(KernelError::Database)?;
                events.push(AgentHistoryEventRecord {
                    id: format!("swo-{}", swo.id),
                    agent_id: agent_id.to_string(),
                    kind: "swo".to_string(),
                    timestamp: swo.created_at.clone(),
                    title: swo
                        .work_order_title
                        .clone()
                        .unwrap_or_else(|| format!("Work order #{}", swo.id)),
                    summary: swo
                        .work_order_outcome
                        .clone()
                        .unwrap_or_else(|| swo.payload.clone()),
                    detail: swo.work_order_constraints.clone(),
                    status: Some(swo.status.clone()),
                    swo_id: Some(swo.id),
                    artifact_id: None,
                    project_id: swo.initiative_id.clone(),
                    project_name: swo.initiative_name.clone(),
                    run_id: None,
                });
            }
        }

        for file in self.list_agent_files(agent_id, safe_limit)? {
            events.push(AgentHistoryEventRecord {
                id: format!("file-{}", file.id),
                agent_id: agent_id.to_string(),
                kind: if file.kind == "output" {
                    "artifact".to_string()
                } else {
                    "attachment".to_string()
                },
                timestamp: file.created_at.clone(),
                title: file.display_name.clone(),
                summary: if file.kind == "output" {
                    "Artifact produced.".to_string()
                } else {
                    "Context delivered.".to_string()
                },
                detail: file.absolute_path.clone().or(file.workspace_path.clone()),
                status: file.delivery_status.clone(),
                swo_id: file.swo_id,
                artifact_id: file.artifact_id,
                project_id: file.project_id.clone(),
                project_name: file.project_name.clone(),
                run_id: None,
            });
        }

        for entry in self.list_decision_log(agent_id, safe_limit)? {
            events.push(AgentHistoryEventRecord {
                id: format!("decision-{}", entry.entry_id),
                agent_id: agent_id.to_string(),
                kind: "decision".to_string(),
                timestamp: entry.created_at.clone(),
                title: entry.summary.clone(),
                summary: entry.outcome.clone(),
                detail: Some(entry.rationale.clone()),
                status: Some(entry.outcome.clone()),
                swo_id: entry.linked_swo_id,
                artifact_id: None,
                project_id: None,
                project_name: None,
                run_id: entry.linked_run_id.clone(),
            });
        }

        for event in self.list_audit_events_for_agent(agent_id, safe_limit)? {
            events.push(AgentHistoryEventRecord {
                id: format!("audit-{}", event.id),
                agent_id: agent_id.to_string(),
                kind: "audit".to_string(),
                timestamp: event.created_at.clone(),
                title: event.event_kind.clone(),
                summary: event.payload_json.clone(),
                detail: Some(event.chain_hash.clone()),
                status: None,
                swo_id: event.swo_id,
                artifact_id: None,
                project_id: None,
                project_name: None,
                run_id: None,
            });
        }

        events.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));
        events.truncate(safe_limit);
        Ok(events)
    }

    pub fn update_swo_work_order_fields(
        &self,
        swo_id: i64,
        title: Option<&str>,
        outcome: Option<&str>,
        constraints: Option<Option<&str>>,
    ) -> Result<()> {
        let constraints_present = constraints.is_some();
        let constraints_value = constraints.flatten();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "
            UPDATE active_swos
            SET work_order_title = COALESCE(?2, work_order_title),
                work_order_outcome = COALESCE(?3, work_order_outcome),
                work_order_constraints = CASE
                    WHEN ?4 THEN ?5
                    ELSE work_order_constraints
                END
            WHERE id = ?1
            ",
            params![
                swo_id,
                title,
                outcome,
                constraints_present,
                constraints_value
            ],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn create_recurring_template(
        &self,
        params: CreateRecurringWorkOrderTemplateParams<'_>,
    ) -> Result<RecurringWorkOrderTemplateRecord> {
        self.get_agent(params.owner_agent_id)?;
        if let Some(assignee_agent_id) = params.assignee_agent_id {
            self.get_agent(assignee_agent_id)?;
        }
        if let Some(project_id) = params.project_id {
            self.get_project(project_id)?
                .ok_or_else(|| KernelError::Internal(format!("Unknown project {}", project_id)))?;
        }
        if let Some(source_swo_id) = params.source_swo_id {
            self.get_swo_detail(source_swo_id)?.ok_or_else(|| {
                KernelError::Internal(format!("Unknown source SWO {}", source_swo_id))
            })?;
        }

        let schedule_json = serde_json::to_string(params.schedule).map_err(|error| {
            KernelError::Internal(format!("Failed to serialize recurring schedule: {}", error))
        })?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO rwo_templates (
                template_id,
                project_id,
                source_swo_id,
                owner_agent_id,
                assignee_agent_id,
                name,
                title,
                outcome,
                constraints,
                priority,
                include_prior_artifacts,
                schedule_json,
                status,
                next_run_at,
                last_run_at,
                last_run_status
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                params.template_id,
                params.project_id,
                params.source_swo_id,
                params.owner_agent_id,
                params.assignee_agent_id,
                params.name.trim(),
                params.title.trim(),
                params.outcome.trim(),
                params
                    .constraints
                    .map(str::trim)
                    .filter(|value| !value.is_empty()),
                params.priority.trim(),
                if params.include_prior_artifacts { 1 } else { 0 },
                schedule_json,
                params.status.trim(),
                params.next_run_at,
                params.last_run_at,
                params.last_run_status,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);

        self.get_recurring_template(params.template_id)?
            .ok_or_else(|| {
                KernelError::Internal(format!(
                    "Created recurring template {} could not be reloaded",
                    params.template_id
                ))
            })
    }

    pub fn get_recurring_template(
        &self,
        template_id: &str,
    ) -> Result<Option<RecurringWorkOrderTemplateRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            &Self::recurring_template_select_sql("WHERE t.template_id = ?1"),
            params![template_id],
            Self::recurring_template_from_row,
        )
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn update_recurring_template(
        &self,
        params: UpdateRecurringWorkOrderTemplateParams<'_>,
    ) -> Result<RecurringWorkOrderTemplateRecord> {
        let existing = self
            .get_recurring_template(params.template_id)?
            .ok_or_else(|| {
                KernelError::Internal(format!("Unknown recurring template {}", params.template_id))
            })?;

        let project_id = match params.project_id {
            Some(Some(project_id)) => {
                self.get_project(project_id)?.ok_or_else(|| {
                    KernelError::Internal(format!("Unknown project {}", project_id))
                })?;
                Some(project_id.to_string())
            }
            Some(None) => None,
            None => existing.project_id.clone(),
        };

        let source_swo_id = match params.source_swo_id {
            Some(Some(source_swo_id)) => {
                self.get_swo_detail(source_swo_id)?.ok_or_else(|| {
                    KernelError::Internal(format!("Unknown source SWO {}", source_swo_id))
                })?;
                Some(source_swo_id)
            }
            Some(None) => None,
            None => existing.source_swo_id,
        };

        let owner_agent_id = if let Some(owner_agent_id) = params.owner_agent_id {
            self.get_agent(owner_agent_id)?;
            owner_agent_id.to_string()
        } else {
            existing.owner_agent_id.clone()
        };

        let assignee_agent_id = match params.assignee_agent_id {
            Some(Some(assignee_agent_id)) => {
                self.get_agent(assignee_agent_id)?;
                Some(assignee_agent_id.to_string())
            }
            Some(None) => None,
            None => existing.assignee_agent_id.clone(),
        };

        let schedule = params
            .schedule
            .cloned()
            .unwrap_or(existing.schedule.clone());
        let schedule_json = serde_json::to_string(&schedule).map_err(|error| {
            KernelError::Internal(format!("Failed to serialize recurring schedule: {}", error))
        })?;

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE rwo_templates
             SET project_id = ?1,
                 source_swo_id = ?2,
                 owner_agent_id = ?3,
                 assignee_agent_id = ?4,
                 name = ?5,
                 title = ?6,
                 outcome = ?7,
                 constraints = ?8,
                 priority = ?9,
                 include_prior_artifacts = ?10,
                 schedule_json = ?11,
                 status = ?12,
                 next_run_at = ?13,
                 last_run_at = ?14,
                 last_run_status = ?15,
                 updated_at = CURRENT_TIMESTAMP
             WHERE template_id = ?16",
            params![
                project_id.as_deref(),
                source_swo_id,
                owner_agent_id,
                assignee_agent_id.as_deref(),
                params.name.unwrap_or(existing.name.as_str()).trim(),
                params.title.unwrap_or(existing.title.as_str()).trim(),
                params.outcome.unwrap_or(existing.outcome.as_str()).trim(),
                params
                    .constraints
                    .map(|value| value.map(str::trim).filter(|entry| !entry.is_empty()))
                    .unwrap_or(existing.constraints.as_deref()),
                params.priority.unwrap_or(existing.priority.as_str()).trim(),
                if params
                    .include_prior_artifacts
                    .unwrap_or(existing.include_prior_artifacts)
                {
                    1
                } else {
                    0
                },
                schedule_json,
                params.status.unwrap_or(existing.status.as_str()).trim(),
                params
                    .next_run_at
                    .map(|value| value.map(str::trim).filter(|entry| !entry.is_empty()))
                    .unwrap_or(existing.next_run_at.as_deref()),
                params
                    .last_run_at
                    .map(|value| value.map(str::trim).filter(|entry| !entry.is_empty()))
                    .unwrap_or(existing.last_run_at.as_deref()),
                params
                    .last_run_status
                    .map(|value| value.map(str::trim).filter(|entry| !entry.is_empty()))
                    .unwrap_or(existing.last_run_status.as_deref()),
                params.template_id,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);

        self.get_recurring_template(params.template_id)?
            .ok_or_else(|| {
                KernelError::Internal(format!(
                    "Updated recurring template {} could not be reloaded",
                    params.template_id
                ))
            })
    }

    pub fn list_recurring_templates(&self) -> Result<Vec<RecurringWorkOrderTemplateRecord>> {
        self.sync_recurring_runs_from_swos()?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&Self::recurring_template_select_sql(
                "ORDER BY COALESCE(t.next_run_at, t.updated_at) ASC, t.created_at DESC",
            ))
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map([], Self::recurring_template_from_row)
            .map_err(KernelError::Database)?;

        let mut templates = Vec::new();
        for row in rows {
            templates.push(row.map_err(KernelError::Database)?);
        }
        Ok(templates)
    }

    pub fn list_due_recurring_templates(&self) -> Result<Vec<RecurringWorkOrderTemplateRecord>> {
        self.sync_recurring_runs_from_swos()?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&Self::recurring_template_select_sql(
                "WHERE t.status = 'ACTIVE'
                   AND t.next_run_at IS NOT NULL
                   AND datetime(t.next_run_at) <= CURRENT_TIMESTAMP
                 ORDER BY t.next_run_at ASC, t.created_at ASC",
            ))
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map([], Self::recurring_template_from_row)
            .map_err(KernelError::Database)?;
        let mut templates = Vec::new();
        for row in rows {
            templates.push(row.map_err(KernelError::Database)?);
        }
        Ok(templates)
    }

    pub fn next_recurring_run_number(&self, template_id: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let next: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(run_number), 0) + 1 FROM rwo_runs WHERE template_id = ?1",
                params![template_id],
                |row| row.get(0),
            )
            .map_err(KernelError::Database)?;
        Ok(next.max(1))
    }

    pub fn create_recurring_run(
        &self,
        params: CreateRecurringWorkOrderRunParams<'_>,
    ) -> Result<RecurringWorkOrderRunRecord> {
        let artifact_ids_json = serde_json::to_string(params.artifact_ids).map_err(|error| {
            KernelError::Internal(format!(
                "Failed to serialize recurring artifact ids: {}",
                error
            ))
        })?;
        let queued_at = if let Some(queued_at) = params.queued_at {
            queued_at.to_string()
        } else {
            let conn = self.conn.lock().unwrap();
            Self::current_timestamp(&conn)?
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO rwo_runs (
                run_id,
                template_id,
                swo_id,
                project_id,
                run_number,
                status,
                trigger_source,
                queued_at,
                started_at,
                completed_at,
                error_message,
                artifact_ids_json
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                params.run_id,
                params.template_id,
                params.swo_id,
                params.project_id,
                params.run_number,
                params.status,
                params.trigger_source,
                queued_at,
                params.started_at,
                params.completed_at,
                params.error_message,
                artifact_ids_json,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);

        self.get_recurring_run(params.run_id)?.ok_or_else(|| {
            KernelError::Internal(format!(
                "Created recurring run {} could not be reloaded",
                params.run_id
            ))
        })
    }

    pub fn update_recurring_run(
        &self,
        params: UpdateRecurringWorkOrderRunParams<'_>,
    ) -> Result<RecurringWorkOrderRunRecord> {
        let existing = self.get_recurring_run(params.run_id)?.ok_or_else(|| {
            KernelError::Internal(format!("Unknown recurring run {}", params.run_id))
        })?;
        let artifact_ids = params
            .artifact_ids
            .unwrap_or(existing.artifact_ids.as_slice());
        let artifact_ids_json = serde_json::to_string(artifact_ids).map_err(|error| {
            KernelError::Internal(format!(
                "Failed to serialize recurring artifact ids: {}",
                error
            ))
        })?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE rwo_runs
             SET swo_id = ?1,
                 status = ?2,
                 started_at = ?3,
                 completed_at = ?4,
                 error_message = ?5,
                 artifact_ids_json = ?6
             WHERE run_id = ?7",
            params![
                params.swo_id.unwrap_or(existing.swo_id),
                params.status.unwrap_or(existing.status.as_str()),
                params
                    .started_at
                    .map(|value| value.map(str::trim).filter(|entry| !entry.is_empty()))
                    .unwrap_or(existing.started_at.as_deref()),
                params
                    .completed_at
                    .map(|value| value.map(str::trim).filter(|entry| !entry.is_empty()))
                    .unwrap_or(existing.completed_at.as_deref()),
                params
                    .error_message
                    .map(|value| value.map(str::trim).filter(|entry| !entry.is_empty()))
                    .unwrap_or(existing.error_message.as_deref()),
                artifact_ids_json,
                params.run_id,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);

        self.get_recurring_run(params.run_id)?.ok_or_else(|| {
            KernelError::Internal(format!(
                "Updated recurring run {} could not be reloaded",
                params.run_id
            ))
        })
    }

    pub fn get_recurring_run(&self, run_id: &str) -> Result<Option<RecurringWorkOrderRunRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT
                run_id,
                template_id,
                swo_id,
                project_id,
                run_number,
                status,
                trigger_source,
                queued_at,
                started_at,
                completed_at,
                artifact_ids_json,
                error_message
             FROM rwo_runs
             WHERE run_id = ?1",
            params![run_id],
            Self::recurring_run_from_row,
        )
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn list_recurring_runs(
        &self,
        template_id: Option<&str>,
    ) -> Result<Vec<RecurringWorkOrderRunRecord>> {
        self.sync_recurring_runs_from_swos()?;
        let conn = self.conn.lock().unwrap();
        let sql = if template_id.is_some() {
            "SELECT
                run_id,
                template_id,
                swo_id,
                project_id,
                run_number,
                status,
                trigger_source,
                queued_at,
                started_at,
                completed_at,
                artifact_ids_json,
                error_message
             FROM rwo_runs
             WHERE template_id = ?1
             ORDER BY run_number DESC, queued_at DESC"
        } else {
            "SELECT
                run_id,
                template_id,
                swo_id,
                project_id,
                run_number,
                status,
                trigger_source,
                queued_at,
                started_at,
                completed_at,
                artifact_ids_json,
                error_message
             FROM rwo_runs
             ORDER BY queued_at DESC, run_number DESC"
        };
        let mut stmt = conn.prepare(sql).map_err(KernelError::Database)?;
        let rows = if let Some(template_id) = template_id {
            stmt.query_map(params![template_id], Self::recurring_run_from_row)
                .map_err(KernelError::Database)?
        } else {
            stmt.query_map([], Self::recurring_run_from_row)
                .map_err(KernelError::Database)?
        };
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row.map_err(KernelError::Database)?);
        }
        Ok(runs)
    }

    pub fn latest_recurring_run_for_template(
        &self,
        template_id: &str,
    ) -> Result<Option<RecurringWorkOrderRunRecord>> {
        self.sync_recurring_runs_from_swos()?;
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT
                run_id,
                template_id,
                swo_id,
                project_id,
                run_number,
                status,
                trigger_source,
                queued_at,
                started_at,
                completed_at,
                artifact_ids_json,
                error_message
             FROM rwo_runs
             WHERE template_id = ?1
             ORDER BY run_number DESC, queued_at DESC
             LIMIT 1",
            params![template_id],
            Self::recurring_run_from_row,
        )
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn sync_recurring_runs_from_swos(&self) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(KernelError::Database)?;
        let mut stmt = tx
            .prepare(
                "SELECT run_id, template_id, swo_id
                 FROM rwo_runs
                 WHERE swo_id IS NOT NULL",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(KernelError::Database)?;
        let mut run_refs = Vec::new();
        for row in rows {
            run_refs.push(row.map_err(KernelError::Database)?);
        }
        drop(stmt);

        for (run_id, template_id, swo_id) in run_refs {
            let mut artifact_stmt = tx
                .prepare("SELECT id FROM outbox_artifacts WHERE swo_id = ?1 ORDER BY id ASC")
                .map_err(KernelError::Database)?;
            let artifact_rows = artifact_stmt
                .query_map(params![swo_id], |row| row.get::<_, i64>(0))
                .map_err(KernelError::Database)?;
            let mut artifact_ids = Vec::new();
            for artifact_row in artifact_rows {
                artifact_ids.push(artifact_row.map_err(KernelError::Database)?);
            }
            drop(artifact_stmt);

            let swo_status: String = tx
                .query_row(
                    "SELECT status FROM active_swos WHERE id = ?1",
                    params![swo_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(KernelError::Database)?
                .unwrap_or_else(|| "CANCELLED".to_string());
            let normalized_status = Self::normalize_rwo_status_from_swo(&swo_status);
            let started_at = tx
                .query_row(
                    "SELECT started_at FROM worker_runs WHERE swo_id = ?1 ORDER BY id ASC LIMIT 1",
                    params![swo_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(KernelError::Database)?;
            let completed_at = tx
                .query_row(
                    "SELECT COALESCE(finished_at, started_at)
                     FROM worker_runs
                     WHERE swo_id = ?1
                     ORDER BY id DESC
                     LIMIT 1",
                    params![swo_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(KernelError::Database)?;
            let failure_reason = tx
                .query_row(
                    "SELECT COALESCE(failure_reason, blocked_reason)
                     FROM worker_runs
                     WHERE swo_id = ?1
                       AND (failure_reason IS NOT NULL OR blocked_reason IS NOT NULL)
                     ORDER BY id DESC
                     LIMIT 1",
                    params![swo_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(KernelError::Database)?;
            let artifact_ids_json = serde_json::to_string(&artifact_ids).map_err(|error| {
                KernelError::Internal(format!(
                    "Failed to serialize recurring artifact ids: {}",
                    error
                ))
            })?;
            let current_timestamp: String = tx
                .query_row("SELECT CURRENT_TIMESTAMP", [], |row| row.get(0))
                .map_err(KernelError::Database)?;
            let completed_at_value = if matches!(
                normalized_status.as_str(),
                "COMPLETED" | "FAILED" | "CANCELLED" | "SKIPPED"
            ) {
                completed_at.or(Some(current_timestamp))
            } else {
                None
            };
            tx.execute(
                "UPDATE rwo_runs
                 SET status = ?1,
                     started_at = COALESCE(?2, started_at),
                     completed_at = ?3,
                     error_message = ?4,
                     artifact_ids_json = ?5
                 WHERE run_id = ?6",
                params![
                    normalized_status,
                    started_at,
                    completed_at_value,
                    failure_reason,
                    artifact_ids_json,
                    run_id,
                ],
            )
            .map_err(KernelError::Database)?;
            tx.execute(
                "UPDATE rwo_templates
                 SET last_run_status = ?1,
                     last_run_at = COALESCE(
                        (SELECT queued_at FROM rwo_runs WHERE run_id = ?2),
                        last_run_at
                     )
                 WHERE template_id = ?3",
                params![normalized_status, run_id, template_id],
            )
            .map_err(KernelError::Database)?;
        }
        tx.commit().map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn update_swo_initiative(
        &self,
        swo_id: i64,
        initiative_id: Option<&str>,
        initiative_name: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE active_swos SET initiative_id = ?1, initiative_name = ?2 WHERE id = ?3",
            params![initiative_id, initiative_name, swo_id],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    // ── Pulse Journal ─────────────────────────────────────────────────────────

    pub fn append_pulse_journal_entry(
        &self,
        cadence: &str,
        run_id: Option<&str>,
        agent_id: &str,
        entry_type: &str,
        summary: &str,
        detail_json: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO pulse_journal (cadence, run_id, agent_id, entry_type, summary, detail_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![cadence, run_id, agent_id, entry_type, summary, detail_json],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_pulse_journal(
        &self,
        cadence: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PulseJournalEntry>> {
        let conn = self.conn.lock().unwrap();
        let from_row = |row: &Row<'_>| {
            Ok(PulseJournalEntry {
                id: row.get(0)?,
                cadence: row.get(1)?,
                run_id: row.get(2)?,
                agent_id: row.get(3)?,
                entry_type: row.get(4)?,
                summary: row.get(5)?,
                detail_json: row.get(6)?,
                created_at: row.get(7)?,
            })
        };
        let mut entries = Vec::new();
        if let Some(cadence) = cadence {
            let mut stmt = conn
                .prepare(
                    "SELECT id, cadence, run_id, agent_id, entry_type, summary, detail_json, created_at
                     FROM pulse_journal
                     WHERE cadence = ?1
                     ORDER BY created_at DESC
                     LIMIT ?2",
                )
                .map_err(KernelError::Database)?;
            let rows = stmt
                .query_map(params![cadence, limit as i64], from_row)
                .map_err(KernelError::Database)?;
            for row in rows {
                entries.push(row.map_err(KernelError::Database)?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id, cadence, run_id, agent_id, entry_type, summary, detail_json, created_at
                     FROM pulse_journal
                     ORDER BY created_at DESC
                     LIMIT ?1",
                )
                .map_err(KernelError::Database)?;
            let rows = stmt
                .query_map(params![limit as i64], from_row)
                .map_err(KernelError::Database)?;
            for row in rows {
                entries.push(row.map_err(KernelError::Database)?);
            }
        }
        Ok(entries)
    }

    pub fn get_latest_pulse_entry(
        &self,
        cadence: &str,
    ) -> Result<Option<PulseJournalEntry>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, cadence, run_id, agent_id, entry_type, summary, detail_json, created_at
             FROM pulse_journal
             WHERE cadence = ?1
             ORDER BY created_at DESC
             LIMIT 1",
            params![cadence],
            |row| {
                Ok(PulseJournalEntry {
                    id: row.get(0)?,
                    cadence: row.get(1)?,
                    run_id: row.get(2)?,
                    agent_id: row.get(3)?,
                    entry_type: row.get(4)?,
                    summary: row.get(5)?,
                    detail_json: row.get(6)?,
                    created_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(KernelError::Database)
    }

    // ── Cadence State ─────────────────────────────────────────────────────────

    pub fn upsert_cadence_state(
        &self,
        domain: &str,
        check_interval_hours: i64,
        last_run_id: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cadence_state (domain, check_interval_hours, last_checked_at, last_run_id, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP, ?3, CURRENT_TIMESTAMP)
             ON CONFLICT(domain) DO UPDATE SET
                 check_interval_hours = excluded.check_interval_hours,
                 last_checked_at = CURRENT_TIMESTAMP,
                 last_run_id = COALESCE(excluded.last_run_id, last_run_id),
                 updated_at = CURRENT_TIMESTAMP",
            params![domain, check_interval_hours, last_run_id],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn list_due_cadence_domains(&self) -> Result<Vec<CadenceStateRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT domain, check_interval_hours, last_checked_at, last_run_id, created_at, updated_at
                 FROM cadence_state
                 WHERE last_checked_at IS NULL
                    OR (julianday('now') - julianday(last_checked_at)) * 24 >= check_interval_hours
                 ORDER BY domain ASC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(CadenceStateRecord {
                    domain: row.get(0)?,
                    check_interval_hours: row.get(1)?,
                    last_checked_at: row.get(2)?,
                    last_run_id: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn list_cadence_states(&self) -> Result<Vec<CadenceStateRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT domain, check_interval_hours, last_checked_at, last_run_id, created_at, updated_at
                 FROM cadence_state
                 ORDER BY domain ASC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(CadenceStateRecord {
                    domain: row.get(0)?,
                    check_interval_hours: row.get(1)?,
                    last_checked_at: row.get(2)?,
                    last_run_id: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn list_agents(&self) -> Result<Vec<AgentIdentity>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, parent_id, role, COALESCE(persona_prompt, raison_detre), raison_detre, default_provider, default_model, cron_interval_seconds, triage_model, execution_model
                 FROM agents
                 ORDER BY name ASC",
            )
            .map_err(KernelError::Database)?;

        let rows = stmt
            .query_map([], Self::agent_identity_from_row)
            .map_err(KernelError::Database)?;

        let mut agents = Vec::new();
        for row in rows {
            agents.push(row.map_err(KernelError::Database)?);
        }
        Ok(agents)
    }

    pub fn get_agent_presence(&self, now_ms: i64) -> Result<Vec<AgentPresenceRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    a.id,
                    (
                        SELECT h.status
                        FROM agent_heartbeats h
                        WHERE h.agent_id = a.id
                        ORDER BY h.last_seen_unix_ms DESC
                        LIMIT 1
                    ),
                    (
                        SELECT h.last_seen_unix_ms
                        FROM agent_heartbeats h
                        WHERE h.agent_id = a.id
                        ORDER BY h.last_seen_unix_ms DESC
                        LIMIT 1
                    )
                FROM agents a
                ",
            )
            .map_err(KernelError::Database)?;

        let rows = stmt
            .query_map([], |row| {
                let agent_id: String = row.get(0)?;
                let raw_status: Option<String> = row.get(1)?;
                let last_seen_unix_ms: Option<i64> = row.get(2)?;
                let (presence, last_seen_age_ms) =
                    Self::normalize_presence(raw_status.as_deref(), last_seen_unix_ms, now_ms);

                Ok(AgentPresenceRecord {
                    agent_id,
                    raw_status,
                    presence,
                    last_seen_unix_ms,
                    last_seen_age_ms,
                })
            })
            .map_err(KernelError::Database)?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn get_agent_tree_snapshot(&self, now_ms: i64) -> Result<Vec<AgentTreeNodeRecord>> {
        let agents = self.list_agents()?;
        let org_profile_map = agents
            .iter()
            .map(|agent| {
                self.get_agent_org_profile(&agent.id)
                    .map(|profile| (agent.id.clone(), profile))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        let presence_map = self
            .get_agent_presence(now_ms)?
            .into_iter()
            .map(|presence| (presence.agent_id.clone(), presence))
            .collect::<HashMap<_, _>>();
        let cron_last_fired_map = agents
            .iter()
            .map(|agent| {
                self.get_agent_cron_last_fired(&agent.id)
                    .map(|value| (agent.id.clone(), value))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        let agent_map = agents
            .iter()
            .cloned()
            .map(|agent| (agent.id.clone(), agent))
            .collect::<HashMap<_, _>>();
        let mut child_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut root_ids = Vec::new();

        for agent in &agents {
            match agent.parent_id.as_ref() {
                Some(parent_id) if agent_map.contains_key(parent_id) => {
                    child_map
                        .entry(parent_id.clone())
                        .or_default()
                        .push(agent.id.clone());
                }
                _ => root_ids.push(agent.id.clone()),
            }
        }

        fn build_node(
            agent_id: &str,
            depth: usize,
            agent_map: &HashMap<String, AgentIdentity>,
            org_profile_map: &HashMap<String, AgentOrgProfileRecord>,
            child_map: &HashMap<String, Vec<String>>,
            presence_map: &HashMap<String, AgentPresenceRecord>,
            cron_last_fired_map: &HashMap<String, Option<String>>,
        ) -> AgentTreeNodeRecord {
            let agent = agent_map
                .get(agent_id)
                .expect("agent tree requested unknown agent id");
            let manager = agent.parent_id.as_ref().map(|parent_id| {
                agent_map
                    .get(parent_id)
                    .map(Registry::agent_summary_from_identity)
                    .unwrap_or(AgentSummaryRecord {
                        id: parent_id.clone(),
                        name: parent_id.clone(),
                        role: "UNKNOWN".to_string(),
                    })
            });

            let mut child_ids = child_map.get(agent_id).cloned().unwrap_or_default();
            child_ids.sort_by(|a, b| {
                let left = agent_map
                    .get(a)
                    .map(|agent| agent.name.as_str())
                    .unwrap_or(a.as_str());
                let right = agent_map
                    .get(b)
                    .map(|agent| agent.name.as_str())
                    .unwrap_or(b.as_str());
                left.cmp(right)
            });

            let children = child_ids
                .iter()
                .map(|child_id| {
                    build_node(
                        child_id,
                        depth + 1,
                        agent_map,
                        org_profile_map,
                        child_map,
                        presence_map,
                        cron_last_fired_map,
                    )
                })
                .collect::<Vec<_>>();
            let descendant_count = children
                .iter()
                .map(|child| 1 + child.descendant_count)
                .sum::<usize>();
            let presence = presence_map.get(agent_id);

            AgentTreeNodeRecord {
                id: agent.id.clone(),
                name: agent.name.clone(),
                role: agent.role.clone(),
                manager,
                org_profile: org_profile_map
                    .get(agent_id)
                    .cloned()
                    .unwrap_or_else(|| AgentOrgProfileRecord::default_for_agent(agent)),
                depth,
                is_direct_report: depth == 1,
                direct_report_count: children.len(),
                descendant_count,
                cron_enabled: agent.cron_interval_seconds.is_some(),
                presence: presence
                    .map(|presence| presence.presence.clone())
                    .unwrap_or_else(|| "OFFLINE".to_string()),
                last_seen_unix_ms: presence.and_then(|presence| presence.last_seen_unix_ms),
                last_seen_age_ms: presence.and_then(|presence| presence.last_seen_age_ms),
                last_cron_fired_at: cron_last_fired_map.get(agent_id).cloned().flatten(),
                children,
                default_provider: agent.default_provider.clone(),
                model: agent.default_model.clone(),
                triage_model: agent.triage_model.clone(),
                execution_model: agent.execution_model.clone(),
                raison_detre: agent.raison_detre.clone(),
                persona_prompt: agent.persona_prompt.clone(),
            }
        }

        root_ids.sort_by(|a, b| {
            let left = agent_map
                .get(a)
                .map(|agent| agent.name.as_str())
                .unwrap_or(a.as_str());
            let right = agent_map
                .get(b)
                .map(|agent| agent.name.as_str())
                .unwrap_or(b.as_str());
            left.cmp(right)
        });

        Ok(root_ids
            .iter()
            .map(|root_id| {
                build_node(
                    root_id,
                    0,
                    &agent_map,
                    &org_profile_map,
                    &child_map,
                    &presence_map,
                    &cron_last_fired_map,
                )
            })
            .collect())
    }

    pub fn get_agent_detail_snapshot(
        &self,
        agent_id: &str,
        now_ms: i64,
    ) -> Result<AgentDetailRecord> {
        let agent = self.get_agent(agent_id)?;
        let manifest = self.get_agent_manifest(agent_id)?;
        let org_profile = self.get_agent_org_profile(agent_id)?;
        let team_goals = self.list_descendant_team_goals(agent_id)?;
        let delegation_decisions =
            self.list_recent_delegation_decisions_for_manager(agent_id, 20)?;
        let team_gaps = self.list_recent_team_gaps_for_manager(agent_id, 20)?;
        let agents = self.list_agents()?;
        let agent_map = agents
            .iter()
            .cloned()
            .map(|agent| (agent.id.clone(), agent))
            .collect::<HashMap<_, _>>();
        let presence_map = self
            .get_agent_presence(now_ms)?
            .into_iter()
            .map(|presence| (presence.agent_id.clone(), presence))
            .collect::<HashMap<_, _>>();
        let presence = presence_map.get(agent_id);
        let last_cron_fired_at = self.get_agent_cron_last_fired(agent_id)?;

        let direct_reports = self
            .get_subordinates(agent_id)?
            .into_iter()
            .map(|report| {
                let report_presence = presence_map.get(&report.id);
                DirectReportSummaryRecord {
                    id: report.id.clone(),
                    name: report.name.clone(),
                    role: report.role.clone(),
                    cron_enabled: report.cron_interval_seconds.is_some(),
                    presence: report_presence
                        .map(|presence| presence.presence.clone())
                        .unwrap_or_else(|| "OFFLINE".to_string()),
                    last_seen_unix_ms: report_presence
                        .and_then(|presence| presence.last_seen_unix_ms),
                    last_seen_age_ms: report_presence
                        .and_then(|presence| presence.last_seen_age_ms),
                }
            })
            .collect::<Vec<_>>();

        let manager = agent.parent_id.as_ref().map(|parent_id| {
            agent_map
                .get(parent_id)
                .map(Self::agent_summary_from_identity)
                .unwrap_or(AgentSummaryRecord {
                    id: parent_id.clone(),
                    name: parent_id.clone(),
                    role: "UNKNOWN".to_string(),
                })
        });

        let conn = self.conn.lock().unwrap();
        let heartbeat_timeline = Self::load_heartbeat_timeline(&conn, agent_id, now_ms, 20)?;
        let assigned_swos =
            Self::load_agent_swo_summaries_for_field(&conn, "assigned_agent_id", agent_id, 10)?;
        let owned_swos =
            Self::load_agent_swo_summaries_for_field(&conn, "owner_agent_id", agent_id, 10)?;
        let created_swos =
            Self::load_agent_swo_summaries_for_field(&conn, "created_by_agent_id", agent_id, 10)?;
        let recent_hires = Self::load_recent_hires_for_manager(&conn, agent_id, 10)?;
        drop(conn);
        let interactions = self.load_recent_agent_interactions(agent_id, 20)?;
        let bound_skills = self.list_agent_skill_bindings(agent_id)?;
        let bound_tools = self.list_agent_tool_bindings(agent_id)?;
        let bound_mcp_connectors = self.list_agent_mcp_bindings(agent_id)?;
        let external_channel_bindings = self.list_external_channel_bindings(Some(agent_id))?;

        Ok(AgentDetailRecord {
            id: agent.id.clone(),
            name: agent.name,
            role: agent.role,
            manager,
            org_profile,
            team_goals,
            delegation_decisions,
            team_gaps,
            direct_reports,
            persona_prompt: manifest.persona_prompt.clone(),
            raison_detre: manifest.mission.clone(),
            provider: manifest.provider.provider_name.clone(),
            model: manifest.provider.model.clone(),
            cron_interval_seconds: manifest.schedule.cron_interval_seconds,
            presence: presence
                .map(|presence| presence.presence.clone())
                .unwrap_or_else(|| "OFFLINE".to_string()),
            last_seen_unix_ms: presence.and_then(|presence| presence.last_seen_unix_ms),
            last_seen_age_ms: presence.and_then(|presence| presence.last_seen_age_ms),
            last_cron_fired_at,
            heartbeat_timeline,
            assigned_swos,
            owned_swos,
            created_swos,
            recent_hires,
            interactions,
            manifest,
            bound_skills,
            bound_tools,
            bound_mcp_connectors,
            external_channel_bindings,
        })
    }

    pub fn list_swo_summaries(&self, limit: usize) -> Result<Vec<AgentSwoSummaryRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&format!(
                "{} ORDER BY
                    CASE s.status
                        WHEN 'IN_PROGRESS' THEN 0
                        WHEN 'PENDING' THEN 1
                        WHEN 'FAILED' THEN 2
                        WHEN 'CANCELLED' THEN 3
                        WHEN 'COMPLETED' THEN 4
                        ELSE 5
                    END,
                    s.id DESC
                LIMIT ?1",
                Self::active_swo_select_sql("")
            ))
            .map_err(KernelError::Database)?;

        let rows = stmt
            .query_map(
                params![limit.clamp(1, 200) as i64],
                Self::active_swo_from_row,
            )
            .map_err(KernelError::Database)?;

        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(Self::build_swo_summary(
                &conn,
                row.map_err(KernelError::Database)?,
            )?);
        }
        Ok(summaries)
    }

    pub fn repair_runtime_state(&self) -> Result<()> {
        let agents = self.list_agents()?;
        for agent in &agents {
            let memory_dir = self.storage_base_path()?.join("agents").join(&agent.id);
            std::fs::create_dir_all(&memory_dir).map_err(|e| {
                KernelError::Internal(format!(
                    "Failed to create agent storage dir for {}: {}",
                    agent.id, e
                ))
            })?;
            let db_path = memory_dir.join("memory.sqlite");
            let agent_conn = Connection::open(&db_path).map_err(KernelError::Database)?;
            agent_conn
                .execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
                .map_err(KernelError::Database)?;
            Self::ensure_agent_memory_schema(&agent_conn)?;
        }
        Ok(())
    }

    pub fn checkpoint_wal(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(KernelError::Database)?;
        Ok(())
    }

    /// Resolve an agent by name. CHA-428 upgrade: falls back to
    /// case-insensitive match so prompt-emitted names with casing drift
    /// ("felicity" vs "Felicity") still resolve.
    pub fn find_agent_id_by_name(&self, name: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let exact: Option<String> = conn
            .query_row(
                "SELECT id FROM agents WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()
            .map_err(KernelError::Database)?;
        if exact.is_some() {
            return Ok(exact);
        }
        conn.query_row(
            "SELECT id FROM agents WHERE lower(name) = lower(?1) LIMIT 1",
            params![name],
            |row| row.get(0),
        )
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn upsert_runtime_metadata(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO runtime_metadata (key, value)
             VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn get_runtime_metadata(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM runtime_metadata WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn get_runtime_context(&self) -> Result<RuntimeContext> {
        Ok(RuntimeContext {
            company_name: self.get_runtime_metadata("company_name")?,
            profile_id: self.get_runtime_metadata("profile_id")?,
            company_charter_source: self.get_runtime_metadata("company_charter_source")?,
            company_summary: self.get_runtime_metadata("company_summary")?,
            autonomous_hiring_mode: self.get_runtime_metadata("autonomous_hiring_mode")?,
            active_seed_spec_path: self.get_runtime_metadata("active_seed_spec_path")?,
            last_archive_path: self.get_runtime_metadata("last_archive_path")?,
            operating_principles: self.get_runtime_metadata("operating_principles")?,
            non_goals: self.get_runtime_metadata("non_goals")?,
        })
    }

    pub fn get_runtime_archive_counts(&self) -> Result<RuntimeArchiveCounts> {
        let conn = self.conn.lock().unwrap();
        let count = |table: &str| -> Result<usize> {
            let sql = format!("SELECT COUNT(*) FROM {}", table);
            let value: i64 = conn
                .query_row(&sql, [], |row| row.get(0))
                .map_err(KernelError::Database)?;
            Ok(value.max(0) as usize)
        };

        Ok(RuntimeArchiveCounts {
            agents: count("agents")?,
            active_swos: count("active_swos")?,
            heartbeats: count("agent_heartbeats")?,
            swo_results: count("swo_results")?,
            manager_reviews: count("manager_reviews")?,
            outbox_artifacts: count("outbox_artifacts")?,
            agent_hires: count("agent_hires")?,
        })
    }

    pub fn list_agent_interaction_counts(&self) -> Result<Vec<AgentInteractionCount>> {
        let agents = self.list_agents()?;
        let mut counts = Vec::new();

        for agent in agents {
            let db_path = self.agent_memory_db_path(&agent.id)?;
            let interactions = if db_path.exists() {
                let conn = Connection::open(&db_path).map_err(KernelError::Database)?;
                Self::ensure_agent_memory_schema(&conn)?;
                let count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM interactions", [], |row| row.get(0))
                    .map_err(KernelError::Database)?;
                count.max(0) as usize
            } else {
                0
            };

            counts.push(AgentInteractionCount {
                agent_id: agent.id,
                agent_name: agent.name,
                interactions,
            });
        }

        counts.sort_by(|a, b| {
            b.interactions
                .cmp(&a.interactions)
                .then_with(|| a.agent_name.cmp(&b.agent_name))
        });
        Ok(counts)
    }

    pub fn clear_runtime_state(&self) -> Result<()> {
        {
            let conn = self.conn.lock().unwrap();
            conn.execute_batch(
                "
                DELETE FROM manager_reviews;
                DELETE FROM swo_results;
                DELETE FROM message_attachments;
                DELETE FROM swo_attachments;
                DELETE FROM attachments;
                DELETE FROM outbox_artifacts;
                DELETE FROM agent_hires;
                DELETE FROM agent_heartbeats;
                DELETE FROM active_swos;
                DELETE FROM agents;
                DELETE FROM runtime_metadata;
                ",
            )
            .map_err(KernelError::Database)?;
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .map_err(KernelError::Database)?;
        }

        let agents_root = self.storage_base_path()?.join("agents");
        if agents_root.exists() {
            std::fs::remove_dir_all(&agents_root)?;
        }
        std::fs::create_dir_all(&agents_root)?;
        Ok(())
    }

    pub fn get_agent(&self, id: &str) -> Result<AgentIdentity> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, parent_id, role, COALESCE(persona_prompt, raison_detre), raison_detre, default_provider, default_model, cron_interval_seconds, triage_model, execution_model
             FROM agents WHERE id = ?1",
            )
            .map_err(KernelError::Database)?;

        let mut rows = stmt.query(params![id]).map_err(KernelError::Database)?;

        if let Some(row) = rows.next().map_err(KernelError::Database)? {
            Self::agent_identity_from_row(row).map_err(KernelError::Database)
        } else {
            Err(KernelError::AgentNotFound(id.to_string()))
        }
    }

    pub fn get_subordinates(&self, parent_id: &str) -> Result<Vec<AgentIdentity>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, parent_id, role, COALESCE(persona_prompt, raison_detre), raison_detre, default_provider, default_model, cron_interval_seconds, triage_model, execution_model
             FROM agents WHERE parent_id = ?1",
            )
            .map_err(KernelError::Database)?;

        let rows = stmt
            .query_map(params![parent_id], Self::agent_identity_from_row)
            .map_err(KernelError::Database)?;

        let mut subordinates = Vec::new();
        for row_result in rows {
            subordinates.push(row_result.map_err(KernelError::Database)?);
        }

        Ok(subordinates)
    }

    pub fn find_direct_subordinate(
        &self,
        parent_id: &str,
        requested_id: Option<&str>,
        requested_name: Option<&str>,
    ) -> Result<Option<AgentIdentity>> {
        let subordinates = self.get_subordinates(parent_id)?;
        if let Some(requested_id) = requested_id {
            if let Some(found) = subordinates.iter().find(|sub| sub.id == requested_id) {
                return Ok(Some(found.clone()));
            }
        }
        if let Some(requested_name) = requested_name {
            // Try exact match first (case-insensitive)
            if let Some(found) = subordinates
                .iter()
                .find(|sub| sub.name.eq_ignore_ascii_case(requested_name))
            {
                return Ok(Some(found.clone()));
            }
            // Strip " (Role)" suffix — kernel SWO views sometimes use "Name (Role)" format
            let name_without_role = requested_name
                .split('(')
                .next()
                .unwrap_or(requested_name)
                .trim();
            if name_without_role != requested_name {
                if let Some(found) = subordinates
                    .iter()
                    .find(|sub| sub.name.eq_ignore_ascii_case(name_without_role))
                {
                    return Ok(Some(found.clone()));
                }
            }
        }
        Ok(None)
    }

    pub fn bind_agent_token(&self, id: &str, bot_token: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agents SET bot_token = ?1 WHERE id = ?2",
            params![bot_token, id],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn get_agent_by_token(&self, token: &str) -> Result<AgentIdentity> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, parent_id, role, COALESCE(persona_prompt, raison_detre), raison_detre, default_provider, default_model, cron_interval_seconds, triage_model, execution_model
                 FROM agents WHERE bot_token = ?1",
            )
            .map_err(KernelError::Database)?;

        let mut rows = stmt.query(params![token]).map_err(KernelError::Database)?;

        if let Some(row) = rows.next().map_err(KernelError::Database)? {
            Self::agent_identity_from_row(row).map_err(KernelError::Database)
        } else {
            Err(KernelError::AgentNotFound(format!("Token: {}", token)))
        }
    }

    fn external_channel_binding_from_row(
        row: &Row,
        route_token: Option<String>,
        secret_token: Option<String>,
    ) -> rusqlite::Result<ExternalChannelBindingSecretRecord> {
        let binding = ExternalChannelBindingRecord {
            agent_id: row.get(0)?,
            channel: row.get(1)?,
            enabled: row.get::<_, i64>(2)? != 0,
            allowed_chat_id: row.get(3)?,
            allowed_user_id: row.get(4)?,
            has_route_token: route_token
                .as_ref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
            has_secret_token: secret_token
                .as_ref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
            last_inbound_at: row.get(5)?,
            last_delivery_at: row.get(6)?,
            last_delivery_status: row.get(7)?,
            last_delivery_detail: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        };
        Ok(ExternalChannelBindingSecretRecord {
            binding,
            route_token,
            secret_token,
        })
    }

    fn load_external_binding(
        conn: &Connection,
        agent_id: &str,
        channel: &str,
    ) -> Result<Option<ExternalChannelBindingSecretRecord>> {
        let mut stmt = conn
            .prepare(
                "SELECT
                    b.agent_id,
                    b.channel,
                    b.enabled,
                    b.allowed_chat_id,
                    b.allowed_user_id,
                    b.last_inbound_at,
                    b.last_delivery_at,
                    b.last_delivery_status,
                    b.last_delivery_detail,
                    b.created_at,
                    b.updated_at,
                    a.bot_token,
                    b.secret_token
                 FROM external_channel_bindings b
                 JOIN agents a ON a.id = b.agent_id
                 WHERE b.agent_id = ?1 AND b.channel = ?2
                 LIMIT 1",
            )
            .map_err(KernelError::Database)?;

        stmt.query_row(params![agent_id, channel], |row| {
            let route_token: Option<String> = row.get(11)?;
            let secret_token: Option<String> = row.get(12)?;
            Self::external_channel_binding_from_row(row, route_token, secret_token)
        })
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn upsert_external_channel_binding(
        &self,
        params: UpsertExternalChannelBindingParams<'_>,
    ) -> Result<ExternalChannelBindingRecord> {
        let normalized_channel = params.channel.trim().to_ascii_lowercase();
        if normalized_channel.is_empty() {
            return Err(KernelError::Internal(
                "External channel is required".to_string(),
            ));
        }
        let _ = self.get_agent(params.agent_id)?;

        let allowed_chat_id = params
            .allowed_chat_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let allowed_user_id = params
            .allowed_user_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO external_channel_bindings (agent_id, channel, enabled, allowed_chat_id, allowed_user_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(agent_id, channel) DO NOTHING",
            params![
                params.agent_id,
                &normalized_channel,
                if params.enabled { 1 } else { 0 },
                allowed_chat_id.as_deref(),
                allowed_user_id.as_deref()
            ],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "UPDATE external_channel_bindings
             SET enabled = ?3,
                 allowed_chat_id = ?4,
                 allowed_user_id = ?5,
                 updated_at = CURRENT_TIMESTAMP
             WHERE agent_id = ?1 AND channel = ?2",
            params![
                params.agent_id,
                &normalized_channel,
                if params.enabled { 1 } else { 0 },
                allowed_chat_id.as_deref(),
                allowed_user_id.as_deref()
            ],
        )
        .map_err(KernelError::Database)?;

        if let Some(route_token) = params.route_token {
            let normalized_route_token = route_token.trim();
            let route_token =
                (!normalized_route_token.is_empty()).then(|| normalized_route_token.to_string());
            conn.execute(
                "UPDATE agents SET bot_token = ?1 WHERE id = ?2",
                params![route_token, params.agent_id],
            )
            .map_err(KernelError::Database)?;
        }

        if let Some(secret_token) = params.secret_token {
            let normalized_secret_token = secret_token.trim();
            let secret_token =
                (!normalized_secret_token.is_empty()).then(|| normalized_secret_token.to_string());
            conn.execute(
                "UPDATE external_channel_bindings
                 SET secret_token = ?3,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE agent_id = ?1 AND channel = ?2",
                params![params.agent_id, &normalized_channel, secret_token],
            )
            .map_err(KernelError::Database)?;
        }

        Self::load_external_binding(&conn, params.agent_id, &normalized_channel)?
            .map(|record| record.binding)
            .ok_or_else(|| {
                KernelError::Internal(
                    "External channel binding was not persisted correctly".to_string(),
                )
            })
    }

    pub fn list_external_channel_bindings(
        &self,
        agent_id: Option<&str>,
    ) -> Result<Vec<ExternalChannelBindingRecord>> {
        let conn = self.conn.lock().unwrap();
        let sql = if agent_id.is_some() {
            "SELECT
                b.agent_id,
                b.channel,
                b.enabled,
                b.allowed_chat_id,
                b.allowed_user_id,
                b.last_inbound_at,
                b.last_delivery_at,
                b.last_delivery_status,
                b.last_delivery_detail,
                b.created_at,
                b.updated_at,
                a.bot_token,
                b.secret_token
             FROM external_channel_bindings b
             JOIN agents a ON a.id = b.agent_id
             WHERE b.agent_id = ?1
             ORDER BY b.channel ASC"
        } else {
            "SELECT
                b.agent_id,
                b.channel,
                b.enabled,
                b.allowed_chat_id,
                b.allowed_user_id,
                b.last_inbound_at,
                b.last_delivery_at,
                b.last_delivery_status,
                b.last_delivery_detail,
                b.created_at,
                b.updated_at,
                a.bot_token,
                b.secret_token
             FROM external_channel_bindings b
             JOIN agents a ON a.id = b.agent_id
             ORDER BY b.channel ASC, b.agent_id ASC"
        };
        let mut stmt = conn.prepare(sql).map_err(KernelError::Database)?;
        let mapper = |row: &Row| {
            let route_token: Option<String> = row.get(11)?;
            let secret_token: Option<String> = row.get(12)?;
            Self::external_channel_binding_from_row(row, route_token, secret_token)
                .map(|record| record.binding)
        };
        let rows = if let Some(agent_id) = agent_id {
            stmt.query_map(params![agent_id], mapper)
        } else {
            stmt.query_map([], mapper)
        }
        .map_err(KernelError::Database)?;

        let mut bindings = Vec::new();
        for row in rows {
            bindings.push(row.map_err(KernelError::Database)?);
        }
        Ok(bindings)
    }

    pub fn resolve_external_channel_binding_by_route_token(
        &self,
        channel: &str,
        route_token: &str,
    ) -> Result<Option<ExternalChannelBindingSecretRecord>> {
        let normalized_channel = channel.trim().to_ascii_lowercase();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT
                    b.agent_id,
                    b.channel,
                    b.enabled,
                    b.allowed_chat_id,
                    b.allowed_user_id,
                    b.last_inbound_at,
                    b.last_delivery_at,
                    b.last_delivery_status,
                    b.last_delivery_detail,
                    b.created_at,
                    b.updated_at,
                    a.bot_token,
                    b.secret_token
                 FROM external_channel_bindings b
                 JOIN agents a ON a.id = b.agent_id
                 WHERE b.channel = ?1 AND a.bot_token = ?2
                 LIMIT 1",
            )
            .map_err(KernelError::Database)?;
        stmt.query_row(params![normalized_channel, route_token], |row| {
            let route_token: Option<String> = row.get(11)?;
            let secret_token: Option<String> = row.get(12)?;
            Self::external_channel_binding_from_row(row, route_token, secret_token)
        })
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn claim_external_message_receipt(
        &self,
        channel: &str,
        external_chat_id: &str,
        external_message_id: &str,
    ) -> Result<bool> {
        let normalized_channel = channel.trim().to_ascii_lowercase();
        let conn = self.conn.lock().unwrap();
        let changed = conn
            .execute(
                "INSERT OR IGNORE INTO external_message_receipts (channel, external_chat_id, external_message_id)
                 VALUES (?1, ?2, ?3)",
                params![normalized_channel, external_chat_id, external_message_id],
            )
            .map_err(KernelError::Database)?;
        Ok(changed > 0)
    }

    pub fn touch_external_chat_session(
        &self,
        params: TouchExternalChatSessionParams<'_>,
    ) -> Result<ExternalChatSessionRecord> {
        let normalized_channel = params.channel.trim().to_ascii_lowercase();
        let normalized_user_id = params.external_user_id.unwrap_or("").trim().to_string();
        let conn = self.conn.lock().unwrap();
        let existing_session_id = conn
            .query_row(
                "SELECT session_id
                 FROM external_chat_sessions
                 WHERE agent_id = ?1 AND channel = ?2 AND external_chat_id = ?3 AND external_user_id = ?4
                 LIMIT 1",
                params![
                    params.agent_id,
                    &normalized_channel,
                    params.external_chat_id,
                    normalized_user_id.as_str()
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(KernelError::Database)?;
        let session_id = existing_session_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        conn.execute(
            "INSERT INTO external_chat_sessions (
                session_id,
                agent_id,
                channel,
                external_chat_id,
                external_user_id,
                conversation_id,
                last_inbound_message_id,
                last_inbound_at,
                updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
             ON CONFLICT(agent_id, channel, external_chat_id, external_user_id) DO UPDATE SET
                conversation_id = excluded.conversation_id,
                last_inbound_message_id = COALESCE(excluded.last_inbound_message_id, external_chat_sessions.last_inbound_message_id),
                last_inbound_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP",
            params![
                session_id,
                params.agent_id,
                &normalized_channel,
                params.external_chat_id,
                normalized_user_id.as_str(),
                params.conversation_id,
                params.last_inbound_message_id
            ],
        )
        .map_err(KernelError::Database)?;
        conn.execute(
            "UPDATE external_channel_bindings
             SET last_inbound_at = CURRENT_TIMESTAMP,
                 updated_at = CURRENT_TIMESTAMP
             WHERE agent_id = ?1 AND channel = ?2",
            params![params.agent_id, &normalized_channel],
        )
        .map_err(KernelError::Database)?;

        conn.query_row(
            "SELECT
                session_id,
                agent_id,
                channel,
                external_chat_id,
                external_user_id,
                conversation_id,
                last_inbound_message_id,
                last_inbound_at,
                last_delivery_status,
                last_delivery_detail,
                created_at,
                updated_at
             FROM external_chat_sessions
             WHERE session_id = ?1",
            params![session_id],
            |row| {
                let external_user_id: String = row.get(4)?;
                Ok(ExternalChatSessionRecord {
                    session_id: row.get(0)?,
                    agent_id: row.get(1)?,
                    channel: row.get(2)?,
                    external_chat_id: row.get(3)?,
                    external_user_id: (!external_user_id.is_empty()).then_some(external_user_id),
                    conversation_id: row.get(5)?,
                    last_inbound_message_id: row.get(6)?,
                    last_inbound_at: row.get(7)?,
                    last_delivery_status: row.get(8)?,
                    last_delivery_detail: row.get(9)?,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                })
            },
        )
        .map_err(KernelError::Database)
    }

    pub fn record_external_channel_delivery_event(
        &self,
        params: RecordExternalChannelDeliveryEventParams<'_>,
    ) -> Result<ExternalChannelDeliveryEventRecord> {
        let normalized_channel = params.channel.trim().to_ascii_lowercase();
        let normalized_direction = params.direction.trim().to_ascii_lowercase();
        let normalized_status = params.status.trim().to_ascii_lowercase();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO external_channel_delivery_events (
                agent_id,
                channel,
                session_id,
                direction,
                status,
                detail,
                external_chat_id,
                external_user_id,
                external_message_id
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                params.agent_id,
                &normalized_channel,
                params.session_id,
                normalized_direction,
                normalized_status,
                params.detail,
                params.external_chat_id,
                params.external_user_id,
                params.external_message_id
            ],
        )
        .map_err(KernelError::Database)?;
        let event_id = conn.last_insert_rowid();

        conn.execute(
            "UPDATE external_channel_bindings
             SET last_delivery_at = CURRENT_TIMESTAMP,
                 last_delivery_status = ?3,
                 last_delivery_detail = ?4,
                 updated_at = CURRENT_TIMESTAMP
             WHERE agent_id = ?1 AND channel = ?2",
            params![
                params.agent_id,
                &normalized_channel,
                &normalized_status,
                params.detail
            ],
        )
        .map_err(KernelError::Database)?;

        if let Some(session_id) = params.session_id {
            conn.execute(
                "UPDATE external_chat_sessions
                 SET last_delivery_status = ?2,
                     last_delivery_detail = ?3,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE session_id = ?1",
                params![session_id, &normalized_status, params.detail],
            )
            .map_err(KernelError::Database)?;
        }

        conn.query_row(
            "SELECT
                id,
                agent_id,
                channel,
                session_id,
                direction,
                status,
                detail,
                external_chat_id,
                external_user_id,
                external_message_id,
                created_at
             FROM external_channel_delivery_events
             WHERE id = ?1",
            params![event_id],
            |row| {
                Ok(ExternalChannelDeliveryEventRecord {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    channel: row.get(2)?,
                    session_id: row.get(3)?,
                    direction: row.get(4)?,
                    status: row.get(5)?,
                    detail: row.get(6)?,
                    external_chat_id: row.get(7)?,
                    external_user_id: row.get(8)?,
                    external_message_id: row.get(9)?,
                    created_at: row.get(10)?,
                })
            },
        )
        .map_err(KernelError::Database)
    }

    pub fn list_recent_external_channel_delivery_events(
        &self,
        limit: usize,
    ) -> Result<Vec<ExternalChannelDeliveryEventRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT
                    id,
                    agent_id,
                    channel,
                    session_id,
                    direction,
                    status,
                    detail,
                    external_chat_id,
                    external_user_id,
                    external_message_id,
                    created_at
                 FROM external_channel_delivery_events
                 ORDER BY id DESC
                 LIMIT ?1",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 200) as i64], |row| {
                Ok(ExternalChannelDeliveryEventRecord {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    channel: row.get(2)?,
                    session_id: row.get(3)?,
                    direction: row.get(4)?,
                    status: row.get(5)?,
                    detail: row.get(6)?,
                    external_chat_id: row.get(7)?,
                    external_user_id: row.get(8)?,
                    external_message_id: row.get(9)?,
                    created_at: row.get(10)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(KernelError::Database)?);
        }
        Ok(events)
    }

    pub fn create_swo(
        &self,
        assigned_agent_id: &str,
        manager_agent_id: &str,
        swo_payload: &str,
        status: &str,
    ) -> Result<i64> {
        self.create_swo_with_parent(
            assigned_agent_id,
            manager_agent_id,
            swo_payload,
            status,
            None,
        )
    }

    pub fn create_swo_with_parent(
        &self,
        assigned_agent_id: &str,
        manager_agent_id: &str,
        swo_payload: &str,
        status: &str,
        parent_swo_id: Option<i64>,
    ) -> Result<i64> {
        self.create_swo_with_metadata(CreateSwoParams {
            assigned_agent_id,
            owner_agent_id: manager_agent_id,
            created_by_agent_id: manager_agent_id,
            payload: swo_payload,
            status,
            parent_swo_id,
            kind: "TASK",
            source: "HSM",
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
        })
    }

    pub fn create_swo_with_metadata(&self, params: CreateSwoParams<'_>) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO active_swos (
                assigned_agent_id,
                manager_agent_id,
                owner_agent_id,
                created_by_agent_id,
                swo_payload,
                status,
                kind,
                source,
                work_order_title,
                work_order_outcome,
                work_order_constraints,
                requested_owner_agent_id,
                requested_assignee_agent_id,
                routing_policy,
                parent_swo_id,
                originating_swo_id,
                initiative_id,
                initiative_name,
                initiative_owner_agent_id,
                priority_class
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                params.assigned_agent_id,
                params.owner_agent_id,
                params.owner_agent_id,
                params.created_by_agent_id,
                params.payload,
                params.status,
                params.kind,
                params.source,
                params.work_order_title,
                params.work_order_outcome,
                params.work_order_constraints,
                params.requested_owner_agent_id,
                params.requested_assignee_agent_id,
                params.routing_policy,
                params.parent_swo_id,
                params.originating_swo_id,
                params.initiative_id,
                params.initiative_name,
                params.initiative_owner_agent_id,
                params.priority_class.or(Some("CORE")),
            ],
        ).map_err(KernelError::Database)?;

        Ok(conn.last_insert_rowid())
    }

    pub fn create_work_order(
        &self,
        assigned_agent_id: &str,
        created_by_agent_id: &str,
        title: &str,
        outcome: &str,
        constraints: Option<&str>,
        priority_class: Option<&str>,
        requested_owner_agent_id: Option<&str>,
        parent_swo_id: Option<i64>,
        initiative_id: Option<&str>,
    ) -> Result<i64> {
        let payload = if let Some(constraints) =
            constraints.filter(|value| !value.trim().is_empty())
        {
            format!(
                "WORK ORDER\nTitle: {title}\nRequested outcome: {outcome}\nConstraints: {constraints}"
            )
        } else {
            format!("WORK ORDER\nTitle: {title}\nRequested outcome: {outcome}")
        };

        self.create_swo_with_metadata(CreateSwoParams {
            assigned_agent_id,
            owner_agent_id: assigned_agent_id,
            created_by_agent_id,
            payload: &payload,
            status: "PENDING",
            parent_swo_id,
            kind: "WORK_ORDER",
            source: "WORK_ORDER",
            work_order_title: Some(title),
            work_order_outcome: Some(outcome),
            work_order_constraints: constraints,
            requested_owner_agent_id,
            requested_assignee_agent_id: None,
            routing_policy: "NONE",
            originating_swo_id: None,
            initiative_id,
            initiative_name: None,
            initiative_owner_agent_id: None,
            priority_class,
        })
    }

    pub fn update_swo_status(&self, id: i64, status: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE active_swos SET status = ?1 WHERE id = ?2",
            params![status, id],
        )
        .map_err(KernelError::Database)?;

        Ok(())
    }

    pub fn increment_swo_retry_count(&self, id: i64) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE active_swos SET retry_count = retry_count + 1 WHERE id = ?1",
            params![id],
        )
        .map_err(KernelError::Database)?;
        conn.query_row(
            "SELECT retry_count FROM active_swos WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(KernelError::Database)
    }

    // ── CHA-411: Revision ceiling escalation helpers ──────────────────────────

    /// Return the parent_swo_id for a given SWO, if any.
    pub fn get_swo_parent_id(&self, swo_id: i64) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT parent_swo_id FROM active_swos WHERE id = ?1",
            params![swo_id],
            |row| row.get(0),
        )
        .optional()
        .map(|opt| opt.flatten())
        .map_err(KernelError::Database)
    }

    /// Return the assigned_agent_id for a given SWO, if it exists.
    pub fn get_swo_assigned_agent_id(&self, swo_id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT assigned_agent_id FROM active_swos WHERE id = ?1",
            params![swo_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(KernelError::Database)
    }

    // ── CHA-411: Revision ceiling escalation ──────────────────────────────────

    /// Record a structured escalation when a manager's revision loop exhausts
    /// its retry budget. Returns the new escalation's UUID string.
    ///
    /// This is called by the orchestrator BEFORE marking the child SWO FAILED
    /// so parent managers can detect stuck children on their next triage turn
    /// via `list_recent_escalations_for_agent`.
    pub fn record_escalation(
        &self,
        swo_id: i64,
        child_agent_id: &str,
        parent_swo_id: Option<i64>,
        parent_agent_id: Option<&str>,
        attempts: i64,
        reasoning: &str,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO escalations (id, swo_id, child_agent_id, parent_swo_id, parent_agent_id, attempts, reasoning, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, swo_id, child_agent_id, parent_swo_id, parent_agent_id, attempts, reasoning, now],
        )
        .map_err(KernelError::Database)?;
        Ok(id)
    }

    /// List recent escalations visible to a parent manager, newest first.
    ///
    /// Returns escalations where `parent_agent_id` matches, up to `limit` rows.
    /// Parent managers call this on their triage turn to detect if any of their
    /// delegated children have exhausted their revision budget.
    pub fn list_recent_escalations_for_agent(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<EscalationRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, swo_id, child_agent_id, parent_swo_id, parent_agent_id, attempts, reasoning, created_at
                 FROM escalations
                 WHERE parent_agent_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![agent_id, limit.clamp(1, 500) as i64], |row| {
                Ok(EscalationRecord {
                    id: row.get(0)?,
                    swo_id: row.get(1)?,
                    child_agent_id: row.get(2)?,
                    parent_swo_id: row.get(3)?,
                    parent_agent_id: row.get(4)?,
                    attempts: row.get(5)?,
                    reasoning: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(KernelError::Database)?);
        }
        Ok(result)
    }

    // ─────────────────────────────────────────────────────────────────────────

    /// Cancel all non-terminal descendant SWOs (both PENDING and IN_PROGRESS)
    /// when a parent SWO reaches terminal resolution.
    pub fn cancel_active_descendant_swos(&self, root_swo_id: i64) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "
            WITH RECURSIVE descendants(id) AS (
                SELECT id FROM active_swos WHERE parent_swo_id = ?1
                UNION ALL
                SELECT child.id
                FROM active_swos child
                JOIN descendants d ON child.parent_swo_id = d.id
            )
            UPDATE active_swos
            SET status = 'CANCELLED'
            WHERE id IN (SELECT id FROM descendants)
              AND status IN ('PENDING', 'IN_PROGRESS')
            ",
            params![root_swo_id],
        )
        .map_err(KernelError::Database)
    }

    pub fn get_descendant_swo_ids(&self, root_swo_id: i64) -> Result<Vec<i64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                WITH RECURSIVE descendants(id) AS (
                    SELECT id FROM active_swos WHERE parent_swo_id = ?1
                    UNION ALL
                    SELECT child.id
                    FROM active_swos child
                    JOIN descendants d ON child.parent_swo_id = d.id
                )
                SELECT id FROM descendants
                ",
            )
            .map_err(KernelError::Database)?;
        let ids = stmt
            .query_map(params![root_swo_id], |row| row.get::<_, i64>(0))
            .map_err(KernelError::Database)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(KernelError::Database)?;
        Ok(ids)
    }

    pub fn swo_lineage_agent_ids(&self, swo_id: i64) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                WITH RECURSIVE lineage(id, assigned_agent_id, parent_swo_id) AS (
                    SELECT id, assigned_agent_id, parent_swo_id
                    FROM active_swos
                    WHERE id = ?1
                    UNION ALL
                    SELECT s.id, s.assigned_agent_id, s.parent_swo_id
                    FROM active_swos s
                    JOIN lineage l ON s.id = l.parent_swo_id
                )
                SELECT assigned_agent_id FROM lineage
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![swo_id], |row| row.get::<_, String>(0))
            .map_err(KernelError::Database)?;
        let mut agent_ids = Vec::new();
        for row in rows {
            agent_ids.push(row.map_err(KernelError::Database)?);
        }
        Ok(agent_ids)
    }

    pub fn record_swo_result(
        &self,
        swo_id: i64,
        producer_agent_id: &str,
        result_json: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO swo_results (swo_id, producer_agent_id, result_json)
             VALUES (?1, ?2, ?3)",
            params![swo_id, producer_agent_id, result_json],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn record_manager_review(
        &self,
        swo_id: i64,
        reviewer_agent_id: &str,
        action: &str,
        reasoning: &str,
        final_response: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO manager_reviews (swo_id, reviewer_agent_id, action, reasoning, final_response)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![swo_id, reviewer_agent_id, action, reasoning, final_response],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn record_outbox_artifact(
        &self,
        swo_id: i64,
        agent_id: &str,
        absolute_path: &str,
        filename: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO outbox_artifacts (swo_id, agent_id, absolute_path, filename)
             VALUES (?1, ?2, ?3, ?4)",
            params![swo_id, agent_id, absolute_path, filename],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_outbox_artifact(&self, artifact_id: i64) -> Result<Option<OutboxArtifactRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "
            SELECT
                a.id,
                a.swo_id,
                a.agent_id,
                COALESCE(agent.name || ' (' || agent.role || ')', a.agent_id),
                s.initiative_id,
                s.initiative_name,
                s.parent_swo_id,
                s.work_order_title,
                s.work_order_outcome,
                s.status,
                a.absolute_path,
                a.filename,
                a.created_at
            FROM outbox_artifacts a
            LEFT JOIN agents agent ON agent.id = a.agent_id
            LEFT JOIN active_swos s ON s.id = a.swo_id
            WHERE a.id = ?1
            ",
            params![artifact_id],
            Self::outbox_artifact_from_row,
        )
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn list_outbox_artifacts(
        &self,
        filters: OutboxArtifactListFilters,
    ) -> Result<Vec<OutboxArtifactRecord>> {
        let safe_limit = filters.limit.clamp(1, 200) as i64;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    a.id,
                    a.swo_id,
                    a.agent_id,
                    COALESCE(agent.name || ' (' || agent.role || ')', a.agent_id),
                    s.initiative_id,
                    s.initiative_name,
                    s.parent_swo_id,
                    s.work_order_title,
                    s.work_order_outcome,
                    s.status,
                    a.absolute_path,
                    a.filename,
                    a.created_at
                FROM outbox_artifacts a
                LEFT JOIN agents agent ON agent.id = a.agent_id
                LEFT JOIN active_swos s ON s.id = a.swo_id
                WHERE (?1 IS NULL OR a.agent_id = ?1)
                  AND (?2 IS NULL OR a.swo_id = ?2)
                  AND (
                    ?3 IS NULL
                    OR a.filename LIKE '%' || ?3 || '%'
                    OR a.absolute_path LIKE '%' || ?3 || '%'
                  )
                ORDER BY a.id DESC
                LIMIT ?4
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(
                params![
                    filters.agent_id,
                    filters.swo_id,
                    filters.query.as_deref(),
                    safe_limit,
                ],
                Self::outbox_artifact_from_row,
            )
            .map_err(KernelError::Database)?;
        let mut artifacts = Vec::new();
        for row in rows {
            artifacts.push(row.map_err(KernelError::Database)?);
        }
        Ok(artifacts)
    }

    pub fn get_inbox_item(&self, item_id: &str) -> Result<Option<InboxItemRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "
            SELECT
                id,
                kind,
                status,
                priority,
                title,
                summary,
                project_id,
                project_name,
                swo_id,
                artifact_id,
                agent_id,
                created_at,
                updated_at,
                resolved_at,
                resolution
            FROM inbox_items
            WHERE id = ?1
            ",
            params![item_id],
            Self::inbox_item_from_row,
        )
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn list_inbox_items(
        &self,
        include_resolved: bool,
        limit: usize,
    ) -> Result<Vec<InboxItemRecord>> {
        let conn = self.conn.lock().unwrap();
        let safe_limit = limit.clamp(1, 500) as i64;
        let include_resolved_flag = if include_resolved { 1_i64 } else { 0_i64 };
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    id,
                    kind,
                    status,
                    priority,
                    title,
                    summary,
                    project_id,
                    project_name,
                    swo_id,
                    artifact_id,
                    agent_id,
                    created_at,
                    updated_at,
                    resolved_at,
                    resolution
                FROM inbox_items
                WHERE (?1 = 1 OR status != 'RESOLVED')
                ORDER BY
                    CASE status
                        WHEN 'OPEN' THEN 0
                        WHEN 'ACKNOWLEDGED' THEN 1
                        ELSE 2
                    END,
                    updated_at DESC,
                    id DESC
                LIMIT ?2
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(
                params![include_resolved_flag, safe_limit],
                Self::inbox_item_from_row,
            )
            .map_err(KernelError::Database)?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row.map_err(KernelError::Database)?);
        }
        Ok(items)
    }

    pub fn upsert_inbox_item(&self, params: UpsertInboxItemParams<'_>) -> Result<InboxItemRecord> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "
            INSERT INTO inbox_items (
                id,
                kind,
                status,
                priority,
                title,
                summary,
                project_id,
                project_name,
                swo_id,
                artifact_id,
                agent_id
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                status = CASE
                    WHEN inbox_items.status = 'ACKNOWLEDGED' AND excluded.status = 'OPEN'
                        THEN 'ACKNOWLEDGED'
                    ELSE excluded.status
                END,
                priority = excluded.priority,
                title = excluded.title,
                summary = excluded.summary,
                project_id = excluded.project_id,
                project_name = excluded.project_name,
                swo_id = excluded.swo_id,
                artifact_id = excluded.artifact_id,
                agent_id = excluded.agent_id,
                updated_at = CURRENT_TIMESTAMP,
                resolved_at = NULL,
                resolution = NULL
            ",
            params![
                params.id,
                params.kind,
                params.status,
                params.priority,
                params.title,
                params.summary,
                params.project_id,
                params.project_name,
                params.swo_id,
                params.artifact_id,
                params.agent_id,
            ],
        )
        .map_err(KernelError::Database)?;
        drop(conn);
        self.get_inbox_item(params.id)?.ok_or_else(|| {
            KernelError::Internal(format!(
                "Inbox item {} could not be reloaded after upsert",
                params.id
            ))
        })
    }

    pub fn acknowledge_inbox_item(&self, item_id: &str) -> Result<Option<InboxItemRecord>> {
        let conn = self.conn.lock().unwrap();
        let changed = conn
            .execute(
                "
            UPDATE inbox_items
            SET status = 'ACKNOWLEDGED',
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
              AND status = 'OPEN'
            ",
                params![item_id],
            )
            .map_err(KernelError::Database)?;
        drop(conn);
        if changed == 0 {
            return Ok(None);
        }
        self.get_inbox_item(item_id)
    }

    pub fn resolve_inbox_item(
        &self,
        item_id: &str,
        resolution: Option<&str>,
    ) -> Result<Option<InboxItemRecord>> {
        let conn = self.conn.lock().unwrap();
        let changed = conn
            .execute(
                "
            UPDATE inbox_items
            SET status = 'RESOLVED',
                updated_at = CURRENT_TIMESTAMP,
                resolved_at = CURRENT_TIMESTAMP,
                resolution = ?2
            WHERE id = ?1
              AND status != 'RESOLVED'
            ",
                params![item_id, resolution],
            )
            .map_err(KernelError::Database)?;
        drop(conn);
        if changed == 0 {
            return Ok(None);
        }
        self.get_inbox_item(item_id)
    }

    pub fn inbox_attention_summary(&self) -> Result<InboxAttentionSummaryRecord> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "
            SELECT
                COALESCE(SUM(CASE WHEN status = 'OPEN' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'OPEN' AND kind = 'approval' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'OPEN' AND kind = 'deliverable' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'OPEN' AND kind = 'blocked' THEN 1 ELSE 0 END), 0)
            FROM inbox_items
            ",
            [],
            |row| {
                Ok(InboxAttentionSummaryRecord {
                    open_inbox_items: row.get(0)?,
                    open_approval_items: row.get(1)?,
                    open_deliverable_items: row.get(2)?,
                    open_blocked_items: row.get(3)?,
                })
            },
        )
        .map_err(KernelError::Database)
    }

    pub fn record_attachment(
        &self,
        id: &str,
        source_kind: &str,
        display_name: &str,
        original_path: &str,
        content_type: &str,
        size_bytes: i64,
        originating_swo_id: Option<i64>,
        originating_artifact_id: Option<i64>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO attachments (
                id,
                source_kind,
                display_name,
                original_path,
                content_type,
                size_bytes,
                originating_swo_id,
                originating_artifact_id
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                source_kind,
                display_name,
                original_path,
                content_type,
                size_bytes,
                originating_swo_id,
                originating_artifact_id,
            ],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn link_message_attachment(
        &self,
        agent_id: &str,
        message_ref: &str,
        attachment_id: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO message_attachments (agent_id, message_ref, attachment_id)
             VALUES (?1, ?2, ?3)",
            params![agent_id, message_ref, attachment_id],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn link_swo_attachment(
        &self,
        swo_id: i64,
        attachment_id: &str,
        inbox_path: Option<&str>,
        delivery_status: &str,
        delivery_error: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO swo_attachments (swo_id, attachment_id, inbox_path, delivery_status, delivery_error)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(swo_id, attachment_id) DO UPDATE SET
                inbox_path = excluded.inbox_path,
                delivery_status = excluded.delivery_status,
                delivery_error = excluded.delivery_error",
            params![swo_id, attachment_id, inbox_path, delivery_status, delivery_error],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn count_outbox_artifacts_for_swo(&self, swo_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM outbox_artifacts WHERE swo_id = ?1",
            params![swo_id],
            |row| row.get(0),
        )
        .map_err(KernelError::Database)
    }

    pub fn get_artifacts_for_swo(&self, swo_id: i64) -> Result<Vec<OutboxArtifactRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    a.id,
                    a.swo_id,
                    a.agent_id,
                    COALESCE(agent.name || ' (' || agent.role || ')', a.agent_id),
                    s.initiative_id,
                    s.initiative_name,
                    s.parent_swo_id,
                    s.work_order_title,
                    s.work_order_outcome,
                    s.status,
                    a.absolute_path,
                    a.filename,
                    a.created_at
                FROM outbox_artifacts a
                LEFT JOIN agents agent ON agent.id = a.agent_id
                LEFT JOIN active_swos s ON s.id = a.swo_id
                WHERE a.swo_id = ?1
                ORDER BY a.id ASC
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![swo_id], Self::outbox_artifact_from_row)
            .map_err(KernelError::Database)?;
        let mut artifacts = Vec::new();
        for row in rows {
            artifacts.push(row.map_err(KernelError::Database)?);
        }
        Ok(artifacts)
    }

    pub fn record_worker_run_start(
        &self,
        run_id: &str,
        swo_id: Option<i64>,
        agent_id: &str,
        backend: &str,
        mode: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO worker_runs (run_id, swo_id, agent_id, backend, mode, status)
             VALUES (?1, ?2, ?3, ?4, ?5, 'IN_PROGRESS')",
            params![run_id, swo_id, agent_id, backend, mode],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn record_worker_run_finish(
        &self,
        run_id: &str,
        status: &str,
        artifact_count: i64,
        structured_output_present: bool,
        blocked_reason: Option<&str>,
        failure_reason: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE worker_runs
             SET status = ?2,
                 finished_at = CURRENT_TIMESTAMP,
                 artifact_count = ?3,
                 structured_output_present = ?4,
                 blocked_reason = ?5,
                 failure_reason = ?6
             WHERE run_id = ?1",
            params![
                run_id,
                status,
                artifact_count,
                if structured_output_present { 1 } else { 0 },
                blocked_reason,
                failure_reason
            ],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn record_token_usage(
        &self,
        run_id: &str,
        swo_id: Option<i64>,
        agent_id: &str,
        provider: &str,
        model: &str,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
        requests: i64,
        cost_usd: Option<f64>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO token_usage (run_id, swo_id, agent_id, provider, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, requests, cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![run_id, swo_id, agent_id, provider, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, requests, cost_usd],
        )
        .map_err(KernelError::Database)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_token_usage_for_swo(&self, swo_id: i64) -> Result<Vec<TokenUsageRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, run_id, swo_id, agent_id, provider, model,
                        input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                        requests, cost_usd, created_at
                 FROM token_usage WHERE swo_id = ?1 ORDER BY id ASC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![swo_id], |row| {
                Ok(TokenUsageRecord {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    swo_id: row.get(2)?,
                    agent_id: row.get(3)?,
                    provider: row.get(4)?,
                    model: row.get(5)?,
                    input_tokens: row.get(6)?,
                    output_tokens: row.get(7)?,
                    cache_read_tokens: row.get(8)?,
                    cache_write_tokens: row.get(9)?,
                    requests: row.get(10)?,
                    cost_usd: row.get(11)?,
                    created_at: row.get(12)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn get_token_usage_for_agent(&self, agent_id: &str) -> Result<Vec<TokenUsageRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, run_id, swo_id, agent_id, provider, model,
                        input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                        requests, cost_usd, created_at
                 FROM token_usage WHERE agent_id = ?1 ORDER BY id DESC LIMIT 200",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok(TokenUsageRecord {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    swo_id: row.get(2)?,
                    agent_id: row.get(3)?,
                    provider: row.get(4)?,
                    model: row.get(5)?,
                    input_tokens: row.get(6)?,
                    output_tokens: row.get(7)?,
                    cache_read_tokens: row.get(8)?,
                    cache_write_tokens: row.get(9)?,
                    requests: row.get(10)?,
                    cost_usd: row.get(11)?,
                    created_at: row.get(12)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn get_token_usage_totals(&self) -> Result<Vec<AgentTokenTotals>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT agent_id,
                        SUM(input_tokens) AS input_tokens,
                        SUM(output_tokens) AS output_tokens,
                        SUM(cache_read_tokens) AS cache_read_tokens,
                        SUM(input_tokens + output_tokens + cache_read_tokens) AS total_tokens,
                        SUM(cost_usd) AS estimated_cost_usd,
                        COUNT(*) AS run_count
                 FROM token_usage
                 GROUP BY agent_id
                 ORDER BY total_tokens DESC",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AgentTokenTotals {
                    agent_id: row.get(0)?,
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    cache_read_tokens: row.get(3)?,
                    total_tokens: row.get(4)?,
                    estimated_cost_usd: row.get(5)?,
                    run_count: row.get(6)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(KernelError::Database)?);
        }
        Ok(records)
    }

    pub fn append_memory_interaction(
        &self,
        agent_id: &str,
        role: &str,
        content: &str,
        swo_id: Option<i64>,
    ) -> Result<()> {
        self.append_memory_interaction_with_meta(
            agent_id, role, content, swo_id, "legacy", None, "message",
        )
    }

    pub fn append_memory_interaction_with_meta(
        &self,
        agent_id: &str,
        role: &str,
        content: &str,
        swo_id: Option<i64>,
        mode: &str,
        run_id: Option<&str>,
        interaction_kind: &str,
    ) -> Result<()> {
        let db_path = self.agent_memory_db_path(agent_id)?;
        let conn = Connection::open(&db_path).map_err(KernelError::Database)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
            .map_err(KernelError::Database)?;
        Self::ensure_agent_memory_schema(&conn)?;
        conn.execute(
            "INSERT INTO interactions (role, content, swo_id, mode, run_id, interaction_kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![role, content, swo_id, mode, run_id, interaction_kind],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn tag_latest_memory_interactions(
        &self,
        agent_id: &str,
        swo_id: i64,
        limit: usize,
    ) -> Result<()> {
        let db_path = self.agent_memory_db_path(agent_id)?;
        let conn = Connection::open(&db_path).map_err(KernelError::Database)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
            .map_err(KernelError::Database)?;
        Self::ensure_agent_memory_schema(&conn)?;
        conn.execute(
            &format!(
                "UPDATE interactions
                 SET swo_id = ?1
                 WHERE id IN (
                    SELECT id FROM interactions
                    WHERE swo_id IS NULL
                    ORDER BY id DESC
                    LIMIT {}
                 )",
                limit.max(1)
            ),
            params![swo_id],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn insert_decision_log_entry(
        &self,
        agent_id: &str,
        entry: &DecisionLogEntryRecord,
    ) -> Result<DecisionLogEntryRecord> {
        self.get_agent(agent_id)?;
        let db_path = self.agent_memory_db_path(agent_id)?;
        let conn = Connection::open(&db_path).map_err(KernelError::Database)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
            .map_err(KernelError::Database)?;
        Self::ensure_agent_memory_schema(&conn)?;

        let entry_id = if entry.entry_id.trim().is_empty() {
            Uuid::new_v4().to_string()
        } else {
            entry.entry_id.trim().to_string()
        };
        let created_at = (!entry.created_at.trim().is_empty()).then_some(entry.created_at.as_str());

        conn.execute(
            "INSERT INTO decision_log (
                entry_id,
                mode,
                summary,
                rationale,
                outcome,
                confidence,
                self_note,
                linked_swo_id,
                linked_run_id,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, COALESCE(?10, CURRENT_TIMESTAMP))",
            params![
                entry_id,
                entry.mode.trim(),
                entry.summary.trim(),
                entry.rationale.trim(),
                entry.outcome.trim(),
                entry.confidence,
                entry.self_note.as_deref(),
                entry.linked_swo_id,
                entry.linked_run_id.as_deref(),
                created_at,
            ],
        )
        .map_err(KernelError::Database)?;

        conn.query_row(
            "
            SELECT
                entry_id,
                mode,
                summary,
                rationale,
                outcome,
                confidence,
                self_note,
                linked_swo_id,
                linked_run_id,
                created_at
            FROM decision_log
            WHERE entry_id = ?1
            ",
            params![entry_id],
            |row| {
                Ok(DecisionLogEntryRecord {
                    entry_id: row.get(0)?,
                    agent_id: agent_id.to_string(),
                    mode: row.get(1)?,
                    summary: row.get(2)?,
                    rationale: row.get(3)?,
                    outcome: row.get(4)?,
                    confidence: row.get(5)?,
                    self_note: row.get(6)?,
                    linked_swo_id: row.get(7)?,
                    linked_run_id: row.get(8)?,
                    created_at: row.get(9)?,
                })
            },
        )
        .map_err(KernelError::Database)
    }

    pub fn list_decision_log(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<DecisionLogEntryRecord>> {
        self.get_agent(agent_id)?;
        let db_path = self.agent_memory_db_path(agent_id)?;
        if !db_path.exists() {
            return Ok(Vec::new());
        }
        let conn = Connection::open(&db_path).map_err(KernelError::Database)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
            .map_err(KernelError::Database)?;
        Self::ensure_agent_memory_schema(&conn)?;
        Self::load_decision_log_entries(&conn, agent_id, limit)
    }

    pub fn prune_decision_log(&self, agent_id: &str, max_entries: usize) -> Result<usize> {
        self.get_agent(agent_id)?;
        let db_path = self.agent_memory_db_path(agent_id)?;
        let conn = Connection::open(&db_path).map_err(KernelError::Database)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
            .map_err(KernelError::Database)?;
        Self::ensure_agent_memory_schema(&conn)?;

        let pruned = if max_entries == 0 {
            conn.execute("DELETE FROM decision_log", [])
                .map_err(KernelError::Database)?
        } else {
            conn.execute(
                "
                DELETE FROM decision_log
                WHERE entry_id IN (
                    SELECT entry_id
                    FROM decision_log
                    ORDER BY created_at DESC, entry_id DESC
                    LIMIT -1 OFFSET ?1
                )
                ",
                params![max_entries as i64],
            )
            .map_err(KernelError::Database)?
        };
        Ok(pruned)
    }

    pub fn decision_log_retention_limit(&self) -> Result<usize> {
        self.decision_log_max_entries()
    }

    pub fn list_recent_decision_log_entries(
        &self,
        limit: usize,
    ) -> Result<Vec<DecisionLogEntryRecord>> {
        let mut entries = Vec::new();
        for agent in self.list_agents()? {
            let db_path = self.agent_memory_db_path(&agent.id)?;
            if !db_path.exists() {
                continue;
            }
            let conn = Connection::open(&db_path).map_err(KernelError::Database)?;
            conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
                .map_err(KernelError::Database)?;
            Self::ensure_agent_memory_schema(&conn)?;
            entries.extend(Self::load_decision_log_entries(
                &conn,
                &agent.id,
                limit.clamp(1, 100),
            )?);
        }
        entries.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.entry_id.cmp(&left.entry_id))
        });
        entries.truncate(limit.clamp(1, 100));
        Ok(entries)
    }

    pub fn set_agent_cron_last_fired_now(&self, agent_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_cron_last_fired (agent_id, last_fired_at)
             VALUES (?1, CURRENT_TIMESTAMP)
             ON CONFLICT(agent_id) DO UPDATE SET last_fired_at = CURRENT_TIMESTAMP",
            params![agent_id],
        )
        .map_err(KernelError::Database)?;
        conn.query_row(
            "SELECT last_fired_at FROM agent_cron_last_fired WHERE agent_id = ?1",
            params![agent_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn get_agent_cron_last_fired(&self, agent_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT last_fired_at FROM agent_cron_last_fired WHERE agent_id = ?1",
            params![agent_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(KernelError::Database)
    }

    pub fn list_agent_cron_last_fired_unix(&self) -> Result<HashMap<String, i64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "
                SELECT agent_id, CAST(strftime('%s', last_fired_at) AS INTEGER)
                FROM agent_cron_last_fired
                WHERE last_fired_at IS NOT NULL
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map([], |row| {
                let agent_id: String = row.get(0)?;
                let last_fired_unix: Option<i64> = row.get(1)?;
                Ok((agent_id, last_fired_unix))
            })
            .map_err(KernelError::Database)?;

        let mut values = HashMap::new();
        for row in rows {
            let (agent_id, last_fired_unix) = row.map_err(KernelError::Database)?;
            if let Some(last_fired_unix) = last_fired_unix {
                values.insert(agent_id, last_fired_unix);
            }
        }
        Ok(values)
    }

    fn load_active_swo_record(conn: &Connection, swo_id: i64) -> Result<Option<ActiveSwoRecord>> {
        conn.query_row(
            &Self::active_swo_select_sql("WHERE s.id = ?1"),
            params![swo_id],
            Self::active_swo_from_row,
        )
        .optional()
        .map_err(KernelError::Database)
    }

    fn load_agent_display_name(conn: &Connection, agent_id: &str) -> Result<String> {
        conn.query_row(
            "SELECT name, role FROM agents WHERE id = ?1",
            params![agent_id],
            |row| {
                let name: String = row.get(0)?;
                let role: String = row.get(1)?;
                Ok(format!("{} ({})", name, role))
            },
        )
        .optional()
        .map_err(KernelError::Database)?
        .ok_or_else(|| KernelError::AgentNotFound(agent_id.to_string()))
    }

    fn agent_summary_from_identity(agent: &AgentIdentity) -> AgentSummaryRecord {
        AgentSummaryRecord {
            id: agent.id.clone(),
            name: agent.name.clone(),
            role: agent.role.clone(),
        }
    }

    fn normalize_presence(
        raw_status: Option<&str>,
        last_seen_unix_ms: Option<i64>,
        now_ms: i64,
    ) -> (String, Option<i64>) {
        let Some(last_seen_unix_ms) = last_seen_unix_ms else {
            return ("OFFLINE".to_string(), None);
        };
        let age_ms = (now_ms - last_seen_unix_ms).max(0);
        if age_ms > OFFLINE_AFTER_MS {
            return ("OFFLINE".to_string(), Some(age_ms));
        }

        match raw_status.unwrap_or("OFFLINE") {
            "COMPUTING" if age_ms <= COMPUTING_FRESH_MS => ("COMPUTING".to_string(), Some(age_ms)),
            "READY" if age_ms <= READY_FRESH_MS => ("READY".to_string(), Some(age_ms)),
            _ => ("STALE".to_string(), Some(age_ms)),
        }
    }

    fn load_reviews_for_swo(conn: &Connection, swo_id: i64) -> Result<Vec<ManagerReviewRecord>> {
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    r.id,
                    r.swo_id,
                    r.reviewer_agent_id,
                    COALESCE(a.name || ' (' || a.role || ')', r.reviewer_agent_id),
                    r.action,
                    r.reasoning,
                    r.final_response,
                    r.created_at
                FROM manager_reviews r
                LEFT JOIN agents a ON a.id = r.reviewer_agent_id
                WHERE r.swo_id = ?1
                ORDER BY r.id ASC
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![swo_id], |row| {
                Ok(ManagerReviewRecord {
                    id: row.get(0)?,
                    swo_id: row.get(1)?,
                    reviewer_agent_id: row.get(2)?,
                    reviewer_agent_name: row.get(3)?,
                    action: row.get(4)?,
                    reasoning: row.get(5)?,
                    final_response: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut reviews = Vec::new();
        for row in rows {
            reviews.push(row.map_err(KernelError::Database)?);
        }
        Ok(reviews)
    }

    fn load_child_swos(conn: &Connection, swo_id: i64) -> Result<Vec<ActiveSwoRecord>> {
        let mut child_stmt = conn
            .prepare("SELECT id FROM active_swos WHERE parent_swo_id = ?1 ORDER BY id ASC")
            .map_err(KernelError::Database)?;
        let child_ids = child_stmt
            .query_map(params![swo_id], |row| row.get::<_, i64>(0))
            .map_err(KernelError::Database)?;

        let mut child_swos = Vec::new();
        for row in child_ids {
            let child_id = row.map_err(KernelError::Database)?;
            if let Some(child) = Self::load_active_swo_record(conn, child_id)? {
                child_swos.push(child);
            }
        }
        Ok(child_swos)
    }

    fn load_linked_swos(conn: &Connection, swo_id: i64) -> Result<Vec<ActiveSwoRecord>> {
        let mut linked_stmt = conn
            .prepare("SELECT id FROM active_swos WHERE originating_swo_id = ?1 ORDER BY id ASC")
            .map_err(KernelError::Database)?;
        let linked_ids = linked_stmt
            .query_map(params![swo_id], |row| row.get::<_, i64>(0))
            .map_err(KernelError::Database)?;

        let mut linked_swos = Vec::new();
        for row in linked_ids {
            let linked_id = row.map_err(KernelError::Database)?;
            if let Some(linked) = Self::load_active_swo_record(conn, linked_id)? {
                linked_swos.push(linked);
            }
        }
        Ok(linked_swos)
    }

    fn load_worker_runs_for_swo(conn: &Connection, swo_id: i64) -> Result<Vec<WorkerRunRecord>> {
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    wr.id,
                    wr.run_id,
                    wr.swo_id,
                    wr.agent_id,
                    COALESCE(a.name || ' (' || a.role || ')', wr.agent_id),
                    wr.backend,
                    wr.mode,
                    wr.status,
                    wr.started_at,
                    wr.finished_at,
                    wr.artifact_count,
                    wr.structured_output_present,
                    wr.blocked_reason,
                    wr.failure_reason
                FROM worker_runs wr
                LEFT JOIN agents a ON a.id = wr.agent_id
                WHERE wr.swo_id = ?1
                ORDER BY wr.id ASC
                ",
            )
            .map_err(KernelError::Database)?;

        let rows = stmt
            .query_map(params![swo_id], |row| {
                Ok(WorkerRunRecord {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    swo_id: row.get(2)?,
                    agent_id: row.get(3)?,
                    agent_name: row.get(4)?,
                    backend: row.get(5)?,
                    mode: row.get(6)?,
                    status: row.get(7)?,
                    started_at: row.get(8)?,
                    finished_at: row.get(9)?,
                    artifact_count: row.get(10)?,
                    structured_output_present: row.get::<_, i64>(11)? != 0,
                    blocked_reason: row.get(12)?,
                    failure_reason: row.get(13)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut worker_runs = Vec::new();
        for row in rows {
            worker_runs.push(row.map_err(KernelError::Database)?);
        }
        Ok(worker_runs)
    }

    fn compute_delegation_debug(
        swo: &ActiveSwoRecord,
        reviews: &[ManagerReviewRecord],
        child_swos: &[ActiveSwoRecord],
    ) -> DelegationDebugRecord {
        let mut actual_child_assignees = child_swos
            .iter()
            .map(|child| child.assigned_agent_name.clone())
            .collect::<Vec<_>>();
        actual_child_assignees.sort();
        actual_child_assignees.dedup();

        let review_status = reviews
            .last()
            .map(|review| review.action.clone())
            .unwrap_or_else(|| "NO_REVIEW".to_string());

        let requested_id = swo.requested_assignee_agent_id.clone();
        let requested_name = swo
            .requested_assignee_agent_name
            .clone()
            .or_else(|| requested_id.clone());
        let has_matching_child = requested_id.as_ref().is_some_and(|id| {
            child_swos
                .iter()
                .any(|child| child.assigned_agent_id == *id)
        });

        let mut mismatch_flags = Vec::new();
        if requested_id.is_some() && child_swos.is_empty() {
            mismatch_flags.push("no_child_swo".to_string());
        }
        if requested_id.is_some() && !has_matching_child {
            mismatch_flags.push("requested_not_delegated".to_string());
        }
        if let Some(requested_name) = requested_name.clone() {
            if !has_matching_child {
                let mut needles = vec![requested_name.to_lowercase()];
                if let Some(base_name) = requested_name.split(" (").next() {
                    needles.push(base_name.to_lowercase());
                }
                let review_claims_assignment = reviews.iter().any(|review| {
                    let mut content = review.reasoning.to_lowercase();
                    if let Some(final_response) = &review.final_response {
                        content.push('\n');
                        content.push_str(&final_response.to_lowercase());
                    }
                    needles.iter().any(|needle| content.contains(needle))
                });
                if review_claims_assignment {
                    mismatch_flags.push("review_claim_without_child".to_string());
                }
            }
        }

        DelegationDebugRecord {
            requested_assignee_agent_id: requested_id,
            requested_assignee_agent_name: requested_name,
            routing_policy: swo.routing_policy.clone(),
            actual_child_assignees,
            child_swo_count: child_swos.len(),
            review_status,
            mismatch_flags,
        }
    }

    fn build_swo_summary(conn: &Connection, swo: ActiveSwoRecord) -> Result<AgentSwoSummaryRecord> {
        let child_swos = Self::load_child_swos(conn, swo.id)?;
        let reviews = Self::load_reviews_for_swo(conn, swo.id)?;
        let debug = Self::compute_delegation_debug(&swo, &reviews, &child_swos);

        Ok(AgentSwoSummaryRecord {
            swo,
            actual_child_assignees: debug.actual_child_assignees,
            child_swo_count: debug.child_swo_count,
            review_status: debug.review_status,
            mismatch_flags: debug.mismatch_flags,
        })
    }

    fn load_hire_debug_records_for_swo_ids(
        conn: &Connection,
        swo_ids: &[i64],
    ) -> Result<Vec<HireDebugRecord>> {
        let mut hires = Vec::new();
        let mut seen = HashSet::new();

        for swo_id in swo_ids {
            let mut stmt = conn
                .prepare(
                    "
                    SELECT
                        h.id,
                        h.swo_id,
                        h.manager_agent_id,
                        COALESCE(manager.name || ' (' || manager.role || ')', h.manager_agent_id),
                        h.new_agent_id,
                        COALESCE(new_agent.name || ' (' || new_agent.role || ')', h.new_agent_id),
                        h.spec_json,
                        h.created_at,
                        new_agent.parent_id,
                        COALESCE(parent.name || ' (' || parent.role || ')', new_agent.parent_id)
                    FROM agent_hires h
                    LEFT JOIN agents manager ON manager.id = h.manager_agent_id
                    LEFT JOIN agents new_agent ON new_agent.id = h.new_agent_id
                    LEFT JOIN agents parent ON parent.id = new_agent.parent_id
                    WHERE h.swo_id = ?1
                    ORDER BY h.id ASC
                    ",
                )
                .map_err(KernelError::Database)?;

            let rows = stmt
                .query_map(params![swo_id], |row| {
                    let manager_agent_id: String = row.get(2)?;
                    let actual_parent_agent_id: Option<String> = row.get(8)?;
                    Ok(HireDebugRecord {
                        id: row.get(0)?,
                        swo_id: row.get(1)?,
                        manager_agent_id: manager_agent_id.clone(),
                        manager_agent_name: row.get(3)?,
                        new_agent_id: row.get(4)?,
                        new_agent_name: row.get(5)?,
                        spec_json: row.get(6)?,
                        created_at: row.get(7)?,
                        parent_matches_manager: actual_parent_agent_id.as_deref()
                            == Some(manager_agent_id.as_str()),
                        actual_parent_agent_id,
                        actual_parent_agent_name: row.get(9)?,
                    })
                })
                .map_err(KernelError::Database)?;

            for row in rows {
                let hire = row.map_err(KernelError::Database)?;
                if seen.insert(hire.id) {
                    hires.push(hire);
                }
            }
        }

        hires.sort_by_key(|hire| hire.id);
        Ok(hires)
    }

    fn load_recent_hires_for_manager(
        conn: &Connection,
        manager_agent_id: &str,
        limit: usize,
    ) -> Result<Vec<HireDebugRecord>> {
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    h.id,
                    h.swo_id,
                    h.manager_agent_id,
                    COALESCE(manager.name || ' (' || manager.role || ')', h.manager_agent_id),
                    h.new_agent_id,
                    COALESCE(new_agent.name || ' (' || new_agent.role || ')', h.new_agent_id),
                    h.spec_json,
                    h.created_at,
                    new_agent.parent_id,
                    COALESCE(parent.name || ' (' || parent.role || ')', new_agent.parent_id)
                FROM agent_hires h
                LEFT JOIN agents manager ON manager.id = h.manager_agent_id
                LEFT JOIN agents new_agent ON new_agent.id = h.new_agent_id
                LEFT JOIN agents parent ON parent.id = new_agent.parent_id
                WHERE h.manager_agent_id = ?1
                ORDER BY h.id DESC
                LIMIT ?2
                ",
            )
            .map_err(KernelError::Database)?;

        let rows = stmt
            .query_map(
                params![manager_agent_id, limit.clamp(1, 50) as i64],
                |row| {
                    let manager_agent_id: String = row.get(2)?;
                    let actual_parent_agent_id: Option<String> = row.get(8)?;
                    Ok(HireDebugRecord {
                        id: row.get(0)?,
                        swo_id: row.get(1)?,
                        manager_agent_id: manager_agent_id.clone(),
                        manager_agent_name: row.get(3)?,
                        new_agent_id: row.get(4)?,
                        new_agent_name: row.get(5)?,
                        spec_json: row.get(6)?,
                        created_at: row.get(7)?,
                        parent_matches_manager: actual_parent_agent_id.as_deref()
                            == Some(manager_agent_id.as_str()),
                        actual_parent_agent_id,
                        actual_parent_agent_name: row.get(9)?,
                    })
                },
            )
            .map_err(KernelError::Database)?;

        let mut hires = Vec::new();
        for row in rows {
            hires.push(row.map_err(KernelError::Database)?);
        }
        Ok(hires)
    }

    fn load_agent_swo_summaries_for_field(
        conn: &Connection,
        field: &str,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<AgentSwoSummaryRecord>> {
        let column = match field {
            "assigned_agent_id" => "assigned_agent_id",
            "owner_agent_id" => "owner_agent_id",
            "created_by_agent_id" => "created_by_agent_id",
            _ => {
                return Err(KernelError::Internal(format!(
                    "Unsupported SWO field lookup: {}",
                    field
                )));
            }
        };

        let sql = format!(
            "SELECT id FROM active_swos WHERE {} = ?1 ORDER BY id DESC LIMIT ?2",
            column
        );
        let mut stmt = conn.prepare(&sql).map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![agent_id, limit.clamp(1, 50) as i64], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(KernelError::Database)?;

        let mut swos = Vec::new();
        for row in rows {
            let swo_id = row.map_err(KernelError::Database)?;
            if let Some(swo) = Self::load_active_swo_record(conn, swo_id)? {
                swos.push(Self::build_swo_summary(conn, swo)?);
            }
        }
        Ok(swos)
    }

    fn load_heartbeat_timeline(
        conn: &Connection,
        agent_id: &str,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<HeartbeatEventRecord>> {
        let mut stmt = conn
            .prepare(
                "
                SELECT run_id, status, last_seen_unix_ms, seq
                FROM agent_heartbeats
                WHERE agent_id = ?1
                ORDER BY last_seen_unix_ms DESC
                LIMIT ?2
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![agent_id, limit.clamp(1, 100) as i64], |row| {
                let last_seen_unix_ms: i64 = row.get(2)?;
                Ok(HeartbeatEventRecord {
                    run_id: row.get(0)?,
                    status: row.get(1)?,
                    last_seen_unix_ms,
                    last_seen_age_ms: (now_ms - last_seen_unix_ms).max(0),
                    seq: row.get(3)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(KernelError::Database)?);
        }
        Ok(events)
    }

    fn load_decision_log_entries(
        conn: &Connection,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<DecisionLogEntryRecord>> {
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    entry_id,
                    mode,
                    summary,
                    rationale,
                    outcome,
                    confidence,
                    self_note,
                    linked_swo_id,
                    linked_run_id,
                    created_at
                FROM decision_log
                ORDER BY created_at DESC, entry_id DESC
                LIMIT ?1
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 500) as i64], |row| {
                Ok(DecisionLogEntryRecord {
                    entry_id: row.get(0)?,
                    agent_id: agent_id.to_string(),
                    mode: row.get(1)?,
                    summary: row.get(2)?,
                    rationale: row.get(3)?,
                    outcome: row.get(4)?,
                    confidence: row.get(5)?,
                    self_note: row.get(6)?,
                    linked_swo_id: row.get(7)?,
                    linked_run_id: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.map_err(KernelError::Database)?);
        }
        Ok(entries)
    }

    fn load_recent_agent_interactions(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionExcerpt>> {
        let db_path = self.agent_memory_db_path(agent_id)?;
        if !db_path.exists() {
            return Ok(Vec::new());
        }

        let conn = Connection::open(&db_path).map_err(KernelError::Database)?;
        Self::ensure_agent_memory_schema(&conn)?;
        let agent_name = {
            let registry_conn = self.conn.lock().unwrap();
            Self::load_agent_display_name(&registry_conn, agent_id)?
        };
        let mut stmt = conn
            .prepare(
                "
                SELECT id, timestamp, role, COALESCE(mode, 'legacy'), COALESCE(interaction_kind, 'message'), content
                FROM interactions
                ORDER BY id DESC
                LIMIT ?1
                ",
            )
            .map_err(KernelError::Database)?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 100) as i64], |row| {
                Ok(InteractionExcerpt {
                    agent_id: agent_id.to_string(),
                    agent_name: agent_name.clone(),
                    interaction_id: row.get(0)?,
                    timestamp: row.get(1)?,
                    role: row.get(2)?,
                    mode: row.get(3)?,
                    interaction_kind: row.get(4)?,
                    content: row.get(5)?,
                })
            })
            .map_err(KernelError::Database)?;

        let mut interactions = Vec::new();
        for row in rows {
            interactions.push(row.map_err(KernelError::Database)?);
        }
        interactions.reverse();
        Ok(interactions)
    }

    fn load_execution_lineage(
        conn: &Connection,
        swo: &ActiveSwoRecord,
    ) -> Result<ExecutionLineageRecord> {
        let mut root_swo = swo.clone();
        while let Some(parent_id) = root_swo.parent_swo_id {
            let Some(parent) = Self::load_active_swo_record(conn, parent_id)? else {
                break;
            };
            root_swo = parent;
        }

        let parent_swo = swo
            .parent_swo_id
            .map(|parent_id| Self::load_active_swo_record(conn, parent_id))
            .transpose()?
            .flatten();
        let child_swos = Self::load_child_swos(conn, swo.id)?;
        let linked_swos = Self::load_linked_swos(conn, swo.id)?;
        let mut lineage_swo_ids = vec![swo.id, root_swo.id];
        lineage_swo_ids.extend(child_swos.iter().map(|child| child.id));
        let hires = Self::load_hire_debug_records_for_swo_ids(conn, &lineage_swo_ids)?;

        Ok(ExecutionLineageRecord {
            root_swo: Some(root_swo),
            parent_swo,
            child_swos,
            linked_swos,
            hires,
        })
    }

    pub fn list_swos(&self, limit: usize) -> Result<Vec<ActiveSwoRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&format!(
                "{} ORDER BY
                    CASE s.status
                        WHEN 'IN_PROGRESS' THEN 0
                        WHEN 'PENDING' THEN 1
                        WHEN 'FAILED' THEN 2
                        WHEN 'CANCELLED' THEN 3
                        WHEN 'COMPLETED' THEN 4
                        ELSE 5
                    END,
                    s.id DESC
                LIMIT ?1",
                Self::active_swo_select_sql("")
            ))
            .map_err(KernelError::Database)?;

        let rows = stmt
            .query_map(
                params![limit.clamp(1, 200) as i64],
                Self::active_swo_from_row,
            )
            .map_err(KernelError::Database)?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(KernelError::Database)?);
        }
        Ok(out)
    }

    pub fn get_swo_detail(&self, swo_id: i64) -> Result<Option<SwoDetailRecord>> {
        let conn = self.conn.lock().unwrap();
        let Some(swo) = Self::load_active_swo_record(&conn, swo_id)? else {
            return Ok(None);
        };

        let mut results_stmt = conn
            .prepare(
                "
                SELECT
                    r.id,
                    r.swo_id,
                    r.producer_agent_id,
                    COALESCE(a.name || ' (' || a.role || ')', r.producer_agent_id),
                    r.result_json,
                    r.created_at
                FROM swo_results r
                LEFT JOIN agents a ON a.id = r.producer_agent_id
                WHERE r.swo_id = ?1
                ORDER BY r.id ASC
                ",
            )
            .map_err(KernelError::Database)?;
        let results_rows = results_stmt
            .query_map(params![swo_id], |row| {
                Ok(SwoResultRecord {
                    id: row.get(0)?,
                    swo_id: row.get(1)?,
                    producer_agent_id: row.get(2)?,
                    producer_agent_name: row.get(3)?,
                    result_json: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut results = Vec::new();
        for row in results_rows {
            results.push(row.map_err(KernelError::Database)?);
        }

        let reviews = Self::load_reviews_for_swo(&conn, swo_id)?;

        let mut attachments_stmt = conn
            .prepare(
                "
                SELECT
                    a.id,
                    a.source_kind,
                    a.display_name,
                    a.original_path,
                    a.content_type,
                    a.size_bytes,
                    a.originating_swo_id,
                    a.originating_artifact_id,
                    a.created_at,
                    sa.swo_id,
                    sa.inbox_path,
                    sa.delivery_status,
                    sa.delivery_error
                FROM swo_attachments sa
                JOIN attachments a ON a.id = sa.attachment_id
                WHERE sa.swo_id = ?1
                ORDER BY sa.id ASC
                ",
            )
            .map_err(KernelError::Database)?;
        let attachment_rows = attachments_stmt
            .query_map(params![swo_id], |row| {
                Ok(DeliveredAttachmentRecord {
                    attachment: AttachmentRecord {
                        id: row.get(0)?,
                        source_kind: row.get(1)?,
                        display_name: row.get(2)?,
                        original_path: row.get(3)?,
                        content_type: row.get(4)?,
                        size_bytes: row.get(5)?,
                        originating_swo_id: row.get(6)?,
                        originating_artifact_id: row.get(7)?,
                        created_at: row.get(8)?,
                    },
                    swo_id: row.get(9)?,
                    inbox_path: row.get(10)?,
                    delivery_status: row.get(11)?,
                    delivery_error: row.get(12)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut attachments = Vec::new();
        for row in attachment_rows {
            attachments.push(row.map_err(KernelError::Database)?);
        }

        let mut artifacts_stmt = conn
            .prepare(
                "
                SELECT
                    a.id,
                    a.swo_id,
                    a.agent_id,
                    COALESCE(agent.name || ' (' || agent.role || ')', a.agent_id),
                    s.initiative_id,
                    s.initiative_name,
                    s.parent_swo_id,
                    s.work_order_title,
                    s.work_order_outcome,
                    s.status,
                    a.absolute_path,
                    a.filename,
                    a.created_at
                FROM outbox_artifacts a
                LEFT JOIN agents agent ON agent.id = a.agent_id
                LEFT JOIN active_swos s ON s.id = a.swo_id
                WHERE a.swo_id = ?1
                ORDER BY a.id ASC
                ",
            )
            .map_err(KernelError::Database)?;
        let artifact_rows = artifacts_stmt
            .query_map(params![swo_id], Self::outbox_artifact_from_row)
            .map_err(KernelError::Database)?;
        let mut artifacts = Vec::new();
        for row in artifact_rows {
            artifacts.push(row.map_err(KernelError::Database)?);
        }

        let mut hires_stmt = conn
            .prepare(
                "
                SELECT
                    h.id,
                    h.swo_id,
                    h.manager_agent_id,
                    COALESCE(manager.name || ' (' || manager.role || ')', h.manager_agent_id),
                    h.new_agent_id,
                    COALESCE(new_agent.name || ' (' || new_agent.role || ')', h.new_agent_id),
                    h.spec_json,
                    h.created_at
                FROM agent_hires h
                LEFT JOIN agents manager ON manager.id = h.manager_agent_id
                LEFT JOIN agents new_agent ON new_agent.id = h.new_agent_id
                WHERE h.swo_id = ?1
                ORDER BY h.id ASC
                ",
            )
            .map_err(KernelError::Database)?;
        let hire_rows = hires_stmt
            .query_map(params![swo_id], |row| {
                Ok(AgentHireRecord {
                    id: row.get(0)?,
                    swo_id: row.get(1)?,
                    manager_agent_id: row.get(2)?,
                    manager_agent_name: row.get(3)?,
                    new_agent_id: row.get(4)?,
                    new_agent_name: row.get(5)?,
                    spec_json: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(KernelError::Database)?;
        let mut hires = Vec::new();
        for row in hire_rows {
            hires.push(row.map_err(KernelError::Database)?);
        }

        let child_swos = Self::load_child_swos(&conn, swo_id)?;
        let linked_swos = Self::load_linked_swos(&conn, swo_id)?;

        let mut interactions = Vec::new();
        let mut seen_agents = HashSet::new();
        let mut interaction_agent_ids = vec![
            swo.assigned_agent_id.clone(),
            swo.owner_agent_id.clone(),
            swo.created_by_agent_id.clone(),
        ];
        for child in &child_swos {
            interaction_agent_ids.push(child.assigned_agent_id.clone());
        }
        for linked in &linked_swos {
            interaction_agent_ids.push(linked.assigned_agent_id.clone());
        }
        for agent_id in interaction_agent_ids {
            if !seen_agents.insert(agent_id.clone()) {
                continue;
            }
            let agent_name = Self::load_agent_display_name(&conn, &agent_id)
                .unwrap_or_else(|_| agent_id.clone());
            let db_path = self.agent_memory_db_path(&agent_id)?;
            if !db_path.exists() {
                continue;
            }
            let mem_conn = Connection::open(&db_path).map_err(KernelError::Database)?;
            Self::ensure_agent_memory_schema(&mem_conn)?;
            let mut stmt = mem_conn
                .prepare(
                    "SELECT id, timestamp, role, COALESCE(mode, 'legacy'), COALESCE(interaction_kind, 'message'), content
                     FROM interactions
                     WHERE swo_id = ?1
                     ORDER BY id ASC
                     LIMIT 100",
                )
                .map_err(KernelError::Database)?;
            let rows = stmt
                .query_map(params![swo_id], |row| {
                    Ok(InteractionExcerpt {
                        agent_id: agent_id.clone(),
                        agent_name: agent_name.clone(),
                        interaction_id: row.get(0)?,
                        timestamp: row.get(1)?,
                        role: row.get(2)?,
                        mode: row.get(3)?,
                        interaction_kind: row.get(4)?,
                        content: row.get(5)?,
                    })
                })
                .map_err(KernelError::Database)?;
            for row in rows {
                interactions.push(row.map_err(KernelError::Database)?);
            }
        }

        interactions.sort_by(|a, b| a.interaction_id.cmp(&b.interaction_id));
        let delegation_debug = Self::compute_delegation_debug(&swo, &reviews, &child_swos);
        let worker_runs = Self::load_worker_runs_for_swo(&conn, swo_id)?;

        let delegation_status =
            if let Some(requested_id) = swo.requested_assignee_agent_id.as_deref() {
                if let Some(child) = child_swos
                    .iter()
                    .find(|child| child.assigned_agent_id == requested_id)
                {
                    format!(
                        "Delegated to {} via child SWO #{}",
                        child.assigned_agent_name, child.id
                    )
                } else if let Some(other_child) = child_swos.first() {
                    format!(
                        "Requested {} under {} routing, but child SWO delegated to {} (#{}).",
                        swo.requested_assignee_agent_name
                            .clone()
                            .unwrap_or_else(|| requested_id.to_string()),
                        swo.routing_policy,
                        other_child.assigned_agent_name,
                        other_child.id
                    )
                } else if swo.status == "FAILED" {
                    format!(
                        "Requested {} under {} routing, but no valid child delegation was created.",
                        swo.requested_assignee_agent_name
                            .clone()
                            .unwrap_or_else(|| requested_id.to_string()),
                        swo.routing_policy
                    )
                } else {
                    format!(
                        "Requested {} under {} routing. Waiting for matching child delegation.",
                        swo.requested_assignee_agent_name
                            .clone()
                            .unwrap_or_else(|| requested_id.to_string()),
                        swo.routing_policy
                    )
                }
            } else {
                "No requested assignee routing constraint.".to_string()
            };
        let execution_lineage = Self::load_execution_lineage(&conn, &swo)?;

        Ok(Some(SwoDetailRecord {
            swo,
            delegation_status,
            delegation_debug,
            attachments,
            results,
            reviews,
            artifacts,
            hires,
            child_swos,
            linked_swos,
            interactions,
            worker_runs,
            execution_lineage,
        }))
    }

    /// Returns SWOs that are IN_PROGRESS but whose agent's heartbeat is older than `stale_threshold_ms`.
    /// Also returns SWOs with no heartbeat entry at all (agent never pinged) if they are old enough.
    pub fn get_stale_in_progress_swos(
        &self,
        stale_threshold_ms: i64,
    ) -> Result<Vec<(i64, String, i32)>> {
        let conn = self.conn.lock().unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let cutoff_ms = now_ms - stale_threshold_ms;

        // Kryptonite fix #3: Scope staleness to per-SWO run_id, not agent-level MAX(heartbeat).
        // A healthy concurrent run for the same agent no longer masks a dead IN_PROGRESS SWO.
        let mut stmt = conn.prepare(
            "SELECT s.id, s.assigned_agent_id, s.retry_count
             FROM active_swos s
             WHERE s.status = 'IN_PROGRESS'
               AND (
                 -- SWO has a run_id but its specific heartbeat is stale
                 (s.current_run_id IS NOT NULL
                  AND (SELECT h.last_seen_unix_ms FROM agent_heartbeats h WHERE h.run_id = s.current_run_id) < ?1)
                 OR
                 -- SWO was claimed but run_id was never set (legacy / race window)
                 (s.current_run_id IS NULL
                  AND NOT EXISTS (SELECT 1 FROM agent_heartbeats h WHERE h.agent_id = s.assigned_agent_id AND h.last_seen_unix_ms >= ?1))
               )",
        ).map_err(KernelError::Database)?;

        let rows = stmt
            .query_map(params![cutoff_ms], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i32>(2)?,
                ))
            })
            .map_err(KernelError::Database)?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r.map_err(KernelError::Database)?);
        }
        Ok(result)
    }

    pub fn reset_swo_to_pending(&self, swo_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE active_swos SET status = 'PENDING', retry_count = retry_count + 1 WHERE id = ?1",
            params![swo_id],
        ).map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn reset_swo_with_revision_feedback(&self, swo_id: i64, feedback: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE active_swos SET status = 'PENDING', retry_count = retry_count + 1, revision_feedback = ?2 WHERE id = ?1",
            params![swo_id, feedback],
        ).map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn set_swo_status(&self, swo_id: i64, status: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE active_swos SET status = ?1 WHERE id = ?2",
            params![status, swo_id],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn fail_swo(&self, swo_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE active_swos SET status = 'FAILED' WHERE id = ?1",
            params![swo_id],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    /// Returns all agents configured with a cron interval.
    pub fn get_cron_eligible_agents(&self) -> Result<Vec<AgentIdentity>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, parent_id, role, COALESCE(persona_prompt, raison_detre), raison_detre, default_provider, default_model, cron_interval_seconds, triage_model, execution_model
             FROM agents WHERE cron_interval_seconds IS NOT NULL",
        ).map_err(KernelError::Database)?;

        let rows = stmt
            .query_map([], Self::agent_identity_from_row)
            .map_err(KernelError::Database)?;

        let mut agents = Vec::new();
        for r in rows {
            agents.push(r.map_err(KernelError::Database)?);
        }
        Ok(agents)
    }

    /// Returns the oldest PENDING SWO for the given agent, if any.
    pub fn get_next_pending_swo_for_agent(&self, agent_id: &str) -> Result<Option<(i64, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, swo_payload FROM active_swos
             WHERE assigned_agent_id = ?1 AND status = 'PENDING'
             ORDER BY created_at ASC LIMIT 1",
            )
            .map_err(KernelError::Database)?;

        let mut rows = stmt
            .query(params![agent_id])
            .map_err(KernelError::Database)?;
        if let Some(row) = rows.next().map_err(KernelError::Database)? {
            Ok(Some((
                row.get(0).map_err(KernelError::Database)?,
                row.get(1).map_err(KernelError::Database)?,
            )))
        } else {
            Ok(None)
        }
    }

    /// Atomically claim a PENDING SWO by transitioning it to IN_PROGRESS and recording the run_id.
    /// Returns the number of rows updated (0 = already claimed by a race).
    pub fn claim_swo_with_run_id(&self, swo_id: i64, run_id: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE active_swos SET status = 'IN_PROGRESS', current_run_id = ?2 WHERE id = ?1 AND status = 'PENDING'",
            params![swo_id, run_id],
        ).map_err(KernelError::Database)?;
        Ok(updated)
    }

    /// Legacy claim without run_id — kept for backward compat, prefer claim_swo_with_run_id.
    pub fn claim_swo(&self, swo_id: i64) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE active_swos SET status = 'IN_PROGRESS' WHERE id = ?1 AND status = 'PENDING'",
            params![swo_id],
        ).map_err(KernelError::Database)?;
        Ok(updated)
    }

    pub fn update_agent_cron(&self, id: &str, interval: Option<i64>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agents SET cron_interval_seconds = ?1 WHERE id = ?2",
            params![interval, id],
        )
        .map_err(KernelError::Database)?;
        Ok(())
    }

    pub fn upsert_heartbeat(
        &self,
        run_id: &str,
        agent_id: &str,
        status: &str,
        seq: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        // Kryptonite fix: enforce monotonic seq — only update if incoming seq is newer.
        // This prevents a stale/replayed heartbeat from overwriting a more recent one.
        conn.execute(
            "INSERT INTO agent_heartbeats (run_id, agent_id, status, last_seen_unix_ms, seq)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(run_id) DO UPDATE SET
                status = excluded.status,
                last_seen_unix_ms = excluded.last_seen_unix_ms,
                seq = excluded.seq
             WHERE excluded.seq > agent_heartbeats.seq OR excluded.seq < 0",
            params![run_id, agent_id, status, now_ms, seq],
        )
        .map_err(KernelError::Database)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_registry() -> Registry {
        let test_root = std::env::temp_dir().join(format!("sairgent-registry-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&test_root).unwrap();
        let db_path = test_root.join("registry.sqlite");
        Registry::new(db_path.to_str().unwrap()).unwrap()
    }

    fn find_node<'a>(
        nodes: &'a [AgentTreeNodeRecord],
        name: &str,
    ) -> Option<&'a AgentTreeNodeRecord> {
        for node in nodes {
            if node.name == name {
                return Some(node);
            }
            if let Some(child) = find_node(&node.children, name) {
                return Some(child);
            }
        }
        None
    }

    fn insert_heartbeat(
        registry: &Registry,
        run_id: &str,
        agent_id: &str,
        status: &str,
        last_seen_unix_ms: i64,
        seq: i64,
    ) {
        let conn = registry.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_heartbeats (run_id, agent_id, status, last_seen_unix_ms, seq)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, agent_id, status, last_seen_unix_ms, seq],
        )
        .unwrap();
    }

    #[test]
    fn inbox_items_persist_and_roll_up_attention_summary() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        let swo_id = registry
            .create_swo_with_metadata(CreateSwoParams {
                assigned_agent_id: &perry_id,
                owner_agent_id: &perry_id,
                created_by_agent_id: &perry_id,
                payload: "Ship inbox route",
                status: "COMPLETED",
                parent_swo_id: None,
                kind: "TASK",
                source: "TEST",
                work_order_title: Some("Ship inbox route"),
                work_order_outcome: Some("Deliver a durable operator inbox."),
                work_order_constraints: None,
                requested_owner_agent_id: None,
                requested_assignee_agent_id: None,
                routing_policy: "NONE",
                originating_swo_id: None,
                initiative_id: Some("proj-inbox"),
                initiative_name: Some("Inbox"),
                initiative_owner_agent_id: Some(&perry_id),
                priority_class: Some("HIGH"),
            })
            .unwrap();

        registry
            .upsert_inbox_item(UpsertInboxItemParams {
                id: "inbox-deliverable-test",
                kind: "deliverable",
                status: "OPEN",
                priority: "HIGH",
                title: "Ship inbox route",
                summary: "Deliverable ready for review.",
                project_id: Some("proj-inbox"),
                project_name: Some("Inbox"),
                swo_id: Some(swo_id),
                artifact_id: None,
                agent_id: Some(&perry_id),
            })
            .unwrap();
        registry
            .upsert_inbox_item(UpsertInboxItemParams {
                id: "inbox-blocked-test",
                kind: "blocked",
                status: "OPEN",
                priority: "HIGH",
                title: "Fix inbox regression",
                summary: "Retry path is still blocked.",
                project_id: Some("proj-inbox"),
                project_name: Some("Inbox"),
                swo_id: Some(swo_id),
                artifact_id: None,
                agent_id: Some(&perry_id),
            })
            .unwrap();

        let summary = registry.inbox_attention_summary().unwrap();
        assert_eq!(summary.open_inbox_items, 2);
        assert_eq!(summary.open_deliverable_items, 1);
        assert_eq!(summary.open_blocked_items, 1);

        let acknowledged = registry
            .acknowledge_inbox_item("inbox-deliverable-test")
            .unwrap()
            .unwrap();
        assert_eq!(acknowledged.status, "ACKNOWLEDGED");

        let after_ack = registry.inbox_attention_summary().unwrap();
        assert_eq!(after_ack.open_inbox_items, 1);
        assert_eq!(after_ack.open_deliverable_items, 0);
        assert_eq!(after_ack.open_blocked_items, 1);

        let resolved = registry
            .resolve_inbox_item("inbox-blocked-test", Some("Cleared after retry."))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.status, "RESOLVED");

        let active_items = registry.list_inbox_items(false, 20).unwrap();
        assert_eq!(active_items.len(), 1);
        assert_eq!(active_items[0].id, "inbox-deliverable-test");

        let all_items = registry.list_inbox_items(true, 20).unwrap();
        assert_eq!(all_items.len(), 2);

        let final_summary = registry.inbox_attention_summary().unwrap();
        assert_eq!(final_summary.open_inbox_items, 0);
    }

    #[test]
    fn builds_true_agent_tree_from_parent_ids() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        let _cat_id = registry
            .hire_subordinate("Cat", Some(&perry_id), "CMO", "Market", "mock", "mock")
            .unwrap();
        let felicity_id = registry
            .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
            .unwrap();
        let _eng_id = registry
            .hire_subordinate(
                "Builder",
                Some(&felicity_id),
                "Engineer",
                "Ship",
                "mock",
                "mock",
            )
            .unwrap();

        let snapshot = registry.get_agent_tree_snapshot(1_000_000).unwrap();
        let perry = find_node(&snapshot, "Perry").unwrap();
        let felicity = find_node(&snapshot, "Felicity").unwrap();
        let builder = find_node(&snapshot, "Builder").unwrap();

        assert_eq!(perry.depth, 0);
        assert_eq!(perry.direct_report_count, 2);
        assert_eq!(perry.descendant_count, 3);
        assert_eq!(felicity.depth, 1);
        assert!(felicity.is_direct_report);
        assert_eq!(builder.depth, 2);
        assert!(!builder.is_direct_report);
    }

    #[test]
    fn agent_detail_flags_parent_mismatch_for_hires() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        let felicity_id = registry
            .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
            .unwrap();
        let root_swo_id = registry
            .create_swo_with_metadata(CreateSwoParams {
                assigned_agent_id: &perry_id,
                owner_agent_id: &perry_id,
                created_by_agent_id: &perry_id,
                payload: "Hire a specialist.",
                status: "COMPLETED",
                parent_swo_id: None,
                kind: "TASK",
                source: "CHAT",
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
            })
            .unwrap();
        let specialist_id = registry
            .hire_subordinate("Specialist", Some(&perry_id), "IC", "Work", "mock", "mock")
            .unwrap();
        registry
            .record_agent_hire(
                root_swo_id,
                &perry_id,
                &specialist_id,
                "{\"name\":\"Specialist\"}",
            )
            .unwrap();

        {
            let conn = registry.conn.lock().unwrap();
            conn.execute(
                "UPDATE agents SET parent_id = ?1 WHERE id = ?2",
                params![felicity_id, specialist_id],
            )
            .unwrap();
        }

        let detail = registry
            .get_agent_detail_snapshot(&perry_id, 1_000_000)
            .unwrap();
        let hire = detail.recent_hires.first().unwrap();

        assert!(!hire.parent_matches_manager);
        assert_eq!(
            hire.actual_parent_agent_id.as_deref(),
            Some(felicity_id.as_str())
        );
        assert!(hire
            .actual_parent_agent_name
            .as_ref()
            .unwrap()
            .contains("Felicity"));
    }

    #[test]
    fn normalizes_presence_windows() {
        let registry = test_registry();
        let now_ms = 1_000_000;
        let computing_id = registry
            .hire_subordinate("Compute", None, "Role", "Goal", "mock", "mock")
            .unwrap();
        let ready_id = registry
            .hire_subordinate("Ready", None, "Role", "Goal", "mock", "mock")
            .unwrap();
        let stale_id = registry
            .hire_subordinate("Stale", None, "Role", "Goal", "mock", "mock")
            .unwrap();
        let offline_id = registry
            .hire_subordinate("Offline", None, "Role", "Goal", "mock", "mock")
            .unwrap();

        insert_heartbeat(
            &registry,
            "run-computing",
            &computing_id,
            "COMPUTING",
            now_ms - 1_000,
            1,
        );
        insert_heartbeat(
            &registry,
            "run-ready",
            &ready_id,
            "READY",
            now_ms - 20_000,
            1,
        );
        insert_heartbeat(
            &registry,
            "run-stale",
            &stale_id,
            "ERROR",
            now_ms - 40_000,
            1,
        );
        insert_heartbeat(
            &registry,
            "run-offline",
            &offline_id,
            "READY",
            now_ms - 100_000,
            1,
        );

        let presence = registry
            .get_agent_presence(now_ms)
            .unwrap()
            .into_iter()
            .map(|row| (row.agent_id, row.presence))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            presence.get(&computing_id).map(String::as_str),
            Some("COMPUTING")
        );
        assert_eq!(presence.get(&ready_id).map(String::as_str), Some("READY"));
        assert_eq!(presence.get(&stale_id).map(String::as_str), Some("STALE"));
        assert_eq!(
            presence.get(&offline_id).map(String::as_str),
            Some("OFFLINE")
        );
    }

    #[test]
    fn swo_detail_marks_delegated_match_when_requested_child_exists() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        let felicity_id = registry
            .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
            .unwrap();
        let root_swo_id = registry
            .create_swo_with_metadata(CreateSwoParams {
                assigned_agent_id: &perry_id,
                owner_agent_id: &perry_id,
                created_by_agent_id: &perry_id,
                payload: "Felicity must handle this.",
                status: "IN_PROGRESS",
                parent_swo_id: None,
                kind: "TASK",
                source: "CHAT",
                work_order_title: None,
                work_order_outcome: None,
                work_order_constraints: None,
                requested_owner_agent_id: None,
                requested_assignee_agent_id: Some(&felicity_id),
                routing_policy: "HARD_ROUTE",
                originating_swo_id: None,
                initiative_id: None,
                initiative_name: None,
                initiative_owner_agent_id: None,
                priority_class: None,
            })
            .unwrap();
        registry
            .create_swo_with_metadata(CreateSwoParams {
                assigned_agent_id: &felicity_id,
                owner_agent_id: &perry_id,
                created_by_agent_id: &perry_id,
                payload: "Delegated to Felicity",
                status: "PENDING",
                parent_swo_id: Some(root_swo_id),
                kind: "TASK",
                source: "HSM",
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
            })
            .unwrap();

        let detail = registry.get_swo_detail(root_swo_id).unwrap().unwrap();
        assert_eq!(detail.delegation_debug.child_swo_count, 1);
        assert!(detail
            .delegation_status
            .contains("Delegated to Felicity (CTO) via child SWO"));
        assert!(detail.delegation_debug.mismatch_flags.is_empty());
    }

    #[test]
    fn swo_detail_surfaces_missing_child_and_review_claim_mismatch() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        let felicity_id = registry
            .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
            .unwrap();
        let root_swo_id = registry
            .create_swo_with_metadata(CreateSwoParams {
                assigned_agent_id: &perry_id,
                owner_agent_id: &perry_id,
                created_by_agent_id: &perry_id,
                payload: "Felicity must handle this.",
                status: "FAILED",
                parent_swo_id: None,
                kind: "TASK",
                source: "CHAT",
                work_order_title: None,
                work_order_outcome: None,
                work_order_constraints: None,
                requested_owner_agent_id: None,
                requested_assignee_agent_id: Some(&felicity_id),
                routing_policy: "HARD_ROUTE",
                originating_swo_id: None,
                initiative_id: None,
                initiative_name: None,
                initiative_owner_agent_id: None,
                priority_class: None,
            })
            .unwrap();
        registry
            .record_manager_review(
                root_swo_id,
                &perry_id,
                "APPROVE_AND_REPLY",
                "Felicity is assigned and already working on it.",
                Some("Felicity has the assignment."),
            )
            .unwrap();

        let detail = registry.get_swo_detail(root_swo_id).unwrap().unwrap();
        assert!(detail
            .delegation_debug
            .mismatch_flags
            .contains(&"no_child_swo".to_string()));
        assert!(detail
            .delegation_debug
            .mismatch_flags
            .contains(&"requested_not_delegated".to_string()));
        assert!(detail
            .delegation_debug
            .mismatch_flags
            .contains(&"review_claim_without_child".to_string()));
    }

    #[test]
    fn tool_binding_requires_capability() {
        let registry = test_registry();
        let agent_id = registry
            .hire_subordinate("Felicity", None, "CTO", "Build", "mock", "mock")
            .unwrap();

        let error = registry
            .bind_tool_to_agent(&agent_id, "web_search_tavily")
            .unwrap_err();

        assert!(format!("{error:?}").contains("lacks required capability"));
    }

    #[test]
    fn tool_binding_replaces_same_kind_provider() {
        let registry = test_registry();
        let agent_id = registry
            .hire_subordinate(
                "Lois",
                None,
                "Research Specialist",
                "Research",
                "mock",
                "mock",
            )
            .unwrap();

        registry
            .bind_tool_to_agent(&agent_id, "web_search_tavily")
            .unwrap();
        registry
            .bind_tool_to_agent(&agent_id, "web_search_exa")
            .unwrap();

        let bindings = registry.list_agent_tool_bindings(&agent_id).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].tool_slug, "web_search_exa");
        assert_eq!(bindings[0].tool_kind, "web_search");
    }

    #[test]
    fn get_agent_manifest_backfills_default_manifest() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();

        let manifest = registry.get_agent_manifest(&perry_id).unwrap();
        assert_eq!(manifest.agent_id.as_deref(), Some(perry_id.as_str()));
        assert!(manifest
            .capabilities
            .contains(&crate::manifest::CapabilityGrant::QueueManagedWork));
    }

    #[test]
    fn create_agent_seeds_least_privilege_manifest() {
        let registry = test_registry();
        let manager_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();

        let agent_id = registry
            .create_agent(
                "Builder",
                Some(&manager_id),
                "Engineer",
                "Ship production-ready changes.",
                "Ship production-ready changes.",
                "openai",
                "gpt-4.1-mini",
            )
            .unwrap();

        let manifest = registry.get_agent_manifest(&agent_id).unwrap();
        assert_eq!(manifest.agent_id.as_deref(), Some(agent_id.as_str()));
        assert!(manifest
            .capabilities
            .contains(&crate::manifest::CapabilityGrant::QueueManagedWork));
        assert!(manifest
            .capabilities
            .contains(&crate::manifest::CapabilityGrant::ReadInbox));
        assert!(manifest
            .capabilities
            .contains(&crate::manifest::CapabilityGrant::WriteOutbox));
        assert!(!manifest
            .capabilities
            .contains(&crate::manifest::CapabilityGrant::HireSubordinate));
        assert!(!manifest
            .capabilities
            .contains(&crate::manifest::CapabilityGrant::DispatchSwo));
    }

    #[test]
    fn create_agent_honors_configured_cap() {
        let registry = test_registry();
        registry.upsert_runtime_metadata("max_agents", "1").unwrap();
        let manager_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();

        let error = registry
            .create_agent(
                "Second",
                Some(&manager_id),
                "Operator",
                "Do careful work.",
                "Do careful work.",
                "openai",
                "gpt-4.1-mini",
            )
            .unwrap_err();

        assert!(matches!(error, KernelError::AgentCapExceeded(_)));
    }

    #[test]
    fn persists_agent_cron_last_fired_and_projects_into_snapshots() {
        let registry = test_registry();
        let agent_id = registry
            .hire_subordinate("Lois", None, "Research", "Research", "mock", "mock")
            .unwrap();

        let stored = registry.set_agent_cron_last_fired_now(&agent_id).unwrap();
        assert!(stored.is_some());

        let detail = registry
            .get_agent_detail_snapshot(&agent_id, 1_000_000)
            .unwrap();
        assert!(detail.last_cron_fired_at.is_some());

        let tree = registry.get_agent_tree_snapshot(1_000_000).unwrap();
        let lois = find_node(&tree, "Lois").unwrap();
        assert!(lois.last_cron_fired_at.is_some());

        let persisted = registry.list_agent_cron_last_fired_unix().unwrap();
        assert!(persisted.contains_key(&agent_id));
    }

    #[test]
    fn decision_log_entries_are_listed_and_pruned() {
        let registry = test_registry();
        let agent_id = registry
            .hire_subordinate("Felicity", None, "CTO", "Build", "mock", "mock")
            .unwrap();

        for (index, created_at) in [
            "2026-03-10 10:00:00",
            "2026-03-10 11:00:00",
            "2026-03-10 12:00:00",
        ]
        .iter()
        .enumerate()
        {
            registry
                .insert_decision_log_entry(
                    &agent_id,
                    &DecisionLogEntryRecord {
                        entry_id: format!("entry-{}", index + 1),
                        agent_id: agent_id.clone(),
                        mode: "ideation".to_string(),
                        summary: format!("Summary {}", index + 1),
                        rationale: format!("Rationale {}", index + 1),
                        outcome: "SUCCESS".to_string(),
                        confidence: Some(0.8),
                        self_note: Some(format!("Note {}", index + 1)),
                        linked_swo_id: Some((index + 1) as i64),
                        linked_run_id: Some(format!("run-{}", index + 1)),
                        created_at: (*created_at).to_string(),
                    },
                )
                .unwrap();
        }

        let listed = registry.list_decision_log(&agent_id, 10).unwrap();
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].summary, "Summary 3");
        assert_eq!(listed[2].summary, "Summary 1");

        let pruned = registry.prune_decision_log(&agent_id, 2).unwrap();
        assert_eq!(pruned, 1);

        let remaining = registry.list_decision_log(&agent_id, 10).unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].summary, "Summary 3");
        assert_eq!(remaining[1].summary, "Summary 2");

        let recent = registry.list_recent_decision_log_entries(5).unwrap();
        assert_eq!(recent.len(), 2);
        assert!(recent.iter().all(|entry| entry.agent_id == agent_id));
    }

    #[test]
    fn project_status_updates_are_persisted() {
        let registry = test_registry();
        let project = registry
            .create_project(
                "proj-test",
                "Project Test",
                Some("Initial summary"),
                "ACTIVE",
                "HIGH",
                None,
                Some("Ship the PM-first improvements"),
                Some("pm,desktop"),
                "Desktop Operator",
            )
            .unwrap();

        assert_eq!(project.status, "ACTIVE");

        let updated = registry
            .update_project_status(
                "proj-test",
                "PAUSED",
                Some("Waiting on operator review."),
                "Desktop Operator",
            )
            .unwrap();

        assert_eq!(updated.status, "PAUSED");

        let history = registry.list_project_status_updates().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].project_id, "proj-test");
        assert_eq!(history[0].previous_status.as_deref(), Some("ACTIVE"));
        assert_eq!(history[0].next_status, "PAUSED");
        assert_eq!(history[1].previous_status, None);
        assert_eq!(history[1].next_status, "ACTIVE");
    }

    #[test]
    fn work_orders_preserve_parent_project_context_and_can_clear_constraints() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        registry
            .create_project(
                "proj-work",
                "Project Work",
                Some("Track operator recovery"),
                "ACTIVE",
                "HIGH",
                Some(&perry_id),
                Some("Keep project-linked SWOs grouped together."),
                Some("pm,queue"),
                "Desktop Operator",
            )
            .unwrap();

        let parent_id = registry
            .create_work_order(
                &perry_id,
                &perry_id,
                "Parent Work Order",
                "Define the operator scope.",
                Some("Stay within the project context."),
                Some("HIGH"),
                Some(&perry_id),
                None,
                Some("proj-work"),
            )
            .unwrap();
        let child_id = registry
            .create_work_order(
                &perry_id,
                &perry_id,
                "Child Work Order",
                "Handle the blocked follow-up.",
                Some("Use the parent context."),
                Some("NORMAL"),
                Some(&perry_id),
                Some(parent_id),
                Some("proj-work"),
            )
            .unwrap();

        let child = registry.get_swo_detail(child_id).unwrap().unwrap();
        assert_eq!(child.swo.parent_swo_id, Some(parent_id));
        assert_eq!(child.swo.initiative_id.as_deref(), Some("proj-work"));
        assert_eq!(
            child.swo.work_order_constraints.as_deref(),
            Some("Use the parent context.")
        );

        registry
            .update_swo_work_order_fields(child_id, Some("Retitled Child"), None, Some(None))
            .unwrap();
        let refreshed = registry.get_swo_detail(child_id).unwrap().unwrap();
        assert_eq!(
            refreshed.swo.work_order_title.as_deref(),
            Some("Retitled Child")
        );
        assert_eq!(refreshed.swo.work_order_constraints, None);
    }

    #[test]
    fn recurring_templates_persist_schedule_and_roster_names() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        let lois_id = registry
            .hire_subordinate(
                "Lois",
                Some(&perry_id),
                "Research",
                "Report",
                "mock",
                "mock",
            )
            .unwrap();
        registry
            .create_project(
                "proj-rwo",
                "Recurring Ops",
                Some("Track weekly ops"),
                "ACTIVE",
                "HIGH",
                Some(&perry_id),
                Some("Keep recurring reporting online"),
                Some("ops,recurring"),
                "Desktop Operator",
            )
            .unwrap();

        let template = registry
            .create_recurring_template(CreateRecurringWorkOrderTemplateParams {
                template_id: "rwo-test",
                project_id: Some("proj-rwo"),
                source_swo_id: None,
                owner_agent_id: &perry_id,
                assignee_agent_id: Some(&lois_id),
                name: "weekly-ops-review",
                title: "Weekly Ops Review",
                outcome: "Summarize operational drift.",
                constraints: Some("Use current project context only."),
                priority: "HIGH",
                include_prior_artifacts: true,
                schedule: &RecurringWorkOrderScheduleRecord {
                    cadence: "weekly".to_string(),
                    interval: 1,
                    timezone: "UTC".to_string(),
                    days_of_week: Some(vec![1]),
                    day_of_month: None,
                    hour: Some(9),
                    minute: Some(30),
                    cron_expression: None,
                },
                status: "ACTIVE",
                next_run_at: Some("2026-03-17 09:30:00"),
                last_run_at: None,
                last_run_status: None,
            })
            .unwrap();

        assert_eq!(template.owner_agent_name, "Perry (COO)");
        assert_eq!(
            template.assignee_agent_name.as_deref(),
            Some("Lois (Research)")
        );

        let listed = registry.list_recurring_templates().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].template_id, "rwo-test");
        assert_eq!(listed[0].schedule.days_of_week.as_deref(), Some(&[1][..]));
        assert_eq!(
            listed[0].next_run_at.as_deref(),
            Some("2026-03-17 09:30:00")
        );
    }

    #[test]
    fn recurring_run_sync_tracks_terminal_swo_state_and_artifacts() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        let swo_id = registry
            .create_swo_with_metadata(CreateSwoParams {
                assigned_agent_id: &perry_id,
                owner_agent_id: &perry_id,
                created_by_agent_id: &perry_id,
                payload:
                    "WORK ORDER\nTitle: Weekly Ops Review\nRequested outcome: Summarize drift.",
                status: "PENDING",
                parent_swo_id: None,
                kind: "WORK_ORDER",
                source: "RECURRING_TEMPLATE",
                work_order_title: Some("Weekly Ops Review (Run #1)"),
                work_order_outcome: Some("Summarize drift."),
                work_order_constraints: None,
                requested_owner_agent_id: Some(&perry_id),
                requested_assignee_agent_id: Some(&perry_id),
                routing_policy: "NONE",
                originating_swo_id: None,
                initiative_id: None,
                initiative_name: None,
                initiative_owner_agent_id: None,
                priority_class: Some("HIGH"),
            })
            .unwrap();
        registry
            .create_recurring_template(CreateRecurringWorkOrderTemplateParams {
                template_id: "rwo-sync",
                project_id: None,
                source_swo_id: None,
                owner_agent_id: &perry_id,
                assignee_agent_id: Some(&perry_id),
                name: "weekly-ops-review",
                title: "Weekly Ops Review",
                outcome: "Summarize drift.",
                constraints: None,
                priority: "HIGH",
                include_prior_artifacts: false,
                schedule: &RecurringWorkOrderScheduleRecord {
                    cadence: "daily".to_string(),
                    interval: 7,
                    timezone: "UTC".to_string(),
                    days_of_week: None,
                    day_of_month: None,
                    hour: Some(9),
                    minute: Some(0),
                    cron_expression: None,
                },
                status: "ACTIVE",
                next_run_at: Some("2026-03-17 09:00:00"),
                last_run_at: None,
                last_run_status: None,
            })
            .unwrap();
        registry
            .create_recurring_run(CreateRecurringWorkOrderRunParams {
                run_id: "rwo-run-sync",
                template_id: "rwo-sync",
                swo_id: Some(swo_id),
                project_id: None,
                run_number: 1,
                status: "QUEUED",
                trigger_source: "manual",
                queued_at: Some("2026-03-10 09:00:00"),
                started_at: None,
                completed_at: None,
                error_message: None,
                artifact_ids: &[],
            })
            .unwrap();

        let artifact_root = std::env::temp_dir().join(format!("rwo-artifact-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&artifact_root).unwrap();
        let artifact_path = artifact_root.join("weekly-review.md");
        std::fs::write(&artifact_path, "report body").unwrap();
        let artifact_id = registry
            .record_outbox_artifact(
                swo_id,
                &perry_id,
                artifact_path.to_str().unwrap(),
                "weekly-review.md",
            )
            .unwrap();
        registry.update_swo_status(swo_id, "COMPLETED").unwrap();
        registry.sync_recurring_runs_from_swos().unwrap();

        let run = registry.get_recurring_run("rwo-run-sync").unwrap().unwrap();
        assert_eq!(run.status, "COMPLETED");
        assert_eq!(run.artifact_ids, vec![artifact_id]);

        let template = registry
            .get_recurring_template("rwo-sync")
            .unwrap()
            .unwrap();
        assert_eq!(template.last_run_status.as_deref(), Some("COMPLETED"));
        assert_eq!(template.last_run_at.as_deref(), Some("2026-03-10 09:00:00"));
    }

    #[test]
    fn audit_events_chain_together() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();

        let first_id = registry
            .record_audit_event(
                Some(&perry_id),
                None,
                "first_event",
                TaintLabel::TrustedSystem,
                &serde_json::json!({"seq": 1}),
            )
            .unwrap();
        let second_id = registry
            .record_audit_event(
                Some(&perry_id),
                None,
                "second_event",
                TaintLabel::UntrustedModelOutput,
                &serde_json::json!({"seq": 2}),
            )
            .unwrap();

        assert!(second_id > first_id);
        let events = registry.list_audit_events(10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_kind, "second_event");
        assert!(events[0].previous_chain_hash.is_some());
        assert_ne!(events[0].chain_hash, events[1].chain_hash);
    }

    #[test]
    fn tool_execution_audit_events_chain_correctly() {
        let registry = test_registry();
        let felicity_id = registry
            .hire_subordinate("Felicity", None, "CTO", "Build", "mock", "mock")
            .unwrap();

        // shell_exec event
        let shell_id = registry
            .record_audit_event(
                Some(&felicity_id),
                None,
                "shell_exec",
                TaintLabel::ToolExecution,
                &serde_json::json!({
                    "command": "cargo check",
                    "cwd": "/workspace",
                    "exit_code": 0,
                    "duration_ms": 1200,
                    "stdout_hash": "sha256:abc",
                    "stderr_hash": "sha256:def",
                    "truncated": false
                }),
            )
            .unwrap();

        // file_mutation event
        let file_id = registry
            .record_audit_event(
                Some(&felicity_id),
                None,
                "file_mutation",
                TaintLabel::ToolExecution,
                &serde_json::json!({
                    "operation": "create",
                    "path": "/workspace/src/main.rs",
                    "size": 512,
                    "content_hash": "sha256:xyz"
                }),
            )
            .unwrap();

        // git_operation event
        let git_id = registry
            .record_audit_event(
                Some(&felicity_id),
                None,
                "git_operation",
                TaintLabel::ToolExecution,
                &serde_json::json!({
                    "operation": "commit",
                    "repo": "/workspace",
                    "branch": null,
                    "commit_hash": "abc1234",
                    "files_changed": ["src/main.rs"]
                }),
            )
            .unwrap();

        assert!(file_id > shell_id);
        assert!(git_id > file_id);

        let events = registry.list_audit_events(10).unwrap();
        assert_eq!(events.len(), 3);
        // Most recent first
        assert_eq!(events[0].event_kind, "git_operation");
        assert_eq!(events[1].event_kind, "file_mutation");
        assert_eq!(events[2].event_kind, "shell_exec");
        // All chained
        assert!(events[0].previous_chain_hash.is_some());
        assert!(events[1].previous_chain_hash.is_some());
        // TaintLabel round-trips
        assert_eq!(events[0].taint_label, TaintLabel::ToolExecution);
    }

    #[test]
    fn external_channel_bindings_track_sessions_and_delivery_state() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();

        let binding = registry
            .upsert_external_channel_binding(UpsertExternalChannelBindingParams {
                agent_id: &perry_id,
                channel: "telegram",
                enabled: true,
                allowed_chat_id: Some("42"),
                allowed_user_id: Some("7"),
                route_token: Some("telegram-route"),
                secret_token: Some("telegram-secret"),
            })
            .unwrap();
        assert!(binding.enabled);
        assert!(binding.has_route_token);
        assert!(binding.has_secret_token);

        let resolved = registry
            .resolve_external_channel_binding_by_route_token("telegram", "telegram-route")
            .unwrap()
            .unwrap();
        assert_eq!(resolved.binding.agent_id, perry_id);
        assert_eq!(resolved.binding.allowed_chat_id.as_deref(), Some("42"));

        assert!(registry
            .claim_external_message_receipt("telegram", "42", "501")
            .unwrap());
        assert!(!registry
            .claim_external_message_receipt("telegram", "42", "501")
            .unwrap());

        let session = registry
            .touch_external_chat_session(TouchExternalChatSessionParams {
                agent_id: &perry_id,
                channel: "telegram",
                external_chat_id: "42",
                external_user_id: Some("7"),
                conversation_id: "sairgent-perry-telegram-42-7",
                last_inbound_message_id: Some("501"),
            })
            .unwrap();
        assert_eq!(session.external_chat_id, "42");
        assert_eq!(session.external_user_id.as_deref(), Some("7"));

        let event = registry
            .record_external_channel_delivery_event(RecordExternalChannelDeliveryEventParams {
                agent_id: &perry_id,
                channel: "telegram",
                session_id: Some(&session.session_id),
                direction: "outbound",
                status: "delivered",
                detail: "Telegram reply prepared successfully.",
                external_chat_id: Some("42"),
                external_user_id: Some("7"),
                external_message_id: Some("501"),
            })
            .unwrap();
        assert_eq!(event.status, "delivered");

        let bindings = registry
            .list_external_channel_bindings(Some(&perry_id))
            .unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].last_delivery_status, "delivered");

        let events = registry
            .list_recent_external_channel_delivery_events(10)
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].detail, "Telegram reply prepared successfully.");
    }

    #[test]
    fn org_profile_defaults_and_team_goal_assignment_round_trip() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();

        let profile = registry.get_agent_org_profile(&perry_id).unwrap();
        assert_eq!(profile.org_class, "manager");
        assert_eq!(profile.delegation_policy, "must_delegate_when_fit_exists");
        assert!(profile.manager_can_hire);

        let goal = registry
            .upsert_team_goal(&TeamGoalRecord {
                goal_id: "goal-ops".to_string(),
                team_owner_agent_id: perry_id.clone(),
                title: "Operational throughput".to_string(),
                summary: "Keep delegation moving without narration-only completion.".to_string(),
                status: "ACTIVE".to_string(),
                priority: "HIGH".to_string(),
                success_criteria: "Every manager SWO ends in a real output or reviewed escalation."
                    .to_string(),
                managed_domain_tags: vec!["operations".to_string()],
                created_at: String::new(),
                updated_at: String::new(),
                archived_at: None,
            })
            .unwrap();
        assert_eq!(goal.goal_id, "goal-ops");

        let updated = registry
            .upsert_agent_org_profile(&AgentOrgProfileRecord {
                team_goal_ids: vec![goal.goal_id.clone()],
                managed_domains: vec!["operations".to_string(), "management".to_string()],
                ..profile
            })
            .unwrap();
        assert_eq!(updated.team_goal_ids, vec!["goal-ops".to_string()]);
        assert_eq!(updated.managed_domains.len(), 2);

        let goals = registry.list_team_goals_for_agent(&perry_id).unwrap();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].title, "Operational throughput");
    }

    #[test]
    fn delegation_decisions_and_team_gaps_persist() {
        let registry = test_registry();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        let swo_id = registry
            .create_swo_with_metadata(CreateSwoParams {
                assigned_agent_id: &perry_id,
                owner_agent_id: &perry_id,
                created_by_agent_id: &perry_id,
                payload: "Route this marketing task".into(),
                status: "IN_PROGRESS",
                parent_swo_id: None,
                kind: "TASK",
                source: "TEST",
                work_order_title: Some("Manager routing"),
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
            })
            .unwrap();

        let decision = registry
            .record_delegation_decision(&DelegationDecisionRecord {
                id: "decision-1".to_string(),
                swo_id,
                manager_agent_id: perry_id.clone(),
                decision: "ESCALATE_TEAM_GAP".to_string(),
                candidate_assignees: Vec::new(),
                selected_agent_id: None,
                fit_reason: Some("No qualified direct report.".to_string()),
                exception_code: None,
                exception_reason: None,
                team_gap_code: Some("NO_QUALIFIED_REPORT".to_string()),
                created_at: String::new(),
            })
            .unwrap();
        assert_eq!(decision.decision, "ESCALATE_TEAM_GAP");

        let gap = registry
            .record_team_gap(&TeamGapRecord {
                id: "gap-1".to_string(),
                swo_id,
                manager_agent_id: perry_id.clone(),
                gap_code: "NO_QUALIFIED_REPORT".to_string(),
                summary: "No marketing direct report exists.".to_string(),
                recommended_action: "HIRE_THEN_DELEGATE".to_string(),
                created_at: String::new(),
            })
            .unwrap();
        assert_eq!(gap.recommended_action, "HIRE_THEN_DELEGATE");

        assert_eq!(
            registry
                .list_delegation_decisions_for_swo(swo_id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(registry.list_team_gaps_for_swo(swo_id).unwrap().len(), 1);
    }

    #[test]
    fn duplicate_agent_name_rejected_by_create_agent() {
        let registry = test_registry();
        let manager_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        assert!(!manager_id.is_empty());

        // create_agent must reject a duplicate name.
        let result = registry.create_agent(
            "Perry",
            None,
            "COO",
            "Operate again",
            "Operate again",
            "mock",
            "mock",
        );
        assert!(
            result.is_err(),
            "create_agent with a duplicate name should return an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("already exists"),
            "Error should mention the agent already exists, got: {}",
            err_msg
        );

        // Verify only one agent row exists.
        assert_eq!(registry.count_agents().unwrap(), 1);
    }

    #[test]
    fn duplicate_agent_name_rejected_by_unique_index() {
        // Exercise the DB-level unique index directly via insert_agent_identity.
        let registry = test_registry();
        registry
            .insert_agent_identity(
                "id-1", "SameName", None, "Engineer", "Build", "Build", "mock", "mock", None, None, None,
            )
            .unwrap();

        let result = registry.insert_agent_identity(
            "id-2", "SameName", None, "Engineer", "Build", "Build", "mock", "mock", None, None, None,
        );
        assert!(
            result.is_err(),
            "insert_agent_identity with a duplicate name should fail"
        );
        assert_eq!(registry.count_agents().unwrap(), 1);
    }

    #[test]
    fn update_agent_model_does_not_duplicate() {
        let registry = test_registry();
        let agent_id = registry
            .hire_subordinate("Felicity", None, "CTO", "Build things", "openai", "gpt-4o")
            .unwrap();
        assert_eq!(registry.count_agents().unwrap(), 1);

        // Simulate a model change via update_agent_manifest_profile.
        use crate::manifest::{AgentManifestV1, ProviderConfigV1, ScheduleSpec, ProviderProtocolFamily};
        let manifest = AgentManifestV1 {
            agent_id: Some(agent_id.clone()),
            name: "Felicity".to_string(),
            role: "CTO".to_string(),
            persona_prompt: "Build things".to_string(),
            mission: "Build things".to_string(),
            provider: ProviderConfigV1 {
                provider_name: "anthropic".to_string(),
                model: "claude-sonnet-4-20250514".to_string(),
                protocol_family: ProviderProtocolFamily::AnthropicCompatible,
                triage_model: None,
                execution_model: None,
            },
            schedule: ScheduleSpec {
                cron_interval_seconds: None,
                autonomous_heartbeat: false,
            },
            version: "1".to_string(),
            capabilities: vec![],
            guardrails: vec![],
        };
        registry.update_agent_manifest_profile(&manifest).unwrap();

        // Still exactly one agent — no duplicate created.
        assert_eq!(registry.count_agents().unwrap(), 1);

        // Verify the model actually changed.
        let agents = registry.list_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].default_model, "claude-sonnet-4-20250514");
        assert_eq!(agents[0].default_provider, "anthropic");
    }

    #[test]
    fn pulse_journal_append_and_query() {
        let registry = test_registry();
        let agent_id = registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();

        // Append a journal entry.
        let entry_id = registry
            .append_pulse_journal_entry(
                "dawn",
                Some("run-001"),
                &agent_id,
                "step_started",
                "Dawn pulse started",
                Some(r#"{"key":"value"}"#),
            )
            .unwrap();
        assert!(entry_id > 0);

        // Append a second entry for a different cadence.
        registry
            .append_pulse_journal_entry(
                "dusk",
                None,
                &agent_id,
                "observation",
                "Dusk observation",
                None,
            )
            .unwrap();

        // list_pulse_journal — all entries (no cadence filter).
        let all = registry.list_pulse_journal(None, 10).unwrap();
        assert_eq!(all.len(), 2);

        // list_pulse_journal — filtered by cadence.
        let dawn_entries = registry.list_pulse_journal(Some("dawn"), 10).unwrap();
        assert_eq!(dawn_entries.len(), 1);
        assert_eq!(dawn_entries[0].cadence, "dawn");
        assert_eq!(dawn_entries[0].run_id.as_deref(), Some("run-001"));
        assert_eq!(dawn_entries[0].entry_type, "step_started");
        assert_eq!(dawn_entries[0].summary, "Dawn pulse started");
        assert!(dawn_entries[0].detail_json.is_some());

        // get_latest_pulse_entry.
        let latest = registry.get_latest_pulse_entry("dawn").unwrap();
        assert!(latest.is_some());
        let latest = latest.unwrap();
        assert_eq!(latest.summary, "Dawn pulse started");

        // Missing cadence returns None.
        let missing = registry.get_latest_pulse_entry("heartbeat").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn cadence_state_upsert_and_due_list() {
        let registry = test_registry();

        // Initially empty.
        assert!(registry.list_cadence_states().unwrap().is_empty());
        assert!(registry.list_due_cadence_domains().unwrap().is_empty());

        // Insert a domain — last_checked_at is NULL so it is immediately due.
        // We achieve NULL by setting check_interval_hours to 0 after upsert and
        // relying on the fact that newly inserted records have last_checked_at = NOW.
        // Instead, insert a domain with a very large interval so it is NOT due after upsert.
        registry
            .upsert_cadence_state("crm", 999, Some("run-crm-01"))
            .unwrap();

        let states = registry.list_cadence_states().unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].domain, "crm");
        assert_eq!(states[0].check_interval_hours, 999);
        assert_eq!(states[0].last_run_id.as_deref(), Some("run-crm-01"));

        // A domain with interval=0 should always be due.
        registry
            .upsert_cadence_state("content", 0, None)
            .unwrap();
        let due = registry.list_due_cadence_domains().unwrap();
        // "content" (interval=0) is always due; "crm" (interval=999) should not be.
        assert!(due.iter().any(|r| r.domain == "content"));
        assert!(!due.iter().any(|r| r.domain == "crm"));

        // Re-upsert crm updates interval and run_id.
        registry
            .upsert_cadence_state("crm", 0, Some("run-crm-02"))
            .unwrap();
        let due2 = registry.list_due_cadence_domains().unwrap();
        assert!(due2.iter().any(|r| r.domain == "crm"));

        let all = registry.list_cadence_states().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn escalations_record_and_query_by_parent_agent() {
        let registry = test_registry();

        // Set up a manager (parent) and a child agent.
        let parent_id = registry
            .hire_subordinate("Minerva", None, "VP Engineering", "Lead all engineering", "mock", "mock")
            .unwrap();
        let child_id = registry
            .hire_subordinate("Felix", Some(&parent_id), "Engineer", "Build software", "mock", "mock")
            .unwrap();

        // Create a parent SWO assigned to the parent manager.
        let parent_swo_id = registry
            .create_swo_with_metadata(CreateSwoParams {
                assigned_agent_id: &parent_id,
                owner_agent_id: &parent_id,
                created_by_agent_id: &parent_id,
                payload: "Parent task: deliver the feature",
                status: "IN_PROGRESS",
                parent_swo_id: None,
                kind: "TASK",
                source: "HSM",
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
            })
            .unwrap();

        // Create a child SWO assigned to the child agent, parented to parent SWO.
        let child_swo_id = registry
            .create_swo_with_metadata(CreateSwoParams {
                assigned_agent_id: &child_id,
                owner_agent_id: &parent_id,
                created_by_agent_id: &parent_id,
                payload: "Child task: implement the auth module",
                status: "IN_PROGRESS",
                parent_swo_id: Some(parent_swo_id),
                kind: "TASK",
                source: "HSM",
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
            })
            .unwrap();

        // Record the escalation as the orchestrator ceiling path would.
        let escalation_id = registry
            .record_escalation(
                child_swo_id,
                &child_id,
                Some(parent_swo_id),
                Some(&parent_id),
                3,
                "Child failed to pass synthesis after 3 revision attempts",
            )
            .unwrap();
        assert!(!escalation_id.is_empty());

        // Query by parent agent — should surface the escalation.
        let escalations = registry
            .list_recent_escalations_for_agent(&parent_id, 10)
            .unwrap();
        assert_eq!(escalations.len(), 1);
        let esc = &escalations[0];
        assert_eq!(esc.swo_id, child_swo_id);
        assert_eq!(esc.child_agent_id, child_id);
        assert_eq!(esc.parent_swo_id, Some(parent_swo_id));
        assert_eq!(esc.parent_agent_id.as_deref(), Some(parent_id.as_str()));
        assert_eq!(esc.attempts, 3);
        assert!(esc.reasoning.contains("revision attempts"));

        // Query by child agent (not the parent) — should return nothing.
        let empty = registry
            .list_recent_escalations_for_agent(&child_id, 10)
            .unwrap();
        assert!(empty.is_empty());

        // Audit event round-trip: emit one and verify it chains.
        let audit_id = registry
            .record_audit_event(
                Some(&child_id),
                Some(child_swo_id),
                "escalation_reported",
                TaintLabel::ManagerEscalation,
                &serde_json::json!({
                    "swo_id": child_swo_id,
                    "child_agent_id": child_id,
                    "attempts": 3,
                    "reasoning": "test escalation audit event",
                }),
            )
            .unwrap();
        assert!(audit_id > 0);
        let events = registry.list_audit_events(5).unwrap();
        assert!(!events.is_empty());
        assert_eq!(events[0].event_kind, "escalation_reported");
        assert_eq!(events[0].taint_label, TaintLabel::ManagerEscalation);
    }

    #[test]
    fn check_autonomous_hire_allowed_perry_only_mode() {
        // CHA-427 — PERRY_ONLY mode: only the agent named "Perry" can hire.
        let registry = test_registry();
        registry
            .upsert_runtime_metadata("autonomous_hiring_mode", "PERRY_ONLY")
            .unwrap();

        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Coordinate", "mock", "mock")
            .unwrap();
        let felicity_id = registry
            .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
            .unwrap();

        // Perry is allowed
        registry.check_autonomous_hire_allowed(&perry_id).unwrap();
        // Felicity (also a Manager, but not Perry) is blocked under PERRY_ONLY
        let err = registry.check_autonomous_hire_allowed(&felicity_id).unwrap_err();
        assert!(err.to_string().contains("PERRY_ONLY"));
        assert!(err.to_string().contains("Felicity"));
    }

    #[test]
    fn check_autonomous_hire_allowed_any_manager_mode() {
        // CHA-427 — ANY_MANAGER mode: any agent whose org_class is Manager can hire.
        let registry = test_registry();
        registry
            .upsert_runtime_metadata("autonomous_hiring_mode", "ANY_MANAGER")
            .unwrap();

        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Coordinate", "mock", "mock")
            .unwrap();
        // Felicity gets org_class=Manager from the role "CTO" C-suite inference
        let felicity_id = registry
            .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
            .unwrap();
        // Generic specialist (not a manager by role inference)
        let lois_id = registry
            .hire_subordinate(
                "Lois",
                Some(&perry_id),
                "Research Analyst",
                "Research",
                "mock",
                "mock",
            )
            .unwrap();

        registry.check_autonomous_hire_allowed(&perry_id).unwrap();
        registry.check_autonomous_hire_allowed(&felicity_id).unwrap();
        let err = registry.check_autonomous_hire_allowed(&lois_id).unwrap_err();
        assert!(err.to_string().contains("ANY_MANAGER"));
        assert!(err.to_string().contains("Lois"));
    }

    #[test]
    fn check_autonomous_hire_allowed_open_mode() {
        // CHA-427 — OPEN mode: any agent with HireSubordinate capability can hire
        // (capability gate is applied upstream in the orchestrator; registry check
        // only enforces mode + per-manager cap).
        let registry = test_registry();
        registry
            .upsert_runtime_metadata("autonomous_hiring_mode", "OPEN")
            .unwrap();

        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Coordinate", "mock", "mock")
            .unwrap();
        let lois_id = registry
            .hire_subordinate(
                "Lois",
                Some(&perry_id),
                "Research Analyst",
                "Research",
                "mock",
                "mock",
            )
            .unwrap();

        registry.check_autonomous_hire_allowed(&perry_id).unwrap();
        registry.check_autonomous_hire_allowed(&lois_id).unwrap();
    }

    #[test]
    fn check_autonomous_hire_allowed_per_manager_cap() {
        // CHA-427 — the per-manager direct-reports cap applies in every mode.
        let registry = test_registry();
        registry
            .upsert_runtime_metadata("autonomous_hiring_mode", "ANY_MANAGER")
            .unwrap();
        // Lower the cap to 2 for test purposes.
        registry
            .upsert_runtime_metadata("max_direct_reports_per_manager", "2")
            .unwrap();

        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Coordinate", "mock", "mock")
            .unwrap();
        // Perry currently has 0 direct reports — allowed.
        registry.check_autonomous_hire_allowed(&perry_id).unwrap();

        // Hire two direct reports under Perry, bringing him to the cap.
        registry
            .hire_subordinate("Sub1", Some(&perry_id), "CTO", "Build1", "mock", "mock")
            .unwrap();
        registry
            .hire_subordinate("Sub2", Some(&perry_id), "CTO", "Build2", "mock", "mock")
            .unwrap();

        // Third hire attempt must be rejected.
        let err = registry.check_autonomous_hire_allowed(&perry_id).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("2"), "expected cap mention: {}", msg);
        assert!(
            msg.contains("per-manager cap") || msg.contains("max_direct_reports_per_manager"),
            "expected cap language: {}",
            msg
        );
    }

    #[test]
    fn check_autonomous_hire_allowed_unknown_mode_rejects() {
        let registry = test_registry();
        registry
            .upsert_runtime_metadata("autonomous_hiring_mode", "BOGUS_MODE")
            .unwrap();
        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Coordinate", "mock", "mock")
            .unwrap();
        let err = registry.check_autonomous_hire_allowed(&perry_id).unwrap_err();
        assert!(err.to_string().contains("BOGUS_MODE"));
        assert!(err.to_string().contains("PERRY_ONLY, ANY_MANAGER, OPEN"));
    }

    #[test]
    fn check_autonomous_hire_allowed_defaults_to_any_manager_when_unset() {
        // CHA-427 — when runtime metadata is unset, default behavior must be
        // ANY_MANAGER (the most common expected case). This prevents existing
        // databases without the setting from locking everyone out.
        let registry = test_registry();

        let perry_id = registry
            .hire_subordinate("Perry", None, "COO", "Coordinate", "mock", "mock")
            .unwrap();
        let felicity_id = registry
            .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
            .unwrap();

        registry.check_autonomous_hire_allowed(&perry_id).unwrap();
        registry.check_autonomous_hire_allowed(&felicity_id).unwrap();
    }

    #[test]
    fn is_ancestor_of_walks_org_tree() {
        let registry = test_registry();
        let perry = registry
            .hire_subordinate("Perry", None, "COO", "Coordinate", "mock", "mock")
            .unwrap();
        let felicity = registry
            .hire_subordinate("Felicity", Some(&perry), "CTO", "Build", "mock", "mock")
            .unwrap();
        let alex = registry
            .hire_subordinate("Alex", Some(&felicity), "Frontend Dev", "Ship UI", "mock", "mock")
            .unwrap();
        let lois = registry
            .hire_subordinate("Lois", Some(&perry), "Analyst", "Research", "mock", "mock")
            .unwrap();

        // Identity
        assert!(registry.is_ancestor_of(&perry, &perry).unwrap());
        // Direct parent
        assert!(registry.is_ancestor_of(&perry, &felicity).unwrap());
        // Transitive ancestor
        assert!(registry.is_ancestor_of(&perry, &alex).unwrap());
        // Felicity is ancestor of Alex but not Lois
        assert!(registry.is_ancestor_of(&felicity, &alex).unwrap());
        assert!(!registry.is_ancestor_of(&felicity, &lois).unwrap());
        // Alex is NOT ancestor of Perry (reverse)
        assert!(!registry.is_ancestor_of(&alex, &perry).unwrap());
    }

    #[test]
    fn check_cross_manager_hire_perry_can_place_under_felicity() {
        // CHA-428 — Perry (root) is authorized to hire new agents under Felicity,
        // and the per-manager cap applies to Felicity's direct reports, not Perry's.
        let registry = test_registry();
        registry
            .upsert_runtime_metadata("autonomous_hiring_mode", "ANY_MANAGER")
            .unwrap();
        registry
            .upsert_runtime_metadata("max_direct_reports_per_manager", "3")
            .unwrap();

        let perry = registry
            .hire_subordinate("Perry", None, "COO", "Coordinate", "mock", "mock")
            .unwrap();
        let felicity = registry
            .hire_subordinate("Felicity", Some(&perry), "CTO", "Build", "mock", "mock")
            .unwrap();

        // Fill Perry's direct reports to the cap (Felicity already counts as 1)
        registry
            .hire_subordinate("Cat Grant", Some(&perry), "CMO", "Brand", "mock", "mock")
            .unwrap();
        registry
            .hire_subordinate("Lex", Some(&perry), "CFO", "Finance", "mock", "mock")
            .unwrap();
        // Perry now has 3 direct reports (at the cap)

        // Perry trying to hire under himself should be blocked by the cap
        let self_err = registry
            .check_cross_manager_hire_allowed(&perry, &perry)
            .unwrap_err();
        assert!(self_err.to_string().contains("Perry"));
        assert!(self_err.to_string().contains("cap"));

        // But Perry hiring for Felicity (who has 0 reports) should be allowed
        registry
            .check_cross_manager_hire_allowed(&perry, &felicity)
            .unwrap();
    }

    #[test]
    fn check_cross_manager_hire_blocks_unauthorized_placement() {
        // CHA-428 — a non-root caller cannot place hires under another
        // manager unless they are an ancestor of that target.
        let registry = test_registry();
        registry
            .upsert_runtime_metadata("autonomous_hiring_mode", "ANY_MANAGER")
            .unwrap();

        let perry = registry
            .hire_subordinate("Perry", None, "COO", "Coordinate", "mock", "mock")
            .unwrap();
        let felicity = registry
            .hire_subordinate("Felicity", Some(&perry), "CTO", "Build", "mock", "mock")
            .unwrap();
        let cat = registry
            .hire_subordinate("Cat Grant", Some(&perry), "CMO", "Brand", "mock", "mock")
            .unwrap();

        // Felicity hiring under herself is fine
        registry
            .check_cross_manager_hire_allowed(&felicity, &felicity)
            .unwrap();

        // Felicity trying to hire under Cat Grant (her peer) should be blocked
        let err = registry
            .check_cross_manager_hire_allowed(&felicity, &cat)
            .unwrap_err();
        assert!(err.to_string().contains("not authorized"));
        assert!(err.to_string().contains("ancestor"));
    }

    #[test]
    fn check_cross_manager_hire_blocks_placement_under_specialist() {
        // CHA-428 — a target manager must itself be a Manager org class.
        // Placing a hire under a specialist makes no org-structural sense.
        let registry = test_registry();
        registry
            .upsert_runtime_metadata("autonomous_hiring_mode", "ANY_MANAGER")
            .unwrap();

        let perry = registry
            .hire_subordinate("Perry", None, "COO", "Coordinate", "mock", "mock")
            .unwrap();
        let lois = registry
            .hire_subordinate("Lois", Some(&perry), "Analyst", "Research", "mock", "mock")
            .unwrap();

        // Lois's default org_class is specialist (role does not match manager
        // inference). Perry can normally hire, but not UNDER Lois.
        let err = registry
            .check_cross_manager_hire_allowed(&perry, &lois)
            .unwrap_err();
        assert!(err.to_string().contains("not a Manager"));
    }

    #[test]
    fn hire_subordinate_refreshes_persona_on_reseed() {
        // CHA-426: re-running the seed with a modified persona must update
        // existing agent records, not silently keep the first-boot persona.
        let registry = test_registry();

        // First hire — creates the agent.
        let original_id = registry
            .hire_subordinate_with_profile_and_cron(
                "Felicity",
                None,
                "CTO",
                "Old persona — delivery systems lead for Syllogism.",
                "Old raison — delivery runbooks and operational guardrails.",
                "mock",
                "mock",
                None,
                None,
                None,
            )
            .unwrap();

        let agent_before = registry.get_agent(&original_id).unwrap();
        assert!(agent_before.persona_prompt.contains("delivery systems lead"));
        assert!(agent_before
            .raison_detre
            .contains("delivery runbooks"));

        // Second hire with the SAME NAME but a new persona — simulates
        // the seed JSON being edited and the kernel restarted.
        let returned_id = registry
            .hire_subordinate_with_profile_and_cron(
                "Felicity",
                None,
                "CTO & Lead Engineer",
                "New persona — hands-on CTO who writes code directly.",
                "New raison — build working software, not runbooks about software.",
                "mock",
                "mock",
                None,
                None,
                None,
            )
            .unwrap();

        // Same ID — idempotent by name.
        assert_eq!(original_id, returned_id);

        // But the persona/role/raison must now be updated.
        let agent_after = registry.get_agent(&original_id).unwrap();
        assert_eq!(agent_after.role, "CTO & Lead Engineer");
        assert!(agent_after.persona_prompt.contains("hands-on CTO"));
        assert!(agent_after.persona_prompt.contains("writes code directly"));
        assert!(agent_after.raison_detre.contains("build working software"));
        // Old persona strings must be gone.
        assert!(!agent_after.persona_prompt.contains("delivery systems lead"));
        assert!(!agent_after.raison_detre.contains("delivery runbooks"));
    }
}
