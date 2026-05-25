use sairgent_kernel::kernel::{Kernel, Secrets};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

fn main() {
    let project_root = resolve_project_root();
    let db_path = project_root.join("storage").join("kernel_registry.sqlite");
    let worker_path = project_root.join("run_worker.sh");
    let seed_path = project_root
        .join("00_Context")
        .join("Seeds")
        .join("default_seed.json");

    std::fs::create_dir_all(project_root.join("storage")).expect("failed to create storage");

    let kernel = Kernel::new(
        "dummy_vault_key_that_is_32_bytes",
        db_path.to_str().expect("db path"),
        worker_path.to_str().expect("worker path"),
        Secrets {
            default_llm_api_key: "dummy".into(),
            llm_api_keys_by_provider: std::collections::HashMap::new(),
            tool_api_keys_by_slug: Arc::new(RwLock::new(std::collections::HashMap::new())),
            sidechannel_token: "dummy".into(),
        },
    )
    .expect("failed to initialize kernel");

    let spec = kernel
        .load_seed_spec_from_path(&seed_path)
        .expect("failed to load seed spec");
    let result = kernel
        .archive_reset_and_seed(&spec, Some(&seed_path))
        .expect("failed to archive, reset, and seed runtime");

    println!("company={}", result.company_name);
    println!("profile={}", result.profile_id);
    println!("perry_id={}", result.perry_agent_id);
    println!("agent_count={}", result.agent_count);
    println!("swo_count={}", result.swo_count);
    if let Some(snapshot) = result.archive_snapshot_id {
        println!("archive_snapshot_id={snapshot}");
    }
    if let Some(path) = result.archive_path {
        println!("archive_path={path}");
    }
}

fn resolve_project_root() -> PathBuf {
    let cwd = std::env::current_dir().expect("cwd");
    let candidates = [cwd.clone(), cwd.join(".."), cwd.join("..").join("..")];
    for candidate in candidates {
        if candidate.join("run_worker.sh").exists()
            && candidate
                .join("00_Context")
                .join("Seeds")
                .join("default_seed.json")
                .exists()
        {
            return candidate
                .canonicalize()
                .unwrap_or_else(|_| Path::new(".").to_path_buf());
        }
    }
    panic!("could not resolve Sairgent project root");
}
