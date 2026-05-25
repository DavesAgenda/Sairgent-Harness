// ---------------------------------------------------------------------------
// Sairgent Workspace — Lean Tauri Backend
// ---------------------------------------------------------------------------
// 27 commands: boot (3), runtime (3), work (4), settings (3), agent (3), discovery (1), token (2), artifacts (2), cli-tools (3), mcp (5), recurring-templates (2)
// Ported from apps/desktop/src-tauri/src/lib.rs with minimal surface area.
// ---------------------------------------------------------------------------

use keyring::{Entry, Error as KeyringError};
use tauri::Manager;
use lru::LruCache;
use sairgent_kernel::audit::TaintLabel;
use sairgent_kernel::kernel::Kernel;
use sairgent_kernel::orchestrator::KernelEvent;
use sairgent_kernel::registry::{
    AgentDetailRecord, AgentSwoSummaryRecord, AgentTreeNodeRecord, ActiveSwoRecord,
    InboxAttentionSummaryRecord, InboxItemRecord, ProjectRecord,
    TokenUsageRecord as KernelTokenUsageRecord, AgentTokenTotals as KernelAgentTokenTotals,
    RecurringWorkOrderTemplateRecord,
};
use sairgent_kernel::tools::{
    McpConnectorRecord, McpConnectorUpsertRequest as KernelMcpConnectorUpsertRequest,
};
use sairgent_kernel::seed::RuntimeContext;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, RwLock};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const KEYRING_SERVICE: &str = "com.sairgent.deck.v2";
const KEY_API_KEY: &str = "api_key";
const KEY_SIDECHANNEL_TOKEN: &str = "sidechannel_token";
const KEY_SECURE_SETTINGS_BUNDLE: &str = "secure_settings_bundle";
const DEFAULT_LLM_PROVIDER: &str = "anthropic";
const DEFAULT_LLM_MODEL: &str = "";
const WORKSPACE_OPERATOR_NAME: &str = "Workspace Operator";
const LEGACY_VAULT_KEY: &str = "dummy_vault_key_that_is_32_bytes";
const VAULT_KEY_KEYRING_ACCOUNT: &str = "vault_key";

const MAX_API_KEY_LEN: usize = 4096;
const MAX_SIDECHANNEL_TOKEN_LEN: usize = 1024;
const MAX_SLUG_LEN: usize = 64;
const MAX_LABEL_LEN: usize = 80;
const MAX_ENV_VAR_LEN: usize = 80;

// ---------------------------------------------------------------------------
// AppState — same pattern as desktop
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AppState {
    kernel: Arc<Mutex<Option<Arc<Kernel>>>>,
    perry_id: Arc<Mutex<Option<String>>>,
    runtime_bus: Arc<RuntimeBusState>,
    last_hsm_status: Arc<StdMutex<String>>,
    processed_command_ids: Arc<StdMutex<LruCache<String, i64>>>,
    bootstrap_cache: Arc<Mutex<Option<CachedBootstrap>>>,
    model_discovery_cache: Arc<StdMutex<HashMap<String, (Vec<String>, Instant)>>>,
}

struct RuntimeBusState {
    next_cursor: StdMutex<u64>,
    event_log: StdMutex<Vec<RuntimeSignalView>>,
}

impl RuntimeBusState {
    fn new() -> Self {
        Self {
            next_cursor: StdMutex::new(0),
            event_log: StdMutex::new(Vec::new()),
        }
    }
}

#[derive(Clone)]
struct CachedBootstrap {
    data: RuntimeBootstrapView,
    #[allow(dead_code)]
    cached_at: u64,
}

// ---------------------------------------------------------------------------
// View types — serialized to the frontend
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePrincipalView {
    kind: String,
    id: Option<String>,
    display_name: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEnvelopeView {
    pub id: String,
    pub correlation_id: String,
    pub source: String,
    pub principal: RuntimePrincipalView,
    pub audience: String,
    pub redaction_class: String,
    pub occurred_at: i64,
    pub cursor: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSignalView {
    pub envelope: RuntimeEnvelopeView,
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCursorView {
    pub value: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeBootstrapView {
    pub cursor: RuntimeCursorView,
    pub hsm_status: String,
    pub runtime_context: Option<RuntimeContextView>,
    pub queue: Vec<SwoRecordView>,
    pub roster: Vec<AgentTreeNodeView>,
    pub approvals: Vec<ApprovalQueueItemView>,
    pub recent_artifacts: Vec<OutboxArtifactView>,
    pub attention_summary: InboxAttentionSummaryView,
    pub projects: Vec<ProjectView>,
    pub inbox_items: Vec<InboxItemView>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeContextView {
    pub company_name: Option<String>,
    pub profile_id: Option<String>,
    pub company_charter_source: Option<String>,
    pub company_summary: Option<String>,
    pub autonomous_hiring_mode: Option<String>,
    pub active_seed_spec_path: Option<String>,
    pub last_archive_path: Option<String>,
    pub sairgent_agent_id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwoRecordView {
    pub id: i64,
    pub assignee: String,
    pub owner: String,
    pub created_by: String,
    pub status: String,
    pub kind: String,
    pub source: String,
    pub work_order_title: Option<String>,
    pub work_order_outcome: Option<String>,
    pub work_order_constraints: Option<String>,
    pub requested_owner: Option<String>,
    pub requested_assignee: Option<String>,
    pub routing_policy: String,
    pub initiative_id: Option<String>,
    pub initiative_name: Option<String>,
    pub initiative_owner: Option<String>,
    pub priority_class: Option<String>,
    pub payload: String,
    pub created_at: String,
    pub retry_count: i32,
    pub actual_child_assignees: Vec<String>,
    pub child_swo_count: usize,
    pub review_status: String,
    pub mismatch_flags: Vec<String>,
    pub parent_swo_id: Option<i64>,
    pub review_response: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalQueueItemView {
    pub id: String,
    pub swo_id: i64,
    pub title: String,
    pub reason: String,
    pub owner: String,
    pub status: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxArtifactView {
    pub id: i64,
    pub agent: String,
    pub agent_id: Option<String>,
    pub swo_id: Option<i64>,
    pub parent_swo_id: Option<i64>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub source_work_order_title: Option<String>,
    pub source_work_order_outcome: Option<String>,
    pub source_status: Option<String>,
    pub absolute_path: String,
    pub filename: String,
    pub created_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItemView {
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
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxAttentionSummaryView {
    pub open_inbox_items: i64,
    pub open_approval_items: i64,
    pub open_deliverable_items: i64,
    pub open_blocked_items: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub project_id: String,
    pub name: String,
    pub summary: String,
    pub status: String,
    pub owner: String,
    pub priority: String,
    pub target_outcome: String,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummaryView {
    pub id: String,
    pub name: String,
    pub role: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOrgProfileView {
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTreeNodeView {
    pub id: String,
    pub name: String,
    pub role: String,
    pub manager: Option<AgentSummaryView>,
    pub org_profile: AgentOrgProfileView,
    pub depth: usize,
    pub is_direct_report: bool,
    pub direct_report_count: usize,
    pub descendant_count: usize,
    pub cron_enabled: bool,
    pub presence: String,
    pub last_seen_unix_ms: Option<i64>,
    pub last_seen_age_ms: Option<i64>,
    pub last_cron_fired_at: Option<String>,
    pub children: Vec<AgentTreeNodeView>,
    pub default_provider: String,
    pub model: String,
    pub triage_model: Option<String>,
    pub execution_model: Option<String>,
    pub raison_detre: String,
    pub persona_prompt: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatEventView {
    pub run_id: String,
    pub status: String,
    pub last_seen_unix_ms: i64,
    pub last_seen_age_ms: i64,
    pub seq: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectReportSummaryView {
    pub id: String,
    pub name: String,
    pub role: String,
    pub cron_enabled: bool,
    pub presence: String,
    pub last_seen_unix_ms: Option<i64>,
    pub last_seen_age_ms: Option<i64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamGoalView {
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

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationDecisionRecordView {
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

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamGapRecordView {
    pub id: String,
    pub swo_id: i64,
    pub manager_agent_id: String,
    pub gap_code: String,
    pub summary: String,
    pub recommended_action: String,
    pub created_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillBindingView {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub trigger_hints: Vec<String>,
    pub source_uri: Option<String>,
    pub current_version: i64,
    pub priority: i64,
    pub binding_status: String,
    pub preselected: bool,
    pub runtime_path: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolBindingView {
    pub slug: String,
    pub name: String,
    pub summary: String,
    pub tool_kind: String,
    pub provider_slug: String,
    pub required_capability: String,
    pub binding_status: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMcpBindingView {
    connector_id: String,
    connector_slug: String,
    connector_name: String,
    transport: String,
    binding_status: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentManifestView {
    pub version: String,
    pub name: String,
    pub role: String,
    pub mission: String,
    pub persona_prompt: String,
    pub provider_name: String,
    pub model: String,
    pub protocol_family: String,
    pub capabilities: Vec<String>,
    pub cron_interval_seconds: Option<i64>,
    pub autonomous_heartbeat: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharterSettingsView {
    pub raison_detre: String,
    pub provider: String,
    pub model: String,
    pub cron_interval_seconds: Option<i64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDetailView {
    pub id: String,
    pub name: String,
    pub role: String,
    pub manager: Option<AgentSummaryView>,
    pub org_profile: AgentOrgProfileView,
    pub team_goals: Vec<TeamGoalView>,
    pub delegation_decisions: Vec<DelegationDecisionRecordView>,
    pub team_gaps: Vec<TeamGapRecordView>,
    pub direct_reports: Vec<DirectReportSummaryView>,
    pub persona_prompt: String,
    pub raison_detre: String,
    pub provider: String,
    pub model: String,
    pub triage_model: Option<String>,
    pub execution_model: Option<String>,
    pub cron_interval_seconds: Option<i64>,
    pub presence: String,
    pub last_seen_unix_ms: Option<i64>,
    pub last_seen_age_ms: Option<i64>,
    pub last_cron_fired_at: Option<String>,
    pub heartbeat_timeline: Vec<HeartbeatEventView>,
    pub assigned_swos: Vec<SwoRecordView>,
    pub owned_swos: Vec<SwoRecordView>,
    pub created_swos: Vec<SwoRecordView>,
    pub charter_settings: CharterSettingsView,
    pub manifest: AgentManifestView,
    pub bound_skills: Vec<SkillBindingView>,
    pub bound_tools: Vec<AgentToolBindingView>,
    pub mcp_bindings: Vec<AgentMcpBindingView>,
}

// ---------------------------------------------------------------------------
// Artifact preview types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactPreviewView {
    pub artifact_id: i64,
    pub filename: String,
    pub content_type: String,
    pub render_mode: String,
    pub content: String,
    pub size_bytes: i64,
    pub truncated: bool,
}

const MAX_ARTIFACT_PREVIEW_BYTES: u64 = 256 * 1024;

fn infer_artifact_content_type(path: &Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("md") | Some("markdown") => "text/markdown",
        Some("txt") | Some("log") => "text/plain",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("yaml") | Some("yml") => "application/yaml",
        Some("html") | Some("htm") => "text/html",
        Some("rs") | Some("ts") | Some("tsx") | Some("js") | Some("jsx") | Some("py")
        | Some("go") => "text/plain",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn is_text_previewable(content_type: &str) -> bool {
    content_type.starts_with("text/")
        || matches!(content_type, "application/json" | "application/yaml")
}

fn artifact_render_mode(content_type: &str) -> &'static str {
    if content_type.contains("markdown") {
        return "markdown";
    }
    if content_type.contains("json") {
        return "json";
    }
    if content_type.starts_with("text/") {
        return "text";
    }
    "binary"
}

fn read_artifact_preview(path: &Path) -> Result<(String, bool), String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read artifact: {}", e))?;
    let truncated = bytes.len() as u64 > MAX_ARTIFACT_PREVIEW_BYTES;
    let slice = if truncated {
        &bytes[..MAX_ARTIFACT_PREVIEW_BYTES as usize]
    } else {
        &bytes
    };
    let text = String::from_utf8_lossy(slice).to_string();
    Ok((text, truncated))
}

// ---------------------------------------------------------------------------
// Settings types
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
struct ServiceCredentialConfig {
    slug: String,
    label: String,
    env_var: String,
    enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
struct AppConfig {
    llm_api_key: String,
    default_llm_provider: String,
    default_llm_model: String,
    llm_credentials: Vec<ServiceCredentialConfig>,
    tool_credentials: Vec<ServiceCredentialConfig>,
    sidechannel_token: String,
    sairgent_agent_provider: String,
    sairgent_agent_model: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
struct SecureSettingsBundle {
    legacy_api_key: String,
    llm_api_keys_by_provider: HashMap<String, String>,
    tool_api_keys_by_slug: HashMap<String, String>,
    sidechannel_token: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct ServiceCredentialView {
    slug: String,
    label: String,
    env_var: String,
    enabled: bool,
    has_secret: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    default_llm_provider: String,
    default_llm_model: String,
    llm_credentials: Vec<ServiceCredentialView>,
    tool_credentials: Vec<ServiceCredentialView>,
    has_sidechannel_token: bool,
    has_bootable_credentials: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct ServiceCredentialInput {
    slug: String,
    label: String,
    env_var: String,
    enabled: bool,
    api_key: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct SettingsSaveRequest {
    default_llm_provider: String,
    default_llm_model: String,
    llm_credentials: Vec<ServiceCredentialInput>,
    tool_credentials: Vec<ServiceCredentialInput>,
    sidechannel_token: Option<String>,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RuntimeCommandRequest {
    kind: String,
    payload: serde_json::Value,
    command_id: String,
    correlation_id: String,
    source: String,
    principal: RuntimePrincipalView,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RuntimeReplayRequest {
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeCommandMetaInput {
    command_id: String,
    correlation_id: String,
    source: String,
    principal: RuntimePrincipalView,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueReviewDecisionRequest {
    swo_id: i64,
    decision: String,
    reasoning: String,
    final_response: Option<String>,
    meta: RuntimeCommandMetaInput,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitWorkOrderRequest {
    title: String,
    outcome: String,
    constraints: Option<String>,
    priority: String,
    project_id: Option<String>,
    parent_swo_id: Option<i64>,
    requested_owner: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestRevisionSwoRequest {
    swo_id: i64,
    feedback: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecretsSetRequest {
    provider: String,
    key: String,
}

// ---------------------------------------------------------------------------
// Audience / redaction enums
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Clone, Copy)]
enum RuntimeAudienceValue {
    Desktop,
    Operator,
    Internal,
    ExternalAdapter,
}

impl RuntimeAudienceValue {
    fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Operator => "operator",
            Self::Internal => "internal",
            Self::ExternalAdapter => "external_adapter",
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum RuntimeRedactionClassValue {
    OperatorSafe,
    InternalOnly,
    SecretAdjacent,
}

impl RuntimeRedactionClassValue {
    fn as_str(self) -> &'static str {
        match self {
            Self::OperatorSafe => "operator_safe",
            Self::InternalOnly => "internal_only",
            Self::SecretAdjacent => "secret_adjacent",
        }
    }
}

// ---------------------------------------------------------------------------
// Utility functions — ported from desktop lib.rs
// ---------------------------------------------------------------------------

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn resolve_project_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("CWD error: {:?}", e))?;
    cwd.join("..")
        .join("..")
        .join("..")
        .canonicalize()
        .map_err(|e| format!("Cannot resolve project root: {:?}", e))
}

fn default_seed_spec_path(project_root: &Path) -> PathBuf {
    project_root
        .join("00_Context")
        .join("Seeds")
        .join("default_seed.json")
}

fn workspace_operator_principal() -> RuntimePrincipalView {
    RuntimePrincipalView {
        kind: "operator".to_string(),
        id: Some("workspace-operator".to_string()),
        display_name: Some(WORKSPACE_OPERATOR_NAME.to_string()),
    }
}

fn system_runtime_principal() -> RuntimePrincipalView {
    RuntimePrincipalView {
        kind: "system".to_string(),
        id: None,
        display_name: Some("Sairgent Runtime".to_string()),
    }
}

fn parse_runtime_cursor(cursor: Option<&str>) -> Option<u64> {
    cursor?
        .strip_prefix("runtime-")
        .and_then(|value| value.parse::<u64>().ok())
}

fn next_runtime_cursor(state: &AppState) -> String {
    let mut next = state.runtime_bus.next_cursor.lock().unwrap();
    *next += 1;
    format!("runtime-{}", *next)
}

fn remember_runtime_signal(state: &AppState, signal: RuntimeSignalView) {
    let mut events = state.runtime_bus.event_log.lock().unwrap();
    events.push(signal);
    if events.len() > 512 {
        let drop_count = events.len() - 512;
        events.drain(0..drop_count);
    }
}

async fn audit_runtime_bus_payload(
    state: &AppState,
    event_kind: &str,
    payload: &serde_json::Value,
    principal_id: Option<&str>,
    swo_id: Option<i64>,
) {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        guard.as_ref().map(Arc::clone)
    };

    if let Some(kernel) = kernel_arc {
        let _ = kernel.registry.record_audit_event(
            principal_id,
            swo_id,
            event_kind,
            TaintLabel::TrustedSystem,
            payload,
        );
    }
}

async fn emit_runtime_signal(
    app: &AppHandle,
    state: &AppState,
    kind: &str,
    payload: serde_json::Value,
    source: &str,
    principal: RuntimePrincipalView,
    audience: &str,
    redaction_class: &str,
    correlation_id: Option<String>,
) {
    if kind == "runtime.status.changed" {
        if let Some(status) = payload.get("status").and_then(|value| value.as_str()) {
            *state.last_hsm_status.lock().unwrap() = status.to_string();
        }
    }

    let signal = RuntimeSignalView {
        envelope: RuntimeEnvelopeView {
            id: Uuid::new_v4().to_string(),
            correlation_id: correlation_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            source: source.to_string(),
            principal: principal.clone(),
            audience: audience.to_string(),
            redaction_class: redaction_class.to_string(),
            occurred_at: now_unix_ms(),
            cursor: next_runtime_cursor(state),
        },
        kind: kind.to_string(),
        payload,
    };

    remember_runtime_signal(state, signal.clone());

    let audit_payload = serde_json::json!({
        "kind": signal.kind,
        "envelope": signal.envelope,
        "payload": signal.payload,
    });
    let audit_swo_id = audit_payload
        .get("payload")
        .and_then(|value| value.get("swo"))
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_i64());

    audit_runtime_bus_payload(
        state,
        "runtime_signal_emitted",
        &audit_payload,
        signal.envelope.principal.id.as_deref(),
        audit_swo_id,
    )
    .await;

    let _ = app.emit("runtime-signal", signal);
}

async fn publish_operator_safe_signal(
    app: &AppHandle,
    state: &AppState,
    kind: &str,
    payload: serde_json::Value,
    source: &str,
    principal: RuntimePrincipalView,
    correlation_id: Option<String>,
) {
    emit_runtime_signal(
        app,
        state,
        kind,
        payload,
        source,
        principal,
        RuntimeAudienceValue::Desktop.as_str(),
        RuntimeRedactionClassValue::OperatorSafe.as_str(),
        correlation_id,
    )
    .await;
}

async fn publish_sync_required(
    app: &AppHandle,
    state: &AppState,
    source: &str,
    principal: RuntimePrincipalView,
    correlation_id: Option<String>,
    audit_event: &str,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    audit_runtime_bus_payload(
        state,
        audit_event,
        &serde_json::json!({ "source": source, "reason": reason }),
        principal.id.as_deref(),
        None,
    )
    .await;
    publish_operator_safe_signal(
        app,
        state,
        "runtime.sync.required",
        serde_json::json!({ "reason": "Runtime requested resync because a safe live update could not be delivered." }),
        source,
        principal,
        correlation_id,
    )
    .await;
}

// ---------------------------------------------------------------------------
// Keyring / config / secret management — ported from desktop
// ---------------------------------------------------------------------------

fn get_config_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| home.join(".sairgent").join("config.toml"))
        .ok_or_else(|| "Could not determine home directory.".to_string())
}

fn read_saved_config() -> Result<Option<AppConfig>, String> {
    let path = get_config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    if let Some(parent) = path.parent() {
        if let Ok(meta) = fs::symlink_metadata(parent) {
            if meta.file_type().is_symlink() {
                return Err("Config directory is a symlink; refusing to read.".to_string());
            }
        }
    }
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if meta.file_type().is_symlink() {
            return Err("Config file is a symlink; refusing to read.".to_string());
        }
    }
    let contents =
        fs::read_to_string(&path).map_err(|e| format!("Failed to read saved config: {}", e))?;
    let config: AppConfig =
        toml::from_str(&contents).map_err(|e| format!("Failed to parse saved config: {}", e))?;
    Ok(Some(config))
}

fn save_saved_config(config: &AppConfig) -> Result<(), String> {
    let path = get_config_path()?;
    if let Some(parent) = path.parent() {
        if let Ok(meta) = fs::symlink_metadata(parent) {
            if meta.file_type().is_symlink() {
                return Err("Config directory is a symlink; refusing to save.".to_string());
            }
        }
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(parent) {
                let mut perms = metadata.permissions();
                if perms.mode() & 0o777 != 0o700 {
                    perms.set_mode(0o700);
                    fs::set_permissions(parent, perms)
                        .map_err(|e| format!("Failed to secure config directory: {}", e))?;
                }
            }
        }
    }
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if meta.file_type().is_symlink() {
            return Err("Config file is a symlink; refusing to save.".to_string());
        }
    }
    let toml_string =
        toml::to_string(config).map_err(|e| format!("Failed to serialize config: {}", e))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|e| format!("Failed to open config file: {}", e))?;
    use std::io::Write;
    file.write_all(toml_string.as_bytes())
        .map_err(|e| format!("Failed to write config file: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(&path) {
            let mut perms = metadata.permissions();
            if perms.mode() & 0o777 != 0o600 {
                perms.set_mode(0o600);
                fs::set_permissions(&path, perms)
                    .map_err(|e| format!("Failed to secure config file: {}", e))?;
            }
        }
    }
    Ok(())
}

fn get_keyring_entry(account: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, account)
        .map_err(|e| format!("Failed to access OS Keychain for '{}': {}", account, e))
}

fn map_keyring_error(account: &str, err: KeyringError) -> String {
    match err {
        KeyringError::NoEntry => format!("No keychain entry found for '{}'.", account),
        KeyringError::NoStorageAccess(_) => {
            "OS keychain is inaccessible or locked. Unlock and retry.".to_string()
        }
        KeyringError::PlatformFailure(_) => {
            "OS keychain operation failed. Check keychain permissions and retry.".to_string()
        }
        KeyringError::TooLong(_, _) => {
            format!("Secret for '{}' exceeds keychain limits.", account)
        }
        KeyringError::Invalid(_, reason) => {
            format!("Invalid keychain attribute for '{}': {}", account, reason)
        }
        KeyringError::Ambiguous(_) => {
            format!("Multiple keychain entries found for '{}'.", account)
        }
        KeyringError::BadEncoding(_) => {
            format!("Stored secret for '{}' is not valid UTF-8.", account)
        }
        _ => format!("Unexpected keychain error for '{}'.", account),
    }
}

fn keyring_get_password_direct(account: &str) -> Result<Option<String>, String> {
    let entry = match get_keyring_entry(account) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(KeyringError::NoStorageAccess(_)) => Ok(None),
        Err(KeyringError::PlatformFailure(_)) => Ok(None),
        Err(err) => Err(map_keyring_error(account, err)),
    }
}

fn keyring_set_password_direct(account: &str, value: &str) -> Result<(), String> {
    let entry = match get_keyring_entry(account) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    match entry.set_password(value) {
        Ok(()) => Ok(()),
        Err(KeyringError::NoStorageAccess(_)) => Ok(()),
        Err(KeyringError::PlatformFailure(_)) => Ok(()),
        Err(err) => Err(map_keyring_error(account, err)),
    }
}

fn keyring_set_password_with_status(account: &str, value: &str) -> Result<bool, String> {
    let entry = match get_keyring_entry(account) {
        Ok(e) => e,
        Err(_) => return Ok(false),
    };
    match entry.set_password(value) {
        Ok(()) => Ok(true),
        Err(KeyringError::NoStorageAccess(_)) => Ok(false),
        Err(KeyringError::PlatformFailure(_)) => Ok(false),
        Err(err) => Err(map_keyring_error(account, err)),
    }
}

// Secure settings bundle — file-based fallback for WSL/headless
fn secure_settings_cache() -> &'static StdMutex<Option<SecureSettingsBundle>> {
    static CACHE: OnceLock<StdMutex<Option<SecureSettingsBundle>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(None))
}

fn bundle_file_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| home.join(".sairgent").join("secure_bundle.json"))
        .ok_or_else(|| "Could not determine home directory.".to_string())
}

fn load_bundle_from_file() -> Result<Option<SecureSettingsBundle>, String> {
    let path = bundle_file_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read secure bundle file: {}", e))?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let parsed: SecureSettingsBundle = serde_json::from_str(&raw)
        .map_err(|e| format!("Failed to parse secure bundle file: {}", e))?;
    Ok(Some(parsed))
}

fn persist_bundle_to_file(bundle: &SecureSettingsBundle) -> Result<(), String> {
    let path = bundle_file_path()?;
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    let encoded = serde_json::to_string(bundle)
        .map_err(|e| format!("Failed to serialize secure bundle: {}", e))?;
    fs::write(&path, &encoded)
        .map_err(|e| format!("Failed to write secure bundle file: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn normalize_secure_bundle(mut bundle: SecureSettingsBundle) -> SecureSettingsBundle {
    bundle.legacy_api_key = bundle.legacy_api_key.trim().to_string();
    bundle.sidechannel_token = bundle.sidechannel_token.trim().to_string();
    bundle.llm_api_keys_by_provider = bundle
        .llm_api_keys_by_provider
        .into_iter()
        .filter_map(|(slug, value)| {
            let ns = slug.trim().to_lowercase();
            let nv = value.trim().to_string();
            if ns.is_empty() || nv.is_empty() { None } else { Some((ns, nv)) }
        })
        .collect();
    bundle.tool_api_keys_by_slug = bundle
        .tool_api_keys_by_slug
        .into_iter()
        .filter_map(|(slug, value)| {
            let ns = slug.trim().to_lowercase();
            let nv = value.trim().to_string();
            if ns.is_empty() || nv.is_empty() { None } else { Some((ns, nv)) }
        })
        .collect();
    bundle
}

fn load_secure_settings_bundle() -> Result<SecureSettingsBundle, String> {
    if let Some(cached) = secure_settings_cache()
        .lock()
        .map_err(|_| "Secure settings cache lock poisoned.".to_string())?
        .clone()
    {
        return Ok(cached);
    }
    let bundle = match keyring_get_password_direct(KEY_SECURE_SETTINGS_BUNDLE)? {
        Some(raw) if !raw.trim().is_empty() => {
            let parsed: SecureSettingsBundle = serde_json::from_str(&raw)
                .map_err(|e| format!("Failed to parse secure settings bundle: {}", e))?;
            normalize_secure_bundle(parsed)
        }
        _ => match load_bundle_from_file()? {
            Some(parsed) => normalize_secure_bundle(parsed),
            None => SecureSettingsBundle::default(),
        },
    };
    *secure_settings_cache()
        .lock()
        .map_err(|_| "Secure settings cache lock poisoned.".to_string())? = Some(bundle.clone());
    Ok(bundle)
}

fn persist_secure_settings_bundle(bundle: &SecureSettingsBundle) -> Result<(), String> {
    let normalized = normalize_secure_bundle(bundle.clone());
    log::info!("persist_secure_settings_bundle: providers={:?}", normalized.llm_api_keys_by_provider.keys().collect::<Vec<_>>());
    let encoded = serde_json::to_string(&normalized)
        .map_err(|e| format!("Failed to serialize secure settings bundle: {}", e))?;

    // Always write to file — WSL keyring accepts writes but doesn't persist across restarts
    persist_bundle_to_file(&normalized)?;
    log::info!("persist_secure_settings_bundle: file written");

    // Also try keyring as a secondary store (works on macOS/native Windows)
    let _ = keyring_set_password_with_status(KEY_SECURE_SETTINGS_BUNDLE, &encoded);
    *secure_settings_cache()
        .lock()
        .map_err(|_| "Secure settings cache lock poisoned.".to_string())? = Some(normalized);
    Ok(())
}

fn update_secure_settings_bundle(
    updater: impl FnOnce(&mut SecureSettingsBundle),
) -> Result<(), String> {
    let mut bundle = load_secure_settings_bundle()?;
    updater(&mut bundle);
    persist_secure_settings_bundle(&bundle)
}

fn bundle_value_for_account(bundle: &SecureSettingsBundle, account: &str) -> Option<String> {
    if account == KEY_API_KEY {
        return (!bundle.legacy_api_key.is_empty()).then(|| bundle.legacy_api_key.clone());
    }
    if account == KEY_SIDECHANNEL_TOKEN {
        return (!bundle.sidechannel_token.is_empty()).then(|| bundle.sidechannel_token.clone());
    }
    if let Some(slug) = account.strip_prefix("llm_provider::") {
        return bundle.llm_api_keys_by_provider.get(slug).cloned().filter(|v| !v.trim().is_empty());
    }
    if let Some(slug) = account.strip_prefix("tool_provider::") {
        return bundle.tool_api_keys_by_slug.get(slug).cloned().filter(|v| !v.trim().is_empty());
    }
    None
}

fn store_bundle_value_for_account(
    bundle: &mut SecureSettingsBundle,
    account: &str,
    value: &str,
) -> bool {
    let trimmed = value.trim().to_string();
    if account == KEY_API_KEY {
        bundle.legacy_api_key = trimmed;
        return true;
    }
    if account == KEY_SIDECHANNEL_TOKEN {
        bundle.sidechannel_token = trimmed;
        return true;
    }
    if let Some(slug) = account.strip_prefix("llm_provider::") {
        if trimmed.is_empty() {
            bundle.llm_api_keys_by_provider.remove(slug);
        } else {
            bundle.llm_api_keys_by_provider.insert(slug.to_string(), trimmed);
        }
        return true;
    }
    if let Some(slug) = account.strip_prefix("tool_provider::") {
        if trimmed.is_empty() {
            bundle.tool_api_keys_by_slug.remove(slug);
        } else {
            bundle.tool_api_keys_by_slug.insert(slug.to_string(), trimmed);
        }
        return true;
    }
    false
}

fn load_secret_optional(account: &str) -> Result<Option<String>, String> {
    let bundle = load_secure_settings_bundle()?;
    if let Some(value) = bundle_value_for_account(&bundle, account) {
        return Ok(Some(value));
    }
    let legacy = keyring_get_password_direct(account)?;
    if let Some(value) = legacy.as_ref().filter(|value| !value.trim().is_empty()) {
        let value = value.trim().to_string();
        update_secure_settings_bundle(|bundle| {
            let _ = store_bundle_value_for_account(bundle, account, &value);
        })?;
        return Ok(Some(value));
    }
    Ok(None)
}

fn save_secret(account: &str, value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        log::warn!("save_secret: empty value for account={}, skipping", account);
        return Ok(());
    }
    log::info!("save_secret: account={}, value_len={}", account, trimmed.len());
    let mut handled = false;
    update_secure_settings_bundle(|bundle| {
        handled = store_bundle_value_for_account(bundle, account, trimmed);
        log::info!("save_secret: bundle updated, handled={}, providers={:?}", handled, bundle.llm_api_keys_by_provider.keys().collect::<Vec<_>>());
    })?;
    if handled {
        log::info!("save_secret: persisted via secure bundle");
        return Ok(());
    }
    log::info!("save_secret: falling back to keyring direct");
    keyring_set_password_direct(account, trimmed)
}

fn keyring_account_for_llm_provider(slug: &str) -> String {
    format!("llm_provider::{}", slug.trim().to_lowercase())
}

fn keyring_account_for_tool(slug: &str) -> String {
    format!("tool_provider::{}", slug.trim().to_lowercase())
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_secret(label: &str, value: &str, max_len: usize) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{} cannot be empty.", label));
    }
    if trimmed.len() > max_len {
        return Err(format!("{} exceeds maximum length of {} bytes.", label, max_len));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(format!("{} contains invalid control characters.", label));
    }
    Ok(())
}

fn validate_slug(label: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim().to_lowercase();
    if trimmed.is_empty() {
        return Err(format!("{} slug cannot be empty.", label));
    }
    if trimmed.len() > MAX_SLUG_LEN {
        return Err(format!("{} slug exceeds {} characters.", label, MAX_SLUG_LEN));
    }
    if !trimmed.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err(format!("{} slug must only contain lowercase letters, numbers, '-' or '_'.", label));
    }
    Ok(trimmed)
}

fn validate_label(label: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{} label cannot be empty.", label));
    }
    if trimmed.len() > MAX_LABEL_LEN {
        return Err(format!("{} label exceeds {} characters.", label, MAX_LABEL_LEN));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(format!("{} label contains invalid control characters.", label));
    }
    Ok(trimmed.to_string())
}

fn validate_env_var(label: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{} env var cannot be empty.", label));
    }
    if trimmed.len() > MAX_ENV_VAR_LEN {
        return Err(format!("{} env var exceeds {} characters.", label, MAX_ENV_VAR_LEN));
    }
    if !trimmed.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        return Err(format!("{} env var must only contain uppercase letters, numbers, and underscores.", label));
    }
    Ok(trimmed.to_string())
}

fn validate_agent_id(agent_id: &str) -> Result<(), String> {
    if agent_id == "SYSTEM" {
        return Ok(());
    }
    Uuid::parse_str(agent_id)
        .map(|_| ())
        .map_err(|_| "agent_id must be SYSTEM or a valid UUID.".to_string())
}

// ---------------------------------------------------------------------------
// Default credential configs
// ---------------------------------------------------------------------------

fn default_llm_credential_configs() -> Vec<ServiceCredentialConfig> {
    vec![
        ServiceCredentialConfig {
            slug: "openai".to_string(),
            label: "OpenAI".to_string(),
            env_var: "OPENAI_API_KEY".to_string(),
            enabled: true,
        },
        ServiceCredentialConfig {
            slug: "anthropic".to_string(),
            label: "Anthropic".to_string(),
            env_var: "ANTHROPIC_API_KEY".to_string(),
            enabled: false,
        },
        ServiceCredentialConfig {
            slug: "openrouter".to_string(),
            label: "OpenRouter".to_string(),
            env_var: "OPENROUTER_API_KEY".to_string(),
            enabled: false,
        },
        ServiceCredentialConfig {
            slug: "groq".to_string(),
            label: "Groq".to_string(),
            env_var: "GROQ_API_KEY".to_string(),
            enabled: false,
        },
    ]
}

fn default_tool_credential_configs() -> Vec<ServiceCredentialConfig> {
    vec![
        ServiceCredentialConfig {
            slug: "tavily".to_string(),
            label: "Tavily Search".to_string(),
            env_var: "TAVILY_API_KEY".to_string(),
            enabled: false,
        },
        ServiceCredentialConfig {
            slug: "exa".to_string(),
            label: "Exa Search".to_string(),
            env_var: "EXA_API_KEY".to_string(),
            enabled: false,
        },
        ServiceCredentialConfig {
            slug: "firecrawl".to_string(),
            label: "Firecrawl".to_string(),
            env_var: "FIRECRAWL_API_KEY".to_string(),
            enabled: false,
        },
        ServiceCredentialConfig {
            slug: "replicate".to_string(),
            label: "Replicate".to_string(),
            env_var: "REPLICATE_API_TOKEN".to_string(),
            enabled: false,
        },
    ]
}

fn merge_service_configs(
    configured: &[ServiceCredentialConfig],
    defaults: Vec<ServiceCredentialConfig>,
) -> Vec<ServiceCredentialConfig> {
    let mut merged = HashMap::new();
    for default in defaults {
        merged.insert(default.slug.clone(), default);
    }
    for item in configured {
        merged.insert(item.slug.clone(), item.clone());
    }
    let mut values = merged.into_values().collect::<Vec<_>>();
    values.sort_by(|a, b| a.label.cmp(&b.label));
    values
}

fn normalize_service_config(
    label_prefix: &str,
    input: &ServiceCredentialInput,
) -> Result<ServiceCredentialConfig, String> {
    let slug = validate_slug(label_prefix, &input.slug)?;
    let label = validate_label(label_prefix, &input.label)?;
    let env_var = validate_env_var(label_prefix, &input.env_var)?;
    if let Some(secret) = input.api_key.as_ref() {
        let secret_label = format!("{} API key", label_prefix);
        validate_secret(&secret_label, secret, MAX_API_KEY_LEN)?;
    }
    Ok(ServiceCredentialConfig { slug, label, env_var, enabled: input.enabled })
}

// ---------------------------------------------------------------------------
// Vault key management
// ---------------------------------------------------------------------------

fn load_or_generate_vault_key() -> Result<String, String> {
    match keyring_get_password_direct(VAULT_KEY_KEYRING_ACCOUNT)? {
        Some(key) if !key.trim().is_empty() => {
            log::info!("Vault key loaded from OS keyring.");
            return Ok(key);
        }
        _ => {}
    }
    let key_file_path = vault_key_file_path()?;
    if key_file_path.exists() {
        match fs::read_to_string(&key_file_path) {
            Ok(contents) if !contents.trim().is_empty() => {
                log::warn!("Running in degraded security mode -- vault key stored as file at {}", key_file_path.display());
                let key = contents.trim().to_string();
                let _ = keyring_set_password_with_status(VAULT_KEY_KEYRING_ACCOUNT, &key);
                return Ok(key);
            }
            _ => {}
        }
    }
    log::info!("Generating new vault encryption key.");
    let key = generate_vault_key();
    match keyring_set_password_with_status(VAULT_KEY_KEYRING_ACCOUNT, &key)? {
        true => {
            log::info!("New vault key stored in OS keyring.");
            return Ok(key);
        }
        false => {
            log::warn!("Keyring unavailable; falling back to file-based vault key.");
        }
    }
    persist_vault_key_to_file(&key_file_path, &key)?;
    Ok(key)
}

fn generate_vault_key() -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rand::Rng::gen(&mut rng);
    STANDARD.encode(bytes)
}

fn vault_key_file_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| home.join(".sairgent").join("vault.key"))
        .ok_or_else(|| "Cannot determine home directory for vault key file.".to_string())
}

fn persist_vault_key_to_file(path: &Path, key: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create vault key directory: {}", e))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    fs::write(path, key).map_err(|e| format!("Failed to write vault key file: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Runtime secrets — loads all configured LLM keys + sidechannel token
// ---------------------------------------------------------------------------

fn load_runtime_secrets(config: &AppConfig) -> Result<sairgent_kernel::kernel::Secrets, String> {
    log::info!("load_runtime_secrets: loading bundle from file...");
    let llm_configs = merge_service_configs(&config.llm_credentials, default_llm_credential_configs());

    let mut llm_api_keys_by_provider = HashMap::new();
    for item in &llm_configs {
        if let Some(secret) = load_secret_optional(&keyring_account_for_llm_provider(&item.slug))? {
            if !secret.trim().is_empty() {
                llm_api_keys_by_provider.insert(item.slug.clone(), secret);
            }
        }
    }

    let legacy_llm_api_key = if !config.llm_api_key.trim().is_empty() {
        Some(config.llm_api_key.trim().to_string())
    } else {
        load_secret_optional(KEY_API_KEY)?
    };

    if let Some(secret) = legacy_llm_api_key.clone() {
        if !secret.trim().is_empty() {
            llm_api_keys_by_provider
                .entry(DEFAULT_LLM_PROVIDER.to_string())
                .or_insert(secret);
        }
    }

    let default_llm_provider = if config.default_llm_provider.trim().is_empty() {
        DEFAULT_LLM_PROVIDER.to_string()
    } else {
        config.default_llm_provider.trim().to_lowercase()
    };

    let default_llm_api_key = llm_api_keys_by_provider
        .get(&default_llm_provider)
        .cloned()
        .or_else(|| legacy_llm_api_key.clone())
        .or_else(|| llm_api_keys_by_provider.values().next().cloned())
        .ok_or_else(|| "No usable LLM provider is configured for the default provider.".to_string())?;

    let sidechannel_token = if let Some(token) = load_secret_optional(KEY_SIDECHANNEL_TOKEN)? {
        if token.trim().is_empty() {
            config.sidechannel_token.trim().to_string()
        } else {
            token
        }
    } else {
        config.sidechannel_token.trim().to_string()
    };

    let sidechannel_token = if sidechannel_token.is_empty() {
        let generated = Uuid::new_v4().to_string();
        let _ = save_secret(KEY_SIDECHANNEL_TOKEN, &generated);
        generated
    } else {
        sidechannel_token
    };

    Ok(sairgent_kernel::kernel::Secrets {
        default_llm_api_key,
        llm_api_keys_by_provider,
        tool_api_keys_by_slug: Arc::new(RwLock::new(HashMap::new())),
        sidechannel_token,
    })
}

fn hydrate_bound_tool_secrets(kernel: &Arc<Kernel>) -> Result<(), String> {
    let agents = kernel
        .registry
        .list_agents()
        .map_err(|e| format!("Agent list failed while hydrating tools: {:?}", e))?;
    let mut provider_slugs = Vec::new();
    for agent in agents {
        let bindings = kernel
            .registry
            .list_agent_tool_bindings(&agent.id)
            .map_err(|e| format!("Tool binding query failed while hydrating tools: {:?}", e))?;
        for binding in bindings {
            if binding.provider_slug.trim().is_empty() {
                continue;
            }
            if !provider_slugs.iter().any(|slug| slug == &binding.provider_slug) {
                provider_slugs.push(binding.provider_slug);
            }
        }
    }
    for provider_slug in provider_slugs {
        let secret = load_secret_optional(&keyring_account_for_tool(&provider_slug))?
            .filter(|value| !value.trim().is_empty());
        let mut guard = kernel
            .secrets
            .tool_api_keys_by_slug
            .write()
            .map_err(|_| "Tool credential cache lock poisoned.".to_string())?;
        match secret {
            Some(value) => { guard.insert(provider_slug, value); }
            None => { guard.remove(&provider_slug); }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Kernel init + boot — ported from desktop
// ---------------------------------------------------------------------------

fn init_kernel(secrets: sairgent_kernel::kernel::Secrets) -> Result<(Arc<Kernel>, String), String> {
    let project_root = resolve_project_root()?;
    let db_path = project_root.join("storage").join("kernel_registry.sqlite");
    let worker_binary = project_root.join("run_worker.sh");
    let seed_path = default_seed_spec_path(&project_root);
    let db_path_str = db_path.to_string_lossy().to_string();
    let worker_binary_str = worker_binary.to_string_lossy().to_string();

    log::info!("Initializing kernel with DB: {}", db_path_str);
    log::info!("Seed spec path: {}", seed_path.display());

    std::fs::create_dir_all(project_root.join("storage")).map_err(|e| e.to_string())?;

    let vault_key = load_or_generate_vault_key()?;
    let db_exists = db_path.exists();

    let k = match Kernel::new(&vault_key, &db_path_str, &worker_binary_str, secrets.clone()) {
        Ok(kernel) => kernel,
        Err(e) => {
            if db_exists && vault_key != LEGACY_VAULT_KEY {
                log::warn!("Kernel init failed with new vault key; attempting legacy dummy key: {:?}", e);
                match Kernel::new(LEGACY_VAULT_KEY, &db_path_str, &worker_binary_str, secrets) {
                    Ok(kernel) => {
                        log::warn!("Legacy vault key works. Continuing with legacy key for this session.");
                        kernel
                    }
                    Err(e2) => {
                        return Err(format!("Kernel::new failed with both keys: new={:?}, legacy={:?}", e, e2));
                    }
                }
            } else {
                return Err(format!("Kernel::new failed: {:?}", e));
            }
        }
    };
    log::info!("Kernel created successfully");

    let spec = k.load_seed_spec_from_path(&seed_path).map_err(|e| {
        log::error!("Failed to load seed spec: {:?}", e);
        format!("Failed to load seed spec: {:?}", e)
    })?;
    let seeded = k.ensure_runtime_seeded(&spec, Some(&seed_path)).map_err(|e| {
        log::error!("Failed to seed runtime: {:?}", e);
        format!("Failed to seed runtime: {:?}", e)
    })?;
    log::info!("Runtime seeded, Perry ID: {}", seeded.perry_agent_id);

    Ok((Arc::new(k), seeded.perry_agent_id))
}

async fn boot_kernel(
    state: &AppState,
    secrets: sairgent_kernel::kernel::Secrets,
) -> Result<(), String> {
    validate_secret("default LLM API key", &secrets.default_llm_api_key, MAX_API_KEY_LEN)?;
    validate_secret("sidechannel_token", &secrets.sidechannel_token, MAX_SIDECHANNEL_TOKEN_LEN)?;
    let mut kernel_guard = state.kernel.lock().await;
    if kernel_guard.is_some() {
        // Kernel already booted — update secrets in-place so provider changes take effect
        if let Some(kernel) = kernel_guard.as_ref() {
            let mut keys = kernel.secrets.llm_api_keys_by_provider.clone();
            for (k, v) in &secrets.llm_api_keys_by_provider {
                keys.insert(k.clone(), v.clone());
            }
            // We can't mutate the Arc<Secrets>, but the orchestrator reads keys via resolve_llm_api_key
            // which checks llm_api_keys_by_provider. For a full reload, restart is needed.
            log::info!("boot_kernel: kernel already running, skipping re-init (restart to reload secrets)");
        }
        return Ok(());
    }
    let (kernel, perry_id) = init_kernel(secrets)?;
    kernel.start_background_tasks();
    *kernel_guard = Some(kernel);
    *state.perry_id.lock().await = Some(perry_id);
    *state.last_hsm_status.lock().unwrap() = "READY".to_string();
    Ok(())
}

// ---------------------------------------------------------------------------
// View conversion helpers
// ---------------------------------------------------------------------------

fn to_runtime_context_view(context: RuntimeContext) -> RuntimeContextView {
    RuntimeContextView {
        company_name: context.company_name,
        profile_id: context.profile_id,
        company_charter_source: context.company_charter_source,
        company_summary: context.company_summary,
        autonomous_hiring_mode: context.autonomous_hiring_mode,
        active_seed_spec_path: context.active_seed_spec_path,
        last_archive_path: context.last_archive_path,
        sairgent_agent_id: None,
    }
}

fn to_swo_record_view(record: AgentSwoSummaryRecord) -> SwoRecordView {
    SwoRecordView {
        id: record.swo.id,
        assignee: record.swo.assigned_agent_name,
        owner: record.swo.owner_agent_name,
        created_by: record.swo.created_by_agent_name,
        status: record.swo.status,
        kind: record.swo.kind,
        source: record.swo.source,
        work_order_title: record.swo.work_order_title,
        work_order_outcome: record.swo.work_order_outcome,
        work_order_constraints: record.swo.work_order_constraints,
        requested_owner: record.swo.requested_owner_agent_name,
        requested_assignee: record.swo.requested_assignee_agent_name,
        routing_policy: record.swo.routing_policy,
        initiative_id: record.swo.initiative_id,
        initiative_name: record.swo.initiative_name,
        initiative_owner: record.swo.initiative_owner_agent_name,
        priority_class: record.swo.priority_class,
        payload: record.swo.payload,
        created_at: record.swo.created_at,
        retry_count: record.swo.retry_count,
        actual_child_assignees: record.actual_child_assignees,
        child_swo_count: record.child_swo_count,
        review_status: record.review_status,
        mismatch_flags: record.mismatch_flags,
        parent_swo_id: record.swo.parent_swo_id,
        review_response: None,
    }
}

/// Build a SwoRecordView from a full SwoDetailRecord, extracting review response.
fn to_swo_record_view_from_detail(detail: &sairgent_kernel::registry::SwoDetailRecord) -> SwoRecordView {
    let review_response = extract_review_response(detail);
    let mut view = to_swo_record_view(AgentSwoSummaryRecord {
        swo: detail.swo.clone(),
        actual_child_assignees: detail.delegation_debug.actual_child_assignees.clone(),
        child_swo_count: detail.delegation_debug.child_swo_count,
        review_status: detail.delegation_debug.review_status.clone(),
        mismatch_flags: detail.delegation_debug.mismatch_flags.clone(),
    });
    view.review_response = review_response;
    view
}

/// Extract the deliverable text from a SWO detail's reviews or results.
/// Prefers the final_response from the most recent manager review,
/// then falls back to the result_json from swo_results.
fn extract_review_response(detail: &sairgent_kernel::registry::SwoDetailRecord) -> Option<String> {
    // Check manager reviews first — most recent review with a final_response wins.
    // This is where Perry's synthesis output lives on the parent SWO.
    for review in detail.reviews.iter().rev() {
        if let Some(ref response) = review.final_response {
            if !response.trim().is_empty() {
                return Some(response.clone());
            }
        }
    }
    // Fall back to the most recent SWO result's result_json. Child SWOs that
    // completed via execute_triage (ANSWER_DIRECTLY) or execute_synthesis
    // store their output text here. The harness wraps the decision under
    // either a `triage` or `synthesis` key, so we have to look nested.
    if let Some(result) = detail.results.last() {
        let json_str = &result.result_json;
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
            // Nested: execute_triage → val.triage.direct_answer
            if let Some(triage) = val.get("triage") {
                if let Some(answer) = triage.get("direct_answer").and_then(|v| v.as_str()) {
                    if !answer.trim().is_empty() {
                        return Some(answer.to_string());
                    }
                }
            }
            // Nested: execute_synthesis → val.synthesis.final_response
            if let Some(synthesis) = val.get("synthesis") {
                if let Some(resp) = synthesis.get("final_response").and_then(|v| v.as_str()) {
                    if !resp.trim().is_empty() {
                        return Some(resp.to_string());
                    }
                }
            }
            // Legacy / flat shapes (kept for backward compatibility with older
            // worker result JSONs that put the text at the top level)
            if let Some(answer) = val.get("direct_answer").and_then(|v| v.as_str()) {
                if !answer.trim().is_empty() {
                    return Some(answer.to_string());
                }
            }
            if let Some(resp) = val.get("response").and_then(|v| v.as_str()) {
                if !resp.trim().is_empty() {
                    return Some(resp.to_string());
                }
            }
            if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                if !content.trim().is_empty() {
                    return Some(content.to_string());
                }
            }
            // Chat/ideation modes may put the output under a "reply" field
            if let Some(reply) = val.get("reply").and_then(|v| v.as_str()) {
                if !reply.trim().is_empty() {
                    return Some(reply.to_string());
                }
            }
            // Last-resort fallback: when an agent chose DELEGATE or EXCEPTION
            // (no direct_answer produced), surface the triage reasoning and
            // any exception details so the user can at least see WHAT the
            // agent decided and why. Otherwise the card renders "no inline
            // preview available" which hides real information.
            if let Some(triage) = val.get("triage") {
                let action = triage.get("action").and_then(|v| v.as_str()).unwrap_or("");
                let reasoning = triage.get("reasoning").and_then(|v| v.as_str()).unwrap_or("").trim();
                let exception_reason = triage.get("exception_reason").and_then(|v| v.as_str()).unwrap_or("").trim();
                let user_message = triage.get("user_message").and_then(|v| v.as_str()).unwrap_or("").trim();
                if !reasoning.is_empty() || !exception_reason.is_empty() || !user_message.is_empty() {
                    let mut parts: Vec<String> = Vec::new();
                    if !action.is_empty() {
                        parts.push(format!("_Agent decided: **{}**_", action));
                    }
                    if !reasoning.is_empty() {
                        parts.push(format!("**Reasoning:** {}", reasoning));
                    }
                    if !exception_reason.is_empty() {
                        parts.push(format!("**Exception:** {}", exception_reason));
                    }
                    if !user_message.is_empty() {
                        parts.push(format!("**Message:** {}", user_message));
                    }
                    return Some(parts.join("\n\n"));
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
fn to_swo_record_view_from_active(record: ActiveSwoRecord) -> SwoRecordView {
    SwoRecordView {
        parent_swo_id: record.parent_swo_id,
        id: record.id,
        assignee: record.assigned_agent_name,
        owner: record.owner_agent_name,
        created_by: record.created_by_agent_name,
        status: record.status,
        kind: record.kind,
        source: record.source,
        work_order_title: record.work_order_title,
        work_order_outcome: record.work_order_outcome,
        work_order_constraints: record.work_order_constraints,
        requested_owner: record.requested_owner_agent_name,
        requested_assignee: record.requested_assignee_agent_name,
        routing_policy: record.routing_policy,
        initiative_id: record.initiative_id,
        initiative_name: record.initiative_name,
        initiative_owner: record.initiative_owner_agent_name,
        priority_class: record.priority_class,
        payload: record.payload,
        created_at: record.created_at,
        retry_count: record.retry_count,
        actual_child_assignees: Vec::new(),
        child_swo_count: 0,
        review_status: "NO_REVIEW".to_string(),
        mismatch_flags: Vec::new(),
        review_response: None,
    }
}

fn to_agent_summary_view(
    summary: sairgent_kernel::registry::AgentSummaryRecord,
) -> AgentSummaryView {
    AgentSummaryView {
        id: summary.id,
        name: summary.name,
        role: summary.role,
    }
}

fn to_agent_org_profile_view(
    profile: sairgent_kernel::registry::AgentOrgProfileRecord,
) -> AgentOrgProfileView {
    AgentOrgProfileView {
        agent_id: profile.agent_id,
        org_class: profile.org_class,
        team_goal_ids: profile.team_goal_ids,
        delegation_policy: profile.delegation_policy,
        review_policy: profile.review_policy,
        managed_domains: profile.managed_domains,
        quality_rubric: profile.quality_rubric,
        max_delegation_depth: profile.max_delegation_depth,
        max_parallel_delegates: profile.max_parallel_delegates,
        manager_can_hire: profile.manager_can_hire,
        manager_can_restructure: profile.manager_can_restructure,
        updated_at: profile.updated_at,
    }
}

fn to_agent_tree_node_view(node: AgentTreeNodeRecord) -> AgentTreeNodeView {
    AgentTreeNodeView {
        id: node.id,
        name: node.name,
        role: node.role,
        manager: node.manager.map(to_agent_summary_view),
        org_profile: to_agent_org_profile_view(node.org_profile),
        depth: node.depth,
        is_direct_report: node.is_direct_report,
        direct_report_count: node.direct_report_count,
        descendant_count: node.descendant_count,
        cron_enabled: node.cron_enabled,
        presence: node.presence,
        last_seen_unix_ms: node.last_seen_unix_ms,
        last_seen_age_ms: node.last_seen_age_ms,
        last_cron_fired_at: node.last_cron_fired_at,
        children: node.children.into_iter().map(to_agent_tree_node_view).collect(),
        default_provider: node.default_provider,
        model: node.model,
        triage_model: node.triage_model,
        execution_model: node.execution_model,
        raison_detre: node.raison_detre,
        persona_prompt: node.persona_prompt,
    }
}

fn to_outbox_artifact_view(
    record: sairgent_kernel::registry::OutboxArtifactRecord,
) -> OutboxArtifactView {
    OutboxArtifactView {
        id: record.id,
        agent: record.agent_name,
        agent_id: Some(record.agent_id),
        swo_id: Some(record.swo_id),
        parent_swo_id: record.parent_swo_id,
        project_id: record.project_id,
        project_name: record.project_name,
        source_work_order_title: record.source_work_order_title,
        source_work_order_outcome: record.source_work_order_outcome,
        source_status: record.source_status,
        absolute_path: record.absolute_path,
        filename: record.filename,
        created_at: record.created_at,
    }
}

fn to_project_view(record: ProjectRecord) -> ProjectView {
    ProjectView {
        project_id: record.id,
        name: record.name,
        summary: record.summary,
        status: record.status,
        owner: record.lead_agent_id.unwrap_or_default(),
        priority: record.priority,
        target_outcome: record.target_outcome,
        tags: record.tags.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

#[allow(dead_code)]
fn to_inbox_item_view(record: InboxItemRecord) -> InboxItemView {
    InboxItemView {
        id: record.id,
        kind: record.kind,
        status: record.status,
        priority: record.priority,
        title: record.title,
        summary: record.summary,
        created_at: record.created_at,
        updated_at: record.updated_at,
        project_id: record.project_id,
        project_name: record.project_name,
        swo_id: record.swo_id,
        artifact_id: record.artifact_id,
        agent_id: record.agent_id,
    }
}

fn to_inbox_attention_summary_view(record: InboxAttentionSummaryRecord) -> InboxAttentionSummaryView {
    InboxAttentionSummaryView {
        open_inbox_items: record.open_inbox_items,
        open_approval_items: record.open_approval_items,
        open_deliverable_items: record.open_deliverable_items,
        open_blocked_items: record.open_blocked_items,
    }
}

fn to_agent_detail_view(detail: AgentDetailRecord) -> AgentDetailView {
    let manifest = &detail.manifest;
    AgentDetailView {
        id: detail.id.clone(),
        name: detail.name.clone(),
        role: detail.role.clone(),
        manager: detail.manager.map(to_agent_summary_view),
        org_profile: to_agent_org_profile_view(detail.org_profile),
        team_goals: detail.team_goals.into_iter().map(|g| TeamGoalView {
            goal_id: g.goal_id,
            team_owner_agent_id: g.team_owner_agent_id,
            title: g.title,
            summary: g.summary,
            status: g.status,
            priority: g.priority,
            success_criteria: g.success_criteria,
            managed_domain_tags: g.managed_domain_tags,
            created_at: g.created_at,
            updated_at: g.updated_at,
            archived_at: g.archived_at,
        }).collect(),
        delegation_decisions: detail.delegation_decisions.into_iter().map(|d| DelegationDecisionRecordView {
            id: d.id,
            swo_id: d.swo_id,
            manager_agent_id: d.manager_agent_id,
            decision: d.decision,
            candidate_assignees: d.candidate_assignees,
            selected_agent_id: d.selected_agent_id,
            fit_reason: d.fit_reason,
            exception_code: d.exception_code,
            exception_reason: d.exception_reason,
            team_gap_code: d.team_gap_code,
            created_at: d.created_at,
        }).collect(),
        team_gaps: detail.team_gaps.into_iter().map(|g| TeamGapRecordView {
            id: g.id,
            swo_id: g.swo_id,
            manager_agent_id: g.manager_agent_id,
            gap_code: g.gap_code,
            summary: g.summary,
            recommended_action: g.recommended_action,
            created_at: g.created_at,
        }).collect(),
        direct_reports: detail.direct_reports.into_iter().map(|d| DirectReportSummaryView {
            id: d.id,
            name: d.name,
            role: d.role,
            cron_enabled: d.cron_enabled,
            presence: d.presence,
            last_seen_unix_ms: d.last_seen_unix_ms,
            last_seen_age_ms: d.last_seen_age_ms,
        }).collect(),
        persona_prompt: manifest.persona_prompt.clone(),
        raison_detre: manifest.mission.clone(),
        provider: manifest.provider.provider_name.clone(),
        model: manifest.provider.model.clone(),
        triage_model: manifest.provider.triage_model.clone(),
        execution_model: manifest.provider.execution_model.clone(),
        cron_interval_seconds: manifest.schedule.cron_interval_seconds,
        presence: detail.presence.clone(),
        last_seen_unix_ms: detail.last_seen_unix_ms,
        last_seen_age_ms: detail.last_seen_age_ms,
        last_cron_fired_at: detail.last_cron_fired_at.clone(),
        heartbeat_timeline: detail.heartbeat_timeline.into_iter().map(|h| HeartbeatEventView {
            run_id: h.run_id,
            status: h.status,
            last_seen_unix_ms: h.last_seen_unix_ms,
            last_seen_age_ms: h.last_seen_age_ms,
            seq: h.seq,
        }).collect(),
        assigned_swos: detail.assigned_swos.into_iter().map(to_swo_record_view).collect(),
        owned_swos: detail.owned_swos.into_iter().map(to_swo_record_view).collect(),
        created_swos: detail.created_swos.into_iter().map(to_swo_record_view).collect(),
        charter_settings: CharterSettingsView {
            raison_detre: manifest.mission.clone(),
            provider: manifest.provider.provider_name.clone(),
            model: manifest.provider.model.clone(),
            cron_interval_seconds: manifest.schedule.cron_interval_seconds,
        },
        manifest: AgentManifestView {
            version: manifest.version.clone(),
            name: manifest.name.clone(),
            role: manifest.role.clone(),
            mission: manifest.mission.clone(),
            persona_prompt: manifest.persona_prompt.clone(),
            provider_name: manifest.provider.provider_name.clone(),
            model: manifest.provider.model.clone(),
            protocol_family: format!("{:?}", manifest.provider.protocol_family),
            capabilities: manifest.capabilities.iter().map(|c| format!("{:?}", c)).collect(),
            cron_interval_seconds: manifest.schedule.cron_interval_seconds,
            autonomous_heartbeat: manifest.schedule.autonomous_heartbeat,
        },
        bound_skills: detail.bound_skills.into_iter().map(|s| SkillBindingView {
            id: s.skill_id.clone(),
            name: s.skill_name,
            slug: s.skill_slug,
            summary: s.summary,
            tags: s.tags,
            trigger_hints: s.trigger_hints,
            source_uri: s.source_uri,
            current_version: s.current_version,
            priority: s.priority,
            binding_status: s.binding_status,
            preselected: false,
            runtime_path: None,
        }).collect(),
        bound_tools: detail.bound_tools.into_iter().map(|t| AgentToolBindingView {
            slug: t.tool_slug,
            name: t.name,
            summary: t.summary,
            tool_kind: t.tool_kind,
            provider_slug: t.provider_slug,
            required_capability: t.required_capability,
            binding_status: t.binding_status,
        }).collect(),
        mcp_bindings: detail.bound_mcp_connectors.into_iter().map(|m| AgentMcpBindingView {
            connector_id: m.connector_id,
            connector_slug: m.connector_slug,
            connector_name: m.connector_name,
            transport: m.transport,
            binding_status: m.binding_status,
        }).collect(),
    }
}

fn build_approval_queue_item(record: &SwoRecordView) -> Option<ApprovalQueueItemView> {
    let needs_review = record.review_status != "NO_REVIEW"
        || !record.mismatch_flags.is_empty()
        || record.priority_class.as_deref() == Some("REVIEW");
    if !needs_review {
        return None;
    }
    let reason = if !record.mismatch_flags.is_empty() {
        record.mismatch_flags.join(", ").replace('_', " ")
    } else if record.review_status != "NO_REVIEW" {
        record.review_status.replace('_', " ")
    } else {
        "Review required".to_string()
    };
    Some(ApprovalQueueItemView {
        id: format!("approval-{}", record.id),
        swo_id: record.id,
        title: record.work_order_title.clone().unwrap_or_else(|| format!("Work order #{}", record.id)),
        reason,
        owner: record.owner.clone(),
        status: record.status.clone(),
    })
}

fn sanitize_runtime_status(status: &str) -> Option<String> {
    let trimmed = status.trim();
    if trimmed.is_empty() { return None; }
    if trimmed.contains("stdout:") || trimmed.contains("stderr:") { return None; }
    if trimmed == "READY" || trimmed == "THINKING" || trimmed == "ERROR" {
        return Some(trimmed.to_string());
    }
    if trimmed.contains("DELEGATING") { return Some("DELEGATING".to_string()); }
    if trimmed.contains("SYNTHESIZING") { return Some("SYNTHESIZING".to_string()); }
    if trimmed.contains("IN_PROGRESS") { return Some("WORKING".to_string()); }
    None
}

// ---------------------------------------------------------------------------
// Build bootstrap
// ---------------------------------------------------------------------------

async fn build_runtime_bootstrap(state: &AppState) -> Result<RuntimeBootstrapView, String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(kernel) = guard.as_ref() {
            Arc::clone(kernel)
        } else {
            return Err("Kernel not initialized".into());
        }
    };
    let perry_agent_id = state.perry_id.lock().await.clone()
        .ok_or_else(|| "Perry is not initialized".to_string())?;
    let hsm_status = state.last_hsm_status.lock().unwrap().clone();

    let queue_kernel = Arc::clone(&kernel_arc);
    let queue = tokio::task::spawn_blocking(move || {
        let rows = queue_kernel.registry.list_swo_summaries(80)
            .map_err(|e| format!("Queue query failed: {:?}", e))?;
        let mut views = Vec::with_capacity(rows.len());
        for row in rows {
            let is_completed = row.swo.status == "COMPLETED";
            let swo_id = row.swo.id;
            let mut view = to_swo_record_view(row);
            if is_completed {
                if let Ok(Some(detail)) = queue_kernel.registry.get_swo_detail(swo_id) {
                    view.review_response = extract_review_response(&detail);
                }
            }
            views.push(view);
        }
        Ok::<Vec<SwoRecordView>, String>(views)
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;

    let roster_kernel = Arc::clone(&kernel_arc);
    let roster = tokio::task::spawn_blocking(move || {
        let rows = roster_kernel.registry.get_agent_tree_snapshot(now_unix_ms())
            .map_err(|e| format!("Roster query failed: {:?}", e))?;
        Ok::<Vec<AgentTreeNodeView>, String>(rows.into_iter().map(to_agent_tree_node_view).collect())
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;

    let runtime_kernel = Arc::clone(&kernel_arc);
    let runtime_context = tokio::task::spawn_blocking(move || {
        runtime_kernel.runtime_context()
            .map(to_runtime_context_view)
            .map(Some)
            .map_err(|e| format!("Runtime context query failed: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;

    let artifact_kernel = Arc::clone(&kernel_arc);
    let recent_artifacts = tokio::task::spawn_blocking(move || {
        artifact_kernel.registry.list_outbox_artifacts(sairgent_kernel::registry::OutboxArtifactListFilters {
            agent_id: None, swo_id: None, query: None, limit: 20,
        }).map(|rows| rows.into_iter().map(to_outbox_artifact_view).collect::<Vec<_>>())
            .map_err(|e| format!("Artifact query failed: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;

    let inbox_kernel = Arc::clone(&kernel_arc);
    let attention_summary = tokio::task::spawn_blocking(move || {
        inbox_kernel.registry.inbox_attention_summary()
            .map(to_inbox_attention_summary_view)
            .map_err(|e| format!("Inbox attention query failed: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;

    let project_kernel = Arc::clone(&kernel_arc);
    let projects = tokio::task::spawn_blocking(move || {
        project_kernel.registry.list_projects()
            .map(|rows| rows.into_iter().map(to_project_view).collect::<Vec<_>>())
            .map_err(|e| format!("Project query failed: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;

    let inbox_items_kernel = Arc::clone(&kernel_arc);
    let inbox_items = tokio::task::spawn_blocking(move || {
        inbox_items_kernel.registry.list_inbox_items(false, 50)
            .map(|rows| rows.into_iter().map(to_inbox_item_view).collect::<Vec<_>>())
            .map_err(|e| format!("Inbox items query failed: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;

    let approvals = queue.iter().filter_map(build_approval_queue_item).collect::<Vec<_>>();
    let cursor = {
        let current = *state.runtime_bus.next_cursor.lock().unwrap();
        format!("runtime-{current}")
    };

    let runtime_context = runtime_context.map(|mut context| {
        context.sairgent_agent_id = Some(perry_agent_id.clone());
        context
    });

    Ok(RuntimeBootstrapView {
        cursor: RuntimeCursorView { value: cursor },
        hsm_status,
        runtime_context,
        queue,
        roster,
        approvals,
        recent_artifacts,
        attention_summary,
        projects,
        inbox_items,
    })
}

async fn cache_bootstrap_result(state: &AppState, data: RuntimeBootstrapView) {
    let cached = CachedBootstrap { data, cached_at: now_unix_ms() as u64 };
    let mut cache_guard = state.bootstrap_cache.lock().await;
    *cache_guard = Some(cached);
}

async fn get_cached_bootstrap(state: &AppState) -> Option<RuntimeBootstrapView> {
    let cache_guard = state.bootstrap_cache.lock().await;
    cache_guard.as_ref().map(|cached| cached.data.clone())
}

// ---------------------------------------------------------------------------
// Settings view builder
// ---------------------------------------------------------------------------

fn build_settings_view(config: &AppConfig) -> Result<SettingsView, String> {
    let llm_credentials = merge_service_configs(&config.llm_credentials, default_llm_credential_configs());
    let tool_credentials = merge_service_configs(&config.tool_credentials, default_tool_credential_configs());
    let legacy_llm = load_secret_optional(KEY_API_KEY)?;

    let llm_views = llm_credentials.iter().map(|item| {
        let has_secret = if item.slug == DEFAULT_LLM_PROVIDER {
            load_secret_optional(&keyring_account_for_llm_provider(&item.slug))?
                .map(|v| !v.trim().is_empty()).unwrap_or(false)
                || legacy_llm.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false)
                || (!config.llm_api_key.trim().is_empty() && item.slug == DEFAULT_LLM_PROVIDER)
        } else {
            load_secret_optional(&keyring_account_for_llm_provider(&item.slug))?
                .map(|v| !v.trim().is_empty()).unwrap_or(false)
        };
        Ok(ServiceCredentialView {
            slug: item.slug.clone(), label: item.label.clone(),
            env_var: item.env_var.clone(), enabled: item.enabled, has_secret,
        })
    }).collect::<Result<Vec<_>, String>>()?;

    let tool_views = tool_credentials.iter().map(|item| {
        let has_secret = load_secret_optional(&keyring_account_for_tool(&item.slug))?
            .map(|v| !v.trim().is_empty()).unwrap_or(false);
        Ok(ServiceCredentialView {
            slug: item.slug.clone(), label: item.label.clone(),
            env_var: item.env_var.clone(), enabled: item.enabled, has_secret,
        })
    }).collect::<Result<Vec<_>, String>>()?;

    let default_provider = if config.default_llm_provider.trim().is_empty() {
        DEFAULT_LLM_PROVIDER.to_string()
    } else {
        config.default_llm_provider.trim().to_lowercase()
    };

    let has_bootable = llm_views.iter().any(|v| v.slug == default_provider && v.has_secret);

    Ok(SettingsView {
        default_llm_provider: default_provider,
        default_llm_model: config.default_llm_model.trim().to_string(),
        llm_credentials: llm_views,
        tool_credentials: tool_views,
        has_sidechannel_token: true,
        has_bootable_credentials: has_bootable,
    })
}

// =========================================================================
// TAURI COMMANDS — 13 total
// =========================================================================

// ---- Boot (3) ----

#[tauri::command]
async fn secrets_status() -> Result<bool, String> {
    let config = read_saved_config()?.unwrap_or_default();
    let default_provider = if config.default_llm_provider.trim().is_empty() {
        DEFAULT_LLM_PROVIDER.to_string()
    } else {
        config.default_llm_provider.trim().to_lowercase()
    };
    let llm_credentials = merge_service_configs(&config.llm_credentials, default_llm_credential_configs());
    let legacy_llm = load_secret_optional(KEY_API_KEY)?;
    let has_default_secret = llm_credentials.iter().any(|item| {
        if item.slug != default_provider { return false; }
        if item.slug == DEFAULT_LLM_PROVIDER {
            load_secret_optional(&keyring_account_for_llm_provider(&item.slug))
                .ok().flatten().map(|v| !v.trim().is_empty()).unwrap_or(false)
                || legacy_llm.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false)
                || !config.llm_api_key.trim().is_empty()
        } else {
            load_secret_optional(&keyring_account_for_llm_provider(&item.slug))
                .ok().flatten().map(|v| !v.trim().is_empty()).unwrap_or(false)
        }
    });
    Ok(has_default_secret)
}

#[tauri::command]
async fn kernel_boot_from_keychain(state: State<'_, AppState>) -> Result<(), String> {
    log::info!("kernel_boot_from_keychain called");
    let config = read_saved_config()?.unwrap_or_default();
    log::info!("Config loaded, default_llm_provider={}", config.default_llm_provider);
    let secrets = match load_runtime_secrets(&config) {
        Ok(s) => {
            log::info!("Secrets loaded, providers={:?}", s.llm_api_keys_by_provider.keys().collect::<Vec<_>>());
            s
        }
        Err(e) => {
            log::error!("Failed to load runtime secrets: {}", e);
            return Err(e);
        }
    };
    if config.llm_api_key.trim().is_empty() || config.sidechannel_token.trim().is_empty() {
        save_saved_config(&AppConfig {
            llm_api_key: String::new(),
            sidechannel_token: String::new(),
            ..config
        })?;
    }
    boot_kernel(&state, secrets).await?;
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        guard.as_ref().map(Arc::clone)
    };
    if let Some(kernel) = kernel_arc.as_ref() {
        hydrate_bound_tool_secrets(kernel)?;
    }
    Ok(())
}

#[tauri::command]
async fn kernel_boot_with_secrets(
    api_key: String,
    sidechannel_token: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let api_key_trimmed = api_key.trim().to_string();
    boot_kernel(
        &state,
        sairgent_kernel::kernel::Secrets {
            default_llm_api_key: api_key_trimmed.clone(),
            llm_api_keys_by_provider: HashMap::from([(DEFAULT_LLM_PROVIDER.to_string(), api_key_trimmed)]),
            tool_api_keys_by_slug: Arc::new(RwLock::new(HashMap::new())),
            sidechannel_token,
        },
    )
    .await
}

// ---- Runtime (3) ----

#[tauri::command]
async fn runtime_bootstrap(state: State<'_, AppState>) -> Result<RuntimeBootstrapView, String> {
    log::info!("runtime_bootstrap command called");
    let start_time = Instant::now();

    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        build_runtime_bootstrap(state.inner()),
    ).await {
        Ok(result) => match result {
            Ok(bootstrap) => {
                let duration_ms = start_time.elapsed().as_millis();
                log::info!("Bootstrap completed in {}ms", duration_ms);
                cache_bootstrap_result(state.inner(), bootstrap.clone()).await;
                Ok(bootstrap)
            }
            Err(e) => {
                log::error!("Bootstrap failed: {}", e);
                if let Some(cached) = get_cached_bootstrap(state.inner()).await {
                    log::warn!("Returning cached bootstrap data after error");
                    Ok(cached)
                } else {
                    Err(e)
                }
            }
        },
        Err(_) => {
            log::warn!("Bootstrap timeout after 2s, attempting cache fallback");
            if let Some(cached) = get_cached_bootstrap(state.inner()).await {
                Ok(cached)
            } else {
                Err("Bootstrap timeout: kernel not responding. No cached data available.".to_string())
            }
        }
    }
}

#[tauri::command]
async fn runtime_replay(
    request: RuntimeReplayRequest,
    state: State<'_, AppState>,
) -> Result<Vec<RuntimeSignalView>, String> {
    let after_cursor = parse_runtime_cursor(request.cursor.as_deref());
    let limit = request.limit.unwrap_or(200).clamp(1, 500);
    let events = state.runtime_bus.event_log.lock().unwrap();
    let filtered = events
        .iter()
        .filter(|signal| {
            let signal_cursor = parse_runtime_cursor(Some(&signal.envelope.cursor)).unwrap_or(0);
            after_cursor.map(|cursor| signal_cursor > cursor).unwrap_or(true)
        })
        .filter(|signal| signal.envelope.redaction_class == RuntimeRedactionClassValue::OperatorSafe.as_str())
        .filter(|signal| signal.envelope.audience != RuntimeAudienceValue::ExternalAdapter.as_str())
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    Ok(filtered)
}

#[tauri::command]
async fn runtime_command(
    request: RuntimeCommandRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    let runtime_state = state.inner().clone();
    let correlation_id = request.correlation_id.clone();
    let kind = request.kind.clone();
    let payload = request.payload.clone();

    // Idempotency check
    {
        let cmd_id = &request.command_id;
        if cmd_id.is_empty() || cmd_id.len() > 128
            || !cmd_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("commandId '{}' is invalid", cmd_id));
        }
        let mut cache = runtime_state.processed_command_ids.lock().unwrap();
        if cache.contains(&request.command_id) {
            return Err(format!("Duplicate command rejected: {}", request.command_id));
        }
        cache.put(request.command_id.clone(), now_unix_ms() as i64);
    }

    audit_runtime_bus_payload(
        &runtime_state,
        "runtime_command_received",
        &serde_json::json!({
            "commandId": request.command_id,
            "correlationId": request.correlation_id,
            "kind": request.kind,
            "source": request.source,
            "principal": request.principal,
        }),
        request.principal.id.as_deref(),
        None,
    ).await;

    match kind.as_str() {
        "project.create" => {
            let name = payload.get("name").and_then(|v| v.as_str())
                .ok_or("Missing name")?.to_string();
            let summary = payload.get("summary").and_then(|v| v.as_str()).map(|s| s.to_string());
            let owner = payload.get("owner").or_else(|| payload.get("leadAgentId"))
                .and_then(|v| v.as_str()).ok_or("Missing owner or leadAgentId")?.to_string();
            let priority = payload.get("priority").and_then(|v| v.as_str()).unwrap_or("NORMAL").to_string();
            let target_outcome = payload.get("targetOutcome").and_then(|v| v.as_str()).map(|s| s.to_string());
            let tags_vec = payload.get("tags").and_then(|v| v.as_array())
                .map(|t| t.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect::<Vec<_>>())
                .unwrap_or_default();
            let tags_str = tags_vec.join(",");
            let project_id = Uuid::new_v4().to_string();
            let pid = project_id.clone();
            let nc = name.clone(); let sc = summary.clone(); let oc = owner.clone();
            let pc = priority.clone(); let tc = target_outcome.clone();
            tokio::task::spawn_blocking(move || {
                kernel_arc.registry.create_project(
                    &pid, &nc, sc.as_deref(), "ACTIVE", &pc,
                    Some(&oc), tc.as_deref(), Some(&tags_str), WORKSPACE_OPERATOR_NAME,
                ).map_err(|e| format!("Project creation failed: {:?}", e))?;
                Ok::<(), String>(())
            }).await.map_err(|e| format!("Task join failed: {:?}", e))??;

            let now_ms = now_unix_ms();
            emit_runtime_signal(
                &app, &runtime_state, "project.upserted",
                serde_json::json!({
                    "project": {
                        "projectId": project_id, "name": name, "summary": summary,
                        "owner": owner, "status": "ACTIVE", "priority": priority,
                        "targetOutcome": target_outcome, "tags": tags_vec,
                        "createdAt": format!("{}", now_ms), "updatedAt": format!("{}", now_ms),
                    }
                }),
                "kernel.command.handler", system_runtime_principal(),
                "desktop", "operator_safe", Some(correlation_id),
            ).await;
        }
        "work_order.submit" => {
            return Err("work_order.submit: use submit_work_order command directly".to_string());
        }
        _ => {
            return Err(format!("Unknown runtime command kind: {}", kind));
        }
    }
    Ok(())
}

// ---- Work (2) ----

#[tauri::command]
async fn submit_work_order(
    request: SubmitWorkOrderRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SwoRecordView, String> {
    log::info!("submit_work_order called: title={}", request.title);
    let title = request.title.trim();
    let outcome = request.outcome.trim();
    if title.is_empty() || outcome.is_empty() {
        return Err("Title and requested outcome are required.".to_string());
    }

    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };

    let requested_owner_agent_id = if let Some(owner) = request.requested_owner.as_deref() {
        validate_agent_id(owner)?;
        owner.to_string()
    } else {
        state.perry_id.lock().await.clone()
            .ok_or_else(|| "Perry is not initialized".to_string())?
    };

    let priority = request.priority.trim().to_uppercase();
    let priority_class = if priority.is_empty() { "CORE".to_string() } else { priority };
    let constraints = request.constraints.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);

    let project_id = request.project_id.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    if let Some(pid) = project_id.as_deref() {
        kernel_arc.registry.get_project(pid)
            .map_err(|e| format!("Project lookup failed: {:?}", e))?
            .ok_or_else(|| format!("Unknown project '{}'.", pid))?;
    }

    let runtime_state = state.inner().clone();

    let swo_id = kernel_arc.registry.create_work_order(
        &requested_owner_agent_id, &requested_owner_agent_id,
        title, outcome, constraints.as_deref(),
        Some(priority_class.as_str()), Some(&requested_owner_agent_id),
        request.parent_swo_id, project_id.as_deref(),
    ).map_err(|e| format!("Failed to create work order: {:?}", e))?;

    let correlation_id = format!("work-order-{swo_id}");
    let run_id = format!("work-order-{}", Uuid::new_v4());
    let claimed = kernel_arc.registry.claim_swo_with_run_id(swo_id, &run_id)
        .map_err(|e| format!("Failed to claim work order: {:?}", e))?;

    if claimed > 0 {
        // Publish swo.upserted for the claimed SWO
        if let Ok(Some(detail)) = kernel_arc.registry.get_swo_detail(swo_id) {
            let swo_view = to_swo_record_view_from_detail(&detail);
            publish_operator_safe_signal(
                &app, &runtime_state, "swo.upserted",
                serde_json::json!({ "swo": swo_view }),
                "workspace.tauri.submit_work_order",
                workspace_operator_principal(), Some(correlation_id.clone()),
            ).await;
        }

        // Spawn orchestrator execution in background
        let (tx, mut rx) = tokio::sync::mpsc::channel::<KernelEvent>(32);
        let relay_app = app.clone();
        let relay_state = runtime_state.clone();
        let relay_kernel = Arc::clone(&kernel_arc);
        let correlation_for_stream = correlation_id.clone();
        let owner_agent_id_for_relay = requested_owner_agent_id.clone();

        // Emit agent presence → COMPUTING and SWO → IN_PROGRESS immediately
        {
            publish_operator_safe_signal(
                &app, &runtime_state, "agent.presence.changed",
                serde_json::json!({ "agentId": &requested_owner_agent_id, "presence": "COMPUTING" }),
                "workspace.tauri.submit_work_order.start",
                workspace_operator_principal(), Some(correlation_id.clone()),
            ).await;
            // Re-emit SWO as IN_PROGRESS
            if let Ok(Some(detail)) = kernel_arc.registry.get_swo_detail(swo_id) {
                let mut swo_view = to_swo_record_view(AgentSwoSummaryRecord {
                    swo: detail.swo.clone(),
                    actual_child_assignees: detail.delegation_debug.actual_child_assignees.clone(),
                    child_swo_count: detail.delegation_debug.child_swo_count,
                    review_status: detail.delegation_debug.review_status.clone(),
                    mismatch_flags: detail.delegation_debug.mismatch_flags.clone(),
                });
                swo_view.status = "IN_PROGRESS".to_string();
                publish_operator_safe_signal(
                    &app, &runtime_state, "swo.upserted",
                    serde_json::json!({ "swo": swo_view }),
                    "workspace.tauri.submit_work_order.progress",
                    workspace_operator_principal(), Some(correlation_id.clone()),
                ).await;
            }
        }

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    KernelEvent::Status(s) => {
                        if let Some(next_status) = sanitize_runtime_status(&s) {
                            emit_runtime_signal(
                                &relay_app, &relay_state, "runtime.status.changed",
                                serde_json::json!({ "status": next_status }),
                                "kernel.event.status", system_runtime_principal(),
                                "desktop", "operator_safe",
                                Some(correlation_for_stream.clone()),
                            ).await;
                        }
                    }
                    KernelEvent::Error(err) => {
                        // Emit agent back to IDLE on error
                        publish_operator_safe_signal(
                            &relay_app, &relay_state, "agent.presence.changed",
                            serde_json::json!({ "agentId": &owner_agent_id_for_relay, "presence": "IDLE" }),
                            "kernel.event.error.presence",
                            system_runtime_principal(), Some(correlation_for_stream.clone()),
                        ).await;
                        publish_sync_required(
                            &relay_app, &relay_state, "kernel.event.error",
                            system_runtime_principal(), Some(correlation_for_stream.clone()),
                            "runtime_execution_error",
                            format!("Work-order stream error for SWO {}: {}", swo_id, err),
                        ).await;
                    }
                    KernelEvent::SwoTerminal { swo_id: child_swo_id } => {
                        if let Ok(Some(detail)) = relay_kernel.registry.get_swo_detail(child_swo_id) {
                            let assignee_id = detail.swo.assigned_agent_id.clone();
                            let swo_view = to_swo_record_view_from_detail(&detail);
                            publish_operator_safe_signal(
                                &relay_app, &relay_state, "swo.upserted",
                                serde_json::json!({ "swo": swo_view }),
                                "kernel.event.swo_terminal", system_runtime_principal(),
                                Some(correlation_for_stream.clone()),
                            ).await;
                            // Emit agent back to IDLE when their SWO completes
                            publish_operator_safe_signal(
                                &relay_app, &relay_state, "agent.presence.changed",
                                serde_json::json!({ "agentId": assignee_id, "presence": "IDLE" }),
                                "kernel.event.swo_terminal.presence",
                                system_runtime_principal(), Some(correlation_for_stream.clone()),
                            ).await;
                        }
                    }
                    KernelEvent::ArtifactRegistered { swo_id: artifact_swo_id } => {
                        if let Ok(artifacts) = relay_kernel.registry.get_artifacts_for_swo(artifact_swo_id) {
                            for artifact in artifacts.into_iter().map(to_outbox_artifact_view) {
                                publish_operator_safe_signal(
                                    &relay_app, &relay_state, "artifact.created",
                                    serde_json::json!({ "artifact": artifact }),
                                    "kernel.event.artifact_registered",
                                    system_runtime_principal(),
                                    Some(correlation_for_stream.clone()),
                                ).await;
                            }
                        }
                    }
                    KernelEvent::SwoCreated { swo_id: new_swo_id, assigned_agent_id, parent_swo_id: _ } => {
                        if let Ok(Some(detail)) = relay_kernel.registry.get_swo_detail(new_swo_id) {
                            let swo_view = to_swo_record_view_from_detail(&detail);
                            publish_operator_safe_signal(
                                &relay_app, &relay_state, "swo.upserted",
                                serde_json::json!({ "swo": swo_view }),
                                "kernel.event.swo_created", system_runtime_principal(),
                                Some(correlation_for_stream.clone()),
                            ).await;
                        }
                        // Mark assigned agent as COMPUTING
                        publish_operator_safe_signal(
                            &relay_app, &relay_state, "agent.presence.changed",
                            serde_json::json!({ "agentId": assigned_agent_id, "presence": "COMPUTING" }),
                            "kernel.event.swo_created.presence",
                            system_runtime_principal(), Some(correlation_for_stream.clone()),
                        ).await;
                    }
                    KernelEvent::SwoStatusChanged { swo_id: changed_swo_id, new_status: _ } => {
                        if let Ok(Some(detail)) = relay_kernel.registry.get_swo_detail(changed_swo_id) {
                            let swo_view = to_swo_record_view_from_detail(&detail);
                            publish_operator_safe_signal(
                                &relay_app, &relay_state, "swo.upserted",
                                serde_json::json!({ "swo": swo_view }),
                                "kernel.event.swo_status_changed", system_runtime_principal(),
                                Some(correlation_for_stream.clone()),
                            ).await;
                        }
                    }
                    KernelEvent::DelegationStarted { parent_swo_id: del_parent, child_swo_ids, to_agent_ids } => {
                        publish_operator_safe_signal(
                            &relay_app, &relay_state, "delegation.decision.recorded",
                            serde_json::json!({
                                "parentSwoId": del_parent,
                                "childSwoIds": child_swo_ids,
                                "toAgentIds": to_agent_ids,
                            }),
                            "kernel.event.delegation_started", system_runtime_principal(),
                            Some(correlation_for_stream.clone()),
                        ).await;
                    }
                    KernelEvent::AgentPresenceChanged { agent_id: presence_agent_id, presence } => {
                        publish_operator_safe_signal(
                            &relay_app, &relay_state, "agent.presence.changed",
                            serde_json::json!({ "agentId": presence_agent_id, "presence": presence }),
                            "kernel.event.agent_presence_changed",
                            system_runtime_principal(), Some(correlation_for_stream.clone()),
                        ).await;
                    }
                    KernelEvent::StreamingDelta { message_id: _, delta, is_final, agent_id } => {
                        if let Some(aid) = agent_id {
                            publish_operator_safe_signal(
                                &relay_app, &relay_state, "agent.activity.delta",
                                serde_json::json!({
                                    "agentId": aid,
                                    "delta": delta,
                                    "isFinal": is_final,
                                }),
                                "kernel.event.streaming_delta",
                                system_runtime_principal(),
                                Some(correlation_for_stream.clone()),
                            ).await;
                        }
                    }
                    KernelEvent::AgentCreated { agent_id: new_agent_id, name, role, parent_id, reason } => {
                        publish_operator_safe_signal(
                            &relay_app, &relay_state, "agent.upserted",
                            serde_json::json!({
                                "agent": {
                                    "id": new_agent_id,
                                    "name": name,
                                    "role": role,
                                    "parentId": parent_id,
                                    "reason": reason,
                                    "presence": "IDLE",
                                }
                            }),
                            "kernel.event.agent_created",
                            system_runtime_principal(),
                            Some(correlation_for_stream.clone()),
                        ).await;
                    }
                    _ => {} // ChatMessage and other events not needed for workspace
                }
            }
            // Worker loop finished — emit agent back to IDLE
            publish_operator_safe_signal(
                &relay_app, &relay_state, "agent.presence.changed",
                serde_json::json!({ "agentId": &owner_agent_id_for_relay, "presence": "IDLE" }),
                "workspace.tauri.submit_work_order.done",
                system_runtime_principal(), Some(correlation_for_stream.clone()),
            ).await;
        });

        let kernel_for_task = Arc::clone(&kernel_arc);
        let payload_str = format!(
            "WORK ORDER\nTitle: {title}\nRequested outcome: {outcome}{}",
            constraints.as_deref().map(|v| format!("\nConstraints: {v}")).unwrap_or_default()
        );
        let task_owner = requested_owner_agent_id.clone();
        let task_app = app.clone();
        let task_state = runtime_state.clone();
        let task_correlation = correlation_id.clone();
        tokio::spawn(async move {
            log::info!("submit_work_order: spawning HSM loop for SWO {} agent={}", swo_id, task_owner);
            let result = Arc::clone(&kernel_for_task.orchestrator)
                .execute_hsm_loop_with_context(
                    task_owner.clone(), None, payload_str, Some(tx),
                    Some(swo_id), None, Some("WORK_ORDER".to_string()),
                    Some("WORK_ORDER".to_string()), Some(task_owner.clone()),
                    Some(task_owner.clone()), None, None, None, None, Some(run_id),
                ).await;
            match &result {
                Ok(v) => log::info!("submit_work_order: HSM loop completed for SWO {}: {:?}", swo_id, v),
                Err(e) => log::error!("submit_work_order: HSM loop FAILED for SWO {}: {:?}", swo_id, e),
            }
            if let Err(error) = result {
                publish_sync_required(
                    &task_app, &task_state, "workspace.tauri.submit_work_order",
                    workspace_operator_principal(), Some(task_correlation.clone()),
                    "runtime_projection_failure",
                    format!("Terminal publish failed for SWO {}: {:?}", swo_id, error),
                ).await;
                return;
            }
            // Publish terminal state for root SWO + descendants
            let mut root_review_response: Option<String> = None;
            let mut root_title: Option<String> = None;
            let mut root_agent_id: Option<String> = None;
            for id in std::iter::once(swo_id).chain(
                kernel_for_task.registry.get_descendant_swo_ids(swo_id).unwrap_or_default()
            ) {
                if let Ok(Some(detail)) = kernel_for_task.registry.get_swo_detail(id) {
                    let swo_view = to_swo_record_view_from_detail(&detail);
                    // Capture root SWO info for inbox item
                    if id == swo_id {
                        root_review_response = swo_view.review_response.clone();
                        root_title = swo_view.work_order_title.clone();
                        root_agent_id = Some(detail.swo.assigned_agent_id.clone());
                    }
                    publish_operator_safe_signal(
                        &task_app, &task_state, "swo.upserted",
                        serde_json::json!({ "swo": swo_view }),
                        "workspace.tauri.submit_work_order.terminal",
                        workspace_operator_principal(), Some(task_correlation.clone()),
                    ).await;
                }
            }

            // Emit inbox.item.upserted for the root SWO so the user sees the deliverable
            if let Some(response) = root_review_response {
                let title = root_title.unwrap_or_else(|| "Work order completed".to_string());
                let agent_id = root_agent_id.unwrap_or_default();
                // Truncate summary for the inbox card (keep full content in the item)
                let summary = if response.len() > 500 {
                    format!("{}...", &response[..497])
                } else {
                    response.clone()
                };
                publish_operator_safe_signal(
                    &task_app, &task_state, "inbox.item.upserted",
                    serde_json::json!({
                        "item": {
                            "id": swo_id.to_string(),
                            "swoId": swo_id.to_string(),
                            "agentId": agent_id,
                            "title": title,
                            "summary": summary,
                        }
                    }),
                    "workspace.tauri.submit_work_order.deliverable",
                    workspace_operator_principal(), Some(task_correlation.clone()),
                ).await;
            }
        });
    }

    let detail = kernel_arc.registry.get_swo_detail(swo_id)
        .map_err(|e| format!("Failed to read work order: {:?}", e))?
        .ok_or_else(|| "Created work order could not be reloaded".to_string())?;
    let created = to_swo_record_view_from_detail(&detail);

    if claimed == 0 {
        publish_operator_safe_signal(
            &app, &runtime_state, "swo.upserted",
            serde_json::json!({ "swo": created.clone() }),
            "workspace.tauri.submit_work_order",
            workspace_operator_principal(), Some(correlation_id),
        ).await;
    }

    Ok(created)
}

#[tauri::command]
async fn cancel_work_order(
    swo_id: i64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        guard.as_ref().cloned().ok_or("Kernel not initialized")?
    };

    tokio::task::spawn_blocking({
        let kernel = Arc::clone(&kernel_arc);
        move || {
            kernel.registry.update_swo_status(swo_id, "CANCELLED")
                .map_err(|e| format!("Failed to cancel SWO {swo_id}: {:?}", e))?;
            kernel.registry.cancel_active_descendant_swos(swo_id)
                .map_err(|e| format!("Failed to cancel descendants of SWO {swo_id}: {:?}", e))?;
            Ok::<(), String>(())
        }
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;

    // Emit updated SWO signal
    if let Ok(Some(detail)) = kernel_arc.registry.get_swo_detail(swo_id) {
        let swo_view = to_swo_record_view_from_detail(&detail);
        publish_operator_safe_signal(
            &app, &state, "swo.upserted",
            serde_json::json!({ "swo": swo_view }),
            "workspace.tauri.cancel_work_order",
            system_runtime_principal(), None,
        ).await;
    }

    Ok(())
}

/// CHA-344 rework loop — re-run a COMPLETED SWO with human revision feedback.
/// Reuses the same SWO row (preserving lineage + delegation tree), stores the
/// feedback in `revision_feedback`, re-opens ancestor chain, and re-enters the
/// HSM loop. The harness receives the feedback via AGENT_REVISION_FEEDBACK env.
#[tauri::command]
async fn queue_request_revision(
    request: RequestRevisionSwoRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SwoRecordView, String> {
    let feedback = request.feedback.trim().to_string();
    if feedback.is_empty() {
        return Err("Revision feedback is required.".to_string());
    }

    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };

    let detail = kernel_arc.registry
        .get_swo_detail(request.swo_id)
        .map_err(|e| format!("SWO detail query failed: {:?}", e))?
        .ok_or_else(|| format!("SWO {} not found", request.swo_id))?;

    if detail.swo.status != "COMPLETED" {
        return Err(format!(
            "SWO {} must be COMPLETED to request a revision (current status: {}).",
            request.swo_id, detail.swo.status
        ));
    }

    // Reopen ancestor SWOs that were closed when this one completed
    let mut ancestor_id = detail.swo.parent_swo_id;
    while let Some(parent_id) = ancestor_id {
        kernel_arc.registry
            .set_swo_status(parent_id, "IN_PROGRESS")
            .map_err(|e| format!("Failed to reopen parent SWO {}: {:?}", parent_id, e))?;
        ancestor_id = kernel_arc.registry
            .get_swo_detail(parent_id)
            .map_err(|e| format!("Failed to read ancestor SWO {}: {:?}", parent_id, e))?
            .and_then(|parent| parent.swo.parent_swo_id);
    }

    // Store the feedback and reset the SWO to PENDING + bump retry_count
    kernel_arc.registry
        .reset_swo_with_revision_feedback(request.swo_id, &feedback)
        .map_err(|e| format!(
            "Failed to reset SWO {} with revision feedback: {:?}",
            request.swo_id, e
        ))?;

    // Audit trail
    let _ = kernel_arc.registry.record_audit_event(
        Some("workspace-operator"),
        Some(request.swo_id),
        "swo_revision_requested",
        TaintLabel::TrustedSystem,
        &serde_json::json!({
            "swoId": request.swo_id,
            "feedback": feedback,
            "mode": "request_revision",
        }),
    );

    let run_id = format!("revision-{}", Uuid::new_v4());
    let claimed = kernel_arc.registry
        .claim_swo_with_run_id(request.swo_id, &run_id)
        .map_err(|e| format!("Failed to claim SWO {} for revision: {:?}", request.swo_id, e))?;
    if claimed == 0 {
        return Err(format!("SWO {} could not be claimed for revision.", request.swo_id));
    }

    let runtime_state = state.inner().clone();
    let correlation_id = format!("revision-{}", request.swo_id);
    let owner_agent_id = detail.swo.owner_agent_id.clone();
    let assigned_agent_id = detail.swo.assigned_agent_id.clone();

    // Emit agent presence → COMPUTING and SWO → IN_PROGRESS immediately
    publish_operator_safe_signal(
        &app, &runtime_state, "agent.presence.changed",
        serde_json::json!({ "agentId": &assigned_agent_id, "presence": "COMPUTING" }),
        "workspace.tauri.queue_request_revision.start",
        workspace_operator_principal(), Some(correlation_id.clone()),
    ).await;
    if let Ok(Some(reloaded)) = kernel_arc.registry.get_swo_detail(request.swo_id) {
        let mut swo_view = to_swo_record_view_from_detail(&reloaded);
        swo_view.status = "IN_PROGRESS".to_string();
        publish_operator_safe_signal(
            &app, &runtime_state, "swo.upserted",
            serde_json::json!({ "swo": swo_view }),
            "workspace.tauri.queue_request_revision.progress",
            workspace_operator_principal(), Some(correlation_id.clone()),
        ).await;
    }

    // Set up kernel event relay (same pattern as submit_work_order)
    let (tx, mut rx) = tokio::sync::mpsc::channel::<KernelEvent>(32);
    let relay_app = app.clone();
    let relay_state = runtime_state.clone();
    let relay_kernel = Arc::clone(&kernel_arc);
    let correlation_for_stream = correlation_id.clone();
    let owner_agent_id_for_relay = assigned_agent_id.clone();
    let revision_swo_id = request.swo_id;

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                KernelEvent::Status(s) => {
                    if let Some(next_status) = sanitize_runtime_status(&s) {
                        emit_runtime_signal(
                            &relay_app, &relay_state, "runtime.status.changed",
                            serde_json::json!({ "status": next_status }),
                            "kernel.event.status", system_runtime_principal(),
                            "desktop", "operator_safe",
                            Some(correlation_for_stream.clone()),
                        ).await;
                    }
                }
                KernelEvent::Error(err) => {
                    publish_operator_safe_signal(
                        &relay_app, &relay_state, "agent.presence.changed",
                        serde_json::json!({ "agentId": &owner_agent_id_for_relay, "presence": "IDLE" }),
                        "kernel.event.error.presence",
                        system_runtime_principal(), Some(correlation_for_stream.clone()),
                    ).await;
                    publish_sync_required(
                        &relay_app, &relay_state, "kernel.event.error",
                        system_runtime_principal(), Some(correlation_for_stream.clone()),
                        "runtime_execution_error",
                        format!("Revision stream error for SWO {}: {}", revision_swo_id, err),
                    ).await;
                }
                KernelEvent::SwoTerminal { swo_id: child_swo_id } => {
                    if let Ok(Some(detail)) = relay_kernel.registry.get_swo_detail(child_swo_id) {
                        let assignee_id = detail.swo.assigned_agent_id.clone();
                        let swo_view = to_swo_record_view_from_detail(&detail);
                        publish_operator_safe_signal(
                            &relay_app, &relay_state, "swo.upserted",
                            serde_json::json!({ "swo": swo_view }),
                            "kernel.event.swo_terminal", system_runtime_principal(),
                            Some(correlation_for_stream.clone()),
                        ).await;
                        publish_operator_safe_signal(
                            &relay_app, &relay_state, "agent.presence.changed",
                            serde_json::json!({ "agentId": assignee_id, "presence": "IDLE" }),
                            "kernel.event.swo_terminal.presence",
                            system_runtime_principal(), Some(correlation_for_stream.clone()),
                        ).await;
                    }
                }
                KernelEvent::ArtifactRegistered { swo_id: artifact_swo_id } => {
                    if let Ok(artifacts) = relay_kernel.registry.get_artifacts_for_swo(artifact_swo_id) {
                        for artifact in artifacts.into_iter().map(to_outbox_artifact_view) {
                            publish_operator_safe_signal(
                                &relay_app, &relay_state, "artifact.created",
                                serde_json::json!({ "artifact": artifact }),
                                "kernel.event.artifact_registered",
                                system_runtime_principal(),
                                Some(correlation_for_stream.clone()),
                            ).await;
                        }
                    }
                }
                KernelEvent::SwoCreated { swo_id: new_swo_id, assigned_agent_id: new_assignee, parent_swo_id: _ } => {
                    if let Ok(Some(detail)) = relay_kernel.registry.get_swo_detail(new_swo_id) {
                        let swo_view = to_swo_record_view_from_detail(&detail);
                        publish_operator_safe_signal(
                            &relay_app, &relay_state, "swo.upserted",
                            serde_json::json!({ "swo": swo_view }),
                            "kernel.event.swo_created", system_runtime_principal(),
                            Some(correlation_for_stream.clone()),
                        ).await;
                    }
                    publish_operator_safe_signal(
                        &relay_app, &relay_state, "agent.presence.changed",
                        serde_json::json!({ "agentId": new_assignee, "presence": "COMPUTING" }),
                        "kernel.event.swo_created.presence",
                        system_runtime_principal(), Some(correlation_for_stream.clone()),
                    ).await;
                }
                KernelEvent::SwoStatusChanged { swo_id: changed_swo_id, new_status: _ } => {
                    if let Ok(Some(detail)) = relay_kernel.registry.get_swo_detail(changed_swo_id) {
                        let swo_view = to_swo_record_view_from_detail(&detail);
                        publish_operator_safe_signal(
                            &relay_app, &relay_state, "swo.upserted",
                            serde_json::json!({ "swo": swo_view }),
                            "kernel.event.swo_status_changed", system_runtime_principal(),
                            Some(correlation_for_stream.clone()),
                        ).await;
                    }
                }
                KernelEvent::DelegationStarted { parent_swo_id: del_parent, child_swo_ids, to_agent_ids } => {
                    publish_operator_safe_signal(
                        &relay_app, &relay_state, "delegation.decision.recorded",
                        serde_json::json!({
                            "parentSwoId": del_parent,
                            "childSwoIds": child_swo_ids,
                            "toAgentIds": to_agent_ids,
                        }),
                        "kernel.event.delegation_started", system_runtime_principal(),
                        Some(correlation_for_stream.clone()),
                    ).await;
                }
                KernelEvent::AgentPresenceChanged { agent_id: presence_agent_id, presence } => {
                    publish_operator_safe_signal(
                        &relay_app, &relay_state, "agent.presence.changed",
                        serde_json::json!({ "agentId": presence_agent_id, "presence": presence }),
                        "kernel.event.agent_presence_changed",
                        system_runtime_principal(), Some(correlation_for_stream.clone()),
                    ).await;
                }
                KernelEvent::StreamingDelta { message_id: _, delta, is_final, agent_id } => {
                    if let Some(aid) = agent_id {
                        publish_operator_safe_signal(
                            &relay_app, &relay_state, "agent.activity.delta",
                            serde_json::json!({
                                "agentId": aid,
                                "delta": delta,
                                "isFinal": is_final,
                            }),
                            "kernel.event.streaming_delta",
                            system_runtime_principal(),
                            Some(correlation_for_stream.clone()),
                        ).await;
                    }
                }
                _ => {} // ChatMessage and other events not needed here
            }
        }
        // Worker loop finished — emit agent back to IDLE
        publish_operator_safe_signal(
            &relay_app, &relay_state, "agent.presence.changed",
            serde_json::json!({ "agentId": &owner_agent_id_for_relay, "presence": "IDLE" }),
            "workspace.tauri.queue_request_revision.done",
            system_runtime_principal(), Some(correlation_for_stream.clone()),
        ).await;
    });

    // Re-enter the HSM loop with the same SWO id. The orchestrator will read
    // revision_feedback from the SWO record and pass AGENT_REVISION_FEEDBACK
    // through to the Python harness.
    let kernel_for_task = Arc::clone(&kernel_arc);
    let payload_str = format!(
        "WORK ORDER (REVISION)\nTitle: {}\nRequested outcome: {}",
        detail.swo.work_order_title.clone().unwrap_or_default(),
        detail.swo.work_order_outcome.clone().unwrap_or_default()
    );
    let task_owner = assigned_agent_id.clone();
    let task_app = app.clone();
    let task_state = runtime_state.clone();
    let task_correlation = correlation_id.clone();
    tokio::spawn(async move {
        log::info!("queue_request_revision: spawning HSM loop for SWO {} agent={}", revision_swo_id, task_owner);
        let result = Arc::clone(&kernel_for_task.orchestrator)
            .execute_hsm_loop_with_context(
                task_owner.clone(), None, payload_str, Some(tx),
                Some(revision_swo_id), None, Some("WORK_ORDER".to_string()),
                Some("WORK_ORDER".to_string()), Some(task_owner.clone()),
                Some(task_owner.clone()), None, None, None, None, Some(run_id),
            ).await;
        match &result {
            Ok(v) => log::info!("queue_request_revision: HSM loop completed for SWO {}: {:?}", revision_swo_id, v),
            Err(e) => log::error!("queue_request_revision: HSM loop FAILED for SWO {}: {:?}", revision_swo_id, e),
        }
        if let Err(error) = result {
            publish_sync_required(
                &task_app, &task_state, "workspace.tauri.queue_request_revision",
                workspace_operator_principal(), Some(task_correlation.clone()),
                "runtime_projection_failure",
                format!("Revision execution failed for SWO {}: {:?}", revision_swo_id, error),
            ).await;
            return;
        }
        // Publish terminal state for the revised SWO and descendants
        for id in std::iter::once(revision_swo_id).chain(
            kernel_for_task.registry.get_descendant_swo_ids(revision_swo_id).unwrap_or_default()
        ) {
            if let Ok(Some(detail)) = kernel_for_task.registry.get_swo_detail(id) {
                let swo_view = to_swo_record_view_from_detail(&detail);
                publish_operator_safe_signal(
                    &task_app, &task_state, "swo.upserted",
                    serde_json::json!({ "swo": swo_view }),
                    "workspace.tauri.queue_request_revision.terminal",
                    workspace_operator_principal(), Some(task_correlation.clone()),
                ).await;
            }
        }
    });

    let refreshed = kernel_arc.registry
        .get_swo_detail(request.swo_id)
        .map_err(|e| format!("Failed to reload SWO after revision reset: {:?}", e))?
        .ok_or_else(|| format!("SWO {} not found after revision reset", request.swo_id))?;
    let _ = owner_agent_id; // reserved for future provenance checks
    Ok(to_swo_record_view_from_detail(&refreshed))
}

#[tauri::command]
async fn queue_review_decide(
    request: QueueReviewDecisionRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let decision = request.decision.trim().to_lowercase();
    if !matches!(decision.as_str(), "approve" | "reject" | "revise") {
        return Err("Decision must be approve, reject, or revise.".to_string());
    }
    if request.meta.command_id.trim().is_empty()
        || request.meta.correlation_id.trim().is_empty()
        || request.meta.source.trim().is_empty()
    {
        return Err("Command metadata requires commandId, correlationId, and source.".to_string());
    }

    let reviewer_agent_id = state.perry_id.lock().await.clone()
        .ok_or_else(|| "Perry is not initialized".to_string())?;
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(kernel) = guard.as_ref() { Arc::clone(kernel) }
        else { return Err("Kernel not initialized".into()); }
    };
    let runtime_state = state.inner().clone();

    audit_runtime_bus_payload(
        &runtime_state, "runtime_command_received",
        &serde_json::json!({
            "command": "approval.decide",
            "meta": request.meta, "swoId": request.swo_id, "decision": decision,
        }),
        request.meta.principal.id.as_deref(), Some(request.swo_id),
    ).await;

    let swo_id = request.swo_id;
    let reasoning = request.reasoning.clone();
    let final_response = request.final_response.clone();
    let kernel_for_task = Arc::clone(&kernel_arc);
    let reviewer = reviewer_agent_id.clone();
    let dec = decision.clone();

    tokio::task::spawn_blocking(move || {
        let detail = kernel_for_task.registry.get_swo_detail(swo_id)
            .map_err(|e| format!("SWO detail query failed: {:?}", e))?
            .ok_or_else(|| format!("SWO {} not found", swo_id))?;
        if matches!(detail.swo.status.as_str(), "COMPLETED" | "FAILED" | "CANCELLED") {
            return Err(format!("SWO {} is already terminal with status {}.", swo_id, detail.swo.status));
        }
        let (action, next_status) = match dec.as_str() {
            "approve" => ("APPROVE_AND_REPLY", "COMPLETED"),
            "reject" => ("CLOSED_FAILED", "FAILED"),
            "revise" => ("REJECTED_ROUTE_CONTRACT", "FAILED"),
            _ => unreachable!(),
        };
        let trimmed_reasoning = reasoning.trim();
        if trimmed_reasoning.is_empty() {
            return Err("Review reasoning is required.".to_string());
        }
        let fr = final_response.as_deref().filter(|v| !v.trim().is_empty());
        kernel_for_task.registry.record_manager_review(
            swo_id, &reviewer, action, trimmed_reasoning, fr,
        ).map_err(|e| format!("Failed to record review: {:?}", e))?;
        kernel_for_task.registry.update_swo_status(swo_id, next_status)
            .map_err(|e| format!("Failed to update SWO status: {:?}", e))?;
        if next_status == "COMPLETED" {
            let _ = kernel_for_task.registry.cancel_active_descendant_swos(swo_id);
        }
        Ok::<(), String>(())
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;

    // Publish approval removed + swo upserted signals
    publish_operator_safe_signal(
        &app, &runtime_state, "approval.removed",
        serde_json::json!({ "approvalId": format!("approval-{swo_id}"), "swoId": swo_id }),
        "workspace.tauri.queue_review_decide",
        workspace_operator_principal(), Some(request.meta.correlation_id.clone()),
    ).await;

    if let Ok(Some(detail)) = kernel_arc.registry.get_swo_detail(swo_id) {
        let swo_view = to_swo_record_view_from_detail(&detail);
        publish_operator_safe_signal(
            &app, &runtime_state, "swo.upserted",
            serde_json::json!({ "swo": swo_view }),
            "workspace.tauri.queue_review_decide",
            workspace_operator_principal(), Some(request.meta.correlation_id),
        ).await;
    }

    Ok(())
}

// ---- Settings (3) ----

#[tauri::command]
async fn settings_load() -> Result<SettingsView, String> {
    let config = read_saved_config()?.unwrap_or_else(|| AppConfig {
        default_llm_provider: DEFAULT_LLM_PROVIDER.to_string(),
        default_llm_model: DEFAULT_LLM_MODEL.to_string(),
        llm_credentials: default_llm_credential_configs(),
        tool_credentials: default_tool_credential_configs(),
        ..AppConfig::default()
    });
    build_settings_view(&config)
}

#[tauri::command]
async fn settings_save(request: SettingsSaveRequest, state: State<'_, AppState>) -> Result<SettingsView, String> {
    let default_llm_provider = validate_slug("Default provider", &request.default_llm_provider)?;
    // Model can be empty (means "use provider default")
    let default_llm_model = request.default_llm_model.trim().to_string();

    if let Some(token) = request.sidechannel_token.as_ref() {
        validate_secret("sidechannel_token", token, MAX_SIDECHANNEL_TOKEN_LEN)?;
        save_secret(KEY_SIDECHANNEL_TOKEN, token.trim())?;
    }

    let mut llm_credentials = Vec::new();
    for entry in &request.llm_credentials {
        let normalized = normalize_service_config("LLM provider", entry)?;
        if let Some(secret) = entry.api_key.as_ref() {
            save_secret(&keyring_account_for_llm_provider(&normalized.slug), secret.trim())?;
            if normalized.slug == DEFAULT_LLM_PROVIDER {
                save_secret(KEY_API_KEY, secret.trim())?;
            }
        }
        llm_credentials.push(normalized);
    }

    let mut tool_credentials = Vec::new();
    for entry in &request.tool_credentials {
        let normalized = normalize_service_config("Tool provider", entry)?;
        if let Some(secret) = entry.api_key.as_ref() {
            save_secret(&keyring_account_for_tool(&normalized.slug), secret.trim())?;
        }
        tool_credentials.push(normalized);
    }

    let config = AppConfig {
        llm_api_key: String::new(),
        default_llm_provider,
        default_llm_model,
        llm_credentials,
        tool_credentials,
        sidechannel_token: String::new(),
        sairgent_agent_provider: String::new(),
        sairgent_agent_model: String::new(),
    };
    let provider_for_agents = config.default_llm_provider.clone();
    let model_for_agents = config.default_llm_model.clone();
    log::info!("settings_save: provider={}, model={}", provider_for_agents, model_for_agents);
    save_saved_config(&config)?;

    // Propagate provider/model to all agents in the kernel DB
    if let Some(kernel) = state.kernel.lock().await.as_ref().map(Arc::clone) {
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(agents) = kernel.registry.list_agents() {
                let ids: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
                match kernel.registry.update_agent_models_bulk(&ids, &provider_for_agents, &model_for_agents) {
                    Ok(count) => log::info!("settings_save: updated {} agents to provider={}, model={}", count, provider_for_agents, model_for_agents),
                    Err(e) => log::error!("settings_save: failed to update agent models: {:?}", e),
                }
            }
        }).await;
    }

    build_settings_view(&config)
}

#[tauri::command]
async fn secrets_set(request: SecretsSetRequest) -> Result<(), String> {
    log::info!("secrets_set called: provider={}", request.provider);
    let provider = validate_slug("Provider", &request.provider)?;
    validate_secret("API key", &request.key, MAX_API_KEY_LEN)?;
    let account = keyring_account_for_llm_provider(&provider);
    log::info!("Saving secret for account: {}", account);
    save_secret(&account, request.key.trim())?;
    log::info!("Secret saved successfully for {}", account);
    if provider == DEFAULT_LLM_PROVIDER {
        save_secret(KEY_API_KEY, request.key.trim())?;
        log::info!("Also saved as default LLM key");
    }
    Ok(())
}

// ---- Agent (2) ----

#[tauri::command]
async fn roster_tree(state: State<'_, AppState>) -> Result<Vec<AgentTreeNodeView>, String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    let records = tokio::task::spawn_blocking(move || {
        let rows = kernel_arc.registry.get_agent_tree_snapshot(now_unix_ms())
            .map_err(|e| format!("Roster query failed: {:?}", e))?;
        Ok::<Vec<AgentTreeNodeView>, String>(rows.into_iter().map(to_agent_tree_node_view).collect())
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;
    Ok(records)
}

#[tauri::command]
async fn agent_detail(
    agent_id: String,
    state: State<'_, AppState>,
) -> Result<AgentDetailView, String> {
    validate_agent_id(&agent_id)?;
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    tokio::task::spawn_blocking(move || {
        let detail = kernel_arc.registry.get_agent_detail_snapshot(&agent_id, now_unix_ms())
            .map_err(|e| format!("Agent detail query failed: {:?}", e))?;
        Ok::<AgentDetailView, String>(to_agent_detail_view(detail))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentIdentityUpdateRequest {
    agent_id: String,
    role: Option<String>,
    raison_detre: Option<String>,
    persona_prompt: Option<String>,
    default_provider: Option<String>,
    default_model: Option<String>,
    triage_model: Option<Option<String>>,
    execution_model: Option<Option<String>>,
}

#[tauri::command]
async fn agent_identity_update(
    request: AgentIdentityUpdateRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_agent_id(&request.agent_id)?;
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    tokio::task::spawn_blocking(move || {
        kernel_arc.registry.update_agent_identity(
            &request.agent_id,
            request.role.as_deref(),
            request.raison_detre.as_deref(),
            request.persona_prompt.as_deref(),
            request.default_provider.as_deref(),
            request.default_model.as_deref(),
            request.triage_model.as_ref().map(|o| o.as_deref()),
            request.execution_model.as_ref().map(|o| o.as_deref()),
        ).map_err(|e| format!("Agent identity update failed: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))?
}

// =========================================================================
// CLI Tool commands
// =========================================================================

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CliToolView {
    id: String,
    slug: String,
    name: String,
    summary: Option<String>,
    command: String,
    args: Option<Vec<String>>,
    env: Option<serde_json::Value>,
    cwd: Option<String>,
    enabled: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliToolUpsertRequest {
    id: Option<String>,
    slug: String,
    name: String,
    summary: Option<String>,
    command: String,
    args: Option<Vec<String>>,
    env: Option<serde_json::Value>,
    cwd: Option<String>,
    enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// MCP Connector view / request types
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct McpConnectorView {
    id: String,
    slug: String,
    name: String,
    summary: String,
    transport: String,
    command: Option<String>,
    args: Option<Vec<String>>,
    url: Option<String>,
    enabled: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpConnectorUpsertRequest {
    connector_id: Option<String>,
    slug: String,
    name: String,
    summary: String,
    transport: String,
    command: Option<String>,
    args: Option<Vec<String>>,
    url: Option<String>,
    enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// Recurring Template view / request types
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecurringTemplateView {
    template_id: String,
    name: String,
    title: String,
    outcome: String,
    constraints: Option<String>,
    priority: String,
    assignee_agent_id: Option<String>,
    assignee_agent_name: Option<String>,
    schedule_json: String,
    status: String,
    next_run_at: Option<String>,
    last_run_at: Option<String>,
    last_run_status: Option<String>,
    created_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TriggerTemplateRequest {
    template_id: String,
}

// ---------------------------------------------------------------------------
// Converter: McpConnectorRecord → McpConnectorView
// ---------------------------------------------------------------------------

fn to_mcp_connector_view(record: McpConnectorRecord) -> McpConnectorView {
    McpConnectorView {
        id: record.id,
        slug: record.slug,
        name: record.name,
        summary: record.summary,
        transport: record.transport.as_str().to_string(),
        command: record.command,
        args: record.args,
        url: record.url,
        enabled: record.enabled,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

// ---------------------------------------------------------------------------
// Converter: RecurringWorkOrderTemplateRecord → RecurringTemplateView
// ---------------------------------------------------------------------------

fn to_recurring_template_view(record: RecurringWorkOrderTemplateRecord) -> RecurringTemplateView {
    RecurringTemplateView {
        template_id: record.template_id,
        name: record.name,
        title: record.title,
        outcome: record.outcome,
        constraints: record.constraints,
        priority: record.priority,
        assignee_agent_id: record.assignee_agent_id.clone(),
        assignee_agent_name: record.assignee_agent_name,
        schedule_json: serde_json::to_string(&record.schedule).unwrap_or_default(),
        status: record.status,
        next_run_at: record.next_run_at,
        last_run_at: record.last_run_at,
        last_run_status: record.last_run_status,
        created_at: record.created_at,
    }
}

fn to_cli_tool_view(r: sairgent_kernel::tools::CliToolRecord) -> CliToolView {
    let args: Option<Vec<String>> = r.args_json.as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    let env: Option<serde_json::Value> = r.env_json.as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    CliToolView {
        id: r.id,
        slug: r.slug,
        name: r.name,
        summary: r.summary,
        command: r.command,
        args,
        env,
        cwd: r.cwd,
        enabled: r.enabled,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

#[tauri::command]
async fn cli_tools_list(state: State<'_, AppState>) -> Result<Vec<CliToolView>, String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    let records = tokio::task::spawn_blocking(move || {
        kernel_arc.registry.list_cli_tools()
            .map_err(|e| format!("Failed to list CLI tools: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;
    Ok(records.into_iter().map(to_cli_tool_view).collect())
}

#[tauri::command]
async fn cli_tool_upsert(
    request: CliToolUpsertRequest,
    state: State<'_, AppState>,
) -> Result<CliToolView, String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    let kernel_req = sairgent_kernel::tools::CliToolUpsertRequest {
        id: request.id,
        slug: request.slug,
        name: request.name,
        summary: request.summary,
        command: request.command,
        args: request.args,
        env: request.env.as_ref().and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
        }),
        cwd: request.cwd,
        enabled: request.enabled,
    };
    let record = tokio::task::spawn_blocking(move || {
        kernel_arc.registry.upsert_cli_tool(&kernel_req)
            .map_err(|e| format!("Failed to upsert CLI tool: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;
    Ok(to_cli_tool_view(record))
}

#[tauri::command]
async fn cli_tool_delete(
    tool_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    tokio::task::spawn_blocking(move || {
        kernel_arc.registry.delete_cli_tool(&tool_id)
            .map_err(|e| format!("Failed to delete CLI tool: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))?
}

// =========================================================================
// MCP Connector commands
// =========================================================================

#[tauri::command]
async fn mcp_connectors_list(state: State<'_, AppState>) -> Result<Vec<McpConnectorView>, String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    let records = tokio::task::spawn_blocking(move || {
        kernel_arc.registry.list_mcp_connectors()
            .map_err(|e| format!("Failed to list MCP connectors: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;
    Ok(records.into_iter().map(to_mcp_connector_view).collect())
}

#[tauri::command]
async fn mcp_connector_upsert(
    request: McpConnectorUpsertRequest,
    state: State<'_, AppState>,
) -> Result<McpConnectorView, String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    let kernel_req = KernelMcpConnectorUpsertRequest {
        id: request.connector_id,
        slug: request.slug,
        name: request.name,
        summary: Some(request.summary),
        transport: request.transport,
        command: request.command,
        args: request.args,
        env: None,
        url: request.url,
        headers: None,
        cwd: None,
        enabled: request.enabled,
    };
    let record = tokio::task::spawn_blocking(move || {
        kernel_arc.registry.upsert_mcp_connector(&kernel_req)
            .map_err(|e| format!("Failed to upsert MCP connector: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;
    Ok(to_mcp_connector_view(record))
}

#[tauri::command]
async fn mcp_connector_delete(
    connector_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    tokio::task::spawn_blocking(move || {
        kernel_arc.registry.delete_mcp_connector(&connector_id)
            .map_err(|e| format!("Failed to delete MCP connector: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))?
}

#[tauri::command]
async fn agent_bind_mcp(
    agent_id: String,
    connector_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_agent_id(&agent_id)?;
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    tokio::task::spawn_blocking(move || {
        kernel_arc.registry.bind_mcp_connector_to_agent(&agent_id, &connector_id)
            .map_err(|e| format!("Failed to bind MCP connector: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))?
}

#[tauri::command]
async fn agent_unbind_mcp(
    agent_id: String,
    connector_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_agent_id(&agent_id)?;
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    tokio::task::spawn_blocking(move || {
        kernel_arc.registry.unbind_mcp_connector_from_agent(&agent_id, &connector_id)
            .map_err(|e| format!("Failed to unbind MCP connector: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))?
}

#[tauri::command]
async fn agent_bind_tool(
    agent_id: String,
    tool_slug: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_agent_id(&agent_id)?;
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    tokio::task::spawn_blocking(move || {
        kernel_arc.registry.bind_tool_to_agent(&agent_id, &tool_slug)
            .map_err(|e| format!("Failed to bind tool: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))?
}

#[tauri::command]
async fn agent_unbind_tool(
    agent_id: String,
    tool_slug: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_agent_id(&agent_id)?;
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    tokio::task::spawn_blocking(move || {
        kernel_arc.registry.unbind_tool_from_agent(&agent_id, &tool_slug)
            .map_err(|e| format!("Failed to unbind tool: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))?
}

// =========================================================================
// Recurring Template commands
// =========================================================================

#[tauri::command]
async fn recurring_templates_list(
    state: State<'_, AppState>,
) -> Result<Vec<RecurringTemplateView>, String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    let records = tokio::task::spawn_blocking(move || {
        kernel_arc.registry.list_recurring_templates()
            .map_err(|e| format!("Failed to list recurring templates: {:?}", e))
    }).await.map_err(|e| format!("Task join failed: {:?}", e))??;
    Ok(records.into_iter().map(to_recurring_template_view).collect())
}

#[tauri::command]
async fn recurring_template_trigger(
    request: TriggerTemplateRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() { Arc::clone(k) }
        else { return Err("Kernel not initialized".into()); }
    };
    kernel_arc
        .orchestrator
        .clone()
        .trigger_recurring_template_now(request.template_id)
        .await
        .map(|_| ())
        .map_err(|e| format!("Failed to trigger recurring template: {:?}", e))
}

// =========================================================================
// Model discovery
// =========================================================================

const MODEL_CACHE_TTL_SECS: u64 = 300;

#[derive(Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelEntry>,
}

#[derive(Deserialize)]
struct OpenAiModelEntry {
    id: String,
}

#[derive(Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModelEntry>,
}

#[derive(Deserialize)]
struct AnthropicModelEntry {
    id: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModelEntry>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct OllamaModelEntry {
    name: String,
}

async fn discover_models_for_provider(slug: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    match slug {
        "anthropic" => {
            let key = load_secret_optional(&keyring_account_for_llm_provider("anthropic"))?
                .ok_or_else(|| "No Anthropic API key configured.".to_string())?;
            let resp = client
                .get("https://api.anthropic.com/v1/models")
                .header("x-api-key", key.trim())
                .header("anthropic-version", "2023-06-01")
                .send().await.map_err(|e| format!("Anthropic request failed: {}", e))?;
            let body: AnthropicModelsResponse = resp.json().await
                .map_err(|e| format!("Anthropic parse failed: {}", e))?;
            let mut ids: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
            ids.sort(); ids.dedup(); Ok(ids)
        }
        "openai" => {
            let key = load_secret_optional(&keyring_account_for_llm_provider("openai"))?
                .or(load_secret_optional(KEY_API_KEY)?)
                .ok_or_else(|| "No OpenAI API key configured.".to_string())?;
            let resp = client
                .get("https://api.openai.com/v1/models")
                .header("Authorization", format!("Bearer {}", key.trim()))
                .send().await.map_err(|e| format!("OpenAI request failed: {}", e))?;
            let body: OpenAiModelsResponse = resp.json().await
                .map_err(|e| format!("OpenAI parse failed: {}", e))?;
            let mut ids: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
            ids.sort(); ids.dedup(); Ok(ids)
        }
        "openrouter" => {
            let key = load_secret_optional(&keyring_account_for_llm_provider("openrouter"))?
                .ok_or_else(|| "No OpenRouter API key configured.".to_string())?;
            let resp = client
                .get("https://openrouter.ai/api/v1/models")
                .header("Authorization", format!("Bearer {}", key.trim()))
                .send().await.map_err(|e| format!("OpenRouter request failed: {}", e))?;
            let body: OpenAiModelsResponse = resp.json().await
                .map_err(|e| format!("OpenRouter parse failed: {}", e))?;
            let mut ids: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
            ids.sort(); ids.dedup(); Ok(ids)
        }
        "groq" => {
            let key = load_secret_optional(&keyring_account_for_llm_provider("groq"))?
                .ok_or_else(|| "No Groq API key configured.".to_string())?;
            let resp = client
                .get("https://api.groq.com/openai/v1/models")
                .header("Authorization", format!("Bearer {}", key.trim()))
                .send().await.map_err(|e| format!("Groq request failed: {}", e))?;
            let body: OpenAiModelsResponse = resp.json().await
                .map_err(|e| format!("Groq parse failed: {}", e))?;
            let mut ids: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
            ids.sort(); ids.dedup(); Ok(ids)
        }
        _ => Err(format!("Unknown provider: {}", slug)),
    }
}

#[tauri::command]
async fn provider_discover_models(
    slug: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let normalized = slug.trim().to_lowercase();
    // Check cache
    {
        let cache = state.model_discovery_cache.lock().map_err(|e| e.to_string())?;
        if let Some((models, fetched_at)) = cache.get(&normalized) {
            if fetched_at.elapsed().as_secs() < MODEL_CACHE_TTL_SECS {
                return Ok(models.clone());
            }
        }
    }
    let models = discover_models_for_provider(&normalized).await?;
    {
        let mut cache = state.model_discovery_cache.lock().map_err(|e| e.to_string())?;
        cache.insert(normalized, (models.clone(), Instant::now()));
    }
    Ok(models)
}

// ---- Token Usage (2) ----

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenUsageView {
    id: i64,
    run_id: String,
    swo_id: Option<i64>,
    agent_id: String,
    provider: String,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    requests: i64,
    cost_usd: Option<f64>,
    created_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentTokenTotalsView {
    agent_id: String,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    total_tokens: i64,
    estimated_cost_usd: Option<f64>,
    run_count: i64,
}

impl From<KernelTokenUsageRecord> for TokenUsageView {
    fn from(r: KernelTokenUsageRecord) -> Self {
        Self {
            id: r.id,
            run_id: r.run_id,
            swo_id: r.swo_id,
            agent_id: r.agent_id,
            provider: r.provider,
            model: r.model,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cache_read_tokens: r.cache_read_tokens,
            cache_write_tokens: r.cache_write_tokens,
            requests: r.requests,
            cost_usd: r.cost_usd,
            created_at: r.created_at,
        }
    }
}

impl From<KernelAgentTokenTotals> for AgentTokenTotalsView {
    fn from(t: KernelAgentTokenTotals) -> Self {
        Self {
            agent_id: t.agent_id,
            input_tokens: t.input_tokens,
            output_tokens: t.output_tokens,
            cache_read_tokens: t.cache_read_tokens,
            total_tokens: t.total_tokens,
            estimated_cost_usd: t.estimated_cost_usd,
            run_count: t.run_count,
        }
    }
}

#[tauri::command]
async fn token_usage_for_swo(
    swo_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<TokenUsageView>, String> {
    let kernel_arc = {
        let g = state.kernel.lock().await;
        g.as_ref().cloned().ok_or("Kernel not initialized")?
    };
    let records = kernel_arc
        .registry
        .get_token_usage_for_swo(swo_id)
        .map_err(|e| format!("Failed to load token usage: {:?}", e))?;
    Ok(records.into_iter().map(TokenUsageView::from).collect())
}

#[tauri::command]
async fn token_usage_totals(
    state: State<'_, AppState>,
) -> Result<Vec<AgentTokenTotalsView>, String> {
    let kernel_arc = {
        let g = state.kernel.lock().await;
        g.as_ref().cloned().ok_or("Kernel not initialized")?
    };
    let totals = kernel_arc
        .registry
        .get_token_usage_totals()
        .map_err(|e| format!("Failed to load token totals: {:?}", e))?;
    Ok(totals.into_iter().map(AgentTokenTotalsView::from).collect())
}

// =========================================================================
// Artifact commands
// =========================================================================

#[tauri::command]
async fn artifacts_for_swo(
    swo_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<OutboxArtifactView>, String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() {
            Arc::clone(k)
        } else {
            return Err("Kernel not initialized".into());
        }
    };
    let records = tokio::task::spawn_blocking(move || {
        kernel_arc.registry.get_artifacts_for_swo(swo_id)
    })
    .await
    .map_err(|e| format!("Task join failed: {:?}", e))?
    .map_err(|e| format!("Registry error: {:?}", e))?;
    Ok(records.into_iter().map(to_outbox_artifact_view).collect())
}

#[tauri::command]
async fn preview_generated_artifact(
    artifact_id: i64,
    state: State<'_, AppState>,
) -> Result<ArtifactPreviewView, String> {
    let kernel_arc = {
        let guard = state.kernel.lock().await;
        if let Some(k) = guard.as_ref() {
            Arc::clone(k)
        } else {
            return Err("Kernel not initialized".into());
        }
    };
    let artifact = tokio::task::spawn_blocking(move || {
        kernel_arc.registry.get_outbox_artifact(artifact_id)
    })
    .await
    .map_err(|e| format!("Task join failed: {:?}", e))?
    .map_err(|e| format!("Registry error: {:?}", e))?
    .ok_or_else(|| format!("Artifact {} not found", artifact_id))?;

    let path = PathBuf::from(&artifact.absolute_path);
    let content_type = infer_artifact_content_type(&path);
    let size_bytes = fs::metadata(&path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    if !is_text_previewable(&content_type) {
        return Ok(ArtifactPreviewView {
            artifact_id: artifact.id,
            filename: artifact.filename,
            content_type,
            render_mode: "binary".to_string(),
            content: String::new(),
            size_bytes,
            truncated: false,
        });
    }

    let (content, truncated) = read_artifact_preview(&path)?;
    let render_mode = artifact_render_mode(&content_type).to_string();

    Ok(ArtifactPreviewView {
        artifact_id: artifact.id,
        filename: artifact.filename,
        content_type,
        render_mode,
        content,
        size_bytes,
        truncated,
    })
}

// =========================================================================
// Tauri application entry point
// =========================================================================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    eprintln!("[workspace] run() starting...");

    // Safety cleanup: Ensure no storage exists in src-tauri CWD during dev
    if let Ok(cwd) = std::env::current_dir() {
        eprintln!("[workspace] cwd = {:?}", cwd);
        let stray_storage = cwd.join("storage");
        if stray_storage.exists() {
            let _ = std::fs::remove_dir_all(stray_storage);
        }
    }

    eprintln!("[workspace] building tauri app...");
    tauri::Builder::default()
        .manage(AppState {
            kernel: Arc::new(Mutex::new(None)),
            perry_id: Arc::new(Mutex::new(None)),
            runtime_bus: Arc::new(RuntimeBusState::new()),
            last_hsm_status: Arc::new(StdMutex::new("READY".to_string())),
            processed_command_ids: Arc::new(StdMutex::new(LruCache::new(
                NonZeroUsize::new(10000).unwrap(),
            ))),
            bootstrap_cache: Arc::new(Mutex::new(None)),
            model_discovery_cache: Arc::new(StdMutex::new(HashMap::new())),
        })
        .invoke_handler(tauri::generate_handler![
            // Boot (3)
            secrets_status,
            kernel_boot_from_keychain,
            kernel_boot_with_secrets,
            // Runtime (3)
            runtime_bootstrap,
            runtime_replay,
            runtime_command,
            // Work (4)
            submit_work_order,
            cancel_work_order,
            queue_request_revision,
            queue_review_decide,
            // Settings (3)
            settings_load,
            settings_save,
            secrets_set,
            // Agent (3)
            roster_tree,
            agent_detail,
            agent_identity_update,
            // Discovery (1)
            provider_discover_models,
            // Token Usage (2)
            token_usage_for_swo,
            token_usage_totals,
            // Artifacts (2)
            artifacts_for_swo,
            preview_generated_artifact,
            // CLI Tools (3)
            cli_tools_list,
            cli_tool_upsert,
            cli_tool_delete,
            // MCP Connectors (5)
            mcp_connectors_list,
            mcp_connector_upsert,
            mcp_connector_delete,
            agent_bind_mcp,
            agent_unbind_mcp,
            // CLI Tool Bindings (2)
            agent_bind_tool,
            agent_unbind_tool,
            // Recurring Templates (2)
            recurring_templates_list,
            recurring_template_trigger,
        ])
        .setup(|app| {
            eprintln!("[workspace] setup callback running...");
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // System tray: keep the kernel alive when the window is closed (CHA-365)
            let show_item = tauri::menu::MenuItem::with_id(app, "show", "Open Workspace", true, None::<&str>)?;
            let quit_item = tauri::menu::MenuItem::with_id(app, "quit", "Quit Sairgent", true, None::<&str>)?;
            let separator = tauri::menu::PredefinedMenuItem::separator(app)?;
            let tray_menu = tauri::menu::Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

            let _tray = tauri::tray::TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().expect("no default window icon"))
                .tooltip("Sairgent Workspace")
                .menu(&tray_menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        std::process::exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(event, tauri::tray::TrayIconEvent::DoubleClick { .. }) {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            eprintln!("[workspace] system tray initialized");
            eprintln!("[workspace] setup complete, window should open");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Close-to-tray: hide the window instead of destroying it (CHA-365)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
