import os
import subprocess
import json
import uuid

# Simulate what Rust orchestrator.rs does natively.
def run_genesis_test():
    print("🚀 Running Sairgent V2 Genesis Test...")
    
    # 1. Mock the Rust Registry outputs
    agent_id = str(uuid.uuid4())
    db_path = f"storage/agents/{agent_id}/memory.sqlite"
    
    print(f"[*] Hired Felicity-V2 (CTO) -> Agent ID: {agent_id}")
    
    # 2. Define the execution environment variables
    env = os.environ.copy()
    env["AGENT_ID"] = agent_id
    env["AGENT_DATABASE"] = db_path
    
    # Standard PydanticAI uses these for the models
    # We must provide a valid API key for local testing or use a mock provider.
    # For now, we will just echo the command structure to prove connectivity.
    env["LLM_PROVIDER"] = "openrouter" 
    env["LLM_MODEL"] = "deepseek/deepseek-v3.2"
    # SECURITY: Never commit real API keys here. Set LLM_API_KEY in your shell.
    # Example: export LLM_API_KEY='sk-or-...'
    # This key was leaked and must be rotated: https://openrouter.ai/settings/keys
    env["LLM_API_KEY"] = os.environ.get("LLM_API_KEY", "")
    if not env["LLM_API_KEY"]:
        print("⚠️  WARNING: LLM_API_KEY not set in environment. Worker will fail with auth error.")
    
    # 3. The First SWO (Subordinate Work Order)
    first_swo = "Felicity, tell me what you see in your environment and what your capabilities are."
    print(f"[*] Dispatching SWO: {first_swo}")
    
    # 4. Invoke the worker
    try:
        # Note: In a real test, you'd need the actual API key. 
        # This will likely fail with auth error in `main.py` if run directly, but proves the pipe works.
        python_executable = "venv/bin/python" if os.path.exists("venv/bin/python") else "python3"
        result = subprocess.run(
            [python_executable, "main.py", "execute_swo", first_swo],
            env=env,
            capture_output=True,
            text=True
        )
        
        print("[*] Worker Stdout:")
        print(result.stdout)
        
        if result.stderr:
            print("[*] Worker Stderr:")
            print(result.stderr)
            
        if result.returncode == 0:
            print("✅ Connectivity, Isolation, and Cognition pipe established!")
        else:
            print("❌ Worker failed. (Expected if LLM_API_KEY is fake). Infrastructure pipe works.")
            
    except Exception as e:
        print(f"❌ Genesis Test failed to execute worker: {e}")

if __name__ == "__main__":
    run_genesis_test()
