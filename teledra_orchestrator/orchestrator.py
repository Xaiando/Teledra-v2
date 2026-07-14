import os
import sys
import json
import uuid
import datetime
import subprocess
import jsonschema

RUNS_DIR = r"d:\Teledra\.teledra\runs"
SCHEMA_DIR = r"d:\Teledra\teledra_orchestrator\schema"

def validate_schema(data, schema_name):
    schema_path = os.path.join(SCHEMA_DIR, f"{schema_name}.schema.json")
    with open(schema_path, "r") as f:
        schema = json.load(f)
    jsonschema.validate(instance=data, schema=schema)

def run_pipeline(task_desc: str):
    run_id = f"run-{datetime.date.today().strftime('%Y-%m-%d')}-{uuid.uuid4().hex[:4]}"
    run_dir = os.path.join(RUNS_DIR, run_id)
    
    # 1. Initialize structure
    for d in ["00_intake", "10_contract", "20_build", "30_break", "40_repair", "50_gate", "audit"]:
        os.makedirs(os.path.join(run_dir, d), exist_ok=True)
    
    with open(os.path.join(run_dir, "00_intake", "request.md"), "w") as f:
        f.write(task_desc)
        
    print(f"[{run_id}] Transition: RECEIVED -> CONTRACT_DRAFTED")
    
    # 2. Cartographer (Kraken) drafts contract
    print(f"[{run_id}] Invoking Cartographer (Kraken)...")
    draft_path = os.path.join(run_dir, "10_contract", "task_contract.draft.json")
    # For Phase 6A dry-run, we simulate the Cartographer's output for a tiny bug
    dummy_contract = {
        "schema_version": "1.0",
        "run_id": run_id,
        "task_id": "task-001",
        "repository": {"base_commit": "HEAD", "environment_image": "python:3.12"},
        "objective": {"summary": "Fix simple bug", "user_visible_result": "Works"},
        "scope": {"allowed_paths": ["test_bug.py"]},
        "acceptance_criteria": [],
        "work_units": [],
        "risk": {"tier": "low"},
        "budgets": {"builder_wall_seconds": 300},
        "escalation_conditions": []
    }
    with open(draft_path, "w") as f:
        json.dump(dummy_contract, f, indent=2)
        
    # Validate and lock
    try:
        validate_schema(dummy_contract, "task_contract")
        locked_path = os.path.join(run_dir, "10_contract", "task_contract.locked.json")
        with open(locked_path, "w") as f:
            json.dump(dummy_contract, f, indent=2)
        print(f"[{run_id}] Transition: CONTRACT_DRAFTED -> CONTRACT_VALIDATED")
    except Exception as e:
        print(f"[{run_id}] Contract validation failed: {e}")
        return
        
    print(f"[{run_id}] Transition: CONTRACT_VALIDATED -> BUILD_RUNNING")
    
    # 3. Builder (Kraken Beta) builds
    print(f"[{run_id}] Invoking Builder (qwen25_b)...")
    # Simulate builder creating a patch in its workspace
    # In reality, this would be: subprocess.run(["kraken_beta/kraken.py", ...])
    # For now, we just create a dummy patch
    patch_content = "--- test_bug.py\n+++ test_bug.py\n@@ -1,2 +1,2 @@\n-def bad():\n-  return 1\n+def bad():\n+  return 2\n"
    patch_path = os.path.join(run_dir, "20_build", "patch.diff")
    with open(patch_path, "w") as f:
        f.write(patch_content)
        
    manifest = {
        "schema_version": "1.0",
        "run_id": run_id,
        "contract_hash": "dummy_hash",
        "base_commit": "HEAD",
        "patch_hash": "dummy_patch_hash",
        "changed_files": [{"path": "test_bug.py", "added_lines": 1, "deleted_lines": 1}],
        "policy_findings": [],
        "builder_claims": {}
    }
    with open(os.path.join(run_dir, "20_build", "patch_manifest.json"), "w") as f:
        json.dump(manifest, f, indent=2)
        
    print(f"[{run_id}] Transition: BUILD_RUNNING -> PATCH_CAPTURED")
    print(f"[{run_id}] Transition: PATCH_CAPTURED -> PATCH_POLICY_VALIDATED")
    
    # 3.5 Breaker (Kraken Gamma)
    print(f"[{run_id}] Transition: PATCH_POLICY_VALIDATED -> BREAK_RUNNING")
    print(f"[{run_id}] Invoking Breaker (qwen25_c)...")
    break_dir = os.path.join(run_dir, "30_break")
    public_test_path = os.path.join(break_dir, "public_tests.py")
    with open(public_test_path, "w") as f:
        f.write("def test_bug():\n    assert False, 'Defect found!'\n")
        
    print(f"[{run_id}] Scanning Breaker output for policy violations...")
    ast_scanner_script = r"d:\Teledra\teledra_orchestrator\ast_scanner.py"
    scan_result = subprocess.run([sys.executable, ast_scanner_script, public_test_path], capture_output=True, text=True)
    
    if scan_result.returncode != 0:
        print(f"[{run_id}] Breaker Policy Violation: {scan_result.stdout.strip()}")
        print(f"[{run_id}] Transition: BREAK_RUNNING -> ESCALATED")
        return
        
    print(f"[{run_id}] Breaker result: DEFECT_REPRODUCED")
    print(f"[{run_id}] Transition: BREAK_RUNNING -> REPAIR_RUNNING")
    
    # 3.8 Repairer (Kraken Alpha)
    repair_attempts = 0
    max_repair_attempts = 2
    
    while repair_attempts < max_repair_attempts:
        repair_attempts += 1
        print(f"[{run_id}] Invoking Repairer (qwen25_a) - Attempt {repair_attempts}/{max_repair_attempts}...")
        
        # Simulate Repairer output
        print(f"[{run_id}] Transition: REPAIR_RUNNING -> PATCH_CAPTURED_V2")
        print(f"[{run_id}] Transition: PATCH_CAPTURED_V2 -> BREAK_REGRESSION_CHECK")
        
        if repair_attempts < 2:
            print(f"[{run_id}] Regression Check: FAIL (attempts remain)")
            print(f"[{run_id}] Transition: BREAK_REGRESSION_CHECK -> REPAIR_RUNNING")
        else:
            print(f"[{run_id}] Regression Check: PASS (Simulated success)")
            print(f"[{run_id}] Transition: BREAK_REGRESSION_CHECK -> GATE_RUNNING")
            break
    else:
        # If while loop finishes without breaking
        print(f"[{run_id}] Regression Check: FAIL (budget exhausted)")
        print(f"[{run_id}] Transition: BREAK_REGRESSION_CHECK -> ESCALATED")
        return
    
    # 4. Gatekeeper evaluates
    gatekeeperd_script = r"d:\Teledra\teledra_orchestrator\gatekeeperd.py"
    subprocess.run([sys.executable, gatekeeperd_script, run_dir])
    
    with open(os.path.join(run_dir, "50_gate", "verdict.json"), "r") as f:
        verdict = json.load(f)
        
    status = verdict.get("status")
    if status == "APPROVE":
        print(f"[{run_id}] Transition: GATE_RUNNING -> READY_TO_MERGE")
    elif status == "REWORK":
        print(f"[{run_id}] Transition: GATE_RUNNING -> REPAIR_RUNNING")
    else:
        print(f"[{run_id}] Transition: GATE_RUNNING -> CLOSED_REJECTED / ESCALATED")

if __name__ == '__main__':
    run_pipeline("Fix the dummy bug in test_bug.py")
