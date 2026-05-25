/// Integration test: Queue reconciler behavior (CHA-318 supplement).
///
/// Tests the reconciler's stale SWO detection and retry/fail logic
/// directly via the registry methods it uses, without needing to
/// wait for the 60-second reconciler interval.

use sairgent_kernel::kernel::Kernel;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[tokio::test]
async fn test_stale_swo_detection_with_heartbeat() {
    let test_root = std::env::temp_dir().join(format!("sairgent-reconciler-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_reconciler.sqlite");

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

    let agent_id = kernel
        .registry
        .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
        .unwrap();

    // Create a PENDING SWO, then claim it (PENDING → IN_PROGRESS with run_id)
    let swo_id = kernel
        .registry
        .create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
            assigned_agent_id: &agent_id,
            owner_agent_id: &agent_id,
            created_by_agent_id: &agent_id,
            payload: "Stale task",
            status: "PENDING",
            parent_swo_id: None,
            kind: "TASK",
            source: "TEST",
            work_order_title: Some("Stale task"),
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

    // Claim the SWO (PENDING → IN_PROGRESS, sets current_run_id)
    let run_id = format!("test-run-{}", Uuid::new_v4());
    let claimed = kernel.registry.claim_swo_with_run_id(swo_id, &run_id).unwrap();
    assert_eq!(claimed, 1, "Should claim the SWO");

    // Insert a heartbeat, then backdate it to make it stale
    kernel.orchestrator.upsert_heartbeat_async(
        run_id.clone(),
        agent_id.clone(),
        "COMPUTING".to_string(),
        1,
    ).await;

    // Backdate the heartbeat by 10 seconds so staleness detection works
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "UPDATE agent_heartbeats SET last_seen_unix_ms = last_seen_unix_ms - 10000 WHERE run_id = ?1",
        [&run_id],
    ).unwrap();
    drop(conn);

    // With 5s threshold, the 10s-old heartbeat should be stale
    let stale_swos = kernel.registry.get_stale_in_progress_swos(5_000).unwrap();
    assert!(
        stale_swos.iter().any(|(id, _, _)| *id == swo_id),
        "SWO {} should be detected as stale with 5s threshold",
        swo_id
    );

    // With 30s threshold, the 10s-old heartbeat should NOT be stale
    let not_stale = kernel.registry.get_stale_in_progress_swos(30_000).unwrap();
    assert!(
        !not_stale.iter().any(|(id, _, _)| *id == swo_id),
        "SWO {} should NOT be stale with 30s threshold (heartbeat is only 10s old)",
        swo_id
    );

    // Reset to pending and verify retry count increments
    kernel.registry.reset_swo_to_pending(swo_id).unwrap();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (status, retry_count): (String, i32) = conn
        .query_row(
            "SELECT status, retry_count FROM active_swos WHERE id = ?1",
            [swo_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "PENDING");
    assert_eq!(retry_count, 1);

    let _ = std::fs::remove_dir_all(test_root);
}

#[tokio::test]
async fn test_retry_count_exhaustion_leads_to_failure() {
    let test_root = std::env::temp_dir().join(format!("sairgent-reconciler-exhaust-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_reconciler_exhaust.sqlite");

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

    let agent_id = kernel
        .registry
        .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
        .unwrap();

    // Create an IN_PROGRESS SWO
    let swo_id = kernel
        .registry
        .create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
            assigned_agent_id: &agent_id,
            owner_agent_id: &agent_id,
            created_by_agent_id: &agent_id,
            payload: "Will exhaust retries",
            status: "IN_PROGRESS",
            parent_swo_id: None,
            kind: "TASK",
            source: "TEST",
            work_order_title: Some("Retry exhaustion"),
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

    // Simulate 3 failed retry cycles via the reaper
    // Cycle 1: IN_PROGRESS → PENDING (retry_count: 0→1)
    kernel.registry.reset_swo_to_pending(swo_id).unwrap();
    // Put it back to IN_PROGRESS to simulate re-claim and re-failure
    kernel.registry.set_swo_status(swo_id, "IN_PROGRESS").unwrap();

    // Cycle 2: retry_count 1→2
    kernel.registry.reset_swo_to_pending(swo_id).unwrap();
    kernel.registry.set_swo_status(swo_id, "IN_PROGRESS").unwrap();

    // Cycle 3: retry_count 2→3
    kernel.registry.reset_swo_to_pending(swo_id).unwrap();
    kernel.registry.set_swo_status(swo_id, "IN_PROGRESS").unwrap();

    // Now reap — retry_count is 3, should be FAILED
    let (reset_count, failed_count) = kernel.reap_orphaned_swos().unwrap();
    assert_eq!(reset_count, 0, "Should not reset — retries exhausted");
    assert_eq!(failed_count, 1, "Should fail the SWO");

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let status: String = conn
        .query_row("SELECT status FROM active_swos WHERE id = ?1", [swo_id], |row| row.get(0))
        .unwrap();
    assert_eq!(status, "FAILED");

    let _ = std::fs::remove_dir_all(test_root);
}

#[tokio::test]
async fn test_completed_and_pending_swos_not_reaped() {
    let test_root = std::env::temp_dir().join(format!("sairgent-reconciler-safe-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_reconciler_safe.sqlite");

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

    let agent_id = kernel
        .registry
        .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
        .unwrap();

    // Create SWOs in various non-IN_PROGRESS states
    let completed_id = kernel.registry.create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
        assigned_agent_id: &agent_id, owner_agent_id: &agent_id, created_by_agent_id: &agent_id,
        payload: "Done", status: "COMPLETED", parent_swo_id: None, kind: "TASK", source: "TEST",
        work_order_title: Some("Completed"), work_order_outcome: None, work_order_constraints: None,
        requested_owner_agent_id: None, requested_assignee_agent_id: None, routing_policy: "NONE",
        originating_swo_id: None, initiative_id: None, initiative_name: None,
        initiative_owner_agent_id: None, priority_class: None,
    }).unwrap();

    let pending_id = kernel.registry.create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
        assigned_agent_id: &agent_id, owner_agent_id: &agent_id, created_by_agent_id: &agent_id,
        payload: "Waiting", status: "PENDING", parent_swo_id: None, kind: "TASK", source: "TEST",
        work_order_title: Some("Pending"), work_order_outcome: None, work_order_constraints: None,
        requested_owner_agent_id: None, requested_assignee_agent_id: None, routing_policy: "NONE",
        originating_swo_id: None, initiative_id: None, initiative_name: None,
        initiative_owner_agent_id: None, priority_class: None,
    }).unwrap();

    let failed_id = kernel.registry.create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
        assigned_agent_id: &agent_id, owner_agent_id: &agent_id, created_by_agent_id: &agent_id,
        payload: "Already failed", status: "FAILED", parent_swo_id: None, kind: "TASK", source: "TEST",
        work_order_title: Some("Failed"), work_order_outcome: None, work_order_constraints: None,
        requested_owner_agent_id: None, requested_assignee_agent_id: None, routing_policy: "NONE",
        originating_swo_id: None, initiative_id: None, initiative_name: None,
        initiative_owner_agent_id: None, priority_class: None,
    }).unwrap();

    // Reap should find nothing
    let (reset, failed) = kernel.reap_orphaned_swos().unwrap();
    assert_eq!(reset, 0);
    assert_eq!(failed, 0);

    // Verify statuses unchanged
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let check = |id: i64, expected: &str| {
        let status: String = conn.query_row("SELECT status FROM active_swos WHERE id = ?1", [id], |row| row.get(0)).unwrap();
        assert_eq!(status, expected, "SWO {} should remain {}", id, expected);
    };
    check(completed_id, "COMPLETED");
    check(pending_id, "PENDING");
    check(failed_id, "FAILED");

    let _ = std::fs::remove_dir_all(test_root);
}
