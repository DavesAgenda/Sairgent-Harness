use sairgent_kernel::kernel::Kernel;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};
use uuid::Uuid;

fn test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn test_guard() -> MutexGuard<'static, ()> {
    match test_lock().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[tokio::test]
async fn test_hsm_loop_delegation() {
    let _guard = test_guard();
    let test_root = std::env::temp_dir().join(format!("sairgent-kernel-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry.sqlite");

    let mock_worker = test_root.join("mock_worker.sh");
    let script = r#"#!/usr/bin/env python3
import json
import os
import sys

_ = sys.stdin.read()
mode = sys.argv[1]
role = os.environ.get("AGENT_ROLE", "")

if mode == "write_briefs":
    subordinates = json.loads(os.environ.get("AGENT_SUBORDINATES", "[]"))
    payload = {
        "triage": {
            "action": "DELEGATE",
            "reasoning": "Kernel-routed delegation",
            "delegation_swos": {
                subordinates[0]["name"]: "Fix it",
                subordinates[1]["name"]: "Pay for it",
            },
        }
    }
    print(json.dumps(payload))
elif mode == "execute_triage":
    payload = {
        "triage": {
            "action": "ANSWER_DIRECTLY",
            "reasoning": "I am a sub",
            "direct_answer": f"Subordinate result from {role}",
        }
    }
    print(json.dumps(payload))
elif mode == "execute_synthesis":
    print(json.dumps({
        "synthesis": {
            "action": "APPROVE_AND_REPLY",
            "reasoning": "All good",
            "final_response": "Final synthesized answer",
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
                default_llm_api_key: "dummy_key".into(),
                llm_api_keys_by_provider: std::collections::HashMap::new(),
                tool_api_keys_by_slug: Arc::new(RwLock::new(std::collections::HashMap::new())),
                sidechannel_token: "dummy_token".into(),
            },
        )
        .unwrap(),
    );

    // 3. Hire the tree
    let perry_id = kernel
        .registry
        .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
        .unwrap();
    let _felicity_id = kernel
        .registry
        .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
        .unwrap();
    let _lex_id = kernel
        .registry
        .hire_subordinate("Lex", Some(&perry_id), "CFO", "Finance", "mock", "mock")
        .unwrap();

    // 4. Trigger the HSM Loop on Perry
    let orchestrator = Arc::clone(&kernel.orchestrator);
    let result = orchestrator
        .execute_hsm_loop(
            perry_id.clone(),
            None,
            "Analyze cloud migration costs.".to_string(),
            None,
            None,
            None,
        )
        .await;

    // 5. Assertions
    assert!(
        result.is_ok(),
        "execute_hsm_loop failed: {:?}",
        result.err()
    );

    let final_value = result.unwrap();
    let synthesis = final_value
        .get("synthesis")
        .expect("Expected synthesis response");

    assert_eq!(
        synthesis.get("action").unwrap().as_str().unwrap(),
        "APPROVE_AND_REPLY"
    );
    assert_eq!(
        synthesis.get("final_response").unwrap().as_str().unwrap(),
        "Final synthesized answer"
    );

    let _ = std::fs::remove_dir_all(test_root);
}

#[tokio::test]
async fn test_hard_route_rejects_direct_answer_contract_violation() {
    let _guard = test_guard();
    let test_root = std::env::temp_dir().join(format!("sairgent-kernel-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_hard_route.sqlite");

    let mock_worker = test_root.join("mock_worker_hard_route.sh");
    let script = r#"#!/usr/bin/env python3
import json
import sys

mode = sys.argv[1]
if mode == "write_briefs":
    # Deliberately delegate to wrong agent to test hard-route rejection
    print(json.dumps({
        "triage": {
            "action": "DELEGATE",
            "reasoning": "Delegating to wrong person",
            "delegation_swos": {
                "WrongAgent": "Do the wrong thing."
            },
        }
    }))
elif mode == "execute_triage":
    print(json.dumps({
        "triage": {
            "action": "ANSWER_DIRECTLY",
            "reasoning": "Manager kept it",
            "direct_answer": "I handled this myself."
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
                default_llm_api_key: "dummy_key".into(),
                llm_api_keys_by_provider: std::collections::HashMap::new(),
                tool_api_keys_by_slug: Arc::new(RwLock::new(std::collections::HashMap::new())),
                sidechannel_token: "dummy_token".into(),
            },
        )
        .unwrap(),
    );

    let perry_id = kernel
        .registry
        .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
        .unwrap();
    let felicity_id = kernel
        .registry
        .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
        .unwrap();

    let root_swo = kernel
        .registry
        .create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
            assigned_agent_id: &perry_id,
            owner_agent_id: &perry_id,
            created_by_agent_id: &perry_id,
            payload: "Felicity must lead this.",
            status: "IN_PROGRESS",
            parent_swo_id: None,
            kind: "TASK",
            source: "CHAT",
            work_order_title: None,
            work_order_outcome: None,
            work_order_constraints: None,
            requested_owner_agent_id: None,
            requested_assignee_agent_id: Some(&felicity_id),
            routing_policy: "HARD_ROUTE",
            originating_swo_id: None,
            initiative_id: None,
            initiative_name: None,
            initiative_owner_agent_id: None,
            priority_class: None,
        })
        .unwrap();

    let result = Arc::clone(&kernel.orchestrator)
        .execute_hsm_loop_with_context(
            perry_id.clone(),
            None,
            "Felicity must lead this.".to_string(),
            None,
            Some(root_swo),
            None,
            Some("TASK".to_string()),
            Some("CHAT".to_string()),
            Some(perry_id.clone()),
            Some(perry_id.clone()),
            Some(felicity_id.clone()),
            Some("Felicity".to_string()),
            Some("HARD_ROUTE".to_string()),
            None,
            None,
        )
        .await;

    assert!(result.is_err(), "hard-route violation should fail");

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let status: String = conn
        .query_row(
            "SELECT status FROM active_swos WHERE id = ?1",
            [root_swo],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "FAILED");
    let action: String = conn
        .query_row(
            "SELECT action FROM manager_reviews WHERE swo_id = ?1 ORDER BY id DESC LIMIT 1",
            [root_swo],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(action, "REJECTED_ROUTE_CONTRACT");

    let _ = std::fs::remove_dir_all(test_root);
}

#[tokio::test]
async fn test_subordinate_hiring_is_audited_on_swo() {
    let _guard = test_guard();
    let test_root = std::env::temp_dir().join(format!("sairgent-kernel-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_hire.sqlite");

    let mock_worker = test_root.join("mock_worker_hire.sh");
    let script = r#"#!/usr/bin/env python3
import json
import os
import sys

mode = sys.argv[1]
subordinates = json.loads(os.environ.get("AGENT_SUBORDINATES", "[]"))

if mode == "write_briefs":
    print(json.dumps({
        "triage": {
            "action": "DELEGATE",
            "reasoning": "Kernel-routed delegation",
            "delegation_swos": {
                subordinates[0]["name"]: "Expand the team."
            }
        }
    }))
elif mode == "execute_triage":
    if os.environ.get("AGENT_ROLE") == "COO":
        print(json.dumps({
            "triage": {
                "action": "DELEGATE",
                "reasoning": "Need CTO",
                "delegation_swos": {
                    subordinates[0]["id"]: "Expand the team."
                }
            }
        }))
    else:
        print(json.dumps({"__sairgent_sidechannel": "hire_subordinate", "token": os.environ["SAIRGENT_SIDECHANNEL_TOKEN"], "spec": {
            "name": "Frontend Forge",
            "role": "UI Engineer",
            "raison_detre": "Ship polished frontend work.",
            "provider": "mock",
            "model": "mock",
            "cron_interval_seconds": 120
        }}), file=sys.stderr)
        print(json.dumps({
            "triage": {
                "action": "ANSWER_DIRECTLY",
                "reasoning": "Hired the needed teammate",
                "direct_answer": "I created the frontend hire."
            }
        }))
elif mode == "execute_synthesis":
    print(json.dumps({
        "synthesis": {
            "action": "APPROVE_AND_REPLY",
            "reasoning": "Validated hiring action",
            "final_response": "Felicity expanded the team."
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
        Kernel::new_with_agent_home_root(
            "dummy_vault_key_that_is_32_bytes",
            db_path.to_str().unwrap(),
            mock_worker.to_str().unwrap(),
            sairgent_kernel::kernel::Secrets {
                default_llm_api_key: "dummy_key".into(),
                llm_api_keys_by_provider: std::collections::HashMap::new(),
                tool_api_keys_by_slug: Arc::new(RwLock::new(std::collections::HashMap::new())),
                sidechannel_token: "dummy_token".into(),
            },
            Some(test_root.join("Sairgent_Agents")),
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
    kernel.repair_runtime_state().unwrap();

    let result = Arc::clone(&kernel.orchestrator)
        .execute_hsm_loop(
            perry_id.clone(),
            None,
            "Expand the team".to_string(),
            None,
            None,
            None,
        )
        .await;
    assert!(
        result.is_ok(),
        "delegated hire flow failed: {:?}",
        result.err()
    );

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let hire_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_hires", [], |row| row.get(0))
        .unwrap();
    assert_eq!(hire_count, 1);
    let new_agent_name: String = conn
        .query_row(
            "SELECT a.name
             FROM agent_hires h
             JOIN agents a ON a.id = h.new_agent_id
             ORDER BY h.id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(new_agent_name, "Frontend Forge");

    let _ = std::fs::remove_dir_all(test_root);
}
