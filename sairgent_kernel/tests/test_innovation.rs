use rusqlite::params;
use sairgent_kernel::kernel::Kernel;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[tokio::test]
async fn test_innovation_creates_linked_review_swo() {
    let test_root = std::env::temp_dir().join(format!("sairgent-kernel-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_innovation.sqlite");

    let mock_worker = test_root.join("mock_worker_innovation.sh");
    let script = r#"#!/bin/bash
read -r API_KEY
MODE=$1
AGENT_ROLE=$AGENT_ROLE

if [ "$MODE" == "write_briefs" ]; then
    SUB_1_NAME=$(echo $AGENT_SUBORDINATES | grep -o -E '"name":"[^"]+"' | head -n 1 | cut -d '"' -f 4)
    echo "{\"triage\": {\"action\": \"DELEGATE\", \"reasoning\": \"Kernel-routed\", \"delegation_swos\": {\"$SUB_1_NAME\": \"Do the task.\"}}}"
elif [ "$MODE" == "execute_triage" ]; then
    if [ "$AGENT_ROLE" == "COO" ]; then
        SUB_1_ID=$(echo $AGENT_SUBORDINATES | grep -o -E '"id":"[^"]+"' | head -n 1 | cut -d '"' -f 4)
        echo "{\"triage\": {\"action\": \"DELEGATE\", \"reasoning\": \"Delegating to subs\", \"delegation_swos\": {\"$SUB_1_ID\": \"Do the task.\"}}}"
    else
        echo "{\"__sairgent_sidechannel\": \"innovation_swo\", \"token\": \"$SAIRGENT_SIDECHANNEL_TOKEN\", \"originating_swo_id\": $AGENT_SWO_ID, \"report\": {\"title\": \"Repetitive Task Discovered\", \"context\": \"Doing the task.\", \"proposed_solution\": \"Automate this.\", \"estimated_impact\": \"High\"}}" >&2
        echo "{\"triage\": {\"action\": \"ANSWER_DIRECTLY\", \"reasoning\": \"I am a sub\", \"direct_answer\": \"Subordinate result from $AGENT_ROLE\"}}"
    fi
elif [ "$MODE" == "execute_synthesis" ]; then
    echo "{\"synthesis\": {\"action\": \"APPROVE_AND_REPLY\", \"reasoning\": \"All good\", \"final_response\": \"Final synthesized answer\"}}"
fi
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

    let orchestrator = Arc::clone(&kernel.orchestrator);
    let result = orchestrator
        .execute_hsm_loop(
            perry_id.clone(),
            None,
            "Do a repetitive task.".to_string(),
            None,
            None,
            None,
        )
        .await;

    assert!(
        result.is_ok(),
        "execute_hsm_loop failed: {:?}",
        result.err()
    );
    assert_eq!(
        result.unwrap()["synthesis"]["final_response"].as_str(),
        Some("Final synthesized answer")
    );

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let root_swo_id: i64 = conn
        .query_row(
            "SELECT id FROM active_swos WHERE assigned_agent_id = ?1 AND kind = 'TASK' ORDER BY id ASC LIMIT 1",
            params![perry_id],
            |row| row.get(0),
        )
        .unwrap();

    let review_row: (String, String, i64) = conn
        .query_row(
            "SELECT assigned_agent_id, kind, originating_swo_id
             FROM active_swos
             WHERE kind = 'INNOVATION_REVIEW'
             ORDER BY id DESC
             LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();

    assert_eq!(review_row.0, perry_id);
    assert_eq!(review_row.1, "INNOVATION_REVIEW");
    let origin_lineage: (String, Option<i64>) = conn
        .query_row(
            "SELECT kind, parent_swo_id FROM active_swos WHERE id = ?1",
            params![review_row.2],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(origin_lineage.0, "TASK");
    assert_eq!(origin_lineage.1, Some(root_swo_id));

    let urgent_review_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM manager_reviews WHERE action = 'URGENT_REVIEW'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(urgent_review_count, 0);

    let _ = std::fs::remove_dir_all(test_root);
}
