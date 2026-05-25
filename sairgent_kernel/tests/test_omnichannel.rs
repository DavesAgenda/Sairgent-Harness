use sairgent_kernel::kernel::Kernel;
use std::sync::{Arc, RwLock};
use tokio::time::{Duration, sleep};
use uuid::Uuid;

#[tokio::test]
async fn test_chat_mode_queues_managed_work_root_swo() {
    let test_root = std::env::temp_dir().join(format!("sairgent-kernel-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_omni.sqlite");

    let mock_worker = test_root.join("mock_worker_omni.sh");
    let script = r#"#!/usr/bin/env python3
import json
import os
import sys

_ = sys.stdin.read()
mode = sys.argv[1]
subs = json.loads(os.environ.get("AGENT_SUBORDINATES", "[]"))

if mode == "chat_mode":
    print(json.dumps({
        "__sairgent_sidechannel": "queue_managed_work",
        "token": os.environ["SAIRGENT_SIDECHANNEL_TOKEN"],
        "payload": {
            "payload": "Execute this plan right now.",
            "requested_assignee_name": "Felicity",
            "routing_policy": "HARD_ROUTE",
            "user_visible_summary": "Website team expansion"
        }
    }), file=sys.stderr)
    print(json.dumps({"reply": "placeholder ack"}))
elif mode == "write_briefs":
    print(json.dumps({
        "triage": {
            "action": "DELEGATE",
            "reasoning": "Kernel-routed delegation",
            "delegation_swos": {
                subs[0]["name"]: "Build the plan."
            }
        }
    }))
elif mode == "execute_triage":
    print(json.dumps({
        "triage": {
            "action": "DELEGATE",
            "reasoning": "Need Felicity",
            "delegation_swos": {
                subs[0]["id"]: "Build the plan."
            }
        }
    }))
elif mode == "execute_synthesis":
    print(json.dumps({
        "synthesis": {
            "action": "APPROVE_AND_REPLY",
            "reasoning": "Reviewed",
            "final_response": "Felicity completed the plan."
        }
    }))
"#;
    std::fs::write(&mock_worker, script).unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&mock_worker)
        .status()
        .unwrap();

    let kernel = Arc::new(
        Kernel::new(
            "dummy_vault_key_that_is_32_bytes",
            db_path.to_str().unwrap(),
            mock_worker.to_str().unwrap(),
            sairgent_kernel::kernel::Secrets {
                default_llm_api_key: "dummy".into(),
                llm_api_keys_by_provider: std::collections::HashMap::new(),
                tool_api_keys_by_slug: Arc::new(RwLock::new(std::collections::HashMap::new())),
                sidechannel_token: "dummy".into(),
            },
        )
        .unwrap(),
    );

    let perry_id = kernel
        .registry
        .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
        .unwrap();
    let _felicity_id = kernel
        .registry
        .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
        .unwrap();

    kernel
        .registry
        .bind_agent_token(&perry_id, "secret_bot_token_123")
        .unwrap();
    let agent_from_token = kernel
        .registry
        .get_agent_by_token("secret_bot_token_123")
        .unwrap();
    assert_eq!(agent_from_token.id, perry_id);

    let orchestrator = Arc::clone(&kernel.orchestrator);
    let result = orchestrator
        .run_chat_mode(perry_id.clone(), "User says hello".to_string(), &[], None)
        .await;

    assert!(result.is_ok(), "run_chat_mode failed: {:?}", result.err());
    let res_json = result.unwrap();
    assert_eq!(
        res_json["reply"].as_str(),
        Some(
            "Website team expansion has been queued under Perry for Felicity-led execution. I will confirm once the delegation SWO is actually opened."
        )
    );
    assert_eq!(res_json["queued_work_count"].as_i64(), Some(1));

    // Poll for SWO completion with backoff and timeout.
    // The async HSM chain (Perry triage → delegate → synthesis → COMPLETED)
    // involves multiple tokio::spawn calls that need time to complete.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let poll_conn = rusqlite::Connection::open(&db_path).unwrap();
        let poll_status: Result<String, _> = poll_conn.query_row(
            "SELECT status FROM active_swos WHERE parent_swo_id IS NULL ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        );
        if let Ok(ref s) = poll_status {
            if s == "COMPLETED" || s == "FAILED" || s == "CANCELLED" {
                break;
            }
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "SWO did not reach terminal state within 10s, stuck at: {:?}",
                poll_status
            );
        }
        sleep(Duration::from_millis(250)).await;
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (kind, source, status, requested_name, routing_policy): (
        String,
        String,
        String,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT kind, source, status, COALESCE(a.name, ''), COALESCE(s.routing_policy, 'NONE')
             FROM active_swos s
             LEFT JOIN agents a ON a.id = s.requested_assignee_agent_id
             WHERE s.parent_swo_id IS NULL
             ORDER BY s.id DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(kind, "TASK");
    assert_eq!(source, "CHAT");
    assert_eq!(status, "COMPLETED");
    assert_eq!(requested_name, "Felicity");
    assert_eq!(routing_policy, "HARD_ROUTE");

    let ack_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM manager_reviews WHERE action = 'ACKNOWLEDGED'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(ack_count, 1);

    let child_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM active_swos WHERE parent_swo_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(child_count, 1);

    let _ = std::fs::remove_dir_all(test_root);
}
