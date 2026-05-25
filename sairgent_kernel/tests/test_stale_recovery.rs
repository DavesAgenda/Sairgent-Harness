/// Integration test: Startup reaper for orphaned SWOs (CHA-318).
///
/// Validates that `Kernel::reap_orphaned_swos()` correctly handles SWOs
/// left in IN_PROGRESS after a simulated crash:
///   1. SWOs with retry_count < 3 are reset to PENDING
///   2. SWOs with retry_count >= 3 are marked FAILED
///   3. Audit events are recorded for each reap action

use sairgent_kernel::kernel::Kernel;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[tokio::test]
async fn test_startup_reaper_resets_orphaned_swos() {
    let test_root = std::env::temp_dir().join(format!("sairgent-reaper-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_reaper.sqlite");

    let kernel = Arc::new(
        Kernel::new_with_agent_home_root(
            "dummy_vault_key_that_is_32_bytes",
            db_path.to_str().unwrap(),
            "/bin/false", // No worker needed — we never execute
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

    // Hire an agent
    let perry_id = kernel
        .registry
        .hire_subordinate("Perry", None, "COO", "Operate", "mock", "mock")
        .unwrap();

    // Create 3 SWOs directly in IN_PROGRESS (simulating crash-orphaned state)
    let swo_1 = kernel
        .registry
        .create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
            assigned_agent_id: &perry_id,
            owner_agent_id: &perry_id,
            created_by_agent_id: &perry_id,
            payload: "Orphaned task 1",
            status: "IN_PROGRESS",
            parent_swo_id: None,
            kind: "TASK",
            source: "TEST",
            work_order_title: Some("Orphaned 1"),
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

    let swo_2 = kernel
        .registry
        .create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
            assigned_agent_id: &perry_id,
            owner_agent_id: &perry_id,
            created_by_agent_id: &perry_id,
            payload: "Orphaned task 2",
            status: "IN_PROGRESS",
            parent_swo_id: None,
            kind: "TASK",
            source: "TEST",
            work_order_title: Some("Orphaned 2"),
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

    // SWO 3: already at max retries (simulate by incrementing 3 times)
    let swo_3 = kernel
        .registry
        .create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
            assigned_agent_id: &perry_id,
            owner_agent_id: &perry_id,
            created_by_agent_id: &perry_id,
            payload: "Orphaned task 3 - exhausted retries",
            status: "IN_PROGRESS",
            parent_swo_id: None,
            kind: "TASK",
            source: "TEST",
            work_order_title: Some("Orphaned 3 - max retries"),
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

    // Bump swo_3's retry count to 3 (max)
    kernel.registry.increment_swo_retry_count(swo_3).unwrap();
    kernel.registry.increment_swo_retry_count(swo_3).unwrap();
    kernel.registry.increment_swo_retry_count(swo_3).unwrap();

    // Also create a COMPLETED SWO that should NOT be touched
    let swo_completed = kernel
        .registry
        .create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
            assigned_agent_id: &perry_id,
            owner_agent_id: &perry_id,
            created_by_agent_id: &perry_id,
            payload: "Already done",
            status: "COMPLETED",
            parent_swo_id: None,
            kind: "TASK",
            source: "TEST",
            work_order_title: Some("Completed task"),
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

    // Run the reaper
    let (reset_count, failed_count) = kernel.reap_orphaned_swos().unwrap();

    assert_eq!(reset_count, 2, "Expected 2 SWOs reset to PENDING");
    assert_eq!(failed_count, 1, "Expected 1 SWO marked FAILED (max retries)");

    // Verify SWO statuses in DB
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    let status_1: String = conn
        .query_row("SELECT status FROM active_swos WHERE id = ?1", [swo_1], |row| row.get(0))
        .unwrap();
    assert_eq!(status_1, "PENDING", "SWO 1 should be reset to PENDING");

    let status_2: String = conn
        .query_row("SELECT status FROM active_swos WHERE id = ?1", [swo_2], |row| row.get(0))
        .unwrap();
    assert_eq!(status_2, "PENDING", "SWO 2 should be reset to PENDING");

    let status_3: String = conn
        .query_row("SELECT status FROM active_swos WHERE id = ?1", [swo_3], |row| row.get(0))
        .unwrap();
    assert_eq!(status_3, "FAILED", "SWO 3 should be FAILED (max retries)");

    let status_completed: String = conn
        .query_row("SELECT status FROM active_swos WHERE id = ?1", [swo_completed], |row| row.get(0))
        .unwrap();
    assert_eq!(status_completed, "COMPLETED", "Completed SWO should be untouched");

    // Verify audit trail exists for reaper actions
    let audit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM audit_events WHERE event_kind LIKE 'startup_reaper%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_count, 3, "Expected 3 audit events (one per reaped SWO)");

    let _ = std::fs::remove_dir_all(test_root);
}

#[tokio::test]
async fn test_startup_reaper_is_idempotent() {
    let test_root = std::env::temp_dir().join(format!("sairgent-reaper-idem-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_reaper_idem.sqlite");

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

    // Create an orphaned SWO
    kernel
        .registry
        .create_swo_with_metadata(sairgent_kernel::registry::CreateSwoParams {
            assigned_agent_id: &perry_id,
            owner_agent_id: &perry_id,
            created_by_agent_id: &perry_id,
            payload: "Orphaned",
            status: "IN_PROGRESS",
            parent_swo_id: None,
            kind: "TASK",
            source: "TEST",
            work_order_title: Some("Orphaned"),
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

    // First reap
    let (reset_1, failed_1) = kernel.reap_orphaned_swos().unwrap();
    assert_eq!(reset_1, 1);
    assert_eq!(failed_1, 0);

    // Second reap should find nothing (SWO is now PENDING, not IN_PROGRESS)
    let (reset_2, failed_2) = kernel.reap_orphaned_swos().unwrap();
    assert_eq!(reset_2, 0, "Second reap should find no orphans");
    assert_eq!(failed_2, 0, "Second reap should find no orphans");

    let _ = std::fs::remove_dir_all(test_root);
}
