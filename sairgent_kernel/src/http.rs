use crate::audit::TaintLabel;
use crate::kernel::Kernel;
use crate::manifest::AgentManifestV1;
use crate::skills::{RuntimeSkillIndexEntry, SkillRecord, SkillUpsertRequest, SkillVersionRecord};
use crate::workflow::{WorkflowCompileContext, WorkflowRun, WorkflowTemplate, compile_workflow};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub kernel: Arc<Kernel>,
}

pub fn create_app(state: AppState) -> Router {
    Router::new()
        .route("/v2/webhook", post(handle_webhook))
        .route(
            "/v2/webhook/telegram/:bot_token",
            post(handle_telegram_webhook),
        )
        .route("/v2/agents", post(hire_subordinate))
        .route("/v2/agents/:id", get(get_agent))
        .route("/v2/agents/:id/bind", post(bind_agent_token))
        .route("/v2/agents/:id/manifest", post(update_agent_manifest))
        .route("/v2/agents/:id/skills/bind", post(bind_skill_to_agent))
        .route(
            "/v2/agents/:id/skills/unbind",
            post(unbind_skill_from_agent),
        )
        .route("/v2/agents/:id/skills/preview", post(preview_agent_skills))
        .route("/v2/skills", get(list_skills).post(save_skill))
        .route("/v2/skills/:id", get(get_skill))
        .route("/v2/workflows/compile", post(compile_workflow_handler))
        .route("/v2/workflows/launch", post(launch_workflow_handler))
        .with_state(state)
}

#[derive(Deserialize)]
pub struct WebhookPayload {
    pub bot_id: String,
    pub message: String,
    pub channel: Option<String>,
    pub external_chat_id: Option<String>,
    pub external_user_id: Option<String>,
    pub external_message_id: Option<String>,
}

#[derive(Serialize)]
pub struct WebhookResponse {
    pub status: String,
    pub reply: Option<String>,
}

const SAIRGENT_WEBHOOK_MODE: &str = "sairgent_panel";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SairgentWebhookToolCall {
    call_id: String,
    tool_name: String,
    summary: String,
    arguments_json: String,
    status: String,
    requires_confirmation: bool,
    result_summary: Option<String>,
    error_message: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SairgentWebhookMessage {
    id: String,
    conversation_id: String,
    role: String,
    content: String,
    channel: String,
    created_at: String,
    related_project_id: Option<String>,
    related_swo_id: Option<i64>,
    pending_tool_call: Option<SairgentWebhookToolCall>,
    tool_calls: Vec<SairgentWebhookToolCall>,
}

fn current_iso_timestamp() -> String {
    rusqlite::Connection::open_in_memory()
        .and_then(|conn| {
            conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
                row.get(0)
            })
        })
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn constant_time_equals(left: &str, right: &str) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max_len {
        let left_byte = left.as_bytes().get(index).copied().unwrap_or(0);
        let right_byte = right.as_bytes().get(index).copied().unwrap_or(0);
        diff |= (left_byte ^ right_byte) as usize;
    }
    diff == 0
}

fn webhook_conversation_id(
    agent_id: &str,
    channel: &str,
    external_chat_id: Option<&str>,
    external_user_id: Option<&str>,
) -> String {
    let chat = external_chat_id.unwrap_or("global");
    let user = external_user_id.unwrap_or("operator");
    format!(
        "sairgent-{}-{}-{}-{}",
        agent_id,
        channel.trim().to_ascii_lowercase(),
        chat,
        user
    )
}

fn to_webhook_tool_call(
    proposal: crate::orchestrator::SairgentToolProposal,
) -> SairgentWebhookToolCall {
    SairgentWebhookToolCall {
        call_id: proposal.call_id,
        tool_name: proposal.tool_name,
        summary: proposal.summary,
        arguments_json: proposal.arguments_json,
        status: if proposal.requires_confirmation {
            "pending_confirmation".to_string()
        } else {
            "proposed".to_string()
        },
        requires_confirmation: proposal.requires_confirmation,
        result_summary: None,
        error_message: None,
    }
}

fn make_sairgent_message(
    role: &str,
    content: String,
    channel: &str,
    conversation_id: String,
    tool_calls: Vec<SairgentWebhookToolCall>,
) -> SairgentWebhookMessage {
    let pending_tool_call = tool_calls
        .iter()
        .find(|tool_call| {
            tool_call.requires_confirmation
                && (tool_call.status == "pending_confirmation" || tool_call.status == "proposed")
        })
        .cloned();
    SairgentWebhookMessage {
        id: Uuid::new_v4().to_string(),
        conversation_id,
        role: role.to_string(),
        content,
        channel: channel.trim().to_ascii_lowercase(),
        created_at: current_iso_timestamp(),
        related_project_id: None,
        related_swo_id: None,
        pending_tool_call,
        tool_calls,
    }
}

fn persist_sairgent_message(
    kernel: &Kernel,
    agent_id: &str,
    message: &SairgentWebhookMessage,
) -> Result<(), String> {
    let payload = serde_json::to_string(message)
        .map_err(|error| format!("Failed to serialize Sairgent webhook message: {}", error))?;
    kernel
        .registry
        .append_memory_interaction_with_meta(
            agent_id,
            &message.role,
            &payload,
            None,
            SAIRGENT_WEBHOOK_MODE,
            Some(&message.conversation_id),
            "sairgent_message",
        )
        .map_err(|error| format!("Failed to persist Sairgent webhook message: {:?}", error))
}

async fn run_external_sairgent_chat(
    state: &AppState,
    agent_id: &str,
    channel: &str,
    conversation_id: String,
    text: String,
    attachment_count: usize,
) -> Result<SairgentWebhookMessage, String> {
    let user_message = make_sairgent_message(
        "user",
        text.clone(),
        channel,
        conversation_id.clone(),
        Vec::new(),
    );
    persist_sairgent_message(&state.kernel, agent_id, &user_message)?;

    let result = Arc::clone(&state.kernel.orchestrator)
        .run_sairgent_chat(
            agent_id.to_string(),
            text,
            None,
            None,
            attachment_count,
            None,
            None,
        )
        .await
        .map_err(|error| format!("Sairgent chat failed: {:?}", error))?;

    let assistant_message = make_sairgent_message(
        "assistant",
        result.reply,
        channel,
        conversation_id,
        result.tool_calls.into_iter().map(to_webhook_tool_call).collect(),
    );
    persist_sairgent_message(&state.kernel, agent_id, &assistant_message)?;
    Ok(assistant_message)
}

/// Endpoint for handling omnichannel incoming messages (e.g. from Slack/Telegram)
async fn handle_webhook(
    State(state): State<AppState>,
    Json(payload): Json<WebhookPayload>,
) -> axum::response::Result<Json<WebhookResponse>, String> {
    let channel = payload.channel.unwrap_or_else(|| "system".to_string());
    let conversation_id = webhook_conversation_id(
        &payload.bot_id,
        &channel,
        payload.external_chat_id.as_deref(),
        payload.external_user_id.as_deref(),
    );

    match run_external_sairgent_chat(
        &state,
        &payload.bot_id,
        &channel,
        conversation_id,
        payload.message,
        0,
    )
    .await
    {
        Ok(message) => Ok(Json(WebhookResponse {
            status: "success".to_string(),
            reply: Some(message.content),
        })),
        Err(e) => Ok(Json(WebhookResponse {
            status: "error".to_string(),
            reply: Some(e),
        })),
    }
}

#[derive(Deserialize)]
pub struct HireRequest {
    pub name: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub persona_prompt: Option<String>,
    pub raison_detre: String,
    pub provider: String,
    pub model: String,
    pub manifest: Option<AgentManifestV1>,
}

#[derive(Serialize)]
pub struct HireResponse {
    pub agent_id: String,
}

/// Endpoint for Dynamic Hiring (creating new agents in the nested org chart)
async fn hire_subordinate(
    State(state): State<AppState>,
    Json(payload): Json<HireRequest>,
) -> axum::response::Result<Json<HireResponse>, String> {
    let parent_id_str = payload.parent_id.as_deref();

    let id = state
        .kernel
        .registry
        .hire_subordinate_with_profile_and_cron(
            &payload.name,
            parent_id_str,
            &payload.role,
            payload
                .persona_prompt
                .as_deref()
                .unwrap_or(payload.raison_detre.as_str()),
            &payload.raison_detre,
            &payload.provider,
            &payload.model,
            payload
                .manifest
                .as_ref()
                .map(|manifest| manifest.schedule.cron_interval_seconds)
                .unwrap_or(None),
            None,
            None,
        )
        .map_err(|e| format!("{:?}", e))?;

    if let Some(mut manifest) = payload.manifest {
        manifest.agent_id = Some(id.clone());
        state
            .kernel
            .registry
            .update_agent_manifest_profile(&manifest)
            .map_err(|e| format!("{:?}", e))?;
    }

    Ok(Json(HireResponse { agent_id: id }))
}

async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Result<Json<serde_json::Value>, String> {
    let agent = state
        .kernel
        .registry
        .get_agent(&id)
        .map_err(|e| format!("{:?}", e))?;
    let manifest = state
        .kernel
        .registry
        .get_agent_manifest(&id)
        .map_err(|e| format!("{:?}", e))?;

    Ok(Json(serde_json::json!({
        "id": agent.id,
        "name": agent.name,
        "parent_id": agent.parent_id,
        "role": agent.role,
        "persona_prompt": agent.persona_prompt,
        "raison_detre": agent.raison_detre,
        "default_provider": agent.default_provider,
        "default_model": agent.default_model,
        "triage_model": agent.triage_model,
        "execution_model": agent.execution_model,
        "manifest": manifest,
        "bound_skills": state
            .kernel
            .registry
            .list_agent_skill_bindings(&id)
            .map_err(|e| format!("{:?}", e))?,
    })))
}

#[derive(Deserialize)]
pub struct UpdateAgentManifestRequest {
    pub manifest: AgentManifestV1,
}

#[derive(Deserialize)]
pub struct SkillBindingRequest {
    pub skill_id: String,
    pub priority: Option<i64>,
}

#[derive(Deserialize)]
pub struct SkillPreviewRequest {
    pub mode: String,
    pub payload: String,
}

#[derive(Serialize)]
pub struct SkillPreviewResponse {
    pub skills: Vec<RuntimeSkillIndexEntry>,
}

async fn update_agent_manifest(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateAgentManifestRequest>,
) -> axum::response::Result<Json<serde_json::Value>, String> {
    let mut manifest = payload.manifest;
    manifest.agent_id = Some(id.clone());
    state
        .kernel
        .registry
        .update_agent_manifest_profile(&manifest)
        .map_err(|e| format!("{:?}", e))?;
    Ok(Json(serde_json::json!({ "status": "success" })))
}

async fn bind_skill_to_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<SkillBindingRequest>,
) -> axum::response::Result<Json<serde_json::Value>, String> {
    state
        .kernel
        .registry
        .bind_skill_to_agent(&id, &payload.skill_id, payload.priority.unwrap_or(100))
        .map_err(|e| format!("{:?}", e))?;
    Ok(Json(serde_json::json!({ "status": "success" })))
}

async fn unbind_skill_from_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<SkillBindingRequest>,
) -> axum::response::Result<Json<serde_json::Value>, String> {
    state
        .kernel
        .registry
        .unbind_skill_from_agent(&id, &payload.skill_id)
        .map_err(|e| format!("{:?}", e))?;
    Ok(Json(serde_json::json!({ "status": "success" })))
}

async fn preview_agent_skills(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<SkillPreviewRequest>,
) -> axum::response::Result<Json<SkillPreviewResponse>, String> {
    let skills = state
        .kernel
        .registry
        .preview_agent_skills_for_run(&id, &payload.mode, &payload.payload, 4)
        .map_err(|e| format!("{:?}", e))?;
    Ok(Json(SkillPreviewResponse { skills }))
}

async fn list_skills(
    State(state): State<AppState>,
) -> axum::response::Result<Json<Vec<SkillRecord>>, String> {
    let skills = state
        .kernel
        .registry
        .list_skills(200)
        .map_err(|e| format!("{:?}", e))?;
    Ok(Json(skills))
}

async fn save_skill(
    State(state): State<AppState>,
    Json(payload): Json<SkillUpsertRequest>,
) -> axum::response::Result<Json<SkillRecord>, String> {
    let skill = state
        .kernel
        .registry
        .save_skill(&payload)
        .map_err(|e| format!("{:?}", e))?;
    Ok(Json(skill))
}

async fn get_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Result<Json<SkillVersionRecord>, String> {
    let skill = state
        .kernel
        .registry
        .get_skill(&id)
        .map_err(|e| format!("{:?}", e))?
        .ok_or_else(|| format!("Skill {} not found", id))?;
    Ok(Json(skill))
}

#[derive(Deserialize)]
pub struct BindTokenRequest {
    pub bot_token: String,
}

#[derive(Serialize)]
pub struct BindTokenResponse {
    pub status: String,
}

async fn bind_agent_token(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<BindTokenRequest>,
) -> axum::response::Result<Json<BindTokenResponse>, String> {
    state
        .kernel
        .registry
        .bind_agent_token(&id, &payload.bot_token)
        .map_err(|e| format!("{:?}", e))?;

    Ok(Json(BindTokenResponse {
        status: "success".to_string(),
    }))
}

// Telegram webhook structure (simplified)
#[derive(Deserialize)]
pub struct TelegramWebhook {
    pub update_id: Option<i64>,
    pub message: Option<TelegramMessage>,
}

#[derive(Deserialize)]
pub struct TelegramMessage {
    pub message_id: Option<i64>,
    pub text: Option<String>,
    pub chat: TelegramChat,
    pub from: Option<TelegramUser>,
}

#[derive(Deserialize)]
pub struct TelegramChat {
    pub id: i64,
}

#[derive(Deserialize)]
pub struct TelegramUser {
    pub id: i64,
}

async fn handle_telegram_webhook(
    State(state): State<AppState>,
    Path(bot_token): Path<String>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<TelegramWebhook>,
) -> axum::response::Result<Json<WebhookResponse>, String> {
    let binding = state
        .kernel
        .registry
        .resolve_external_channel_binding_by_route_token("telegram", &bot_token)
        .map_err(|error| format!("{:?}", error))?
        .ok_or_else(|| "Unknown Telegram route token".to_string())?;

    let presented_secret = headers
        .get("X-Telegram-Bot-Api-Secret-Token")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let expected_secret = binding.secret_token.as_deref().unwrap_or("");
    if expected_secret.is_empty() || !constant_time_equals(presented_secret, expected_secret) {
        return Err("Unauthorized webhook secret".to_string());
    }

    if !binding.binding.enabled {
        return Ok(Json(WebhookResponse {
            status: "ignored".to_string(),
            reply: None,
        }));
    }

    let Some(message) = payload.message else {
        return Ok(Json(WebhookResponse {
            status: "ignored".to_string(),
            reply: None,
        }));
    };

    let text = message.text.unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(Json(WebhookResponse {
            status: "ignored".to_string(),
            reply: None,
        }));
    }

    let external_chat_id = message.chat.id.to_string();
    let external_user_id = message.from.as_ref().map(|user| user.id.to_string());
    let external_message_id = message
        .message_id
        .or(payload.update_id)
        .map(|value| value.to_string());

    let allowed_chat_id = binding.binding.allowed_chat_id.as_deref();
    let allowed_user_id = binding.binding.allowed_user_id.as_deref();
    let chat_allowed = allowed_chat_id
        .map(|expected| expected == external_chat_id)
        .unwrap_or(false);
    let user_allowed = match (allowed_user_id, external_user_id.as_deref()) {
        (Some(expected), Some(actual)) => expected == actual,
        (Some(_), None) => false,
        (None, _) => false,
    };

    if !chat_allowed || !user_allowed {
        let payload = serde_json::json!({
            "channel": "telegram",
            "agent_id": binding.binding.agent_id,
            "external_chat_id": external_chat_id.as_str(),
            "external_user_id": external_user_id.as_deref(),
            "text_preview": text.chars().take(160).collect::<String>(),
        });
        let _ = state.kernel.registry.record_audit_event(
            Some(&binding.binding.agent_id),
            None,
            "external_channel_unauthorized",
            TaintLabel::UserInput,
            &payload,
        );
        let _ = state
            .kernel
            .registry
            .record_external_channel_delivery_event(
                crate::registry::RecordExternalChannelDeliveryEventParams {
                    agent_id: &binding.binding.agent_id,
                    channel: "telegram",
                    session_id: None,
                    direction: "inbound",
                    status: "unauthorized",
                    detail: "Telegram chat or user does not match the allowlist binding.",
                    external_chat_id: Some(&external_chat_id),
                    external_user_id: external_user_id.as_deref(),
                    external_message_id: external_message_id.as_deref(),
                },
            );
        return Ok(Json(WebhookResponse {
            status: "rejected".to_string(),
            reply: Some("Not authorized.".to_string()),
        }));
    }

    if let Some(message_id) = external_message_id.as_deref() {
        let claimed = state
            .kernel
            .registry
            .claim_external_message_receipt("telegram", &external_chat_id, message_id)
            .map_err(|error| format!("{:?}", error))?;
        if !claimed {
            let _ = state
                .kernel
                .registry
                .record_external_channel_delivery_event(
                    crate::registry::RecordExternalChannelDeliveryEventParams {
                        agent_id: &binding.binding.agent_id,
                        channel: "telegram",
                        session_id: None,
                        direction: "inbound",
                        status: "duplicate",
                        detail: "Duplicate Telegram update ignored.",
                        external_chat_id: Some(&external_chat_id),
                        external_user_id: external_user_id.as_deref(),
                        external_message_id: Some(message_id),
                    },
                );
            return Ok(Json(WebhookResponse {
                status: "ignored".to_string(),
                reply: None,
            }));
        }
    }

    let conversation_id = webhook_conversation_id(
        &binding.binding.agent_id,
        "telegram",
        Some(&external_chat_id),
        external_user_id.as_deref(),
    );
    let session = state
        .kernel
        .registry
        .touch_external_chat_session(crate::registry::TouchExternalChatSessionParams {
            agent_id: &binding.binding.agent_id,
            channel: "telegram",
            external_chat_id: &external_chat_id,
            external_user_id: external_user_id.as_deref(),
            conversation_id: &conversation_id,
            last_inbound_message_id: external_message_id.as_deref(),
        })
        .map_err(|error| format!("{:?}", error))?;

    let _ = state
        .kernel
        .registry
        .record_external_channel_delivery_event(
            crate::registry::RecordExternalChannelDeliveryEventParams {
                agent_id: &binding.binding.agent_id,
                channel: "telegram",
                session_id: Some(&session.session_id),
                direction: "inbound",
                status: "received",
                detail: "Telegram update accepted for Sairgent processing.",
                external_chat_id: Some(&external_chat_id),
                external_user_id: external_user_id.as_deref(),
                external_message_id: external_message_id.as_deref(),
            },
        );

    match run_external_sairgent_chat(
        &state,
        &binding.binding.agent_id,
        "telegram",
        conversation_id,
        text,
        0,
    )
    .await
    {
        Ok(message) => {
            let _ = state
                .kernel
                .registry
                .record_external_channel_delivery_event(
                    crate::registry::RecordExternalChannelDeliveryEventParams {
                        agent_id: &binding.binding.agent_id,
                        channel: "telegram",
                        session_id: Some(&session.session_id),
                        direction: "outbound",
                        status: "delivered",
                        detail: "Telegram reply prepared successfully.",
                        external_chat_id: Some(&external_chat_id),
                        external_user_id: external_user_id.as_deref(),
                        external_message_id: external_message_id.as_deref(),
                    },
                );
            Ok(Json(WebhookResponse {
                status: "success".to_string(),
                reply: Some(message.content),
            }))
        }
        Err(error) => {
            let _ = state
                .kernel
                .registry
                .record_external_channel_delivery_event(
                    crate::registry::RecordExternalChannelDeliveryEventParams {
                        agent_id: &binding.binding.agent_id,
                        channel: "telegram",
                        session_id: Some(&session.session_id),
                        direction: "outbound",
                        status: "failed",
                        detail: &error,
                        external_chat_id: Some(&external_chat_id),
                        external_user_id: external_user_id.as_deref(),
                        external_message_id: external_message_id.as_deref(),
                    },
                );
            Ok(Json(WebhookResponse {
                status: "error".to_string(),
                reply: Some(error),
            }))
        }
    }
}

#[derive(Deserialize)]
pub struct WorkflowRequest {
    pub template: WorkflowTemplate,
    pub requested_assignee_agent_id: Option<String>,
    pub direct_report_ids: Option<Vec<String>>,
    pub variables: Option<BTreeMap<String, String>>,
}

#[derive(Serialize)]
pub struct WorkflowCompileResponse {
    pub compiled: WorkflowRun,
}

#[derive(Serialize)]
pub struct WorkflowLaunchResponse {
    pub workflow_run_id: i64,
    pub root_swo_id: i64,
    pub compiled: WorkflowRun,
}

fn workflow_context(
    kernel: &Kernel,
    request: &WorkflowRequest,
) -> Result<WorkflowCompileContext, String> {
    let direct_report_ids = match &request.direct_report_ids {
        Some(ids) => ids.clone(),
        None => kernel
            .registry
            .get_subordinates(&request.template.entry_agent_id)
            .map_err(|e| format!("{:?}", e))?
            .into_iter()
            .map(|agent| agent.id)
            .collect(),
    };

    Ok(WorkflowCompileContext {
        requested_assignee_agent_id: request.requested_assignee_agent_id.clone(),
        direct_report_ids,
        variables: request.variables.clone().unwrap_or_default(),
    })
}

async fn compile_workflow_handler(
    State(state): State<AppState>,
    Json(payload): Json<WorkflowRequest>,
) -> axum::response::Result<Json<WorkflowCompileResponse>, String> {
    let context = workflow_context(&state.kernel, &payload)?;
    let compiled = compile_workflow(&payload.template, &context).map_err(|e| format!("{:?}", e))?;
    Ok(Json(WorkflowCompileResponse { compiled }))
}

async fn launch_workflow_handler(
    State(state): State<AppState>,
    Json(payload): Json<WorkflowRequest>,
) -> axum::response::Result<Json<WorkflowLaunchResponse>, String> {
    let context = workflow_context(&state.kernel, &payload)?;
    let compiled = compile_workflow(&payload.template, &context).map_err(|e| format!("{:?}", e))?;
    let (workflow_run_id, root_swo_id) = state
        .kernel
        .launch_workflow(&payload.template, &context, None)
        .map_err(|e| format!("{:?}", e))?;
    Ok(Json(WorkflowLaunchResponse {
        workflow_run_id,
        root_swo_id,
        compiled,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::RwLock;

    const TEST_VAULT_KEY: &str = "SAIRGENT_TEST_KEY_NOT_FOR_PROD!!";

    fn test_kernel() -> (Arc<Kernel>, String) {
        let test_root = std::env::temp_dir().join(format!("sairgent-http-{}", Uuid::new_v4()));
        let storage_dir = test_root.join("storage");
        std::fs::create_dir_all(&storage_dir).unwrap();
        let db_path = storage_dir.join("registry.sqlite");
        let worker_path = test_root.join("mock_worker.sh");
        std::fs::write(&worker_path, "#!/bin/sh\nexit 0\n").unwrap();
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&worker_path)
            .status()
            .unwrap();

        let kernel = Arc::new(
            Kernel::new(
                TEST_VAULT_KEY,
                db_path.to_str().unwrap(),
                worker_path.to_str().unwrap(),
                crate::kernel::Secrets {
                    default_llm_api_key: "dummy".into(),
                    llm_api_keys_by_provider: HashMap::new(),
                    tool_api_keys_by_slug: Arc::new(RwLock::new(HashMap::new())),
                    sidechannel_token: "dummy".into(),
                },
            )
            .unwrap(),
        );
        let perry_id = kernel
            .registry
            .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
            .unwrap();
        (kernel, perry_id)
    }

    fn telegram_headers(secret: &str) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "X-Telegram-Bot-Api-Secret-Token",
            axum::http::HeaderValue::from_str(secret).unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn telegram_webhook_rejects_unbound_user() {
        let (kernel, perry_id) = test_kernel();
        kernel
            .registry
            .upsert_external_channel_binding(crate::registry::UpsertExternalChannelBindingParams {
                agent_id: &perry_id,
                channel: "telegram",
                enabled: true,
                allowed_chat_id: Some("42"),
                allowed_user_id: Some("7"),
                route_token: Some("telegram-route"),
                secret_token: Some("telegram-secret"),
            })
            .unwrap();

        let state = AppState {
            kernel: Arc::clone(&kernel),
        };
        let Json(response) = handle_telegram_webhook(
            State(state),
            Path("telegram-route".to_string()),
            telegram_headers("telegram-secret"),
            Json(TelegramWebhook {
                update_id: Some(9001),
                message: Some(TelegramMessage {
                    message_id: Some(501),
                    text: Some("status".to_string()),
                    chat: TelegramChat { id: 42 },
                    from: Some(TelegramUser { id: 99 }),
                }),
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.status, "rejected");
        assert_eq!(response.reply.as_deref(), Some("Not authorized."));
        let audit_events = kernel.registry.list_audit_events(10).unwrap();
        assert_eq!(audit_events[0].event_kind, "external_channel_unauthorized");
    }

    #[tokio::test]
    async fn telegram_webhook_dedupes_and_persists_sairgent_messages() {
        let (kernel, perry_id) = test_kernel();
        kernel
            .registry
            .upsert_external_channel_binding(crate::registry::UpsertExternalChannelBindingParams {
                agent_id: &perry_id,
                channel: "telegram",
                enabled: true,
                allowed_chat_id: Some("42"),
                allowed_user_id: Some("7"),
                route_token: Some("telegram-route"),
                secret_token: Some("telegram-secret"),
            })
            .unwrap();

        let state = AppState {
            kernel: Arc::clone(&kernel),
        };
        let request = TelegramWebhook {
            update_id: Some(9002),
            message: Some(TelegramMessage {
                message_id: Some(777),
                text: Some("status".to_string()),
                chat: TelegramChat { id: 42 },
                from: Some(TelegramUser { id: 7 }),
            }),
        };

        let Json(first_response) = handle_telegram_webhook(
            State(state.clone()),
            Path("telegram-route".to_string()),
            telegram_headers("telegram-secret"),
            Json(request),
        )
        .await
        .unwrap();
        assert_eq!(first_response.status, "success");
        assert!(first_response.reply.unwrap_or_default().contains("offline mode"));

        let Json(second_response) = handle_telegram_webhook(
            State(state),
            Path("telegram-route".to_string()),
            telegram_headers("telegram-secret"),
            Json(TelegramWebhook {
                update_id: Some(9002),
                message: Some(TelegramMessage {
                    message_id: Some(777),
                    text: Some("status".to_string()),
                    chat: TelegramChat { id: 42 },
                    from: Some(TelegramUser { id: 7 }),
                }),
            }),
        )
        .await
        .unwrap();
        assert_eq!(second_response.status, "ignored");

        let db_path = kernel.registry.agent_memory_db_path(&perry_id).unwrap();
        let conn = rusqlite::Connection::open(db_path).unwrap();
        let message_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM interactions WHERE mode = ?1",
                rusqlite::params![SAIRGENT_WEBHOOK_MODE],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(message_count, 2);

        let delivery_events = kernel
            .registry
            .list_recent_external_channel_delivery_events(10)
            .unwrap();
        assert!(delivery_events.iter().any(|event| event.status == "delivered"));
        assert!(delivery_events.iter().any(|event| event.status == "duplicate"));
    }
}
