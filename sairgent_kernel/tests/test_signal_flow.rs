/// Integration test: Signal flow enrichment (CHA-321).
///
/// Validates that the kernel emits the new KernelEvent variants during delegation:
///   1. SwoCreated fires for each child SWO
///   2. DelegationStarted fires after all children are created
///   3. SwoTerminal fires for each child SWO completion
///   4. Correct ordering: SwoCreated before DelegationStarted before SwoTerminal

use sairgent_kernel::kernel::Kernel;
use sairgent_kernel::orchestrator::KernelEvent;
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
async fn test_delegation_emits_swo_created_and_delegation_started() {
    let _guard = test_guard();

    let test_root = std::env::temp_dir().join(format!("sairgent-signals-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_signals.sqlite");

    let mock_worker = test_root.join("mock_worker_signals.py");
    let script = r#"#!/usr/bin/env python3
import json, sys, os

_ = sys.stdin.read()
mode = sys.argv[1] if len(sys.argv) > 1 else ""
role = os.environ.get("AGENT_ROLE", "")

if mode == "write_briefs":
    subordinates = json.loads(os.environ.get("AGENT_SUBORDINATES", "[]"))
    delegation = {}
    for sub in subordinates:
        delegation[sub["name"]] = "Do your part."
    print(json.dumps({
        "triage": {
            "action": "DELEGATE",
            "reasoning": "Kernel-routed delegation",
            "delegation_swos": delegation,
        }
    }))
elif mode == "execute_triage":
    print(json.dumps({
        "triage": {
            "action": "ANSWER_DIRECTLY",
            "reasoning": "Sub completed",
            "direct_answer": f"Result from {role}",
        }
    }))
elif mode == "execute_synthesis":
    print(json.dumps({
        "synthesis": {
            "action": "APPROVE_AND_REPLY",
            "reasoning": "All good",
            "final_response": "Synthesized result",
        }
    }))
"#;
    std::fs::write(&mock_worker, script).unwrap();
    std::process::Command::new("chmod")
        .args(["+x", mock_worker.to_str().unwrap()])
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
                sidechannel_token: "test_token_signals".into(),
            },
            Some(test_root.join("Sairgent_Agents")),
        )
        .unwrap(),
    );

    // Hire agent tree: Perry → Felicity + Lex
    let perry_id = kernel
        .registry
        .hire_subordinate("Perry", None, "COO", "Orchestrate", "mock", "mock")
        .unwrap();
    let _felicity_id = kernel
        .registry
        .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
        .unwrap();
    let _lex_id = kernel
        .registry
        .hire_subordinate("Lex", Some(&perry_id), "CFO", "Finance", "mock", "mock")
        .unwrap();
    kernel.repair_runtime_state().unwrap();

    // Create root SWO
    let root_swo_id = kernel
        .registry
        .create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
            assigned_agent_id: &perry_id,
            owner_agent_id: &perry_id,
            created_by_agent_id: &perry_id,
            payload: "Analyze something.",
            status: "IN_PROGRESS",
            parent_swo_id: None,
            kind: "TASK",
            source: "TEST",
            work_order_title: Some("Signal Flow Test"),
            work_order_outcome: None,
            work_order_constraints: None,
            requested_owner_agent_id: None,
            requested_assignee_agent_id: None,
            routing_policy: "NONE",
            originating_swo_id: None,
            initiative_id: None,
            initiative_name: None,
            initiative_owner_agent_id: None,
            priority_class: None,
        })
        .unwrap();

    // Set up event collection channel
    let (tx, mut rx) = tokio::sync::mpsc::channel::<KernelEvent>(128);
    let collected_events: Arc<Mutex<Vec<KernelEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&collected_events);

    let collector = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            events_clone.lock().unwrap().push(event);
        }
    });

    // Execute delegation loop
    let orchestrator = Arc::clone(&kernel.orchestrator);
    let result = orchestrator
        .execute_hsm_loop_with_context(
            perry_id.clone(),
            None,
            "Analyze something.".to_string(),
            Some(tx),
            Some(root_swo_id),
            None,
            Some("TASK".to_string()),
            Some("TEST".to_string()),
            Some(perry_id.clone()),
            Some(perry_id.clone()),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

    assert!(result.is_ok(), "HSM loop failed: {:?}", result.err());

    // Give collector time to flush
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    drop(kernel); // drop to close channel
    let _ = collector.await;

    let events = collected_events.lock().unwrap();

    // Check for SwoCreated events
    let swo_created_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, KernelEvent::SwoCreated { .. }))
        .collect();
    assert!(
        swo_created_events.len() >= 2,
        "Expected at least 2 SwoCreated events (one per subordinate), got {}",
        swo_created_events.len()
    );

    // Check for DelegationStarted event
    let delegation_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, KernelEvent::DelegationStarted { .. }))
        .collect();
    assert!(
        !delegation_events.is_empty(),
        "Expected at least 1 DelegationStarted event"
    );

    // Verify DelegationStarted has correct agent IDs
    if let KernelEvent::DelegationStarted { to_agent_ids, .. } = &delegation_events[0] {
        assert_eq!(to_agent_ids.len(), 2, "Expected 2 agents in delegation");
    }

    // Check for SwoTerminal events for child SWOs
    let terminal_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, KernelEvent::SwoTerminal { .. }))
        .collect();
    assert!(
        terminal_events.len() >= 2,
        "Expected at least 2 SwoTerminal events for child SWOs, got {}",
        terminal_events.len()
    );

    // Verify ordering: SwoCreated must appear before DelegationStarted
    let first_created_idx = events.iter().position(|e| matches!(e, KernelEvent::SwoCreated { .. }));
    let first_delegation_idx = events.iter().position(|e| matches!(e, KernelEvent::DelegationStarted { .. }));
    let first_terminal_idx = events.iter().position(|e| matches!(e, KernelEvent::SwoTerminal { .. }));

    if let (Some(created), Some(delegation)) = (first_created_idx, first_delegation_idx) {
        assert!(
            created < delegation,
            "SwoCreated (idx {}) should appear before DelegationStarted (idx {})",
            created, delegation
        );
    }
    if let (Some(delegation), Some(terminal)) = (first_delegation_idx, first_terminal_idx) {
        assert!(
            delegation < terminal,
            "DelegationStarted (idx {}) should appear before SwoTerminal (idx {})",
            delegation, terminal
        );
    }

    let _ = std::fs::remove_dir_all(test_root);
}

/// Test that the exit code masking fix correctly accepts valid JSON output
/// from a worker that exits with non-zero code.
#[tokio::test]
async fn test_exit_code_masking_accepts_valid_json() {
    let _guard = test_guard();

    let test_root = std::env::temp_dir().join(format!("sairgent-exit-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_exit.sqlite");

    // Worker exits non-zero but emits valid triage JSON
    let mock_worker = test_root.join("mock_worker_exit.py");
    let script = r#"#!/usr/bin/env python3
import json, sys, os

_ = sys.stdin.read()
mode = sys.argv[1] if len(sys.argv) > 1 else ""

if mode == "execute_triage":
    print(json.dumps({
        "triage": {
            "action": "ANSWER_DIRECTLY",
            "reasoning": "Done",
            "direct_answer": "Result despite exit code",
        }
    }))
    sys.exit(1)  # Non-zero exit, but valid output
"#;
    std::fs::write(&mock_worker, script).unwrap();
    std::process::Command::new("chmod")
        .args(["+x", mock_worker.to_str().unwrap()])
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
                sidechannel_token: "test_token_exit".into(),
            },
            Some(test_root.join("Sairgent_Agents")),
        )
        .unwrap(),
    );

    // Create a manager parent so Lois is not inferred as a root-level manager.
    // Root-level agents get manager delegation policy, which would block ANSWER_DIRECTLY
    // and mask the exit-code-masking behavior this test is validating.
    let manager_id = kernel
        .registry
        .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
        .unwrap();

    let agent_id = kernel
        .registry
        .hire_subordinate("Lois", Some(&manager_id), "Researcher", "Research", "mock", "mock")
        .unwrap();

    let orchestrator = Arc::clone(&kernel.orchestrator);
    let result = orchestrator
        .execute_hsm_loop(
            agent_id,
            None,
            "Do something.".to_string(),
            None,
            None,
            None,
        )
        .await;

    // Should succeed despite non-zero exit code because stdout had valid triage JSON
    assert!(
        result.is_ok(),
        "Worker with valid JSON output and non-zero exit should succeed, got: {:?}",
        result.err()
    );

    let _ = std::fs::remove_dir_all(test_root);
}

/// Test that agent name resolution handles "Name (Role)" format.
#[tokio::test]
async fn test_agent_name_resolution_strips_role_suffix() {
    let _guard = test_guard();

    let test_root = std::env::temp_dir().join(format!("sairgent-name-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_name.sqlite");

    let kernel = Arc::new(
        Kernel::new_with_agent_home_root(
            "dummy_vault_key_that_is_32_bytes",
            db_path.to_str().unwrap(),
            "/bin/false",
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
    let felicity_id = kernel
        .registry
        .hire_subordinate("Felicity", Some(&perry_id), "CTO", "Build", "mock", "mock")
        .unwrap();

    // Exact name match should work
    let found = kernel.registry.find_direct_subordinate(&perry_id, None, Some("Felicity")).unwrap();
    assert!(found.is_some(), "Should find Felicity by exact name");
    assert_eq!(found.unwrap().id, felicity_id);

    // "Name (Role)" format should also work
    let found = kernel.registry.find_direct_subordinate(&perry_id, None, Some("Felicity (CTO)")).unwrap();
    assert!(found.is_some(), "Should find Felicity by 'Name (Role)' format");
    assert_eq!(found.unwrap().id, felicity_id);

    // Case-insensitive with role suffix
    let found = kernel.registry.find_direct_subordinate(&perry_id, None, Some("felicity (cto)")).unwrap();
    assert!(found.is_some(), "Should find Felicity case-insensitively with role");
    assert_eq!(found.unwrap().id, felicity_id);

    // Non-existent name should return None
    let not_found = kernel.registry.find_direct_subordinate(&perry_id, None, Some("Nobody")).unwrap();
    assert!(not_found.is_none());

    let _ = std::fs::remove_dir_all(test_root);
}
