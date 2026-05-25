use crate::error::{KernelError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InitiativeDefaults {
    pub initiative_id: Option<String>,
    pub initiative_name: Option<String>,
    pub initiative_owner_agent_name: Option<String>,
    pub priority_class: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeedAgentSpec {
    pub name: String,
    pub manager_name: Option<String>,
    pub role: String,
    pub persona_prompt: Option<String>,
    pub raison_detre: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cron_interval_seconds: Option<i64>,
    pub triage_model: Option<String>,
    pub execution_model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeedSwoSpec {
    pub assigned_agent_name: String,
    pub owner_agent_name: String,
    pub created_by_agent_name: String,
    pub payload: String,
    pub status: String,
    pub kind: String,
    pub source: String,
    pub initiative_id: Option<String>,
    pub initiative_name: Option<String>,
    pub initiative_owner_agent_name: Option<String>,
    pub priority_class: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeedMcpConnectorSpec {
    pub slug: String,
    pub name: String,
    pub summary: String,
    pub transport: String,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeedMcpBindingSpec {
    pub agent_name: String,
    pub connector_slugs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeedRecurringTemplateSpec {
    pub name: String,
    pub assignee_agent_name: String,
    pub title: String,
    pub outcome: String,
    pub constraints: Option<String>,
    pub priority: String,
    pub schedule: SeedRecurringSchedule,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeedRecurringSchedule {
    pub cadence: String,      // "HOURLY", "DAILY", "WEEKLY"
    pub interval: i64,        // e.g. 1 for every hour/day
    pub timezone: String,     // e.g. "UTC"
    pub days_of_week: Option<Vec<i64>>, // 1=Mon..5=Fri for DAILY
    pub hour: Option<i64>,    // hour of day (0-23)
    pub minute: Option<i64>,  // minute of hour (0-59)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeSeedSpec {
    pub profile_id: String,
    pub company_name: String,
    pub company_charter_source: String,
    pub company_summary: String,
    pub operating_principles: Vec<String>,
    pub non_goals: Vec<String>,
    pub default_provider: String,
    pub default_model: String,
    pub autonomous_hiring_mode: String,
    pub initiative_defaults: InitiativeDefaults,
    pub agents: Vec<SeedAgentSpec>,
    pub starter_swos: Vec<SeedSwoSpec>,
    #[serde(default)]
    pub mcp_connectors: Vec<SeedMcpConnectorSpec>,
    #[serde(default)]
    pub mcp_bindings: Vec<SeedMcpBindingSpec>,
    #[serde(default)]
    pub recurring_templates: Vec<SeedRecurringTemplateSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeArchiveCounts {
    pub agents: usize,
    pub active_swos: usize,
    pub heartbeats: usize,
    pub swo_results: usize,
    pub manager_reviews: usize,
    pub outbox_artifacts: usize,
    pub agent_hires: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInteractionCount {
    pub agent_id: String,
    pub agent_name: String,
    pub interactions: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeArchiveManifest {
    pub snapshot_id: String,
    pub created_at_unix_ms: i64,
    pub company_name: Option<String>,
    pub profile_id: Option<String>,
    pub counts: RuntimeArchiveCounts,
    pub interaction_counts: Vec<AgentInteractionCount>,
    pub archived_paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeedRuntimeResult {
    pub company_name: String,
    pub profile_id: String,
    pub perry_agent_id: String,
    pub agent_count: usize,
    pub swo_count: usize,
    pub archive_snapshot_id: Option<String>,
    pub archive_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeContext {
    pub company_name: Option<String>,
    pub profile_id: Option<String>,
    pub company_charter_source: Option<String>,
    pub company_summary: Option<String>,
    pub autonomous_hiring_mode: Option<String>,
    pub active_seed_spec_path: Option<String>,
    pub last_archive_path: Option<String>,
    pub operating_principles: Option<String>,
    pub non_goals: Option<String>,
}

pub fn load_seed_spec(path: &Path) -> Result<RuntimeSeedSpec> {
    let raw = std::fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(KernelError::from)
}
