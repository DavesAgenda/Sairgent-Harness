use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Sha256, Digest};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TaintLabel {
    TrustedSystem,
    TrustedOperator,
    UserInput,
    UntrustedModelOutput,
    ExternalMcp,
    ExternalA2A,
    /// Agent-initiated tool executions: shell commands, file mutations, git operations.
    ///
    /// IMPORTANT: Events bearing this label are SELF-REPORTED by the harness AFTER the
    /// operation executed. They are NOT kernel-verified facts. A `capability_violation_reported_*`
    /// event means the harness claims an agent without the grant attempted the action — not
    /// that the kernel prevented it. Consumers (forensics, UI, downstream automation) MUST
    /// treat `ToolExecution` records as untrusted claims, not attestations of ground truth.
    ToolExecution,
    /// Kernel-verified escalation: a manager's revision loop exhausted its retry budget.
    ///
    /// Emitted by the orchestrator before marking the child SWO FAILED. The event is
    /// kernel-authoritative — it represents a structural delegation failure, not an agent
    /// self-report. Parent managers can query `list_recent_escalations_for_agent` on their
    /// next triage turn to detect stuck children and choose reassignment or a new approach.
    ManagerEscalation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditEventRecord {
    pub id: i64,
    pub agent_id: Option<String>,
    pub swo_id: Option<i64>,
    pub event_kind: String,
    pub taint_label: TaintLabel,
    pub payload_json: String,
    pub previous_chain_hash: Option<String>,
    pub chain_hash: String,
    pub created_at: String,
}

pub fn compute_chain_hash(
    previous_chain_hash: Option<&str>,
    event_kind: &str,
    taint_label: &TaintLabel,
    payload: &Value,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(previous_chain_hash.unwrap_or("GENESIS").as_bytes());
    hasher.update(b"|");
    hasher.update(event_kind.as_bytes());
    hasher.update(b"|");
    hasher.update(format!("{:?}", taint_label).as_bytes());
    hasher.update(b"|");
    hasher.update(payload.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}
