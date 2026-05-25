/// Integration test: REJECT_AND_REVISE synthesis cycle.
///
/// Scenario:
///   1. Manager (Perry/COO) delegates to one subordinate.
///   2. Subordinate answers directly.
///   3. On first synthesis pass, manager returns REJECT_AND_REVISE with a revision SWO.
///   4. On second synthesis pass, manager returns APPROVE_AND_REPLY.
///
/// Verifies the kernel drives the retry cycle and ultimately returns an approved synthesis.
use sairgent_kernel::kernel::Kernel;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[tokio::test]
async fn test_reject_then_approve_synthesis() {
    let test_root = std::env::temp_dir().join(format!("sairgent-rar-{}", Uuid::new_v4()));
    let storage_dir = test_root.join("storage");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let db_path = storage_dir.join("test_registry_rar.sqlite");

    // Counter file used so the mock worker behaves differently on first vs second synthesis call.
    let counter_file = test_root.join("synthesis_calls.txt");
    std::fs::write(&counter_file, "0").unwrap();

    let mock_worker = test_root.join("mock_worker_rar.py");
    let counter_path = counter_file.to_str().unwrap().to_string();

    let script = format!(
        r#"#!/usr/bin/env python3
import json, sys, os

_ = sys.stdin.read()
mode = sys.argv[1] if len(sys.argv) > 1 else ""
role = os.environ.get("AGENT_ROLE", "")
counter_file = "{counter_path}"

if mode == "write_briefs":
    subordinates = json.loads(os.environ.get("AGENT_SUBORDINATES", "[]"))
    sub_name = subordinates[0]["name"]
    print(json.dumps({{
        "triage": {{
            "action": "DELEGATE",
            "reasoning": "Kernel-routed delegation",
            "delegation_swos": {{sub_name: "Provide the analysis."}},
        }}
    }}))
elif mode == "execute_triage":
    print(json.dumps({{
        "triage": {{
            "action": "ANSWER_DIRECTLY",
            "reasoning": "I am the sub",
            "direct_answer": "Here is my initial analysis.",
        }}
    }}))

elif mode == "execute_synthesis":
    # First call → REJECT_AND_REVISE; subsequent calls → APPROVE_AND_REPLY
    try:
        count = int(open(counter_file).read().strip())
    except Exception:
        count = 0
    count += 1
    open(counter_file, "w").write(str(count))

    if count == 1:
        subordinates = json.loads(os.environ.get("AGENT_SUBORDINATES", "[]"))
        sub_id = subordinates[0]["id"] if subordinates else "unknown"
        print(json.dumps({{
            "synthesis": {{
                "action": "REJECT_AND_REVISE",
                "reasoning": "Answer was too vague",
                "revision_swos": {{sub_id: "Please provide specific numbers."}},
            }}
        }}))
    else:
        print(json.dumps({{
            "synthesis": {{
                "action": "APPROVE_AND_REPLY",
                "reasoning": "Second pass was satisfactory",
                "final_response": "Approved revised analysis.",
            }}
        }}))
"#
    );

    std::fs::write(&mock_worker, &script).unwrap();
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
                sidechannel_token: "test_token_rar".into(),
            },
        )
        .unwrap(),
    );

    let perry_id = kernel
        .registry
        .hire_subordinate("Perry", None, "COO", "Orchestrate", "mock", "mock")
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
            "Provide cost analysis for Q1.".to_string(),
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

    let final_value = result.unwrap();
    let synthesis = final_value
        .get("synthesis")
        .expect("Expected a synthesis field in the final result");

    assert_eq!(
        synthesis.get("action").and_then(|v| v.as_str()).unwrap_or(""),
        "APPROVE_AND_REPLY",
        "Expected final synthesis to be APPROVE_AND_REPLY after revision cycle"
    );
    assert_eq!(
        synthesis
            .get("final_response")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "Approved revised analysis."
    );

    // The counter file should show ≥ 2 synthesis calls (reject + approve).
    let calls: u32 = std::fs::read_to_string(&counter_file)
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0);
    assert!(
        calls >= 2,
        "Expected at least 2 synthesis calls (reject + approve), got {calls}"
    );

    let _ = std::fs::remove_dir_all(test_root);
}
