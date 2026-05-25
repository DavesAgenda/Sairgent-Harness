use rusqlite::Connection;
use sairgent_kernel::kernel::Kernel;
use std::path::Path;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[tokio::test]
async fn test_sovereign_memory_isolation() {
    let test_root = std::env::temp_dir().join(format!("sairgent-kernel-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_sovereign.sqlite");

    let mock_worker = test_root.join("mock_worker_sovereign.sh");
    let script = r#"#!/bin/bash
read -r API_KEY
MODE=$1

# The mock asserts it was given its own precise DB path
if [[ "$AGENT_DATABASE" != *"storage/agents/$AGENT_ID/memory.sqlite" ]]; then
    echo "{\"error\": \"Isolaton breach! Received DB: $AGENT_DATABASE\"}"
    exit 1
fi

if [ "$MODE" == "execute_triage" ]; then
    echo "{\"triage\": {\"action\": \"ANSWER_DIRECTLY\", \"reasoning\": \"Sovereign memory check complete\", \"direct_answer\": \"I remember\"}}"
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
                default_llm_api_key: "dummy_key".into(),
                llm_api_keys_by_provider: std::collections::HashMap::new(),
                tool_api_keys_by_slug: Arc::new(RwLock::new(std::collections::HashMap::new())),
                sidechannel_token: "dummy_token".into(),
            },
        )
        .unwrap(),
    );

    // 3. Hire Agents
    // Create a manager parent so the test agents are not inferred as root-level managers.
    // Root-level agents get manager delegation policy, which blocks ANSWER_DIRECTLY.
    let manager_id = kernel
        .registry
        .hire_subordinate("TestManager", None, "Manager", "Manage", "mock", "mock")
        .unwrap();

    let agent_a_id = kernel
        .registry
        .hire_subordinate("AgentA", Some(&manager_id), "Worker", "Test", "mock", "mock")
        .unwrap();
    let agent_b_id = kernel
        .registry
        .hire_subordinate("AgentB", Some(&manager_id), "Worker", "Test", "mock", "mock")
        .unwrap();

    assert_ne!(agent_a_id, agent_b_id);

    // 4. Verify disk isolation layout exists
    let db_a_path_str = storage_dir
        .join("agents")
        .join(&agent_a_id)
        .join("memory.sqlite")
        .to_string_lossy()
        .to_string();
    let db_b_path_str = storage_dir
        .join("agents")
        .join(&agent_b_id)
        .join("memory.sqlite")
        .to_string_lossy()
        .to_string();

    assert!(
        Path::new(&db_a_path_str).exists(),
        "Agent A database not provisioned"
    );
    assert!(
        Path::new(&db_b_path_str).exists(),
        "Agent B database not provisioned"
    );

    // 5. Verify schemas were instantiated properly
    let conn_a = Connection::open(&db_a_path_str).unwrap();
    let mut stmt = conn_a
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='interactions'")
        .unwrap();
    let tables_a: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(tables_a.len(), 1, "Agent A interactions table missing");

    let conn_b = Connection::open(&db_b_path_str).unwrap();
    let mut stmt = conn_b
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='interactions'")
        .unwrap();
    let tables_b: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(tables_b.len(), 1, "Agent B interactions table missing");

    // 6. Test Path Traversal Defenses
    let bad_id = "../../../etc/passwd";
    let orchestrator = Arc::clone(&kernel.orchestrator);
    let result = orchestrator
        .execute_hsm_loop(
            bad_id.to_string(),
            None,
            "Malicious payload".to_string(),
            None,
            None,
            None,
        )
        .await;

    // Should fail via path traversal check before spawning
    assert!(result.is_err(), "Path traversal was not caught by HSM");
    let err_msg = format!("{:?}", result.err().unwrap());
    assert!(
        err_msg.contains("potential path traversal detected"),
        "Expected path traversal error message but got: {}",
        err_msg
    );

    // 7. Test execution passes mock checks
    let orchestrator_valid = Arc::clone(&kernel.orchestrator);
    let valid_result = orchestrator_valid
        .execute_hsm_loop(
            agent_a_id.clone(),
            None,
            "Hello".to_string(),
            None,
            None,
            None,
        )
        .await;
    assert!(
        valid_result.is_ok(),
        "Valid execution failed: {:?}",
        valid_result.err()
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(test_root);
}
