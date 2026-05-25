use sairgent_kernel::kernel::Kernel;
use sairgent_kernel::seed::load_seed_spec;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::runtime::Runtime;

fn main() {
    println!("=== Sairgent V2 Loop Environment ===");
    println!("Initializing Control Plane...");

    // Setup runtime
    let rt = Runtime::new().unwrap();

    // Setup Kernel
    let db_path = "storage/kernel_registry.sqlite";
    std::fs::create_dir_all("storage").unwrap();

    // We expect run_worker.sh to be in the workspace root
    let worker_binary = "../run_worker.sh";

    let secrets = sairgent_kernel::kernel::Secrets {
        default_llm_api_key: "dummy".into(),
        llm_api_keys_by_provider: std::collections::HashMap::new(),
        tool_api_keys_by_slug: Arc::new(RwLock::new(std::collections::HashMap::new())),
        sidechannel_token: "dummy".into(),
    };
    let vault_key = std::env::var("SAIRGENT_VAULT_KEY")
        .unwrap_or_else(|_| {
            eprintln!("FATAL: SAIRGENT_VAULT_KEY environment variable is not set.");
            std::process::exit(1);
        });
    let kernel = match Kernel::new(
        &vault_key,
        db_path,
        worker_binary,
        secrets,
    ) {
        Ok(k) => Arc::new(k),
        Err(e) => {
            eprintln!("Failed to initialize kernel: {:?}", e);
            return;
        }
    };

    println!("Bootstrapping runtime seed...");
    let seed_path = resolve_seed_path();
    let spec = load_seed_spec(&seed_path).expect("Failed to load default runtime seed");
    let seeded = kernel
        .ensure_runtime_seeded(&spec, Some(&seed_path))
        .expect("Failed to ensure runtime seed");
    let perry_id = seeded.perry_agent_id;

    println!("Agents registered. Manager ID: {}", perry_id);

    // Launch background tasks (Active Queue Reconciler + Serverless Cron)
    rt.block_on(async {
        kernel.start_background_tasks();
        println!("[Background] Queue Reconciler and Cron Loop started.");
    });

    // Provide a hint about the API key
    if std::env::var("LLM_API_KEY").is_err() {
        println!("\nWARNING: LLM_API_KEY environment variable is not set.");
        println!("The worker subprocesses will likely fail or return stub data.");
        println!("To set it in your current terminal: export LLM_API_KEY='your_key'\n");
    }

    // Interactive Loop
    loop {
        print!("> ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let swo = input.trim();

        if swo.is_empty() {
            continue;
        }
        if swo == "exit" || swo == "quit" {
            break;
        }

        println!("\nSending message to Perry...");
        let orchestrator = Arc::clone(&kernel.orchestrator);
        let perry_id_clone = perry_id.clone();
        let swo_clone = swo.to_string();

        rt.block_on(async {
            match orchestrator
                .run_chat_mode(perry_id_clone, swo_clone, &[], None)
                .await
            {
                Ok(result) => {
                    println!("\n=== Perry Response ===");
                    let pretty_json = serde_json::to_string_pretty(&result)
                        .unwrap_or_else(|_| format!("{:?}", result));
                    println!("{}", pretty_json);
                    println!("========================\n");
                }
                Err(e) => {
                    eprintln!("\n=== HSM Loop Execution Error ===");
                    eprintln!("{:?}", e);
                    eprintln!("================================\n");
                }
            }
        });
    }

    println!("Shutting down Sairgent Control Plane.");
}

fn resolve_seed_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let candidates = [
        cwd.join("00_Context")
            .join("Seeds")
            .join("default_seed.json"),
        cwd.join("..")
            .join("00_Context")
            .join("Seeds")
            .join("default_seed.json"),
    ];
    for candidate in candidates {
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("../00_Context/Seeds/default_seed.json")
}
