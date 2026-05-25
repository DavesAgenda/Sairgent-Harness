use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const WORKER_PROTOCOL_V1: &str = "worker-protocol-v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct WorkerTokenUsage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub cache_read_tokens: i64,
    #[serde(default)]
    pub cache_write_tokens: i64,
    #[serde(default)]
    pub requests: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorkerArtifact {
    pub filename: String,
    pub absolute_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorkerSideEffectsReport {
    pub managed_work_count: usize,
    pub artifact_count: usize,
    pub innovation_count: usize,
    pub hire_request_count: usize,
    pub dispatch_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkerProtocolV1 {
    pub protocol_version: String,
    pub mode: String,
    pub agent_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synthesis: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatted_swo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ideation_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<WorkerArtifact>,
    #[serde(default)]
    pub side_effects: WorkerSideEffectsReport,
}

pub fn normalize_worker_output(mode: &str, agent_id: &str, mut parsed: Value) -> Value {
    let Some(object) = parsed.as_object_mut() else {
        return json!({
            "protocol_version": WORKER_PROTOCOL_V1,
            "mode": mode,
            "agent_id": agent_id,
            "status": "FAILED",
            "error": "Worker returned non-object JSON",
            "artifacts": [],
            "side_effects": WorkerSideEffectsReport::default(),
        });
    };

    object
        .entry("protocol_version".to_string())
        .or_insert_with(|| json!(WORKER_PROTOCOL_V1));
    object
        .entry("mode".to_string())
        .or_insert_with(|| json!(mode));
    object
        .entry("agent_id".to_string())
        .or_insert_with(|| json!(agent_id));
    object
        .entry("status".to_string())
        .or_insert_with(|| json!("COMPLETED"));
    object
        .entry("artifacts".to_string())
        .or_insert_with(|| json!([]));
    object
        .entry("side_effects".to_string())
        .or_insert_with(|| json!(WorkerSideEffectsReport::default()));

    if object.contains_key("error") && !object.contains_key("blocked_reason") {
        object
            .entry("status".to_string())
            .and_modify(|status| *status = json!("FAILED"));
    }

    parsed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_legacy_worker_output_adds_protocol_metadata() {
        let normalized = normalize_worker_output(
            "execute_triage",
            "agent-1",
            json!({
                "triage": {
                    "action": "ANSWER_DIRECTLY",
                    "reasoning": "done",
                    "direct_answer": "answer"
                }
            }),
        );

        assert_eq!(normalized["protocol_version"], json!(WORKER_PROTOCOL_V1));
        assert_eq!(normalized["mode"], json!("execute_triage"));
        assert_eq!(normalized["agent_id"], json!("agent-1"));
        assert_eq!(normalized["status"], json!("COMPLETED"));
        assert_eq!(normalized["artifacts"], json!([]));
    }

    #[test]
    fn normalize_error_payload_marks_worker_run_failed() {
        let normalized = normalize_worker_output(
            "chat_mode",
            "agent-2",
            json!({
                "error": "boom"
            }),
        );

        assert_eq!(normalized["status"], json!("FAILED"));
        assert_eq!(normalized["error"], json!("boom"));
    }
}
