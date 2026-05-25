use crate::audit::TaintLabel;
use crate::error::{KernelError, Result};
use crate::manifest::AgentManifestV1;
use crate::orchestrator::Orchestrator;
use crate::registry::{CreateSwoParams, Registry};
use crate::router::Router;
use crate::seed::{
    RuntimeArchiveManifest, RuntimeContext, RuntimeSeedSpec, SeedRuntimeResult, load_seed_spec,
};
use crate::vault::Vault;
use crate::workflow::{WorkflowCompileContext, WorkflowTemplate, compile_workflow};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct Secrets {
    pub default_llm_api_key: String,
    pub llm_api_keys_by_provider: HashMap<String, String>,
    pub tool_api_keys_by_slug: Arc<RwLock<HashMap<String, String>>>,
    pub sidechannel_token: String,
}

pub struct Kernel {
    pub vault: Arc<Vault>,
    pub registry: Arc<Registry>,
    pub router: Arc<Router>,
    pub orchestrator: Arc<Orchestrator>,
    pub secrets: Arc<Secrets>,
}

impl Kernel {
    pub fn new(
        vault_key: &str,
        db_path: &str,
        worker_binary: &str,
        secrets: Secrets,
    ) -> Result<Self> {
        Self::new_with_agent_home_root(vault_key, db_path, worker_binary, secrets, None)
    }

    pub fn new_with_agent_home_root(
        vault_key: &str,
        db_path: &str,
        worker_binary: &str,
        secrets: Secrets,
        agent_home_root_override: Option<PathBuf>,
    ) -> Result<Self> {
        let vault = Arc::new(Vault::new(vault_key)?);
        let registry = Arc::new(Registry::new(db_path)?);
        let router = Arc::new(Router::new());
        let secrets_arc = Arc::new(secrets);
        let orchestrator = Arc::new(Orchestrator::new(
            worker_binary,
            Arc::clone(&registry),
            Arc::clone(&vault),
            Arc::clone(&router),
            Arc::clone(&secrets_arc),
            agent_home_root_override,
        ));

        Ok(Self {
            vault,
            registry,
            router,
            orchestrator,
            secrets: secrets_arc,
        })
    }

    pub fn repair_runtime_state(&self) -> Result<()> {
        self.registry.repair_runtime_state()?;
        for agent in self.registry.list_agents()? {
            let manifest = self
                .registry
                .get_agent_manifest(&agent.id)
                .unwrap_or_else(|_| AgentManifestV1::default_for_agent(&agent));
            self.registry.upsert_agent_manifest(&manifest)?;
        }
        self.orchestrator.repair_agent_directories()?;
        Ok(())
    }

    /// Reap SWOs orphaned by a prior crash/restart.
    /// On fresh boot no workers are alive, so every IN_PROGRESS SWO is stale.
    /// SWOs with retry_count >= MAX_RETRIES are marked FAILED; others reset to PENDING.
    pub fn reap_orphaned_swos(&self) -> Result<(usize, usize)> {
        const MAX_RETRIES: i32 = 3;
        // Threshold 0: on startup, ALL IN_PROGRESS SWOs are orphaned (no workers survive restart)
        let stale = self.registry.get_stale_in_progress_swos(0)?;
        let mut reset_count = 0usize;
        let mut failed_count = 0usize;

        for (swo_id, agent_id, retry_count) in &stale {
            if *retry_count >= MAX_RETRIES {
                self.registry.fail_swo(*swo_id)?;
                let _ = self.registry.record_audit_event(
                    Some(agent_id),
                    Some(*swo_id),
                    "startup_reaper_failed",
                    TaintLabel::TrustedSystem,
                    &json!({
                        "reason": "orphaned_on_restart",
                        "retry_count": retry_count,
                        "action": "FAILED"
                    }),
                );
                failed_count += 1;
            } else {
                self.registry.reset_swo_to_pending(*swo_id)?;
                let _ = self.registry.record_audit_event(
                    Some(agent_id),
                    Some(*swo_id),
                    "startup_reaper_reset",
                    TaintLabel::TrustedSystem,
                    &json!({
                        "reason": "orphaned_on_restart",
                        "retry_count": retry_count,
                        "action": "RESET_TO_PENDING"
                    }),
                );
                reset_count += 1;
            }
        }

        Ok((reset_count, failed_count))
    }

    pub fn start_background_tasks(&self) {
        // Reap orphaned SWOs from any prior crash before starting background loops
        match self.reap_orphaned_swos() {
            Ok((reset, failed)) => {
                if reset + failed > 0 {
                    eprintln!(
                        "[Kernel] Startup reaper: {} reset to PENDING, {} marked FAILED",
                        reset, failed
                    );
                }
            }
            Err(e) => eprintln!("[Kernel] Startup reaper error: {:?}", e),
        }

        let reconciler_orch = Arc::clone(&self.orchestrator);
        tokio::spawn(async move {
            reconciler_orch.start_queue_reconciler().await;
        });

        let cron_orch = Arc::clone(&self.orchestrator);
        tokio::spawn(async move {
            cron_orch.start_cron_loop().await;
        });

        let recurring_orch = Arc::clone(&self.orchestrator);
        tokio::spawn(async move {
            recurring_orch.start_recurring_work_order_loop().await;
        });
    }

    pub fn load_seed_spec_from_path(&self, path: &Path) -> Result<RuntimeSeedSpec> {
        load_seed_spec(path)
    }

    pub fn runtime_context(&self) -> Result<RuntimeContext> {
        self.registry.get_runtime_context()
    }

    pub fn archive_runtime_snapshot(&self, archive_root: &Path) -> Result<RuntimeArchiveManifest> {
        self.registry.checkpoint_wal()?;

        let now_ms = now_unix_ms();
        let snapshot_id = format!("runtime-{}", now_ms);
        let archive_dir = archive_root.join(&snapshot_id);
        std::fs::create_dir_all(&archive_dir)?;

        let counts = self.registry.get_runtime_archive_counts()?;
        let interaction_counts = self.registry.list_agent_interaction_counts()?;
        let runtime_context = self.registry.get_runtime_context()?;
        let mut archived_paths = Vec::new();

        let registry_db = PathBuf::from(&self.registry.db_path);
        archived_paths.extend(copy_db_bundle(&registry_db, &archive_dir.join("storage"))?);

        let storage_base = registry_db
            .parent()
            .ok_or_else(|| KernelError::Internal("Registry DB path has no parent".to_string()))?;
        let agents_dir = storage_base.join("agents");
        if agents_dir.exists() {
            let archived_agents_dir = archive_dir.join("storage").join("agents");
            copy_dir_recursive(&agents_dir, &archived_agents_dir)?;
            archived_paths.push(archived_agents_dir.to_string_lossy().to_string());
        }
        let archived_home_dirs = self
            .orchestrator
            .archive_all_agent_directories(&archive_dir.join("home-agents"))?;
        archived_paths.extend(archived_home_dirs);

        let project_root = storage_base
            .parent()
            .ok_or_else(|| KernelError::Internal("Storage base path has no parent".to_string()))?;
        let legacy_db = project_root
            .join("sairgent_deck")
            .join("storage")
            .join("kernel_registry.sqlite");
        if legacy_db.exists() {
            archived_paths.extend(copy_db_bundle(
                &legacy_db,
                &archive_dir.join("legacy-deck"),
            )?);
        }

        let manifest = RuntimeArchiveManifest {
            snapshot_id: snapshot_id.clone(),
            created_at_unix_ms: now_ms,
            company_name: runtime_context.company_name,
            profile_id: runtime_context.profile_id,
            counts,
            interaction_counts,
            archived_paths,
        };

        let manifest_path = archive_dir.join("manifest.json");
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
        Ok(manifest)
    }

    pub fn reset_runtime(&self) -> Result<()> {
        self.registry.clear_runtime_state()?;
        self.orchestrator.clear_all_agent_directories()?;
        self.orchestrator.repair_agent_directories()?;
        Ok(())
    }

    pub fn seed_runtime_from_spec(
        &self,
        spec: &RuntimeSeedSpec,
        seed_spec_path: Option<&Path>,
        archive: Option<&RuntimeArchiveManifest>,
    ) -> Result<SeedRuntimeResult> {
        let mut created_agents = HashMap::new();

        while created_agents.len() < spec.agents.len() {
            let before = created_agents.len();

            for agent in &spec.agents {
                if created_agents.contains_key(&agent.name) {
                    continue;
                }
                if let Some(manager_name) = agent.manager_name.as_ref() {
                    if !created_agents.contains_key(manager_name) {
                        continue;
                    }
                }

                let manager_id = agent
                    .manager_name
                    .as_ref()
                    .map(|name| {
                        created_agents.get(name).cloned().ok_or_else(|| {
                            KernelError::Internal(format!("Missing manager {}", name))
                        })
                    })
                    .transpose()?;

                let provider = agent
                    .provider
                    .as_deref()
                    .unwrap_or(spec.default_provider.as_str());
                let model = agent
                    .model
                    .as_deref()
                    .unwrap_or(spec.default_model.as_str());
                let id = self.registry.hire_subordinate_with_profile_and_cron(
                    &agent.name,
                    manager_id.as_deref(),
                    &agent.role,
                    agent
                        .persona_prompt
                        .as_deref()
                        .unwrap_or(&agent.raison_detre),
                    &agent.raison_detre,
                    provider,
                    model,
                    agent.cron_interval_seconds,
                    agent.triage_model.as_deref(),
                    agent.execution_model.as_deref(),
                )?;
                created_agents.insert(agent.name.clone(), id);
            }

            if created_agents.len() == before {
                return Err(KernelError::Internal(
                    "Seed spec contains unresolved manager references".to_string(),
                ));
            }
        }

        let defaults = &spec.initiative_defaults;
        for swo in &spec.starter_swos {
            let assigned_agent_id = created_agents
                .get(&swo.assigned_agent_name)
                .cloned()
                .ok_or_else(|| {
                    KernelError::Internal(format!(
                        "Starter SWO references unknown assignee {}",
                        swo.assigned_agent_name
                    ))
                })?;
            let owner_agent_id = created_agents
                .get(&swo.owner_agent_name)
                .cloned()
                .ok_or_else(|| {
                    KernelError::Internal(format!(
                        "Starter SWO references unknown owner {}",
                        swo.owner_agent_name
                    ))
                })?;
            let created_by_agent_id = created_agents
                .get(&swo.created_by_agent_name)
                .cloned()
                .ok_or_else(|| {
                    KernelError::Internal(format!(
                        "Starter SWO references unknown creator {}",
                        swo.created_by_agent_name
                    ))
                })?;
            let initiative_owner_agent_id = swo
                .initiative_owner_agent_name
                .as_ref()
                .or(defaults.initiative_owner_agent_name.as_ref())
                .and_then(|name| created_agents.get(name))
                .map(String::as_str);

            self.registry.create_swo_with_metadata(CreateSwoParams {
                assigned_agent_id: &assigned_agent_id,
                owner_agent_id: &owner_agent_id,
                created_by_agent_id: &created_by_agent_id,
                payload: &swo.payload,
                status: &swo.status,
                parent_swo_id: None,
                kind: &swo.kind,
                source: &swo.source,
                work_order_title: None,
                work_order_outcome: None,
                work_order_constraints: None,
                requested_owner_agent_id: None,
                requested_assignee_agent_id: None,
                routing_policy: "NONE",
                originating_swo_id: None,
                initiative_id: swo
                    .initiative_id
                    .as_deref()
                    .or(defaults.initiative_id.as_deref()),
                initiative_name: swo
                    .initiative_name
                    .as_deref()
                    .or(defaults.initiative_name.as_deref()),
                initiative_owner_agent_id,
                priority_class: swo
                    .priority_class
                    .as_deref()
                    .or(defaults.priority_class.as_deref()),
            })?;
        }

        // Seed MCP connectors
        for connector_spec in &spec.mcp_connectors {
            let req = crate::tools::McpConnectorUpsertRequest {
                id: None,
                slug: connector_spec.slug.clone(),
                name: connector_spec.name.clone(),
                summary: Some(connector_spec.summary.clone()),
                transport: connector_spec.transport.clone(),
                command: connector_spec.command.clone(),
                args: connector_spec.args.clone(),
                env: None,
                url: connector_spec.url.clone(),
                headers: None,
                cwd: None,
                enabled: Some(connector_spec.enabled.unwrap_or(true)),
            };
            let _ = self.registry.upsert_mcp_connector(&req);
        }

        // Seed MCP bindings (only for enabled connectors; skip silently if disabled or agent lacks capability)
        for binding_spec in &spec.mcp_bindings {
            if let Some(agent_id) = created_agents.get(&binding_spec.agent_name) {
                for slug in &binding_spec.connector_slugs {
                    if let Ok(connectors) = self.registry.list_mcp_connectors() {
                        if let Some(connector) = connectors.iter().find(|c| c.slug == *slug) {
                            let _ = self.registry.bind_mcp_connector_to_agent(agent_id, &connector.id);
                        }
                    }
                }
            }
        }

        // Seed recurring templates
        for template_spec in &spec.recurring_templates {
            if let Some(assignee_id) = created_agents.get(&template_spec.assignee_agent_name) {
                // Find a root agent (Perry) as owner, or use the assignee as owner
                let owner_id = created_agents.values().next().unwrap_or(assignee_id);
                let template_id = uuid::Uuid::new_v4().to_string();
                let schedule = crate::registry::RecurringWorkOrderScheduleRecord {
                    cadence: template_spec.schedule.cadence.clone(),
                    interval: template_spec.schedule.interval,
                    timezone: template_spec.schedule.timezone.clone(),
                    days_of_week: template_spec.schedule.days_of_week.clone(),
                    day_of_month: None,
                    hour: template_spec.schedule.hour,
                    minute: template_spec.schedule.minute,
                    cron_expression: None,
                };
                let _ = self.registry.create_recurring_template(
                    crate::registry::CreateRecurringWorkOrderTemplateParams {
                        template_id: &template_id,
                        project_id: None,
                        source_swo_id: None,
                        owner_agent_id: owner_id,
                        assignee_agent_id: Some(assignee_id),
                        name: &template_spec.name,
                        title: &template_spec.title,
                        outcome: &template_spec.outcome,
                        constraints: template_spec.constraints.as_deref(),
                        priority: &template_spec.priority,
                        include_prior_artifacts: false,
                        schedule: &schedule,
                        status: "ACTIVE",
                        next_run_at: None,
                        last_run_at: None,
                        last_run_status: None,
                    },
                );
            }
        }

        self.registry
            .upsert_runtime_metadata("company_name", &spec.company_name)?;
        self.registry
            .upsert_runtime_metadata("profile_id", &spec.profile_id)?;
        self.registry
            .upsert_runtime_metadata("company_charter_source", &spec.company_charter_source)?;
        self.registry
            .upsert_runtime_metadata("company_summary", &spec.company_summary)?;
        self.registry
            .upsert_runtime_metadata("autonomous_hiring_mode", &spec.autonomous_hiring_mode)?;
        self.registry.upsert_runtime_metadata(
            "operating_principles",
            &serde_json::to_string(&spec.operating_principles)?,
        )?;
        self.registry.upsert_runtime_metadata(
            "non_goals",
            &serde_json::to_string(&spec.non_goals)?,
        )?;
        self.registry
            .upsert_runtime_metadata("seed_spec_json", &serde_json::to_string(spec)?)?;
        if let Some(seed_spec_path) = seed_spec_path {
            self.registry.upsert_runtime_metadata(
                "active_seed_spec_path",
                &seed_spec_path.to_string_lossy(),
            )?;
        }
        if let Some(archive) = archive {
            let archive_path = self
                .archive_root()
                .join(&archive.snapshot_id)
                .to_string_lossy()
                .to_string();
            self.registry
                .upsert_runtime_metadata("last_archive_path", &archive_path)?;
        }

        self.repair_runtime_state()?;

        let perry_agent_id = created_agents.get("Perry").cloned().ok_or_else(|| {
            KernelError::Internal("Seeded runtime did not create Perry".to_string())
        })?;
        Ok(SeedRuntimeResult {
            company_name: spec.company_name.clone(),
            profile_id: spec.profile_id.clone(),
            perry_agent_id,
            agent_count: spec.agents.len(),
            swo_count: spec.starter_swos.len(),
            archive_snapshot_id: archive.map(|item| item.snapshot_id.clone()),
            archive_path: archive.map(|item| {
                self.archive_root()
                    .join(&item.snapshot_id)
                    .to_string_lossy()
                    .to_string()
            }),
        })
    }

    pub fn archive_reset_and_seed(
        &self,
        spec: &RuntimeSeedSpec,
        seed_spec_path: Option<&Path>,
    ) -> Result<SeedRuntimeResult> {
        let archive = self.archive_runtime_snapshot(&self.archive_root())?;
        self.reset_runtime()?;
        self.seed_runtime_from_spec(spec, seed_spec_path, Some(&archive))
    }

    pub fn ensure_runtime_seeded(
        &self,
        spec: &RuntimeSeedSpec,
        seed_spec_path: Option<&Path>,
    ) -> Result<SeedRuntimeResult> {
        let agent_count = self.registry.get_runtime_archive_counts()?.agents;
        let runtime_context = self.registry.get_runtime_context()?;

        if agent_count == 0 {
            return self.seed_runtime_from_spec(spec, seed_spec_path, None);
        }

        if runtime_context.profile_id.as_deref() == Some(spec.profile_id.as_str()) {
            let perry_agent_id =
                self.registry
                    .find_agent_id_by_name("Perry")?
                    .ok_or_else(|| {
                        KernelError::Internal("Perry missing from seeded runtime".to_string())
                    })?;
            return Ok(SeedRuntimeResult {
                company_name: runtime_context
                    .company_name
                    .unwrap_or_else(|| spec.company_name.clone()),
                profile_id: spec.profile_id.clone(),
                perry_agent_id,
                agent_count,
                swo_count: self.registry.get_runtime_archive_counts()?.active_swos,
                archive_snapshot_id: None,
                archive_path: runtime_context.last_archive_path,
            });
        }

        self.archive_reset_and_seed(spec, seed_spec_path)
    }

    pub fn archive_root(&self) -> PathBuf {
        PathBuf::from(&self.registry.db_path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("runtime_archives")
    }

    pub fn launch_workflow(
        &self,
        template: &WorkflowTemplate,
        context: &WorkflowCompileContext,
        created_by_agent_id: Option<&str>,
    ) -> Result<(i64, i64)> {
        let compiled = compile_workflow(template, context)?;
        let creator = created_by_agent_id.unwrap_or(template.entry_agent_id.as_str());
        let root_payload = format!(
            "WORKFLOW RUN\nTemplate: {}\nReview required: {}\nCompiled steps: {}",
            compiled.template_name,
            compiled.review_required,
            compiled.steps.len()
        );
        let root_swo_id = self.registry.create_swo_with_metadata(CreateSwoParams {
            assigned_agent_id: &template.entry_agent_id,
            owner_agent_id: &template.entry_agent_id,
            created_by_agent_id: creator,
            payload: &root_payload,
            status: "PENDING",
            parent_swo_id: None,
            kind: "WORKFLOW",
            source: "WORKFLOW",
            work_order_title: Some(&compiled.template_name),
            work_order_outcome: Some(
                "Execute the compiled workflow steps and synthesize the result.",
            ),
            work_order_constraints: Some("Preserve manager review and subordinate audit lineage."),
            requested_owner_agent_id: Some(&template.entry_agent_id),
            requested_assignee_agent_id: context.requested_assignee_agent_id.as_deref(),
            routing_policy: if context.requested_assignee_agent_id.is_some() {
                "HARD_ROUTE"
            } else {
                "NONE"
            },
            originating_swo_id: None,
            initiative_id: Some(&template.id),
            initiative_name: Some(&template.name),
            initiative_owner_agent_id: Some(&template.entry_agent_id),
            priority_class: Some("WORKFLOW"),
        })?;

        for step in &compiled.steps {
            for (assigned_agent_id, payload) in
                step.assigned_agent_ids.iter().zip(step.payloads.iter())
            {
                let outcome = format!("Complete workflow step '{}'.", step.name);
                let constraints = format!(
                    "Workflow mode: {:?}. Preserve audit trail and manager review.",
                    step.mode
                );
                self.registry.create_swo_with_metadata(CreateSwoParams {
                    assigned_agent_id,
                    owner_agent_id: &template.entry_agent_id,
                    created_by_agent_id: creator,
                    payload,
                    status: "PENDING",
                    parent_swo_id: Some(root_swo_id),
                    kind: "WORKFLOW_STEP",
                    source: "WORKFLOW",
                    work_order_title: Some(&step.name),
                    work_order_outcome: Some(&outcome),
                    work_order_constraints: Some(&constraints),
                    requested_owner_agent_id: Some(&template.entry_agent_id),
                    requested_assignee_agent_id: None,
                    routing_policy: "NONE",
                    originating_swo_id: None,
                    initiative_id: Some(&template.id),
                    initiative_name: Some(&template.name),
                    initiative_owner_agent_id: Some(&template.entry_agent_id),
                    priority_class: Some("WORKFLOW"),
                })?;
            }
        }

        let workflow_run_id =
            self.registry
                .record_workflow_run(&compiled, "LAUNCHED", Some(root_swo_id))?;
        self.registry.record_audit_event(
            Some(&template.entry_agent_id),
            Some(root_swo_id),
            "workflow_launched",
            crate::audit::TaintLabel::TrustedSystem,
            &json!({
                "workflow_run_id": workflow_run_id,
                "template_id": template.id,
                "template_name": template.name,
                "step_count": compiled.steps.len(),
            }),
        )?;

        Ok((workflow_run_id, root_swo_id))
    }
}

fn copy_db_bundle(db_path: &Path, dest_dir: &Path) -> Result<Vec<String>> {
    std::fs::create_dir_all(dest_dir)?;
    let mut archived = Vec::new();

    let main_dest = dest_dir.join(
        db_path
            .file_name()
            .ok_or_else(|| KernelError::Internal("DB file missing name".to_string()))?,
    );
    std::fs::copy(db_path, &main_dest)?;
    archived.push(main_dest.to_string_lossy().to_string());

    for suffix in ["-wal", "-shm"] {
        let sibling = PathBuf::from(format!("{}{}", db_path.to_string_lossy(), suffix));
        if sibling.exists() {
            let dest = dest_dir.join(
                sibling
                    .file_name()
                    .ok_or_else(|| KernelError::Internal("DB sidecar missing name".to_string()))?,
            );
            std::fs::copy(&sibling, &dest)?;
            archived.push(dest.to_string_lossy().to_string());
        }
    }

    Ok(archived)
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
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

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seed::{InitiativeDefaults, SeedAgentSpec, SeedSwoSpec};
    use crate::workflow::{
        WorkflowAssignee, WorkflowCompileContext, WorkflowStepMode, WorkflowStepTemplate,
        WorkflowTemplate,
    };
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn test_kernel() -> (Kernel, PathBuf) {
        let root = std::env::temp_dir().join(format!("sairgent-kernel-{}", Uuid::new_v4()));
        let storage = root.join("storage");
        std::fs::create_dir_all(&storage).unwrap();
        let worker = root.join("worker.sh");
        std::fs::write(&worker, "#!/bin/sh\nexit 0\n").unwrap();
        let kernel = Kernel::new_with_agent_home_root(
            "dummy_vault_key_that_is_32_bytes",
            storage.join("kernel_registry.sqlite").to_str().unwrap(),
            worker.to_str().unwrap(),
            Secrets {
                default_llm_api_key: "dummy".into(),
                llm_api_keys_by_provider: HashMap::new(),
                tool_api_keys_by_slug: Arc::new(RwLock::new(HashMap::new())),
                sidechannel_token: "dummy".into(),
            },
            Some(root.join("agent-home")),
        )
        .unwrap();
        (kernel, root)
    }

    fn test_seed_spec() -> RuntimeSeedSpec {
        RuntimeSeedSpec {
            profile_id: "test-seed-v1".to_string(),
            company_name: "Syllogism".to_string(),
            company_charter_source: "/tmp/manifesto.md".to_string(),
            company_summary: "Decision-grade AI payback blueprints.".to_string(),
            operating_principles: vec!["Workflow first".to_string()],
            non_goals: vec!["Hype".to_string()],
            default_provider: "mock".to_string(),
            default_model: "mock".to_string(),
            autonomous_hiring_mode: "PERRY_ONLY".to_string(),
            initiative_defaults: InitiativeDefaults {
                initiative_id: Some("core".to_string()),
                initiative_name: Some("Core".to_string()),
                initiative_owner_agent_name: Some("Perry".to_string()),
                priority_class: Some("PRIMARY".to_string()),
            },
            agents: vec![
                SeedAgentSpec {
                    name: "Perry".to_string(),
                    manager_name: None,
                    role: "COO".to_string(),
                    persona_prompt: Some("Operate with disciplined coordination.".to_string()),
                    raison_detre: "Operate".to_string(),
                    provider: None,
                    model: None,
                    cron_interval_seconds: Some(60),
                    triage_model: None,
                    execution_model: None,
                },
                SeedAgentSpec {
                    name: "Lex".to_string(),
                    manager_name: Some("Perry".to_string()),
                    role: "CRO".to_string(),
                    persona_prompt: Some(
                        "Drive revenue with sharp commercial judgment.".to_string(),
                    ),
                    raison_detre: "Sell".to_string(),
                    provider: None,
                    model: None,
                    cron_interval_seconds: None,
                    triage_model: None,
                    execution_model: None,
                },
            ],
            starter_swos: vec![SeedSwoSpec {
                assigned_agent_name: "Lex".to_string(),
                owner_agent_name: "Perry".to_string(),
                created_by_agent_name: "Perry".to_string(),
                payload: "Build pricing brief".to_string(),
                status: "PENDING".to_string(),
                kind: "TASK".to_string(),
                source: "SEED".to_string(),
                initiative_id: None,
                initiative_name: None,
                initiative_owner_agent_name: None,
                priority_class: None,
            }],
            mcp_connectors: vec![],
            mcp_bindings: vec![],
            recurring_templates: vec![],
        }
    }

    #[test]
    fn archive_reset_and_seed_replaces_existing_runtime() {
        let (kernel, root) = test_kernel();
        let old_perry = kernel
            .registry
            .hire_subordinate("Perry", None, "COO", "Old", "mock", "mock")
            .unwrap();
        kernel
            .registry
            .append_memory_interaction(&old_perry, "assistant", "legacy", None)
            .unwrap();

        let result = kernel
            .archive_reset_and_seed(&test_seed_spec(), Some(&root.join("seed.json")))
            .unwrap();

        assert_eq!(result.company_name, "Syllogism");
        assert_eq!(kernel.registry.list_agents().unwrap().len(), 2);
        assert_eq!(kernel.registry.list_swos(10).unwrap().len(), 1);
        assert!(
            result
                .archive_path
                .as_ref()
                .is_some_and(|path| Path::new(path).exists())
        );

        let context = kernel.runtime_context().unwrap();
        assert_eq!(context.profile_id.as_deref(), Some("test-seed-v1"));
        assert_eq!(context.company_name.as_deref(), Some("Syllogism"));
        assert_eq!(
            context.autonomous_hiring_mode.as_deref(),
            Some("PERRY_ONLY")
        );
    }

    #[test]
    fn ensure_runtime_seeded_is_idempotent_for_matching_profile() {
        let (kernel, root) = test_kernel();
        let spec = test_seed_spec();

        let first = kernel
            .ensure_runtime_seeded(&spec, Some(&root.join("seed.json")))
            .unwrap();
        let second = kernel
            .ensure_runtime_seeded(&spec, Some(&root.join("seed.json")))
            .unwrap();

        assert_eq!(first.perry_agent_id, second.perry_agent_id);
        assert_eq!(kernel.registry.list_agents().unwrap().len(), 2);
        assert_eq!(kernel.registry.list_swos(10).unwrap().len(), 1);
        assert!(second.archive_snapshot_id.is_none());
    }

    #[test]
    fn repair_runtime_state_backfills_runtime_surface() {
        let (kernel, root) = test_kernel();
        let perry_id = kernel
            .registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();

        kernel.repair_runtime_state().unwrap();

        let agent = kernel.registry.get_agent(&perry_id).unwrap();
        let workspace_root = root.join("agent-home").join(agent.name);
        for dir in ["context", "artifacts"] {
            assert!(
                workspace_root.join(dir).exists(),
                "missing workspace dir {dir}"
            );
        }
        for dir in ["sessions", "memory", "state", "cron", "skills"] {
            assert!(
                !workspace_root.join(dir).exists(),
                "legacy workspace dir {dir} should not exist"
            );
        }
        for file in [
            "AGENTS.md",
            "IDENTITY.md",
            "HEARTBEAT.md",
            "TOOLS.md",
            "PREFERENCES.md",
        ] {
            assert!(
                !workspace_root.join(file).exists(),
                "legacy workspace file {file} should not exist"
            );
        }
    }

    #[test]
    fn launch_workflow_compiles_into_root_and_step_swos() {
        let (kernel, _root) = test_kernel();
        let perry_id = kernel
            .registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        let felicity_id = kernel
            .registry
            .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
            .unwrap();
        let lois_id = kernel
            .registry
            .hire_subordinate("Lois", Some(&perry_id), "CIO", "Research", "mock", "mock")
            .unwrap();

        let template = WorkflowTemplate {
            id: "wf-pricing".to_string(),
            name: "Pricing Review".to_string(),
            entry_agent_id: perry_id.clone(),
            review_required: true,
            steps: vec![
                WorkflowStepTemplate {
                    id: "fanout".to_string(),
                    name: "Gather Inputs".to_string(),
                    mode: WorkflowStepMode::FanOut,
                    assignee: WorkflowAssignee::DirectReports,
                    prompt: "Collect {{topic}} inputs.".to_string(),
                    when: None,
                },
                WorkflowStepTemplate {
                    id: "collect".to_string(),
                    name: "Synthesize".to_string(),
                    mode: WorkflowStepMode::Collect,
                    assignee: WorkflowAssignee::CurrentAgent,
                    prompt: "Synthesize {{topic}} findings.".to_string(),
                    when: None,
                },
            ],
        };

        let (workflow_run_id, root_swo_id) = kernel
            .launch_workflow(
                &template,
                &WorkflowCompileContext {
                    requested_assignee_agent_id: None,
                    direct_report_ids: vec![felicity_id, lois_id],
                    variables: BTreeMap::from([("topic".to_string(), "pricing".to_string())]),
                },
                None,
            )
            .unwrap();

        assert!(workflow_run_id > 0);
        let root_detail = kernel
            .registry
            .get_swo_detail(root_swo_id)
            .unwrap()
            .unwrap();
        assert_eq!(root_detail.swo.kind, "WORKFLOW");
        assert_eq!(root_detail.child_swos.len(), 3);
    }
}
