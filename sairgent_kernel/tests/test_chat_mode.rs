/// Integration test: chat mode round-trip with a mock worker.
///
/// Verifies that `Orchestrator::run_chat_mode` for a single agent:
/// - spawns the mock worker in chat_mode
/// - receives a reply in the WorkerProtocolV1 envelope
/// - returns a non-error Value with a `reply` field
use sairgent_kernel::kernel::Kernel;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[tokio::test]
async fn test_chat_mode_direct_reply() {
    let test_root = std::env::temp_dir().join(format!("sairgent-chat-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_chat.sqlite");

    // Mock worker: for chat_mode emit a valid protocol reply then exit 0.
    let mock_worker = test_root.join("mock_worker_chat.py");
    let script = r#"#!/usr/bin/env python3
import json, sys, os

_ = sys.stdin.read()
mode = sys.argv[1] if len(sys.argv) > 1 else ""
agent_id = os.environ.get("AGENT_ID", "unknown")

if mode == "chat_mode":
    print(json.dumps({
        "protocol_version": "worker-protocol-v1",
        "mode": "chat_mode",
        "agent_id": agent_id,
        "status": "COMPLETED",
        "reply": "Hello from the mock agent!",
        "artifacts": [],
        "side_effects": {
            "managed_work_count": 0,
            "artifact_count": 0,
            "innovation_count": 0,
            "hire_request_count": 0,
            "dispatch_count": 0,
        },
    }))
else:
    print(json.dumps({"error": f"unexpected mode: {mode}"}))
    sys.exit(1)
"#;
    std::fs::write(&mock_worker, script).unwrap();
    std::process::Command::new("chmod")
        .args(["+x", mock_worker.to_str().unwrap()])
        .status()
        .unwrap();

    let kernel = Arc::new(
        Kernel::new(
            "dummy_vault_key_that_is_32_bytes",
            db_path.to_str().unwrap(),
            mock_worker.to_str().unwrap(),
            sairgent_kernel::kernel::Secrets {
                default_llm_api_key: "dummy_key".into(),
                llm_api_keys_by_provider: std::collections::HashMap::new(),
                tool_api_keys_by_slug: Arc::new(RwLock::new(std::collections::HashMap::new())),
                sidechannel_token: "test_token_chat_mode".into(),
            },
        )
        .unwrap(),
    );

    // Hire a single agent (no subordinates needed for direct chat).
    let agent_id = kernel
        .registry
        .hire_subordinate("Lois", None, "Researcher", "Research topics", "mock", "mock")
        .unwrap();

    let orchestrator = Arc::clone(&kernel.orchestrator);
    let result = orchestrator
        .run_chat_mode(agent_id, "What can you tell me about Q1 costs?".to_string(), &[], None)
        .await;

    assert!(result.is_ok(), "run_chat_mode failed: {:?}", result.err());

    let value = result.unwrap();
    let reply = value
        .get("reply")
        .and_then(|v| v.as_str())
        .expect("Expected a 'reply' field in the chat_mode response");

    assert_eq!(reply, "Hello from the mock agent!");
    assert_eq!(
        value.get("status").and_then(|v| v.as_str()).unwrap_or(""),
        "COMPLETED"
    );

    let _ = std::fs::remove_dir_all(test_root);
}

#[tokio::test]
async fn test_chat_mode_worker_error_propagates() {
    let test_root = std::env::temp_dir().join(format!("sairgent-chat-err-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_chat_err.sqlite");

    // Mock worker: always emits an error payload.
    let mock_worker = test_root.join("mock_worker_chat_err.py");
    let script = r#"#!/usr/bin/env python3
import json, sys, os
_ = sys.stdin.read()
agent_id = os.environ.get("AGENT_ID", "unknown")
print(json.dumps({
    "protocol_version": "worker-protocol-v1",
    "mode": "chat_mode",
    "agent_id": agent_id,
    "status": "FAILED",
    "error": "LLM API key missing",
    "artifacts": [],
    "side_effects": {"managed_work_count":0,"artifact_count":0,"innovation_count":0,"hire_request_count":0,"dispatch_count":0},
}))
"#;
    std::fs::write(&mock_worker, script).unwrap();
    std::process::Command::new("chmod")
        .args(["+x", mock_worker.to_str().unwrap()])
        .status()
        .unwrap();

    let kernel = Arc::new(
        Kernel::new(
            "dummy_vault_key_that_is_32_bytes",
            db_path.to_str().unwrap(),
            mock_worker.to_str().unwrap(),
            sairgent_kernel::kernel::Secrets {
                default_llm_api_key: "dummy_key".into(),
                llm_api_keys_by_provider: std::collections::HashMap::new(),
                tool_api_keys_by_slug: Arc::new(RwLock::new(std::collections::HashMap::new())),
                sidechannel_token: "test_token_chat_err".into(),
            },
        )
        .unwrap(),
    );

    let agent_id = kernel
        .registry
        .hire_subordinate("ErrBot", None, "Bot", "Does nothing", "mock", "mock")
        .unwrap();

    let orchestrator = Arc::clone(&kernel.orchestrator);
    let result = orchestrator
        .run_chat_mode(agent_id, "Hello?".to_string(), &[], None)
        .await;

    // The kernel should surface the worker error — either as an Err or as a Value with status=FAILED.
    match result {
        Err(_) => {} // acceptable: kernel turned worker error into Err
        Ok(val) => {
            let status = val.get("status").and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                status == "FAILED" || val.get("error").is_some(),
                "Expected FAILED status or error field, got: {val:?}"
            );
        }
    }

    let _ = std::fs::remove_dir_all(test_root);
}
