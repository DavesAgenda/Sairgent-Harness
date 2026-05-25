use crate::registry::AgentIdentity;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocolFamily {
    OpenAiCompatible,
    AnthropicCompatible,
    OauthCodexStyle,
    Unknown,
}

impl ProviderProtocolFamily {
    pub fn from_provider_name(provider: &str) -> Self {
        match provider.trim().to_lowercase().as_str() {
            "openai" | "openrouter" | "groq" | "ollama" | "lmstudio" | "xai" | "deepseek" => {
                Self::OpenAiCompatible
            }
            "anthropic" => Self::AnthropicCompatible,
            "codex" | "openai_codex" => Self::OauthCodexStyle,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityGrant {
    QueueManagedWork,
    InspectManagedWork,
    DispatchSwo,
    SubmitInnovationSwo,
    WebSearch,
    ReadInbox,
    WriteOutbox,
    HireSubordinate,
    WorkflowCompile,
    WorkflowLaunch,
    HeartbeatReview,
    McpClient,
    A2AIngress,
    A2AEgress,
    /// Read any file in the agent's workspace tree (dark factory Phase 7).
    FileRead,
    /// Create, edit, or delete files in the agent's workspace tree.
    FileWrite,
    /// Run sandboxed shell commands within the agent workspace.
    ShellExec,
    /// Git operations (clone, commit, push) scoped to the agent workspace.
    GitOps,
}

impl CapabilityGrant {
    pub fn worker_tool_name(&self) -> Option<&'static str> {
        match self {
            Self::QueueManagedWork => Some("queue_managed_work"),
            Self::InspectManagedWork => Some("get_swo_queue_status"),
            Self::SubmitInnovationSwo => Some("submit_innovation_swo"),
            Self::WebSearch => None,
            Self::ReadInbox => Some("read_agent_file"),
            Self::WriteOutbox => Some("write_artifact_file"),
            Self::HireSubordinate => Some("hire_subordinate_internal"),
            Self::DispatchSwo => Some("dispatch_swo_internal"),
            Self::HeartbeatReview
            | Self::WorkflowCompile
            | Self::WorkflowLaunch
            | Self::McpClient
            | Self::A2AIngress
            | Self::A2AEgress
            | Self::FileRead
            | Self::FileWrite
            | Self::ShellExec
            | Self::GitOps => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Guardrail {
    pub code: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleSpec {
    pub cron_interval_seconds: Option<i64>,
    pub autonomous_heartbeat: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderConfigV1 {
    pub provider_name: String,
    pub model: String,
    pub protocol_family: ProviderProtocolFamily,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentManifestV1 {
    pub version: String,
    pub agent_id: Option<String>,
    pub name: String,
    pub role: String,
    pub mission: String,
    pub persona_prompt: String,
    pub provider: ProviderConfigV1,
    pub capabilities: Vec<CapabilityGrant>,
    pub guardrails: Vec<Guardrail>,
    pub schedule: ScheduleSpec,
}

impl AgentManifestV1 {
    fn standard_guardrails() -> Vec<Guardrail> {
        vec![
            Guardrail {
                code: "manager_review_required".to_string(),
                description:
                    "Subordinate work must return through manager review and synthesis."
                        .to_string(),
            },
            Guardrail {
                code: "artifact_paths_must_be_outbox_scoped".to_string(),
                description:
                    "Artifacts may only be registered from the agent artifacts workspace."
                        .to_string(),
            },
            Guardrail {
                code: "direct_report_dispatch_only".to_string(),
                description: "Delegation targets must be validated as direct reports in Rust."
                    .to_string(),
            },
            Guardrail {
                code: "web_research_requires_citations".to_string(),
                description:
                    "When web research is authorized and used, factual claims must include attributable sources."
                        .to_string(),
            },
        ]
    }

    pub fn default_for_agent(agent: &AgentIdentity) -> Self {
        let protocol_family = ProviderProtocolFamily::from_provider_name(&agent.default_provider);
        let mut capabilities = vec![
            CapabilityGrant::QueueManagedWork,
            CapabilityGrant::InspectManagedWork,
            CapabilityGrant::DispatchSwo,
            CapabilityGrant::SubmitInnovationSwo,
            CapabilityGrant::ReadInbox,
            CapabilityGrant::WriteOutbox,
            CapabilityGrant::HireSubordinate,
            CapabilityGrant::WorkflowCompile,
            CapabilityGrant::WorkflowLaunch,
            CapabilityGrant::HeartbeatReview,
            // In-sandbox I/O is structurally blast-radius-free: every agent has an
            // isolated workspace at ~/Sairgent_Agents/{name}/ with _resolve_safe_path()
            // traversal enforcement in the harness. ShellExec and GitOps stay opt-in
            // because they have real out-of-sandbox effects (process spawn, git push).
            CapabilityGrant::FileRead,
            CapabilityGrant::FileWrite,
        ];

        if agent.parent_id.is_none() || agent.name == "Perry" {
            capabilities.push(CapabilityGrant::McpClient);
            capabilities.push(CapabilityGrant::A2AIngress);
            capabilities.push(CapabilityGrant::A2AEgress);
        }

        if agent.name == "Lois" || agent.role.to_lowercase().contains("research") {
            capabilities.push(CapabilityGrant::WebSearch);
        }

        Self {
            version: "agent-manifest-v1".to_string(),
            agent_id: Some(agent.id.clone()),
            name: agent.name.clone(),
            role: agent.role.clone(),
            mission: agent.raison_detre.clone(),
            persona_prompt: agent.persona_prompt.clone(),
            provider: ProviderConfigV1 {
                provider_name: agent.default_provider.clone(),
                model: agent.default_model.clone(),
                protocol_family,
                triage_model: agent.triage_model.clone(),
                execution_model: agent.execution_model.clone(),
            },
            capabilities,
            guardrails: Self::standard_guardrails(),
            schedule: ScheduleSpec {
                cron_interval_seconds: agent.cron_interval_seconds,
                autonomous_heartbeat: agent.cron_interval_seconds.is_some(),
            },
        }
    }

    pub fn least_privilege_for_agent(agent: &AgentIdentity) -> Self {
        let protocol_family = ProviderProtocolFamily::from_provider_name(&agent.default_provider);

        Self {
            version: "agent-manifest-v1".to_string(),
            agent_id: Some(agent.id.clone()),
            name: agent.name.clone(),
            role: agent.role.clone(),
            mission: agent.raison_detre.clone(),
            persona_prompt: agent.persona_prompt.clone(),
            provider: ProviderConfigV1 {
                provider_name: agent.default_provider.clone(),
                model: agent.default_model.clone(),
                protocol_family,
                triage_model: agent.triage_model.clone(),
                execution_model: agent.execution_model.clone(),
            },
            capabilities: vec![
                CapabilityGrant::QueueManagedWork,
                CapabilityGrant::ReadInbox,
                CapabilityGrant::WriteOutbox,
            ],
            guardrails: Self::standard_guardrails(),
            schedule: ScheduleSpec {
                cron_interval_seconds: agent.cron_interval_seconds,
                autonomous_heartbeat: agent.cron_interval_seconds.is_some(),
            },
        }
    }

    pub fn has_capability(&self, capability: &CapabilityGrant) -> bool {
        self.capabilities.iter().any(|grant| grant == capability)
    }

    pub fn allowed_worker_tools_for_mode(
        &self,
        mode: &str,
        can_autonomously_hire: bool,
    ) -> Vec<String> {
        let mut tools = Vec::new();

        for capability in &self.capabilities {
            if let Some(tool_name) = capability.worker_tool_name() {
                tools.push(tool_name.to_string());
            }
        }

        match mode {
            "sairgent_chat" => {
                // Super-agent gets ALL tools — no filtering applied
            }
            "chat_mode" => tools.retain(|tool| {
                matches!(
                    tool.as_str(),
                    "queue_managed_work"
                        | "get_swo_queue_status"
                        | "read_agent_file"
                        | "write_artifact_file"
                )
            }),
            "format_swo" => tools.clear(),
            "execute_triage" | "execute_synthesis" => tools.retain(|tool| {
                matches!(
                    tool.as_str(),
                    "submit_innovation_swo"
                        | "read_agent_file"
                        | "write_artifact_file"
                        | "hire_subordinate_internal"
                        | "dispatch_swo_internal"
                )
            }),
            "execute_ideation" => tools.retain(|tool| {
                matches!(
                    tool.as_str(),
                    "submit_innovation_swo" | "read_agent_file" | "write_artifact_file"
                )
            }),
            _ => {}
        }

        if !can_autonomously_hire {
            tools.retain(|tool| tool != "hire_subordinate_internal");
        }

        tools.sort();
        tools.dedup();
        tools
    }

    pub fn standard_workspace_directories() -> &'static [&'static str] {
        &["context", "artifacts"]
    }

    pub fn standard_workspace_files() -> &'static [&'static str] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::AgentIdentity;

    fn test_agent() -> AgentIdentity {
        AgentIdentity {
            id: "agent-1".to_string(),
            name: "Felicity".to_string(),
            parent_id: Some("perry".to_string()),
            role: "CTO".to_string(),
            persona_prompt: "Build".to_string(),
            raison_detre: "Ship".to_string(),
            default_provider: "mock".to_string(),
            default_model: "mock".to_string(),
            cron_interval_seconds: Some(60),
            triage_model: None,
            execution_model: None,
        }
    }

    #[test]
    fn default_manifest_preserves_hiring_for_manager_reviewed_work() {
        let manifest = AgentManifestV1::default_for_agent(&test_agent());
        assert!(manifest.has_capability(&CapabilityGrant::HireSubordinate));
        assert!(
            manifest
                .allowed_worker_tools_for_mode("execute_triage", true)
                .contains(&"hire_subordinate_internal".to_string())
        );
    }

    #[test]
    fn ideation_mode_never_exposes_direct_hiring() {
        let manifest = AgentManifestV1::default_for_agent(&test_agent());
        assert!(
            !manifest
                .allowed_worker_tools_for_mode("execute_ideation", true)
                .contains(&"hire_subordinate_internal".to_string())
        );
    }

    #[test]
    fn least_privilege_manifest_excludes_builder_only_capabilities() {
        let manifest = AgentManifestV1::least_privilege_for_agent(&test_agent());
        assert!(manifest.has_capability(&CapabilityGrant::QueueManagedWork));
        assert!(manifest.has_capability(&CapabilityGrant::ReadInbox));
        assert!(manifest.has_capability(&CapabilityGrant::WriteOutbox));
        assert!(!manifest.has_capability(&CapabilityGrant::HireSubordinate));
        assert!(!manifest.has_capability(&CapabilityGrant::DispatchSwo));
        assert!(!manifest.has_capability(&CapabilityGrant::WorkflowLaunch));
    }

    #[test]
    fn workspace_surface_only_keeps_file_exchange_dirs() {
        assert_eq!(
            AgentManifestV1::standard_workspace_directories(),
            ["context", "artifacts"]
        );
        assert!(AgentManifestV1::standard_workspace_files().is_empty());
    }

    #[test]
    fn dark_factory_grants_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&CapabilityGrant::FileRead).unwrap(),
            "\"file_read\""
        );
        assert_eq!(
            serde_json::to_string(&CapabilityGrant::FileWrite).unwrap(),
            "\"file_write\""
        );
        assert_eq!(
            serde_json::to_string(&CapabilityGrant::ShellExec).unwrap(),
            "\"shell_exec\""
        );
        assert_eq!(
            serde_json::to_string(&CapabilityGrant::GitOps).unwrap(),
            "\"git_ops\""
        );
    }

    #[test]
    fn dark_factory_grants_deserialize_roundtrip() {
        for grant in [
            CapabilityGrant::FileRead,
            CapabilityGrant::FileWrite,
            CapabilityGrant::ShellExec,
            CapabilityGrant::GitOps,
        ] {
            let serialized = serde_json::to_string(&grant).unwrap();
            let deserialized: CapabilityGrant = serde_json::from_str(&serialized).unwrap();
            assert_eq!(grant, deserialized);
        }
    }

    #[test]
    fn default_grants_include_in_sandbox_io() {
        // FileRead and FileWrite are unconditionally granted to all agents because
        // every agent runs in an isolated sandbox (~/Sairgent_Agents/{name}/) with
        // _resolve_safe_path() traversal enforcement making writes structurally
        // blast-radius-free (CHA-408).
        let manifest = AgentManifestV1::default_for_agent(&test_agent());
        assert!(manifest.has_capability(&CapabilityGrant::FileRead));
        assert!(manifest.has_capability(&CapabilityGrant::FileWrite));
    }

    #[test]
    fn default_grants_exclude_out_of_sandbox_effects() {
        // ShellExec (process spawn) and GitOps (git remote push) remain opt-in
        // because they have real out-of-sandbox effects (CHA-408).
        let manifest = AgentManifestV1::default_for_agent(&test_agent());
        assert!(!manifest.has_capability(&CapabilityGrant::ShellExec));
        assert!(!manifest.has_capability(&CapabilityGrant::GitOps));
    }

    #[test]
    fn dark_factory_grants_not_in_least_privilege_manifest() {
        let manifest = AgentManifestV1::least_privilege_for_agent(&test_agent());
        assert!(!manifest.has_capability(&CapabilityGrant::FileRead));
        assert!(!manifest.has_capability(&CapabilityGrant::FileWrite));
        assert!(!manifest.has_capability(&CapabilityGrant::ShellExec));
        assert!(!manifest.has_capability(&CapabilityGrant::GitOps));
    }

    #[test]
    fn dark_factory_grants_can_be_assigned_and_checked() {
        let mut manifest = AgentManifestV1::least_privilege_for_agent(&test_agent());
        assert!(!manifest.has_capability(&CapabilityGrant::ShellExec));
        manifest.capabilities.push(CapabilityGrant::ShellExec);
        assert!(manifest.has_capability(&CapabilityGrant::ShellExec));
    }

    #[test]
    fn dark_factory_grants_have_no_worker_tool_name() {
        assert_eq!(CapabilityGrant::FileRead.worker_tool_name(), None);
        assert_eq!(CapabilityGrant::FileWrite.worker_tool_name(), None);
        assert_eq!(CapabilityGrant::ShellExec.worker_tool_name(), None);
        assert_eq!(CapabilityGrant::GitOps.worker_tool_name(), None);
    }
}
