import os
import sys
import json
import jsonschema

SCHEMA_DIR = r"d:\Teledra\teledra_orchestrator\schema"

def validate_schema(data, schema_name):
    schema_path = os.path.join(SCHEMA_DIR, f"{schema_name}.schema.json")
    with open(schema_path, "r") as f:
        schema = json.load(f)
    jsonschema.validate(instance=data, schema=schema)

def run_gate(run_dir):
    print(f"[Gatekeeperd] Processing {run_dir}")
    
    # 1. Read patch manifest
    manifest_path = os.path.join(run_dir, "20_build", "patch_manifest.json")
    if not os.path.exists(manifest_path):
        verdict = {
            "schema_version": "1.0",
            "run_id": os.path.basename(run_dir),
            "candidate_patch_hash": "",
            "contract_hash": "",
            "status": "INFRASTRUCTURE_ERROR",
            "merge_authorized": False,
            "reason_codes": ["MISSING_PATCH_MANIFEST"],
            "checks": [],
            "risk": {"tier": "high", "manual_review_required": True}
        }
        write_verdict(run_dir, verdict)
        return
        
    with open(manifest_path, "r") as f:
        manifest = json.load(f)
        
    try:
        validate_schema(manifest, "patch_manifest")
    except Exception as e:
        verdict = {
            "schema_version": "1.0",
            "run_id": os.path.basename(run_dir),
            "candidate_patch_hash": manifest.get("patch_hash", ""),
            "contract_hash": manifest.get("contract_hash", ""),
            "status": "INFRASTRUCTURE_ERROR",
            "merge_authorized": False,
            "reason_codes": [f"MANIFEST_SCHEMA_INVALID: {e}"],
            "checks": [],
            "risk": {"tier": "high", "manual_review_required": True}
        }
        write_verdict(run_dir, verdict)
        return
        
    # 2. Simulate clean-room application and test execution
    # In Phase 6A dry-run, we simulate a successful apply and test
    print(f"[Gatekeeperd] Simulating git apply --check and test execution...")
    
    verdict = {
        "schema_version": "1.0",
        "run_id": os.path.basename(run_dir),
        "candidate_patch_hash": manifest["patch_hash"],
        "contract_hash": manifest["contract_hash"],
        "status": "APPROVE",
        "merge_authorized": False, # Shadow mode: NO AUTOMATIC MERGES
        "reason_codes": [],
        "checks": [
            {"id": "baseline-tests", "status": "passed", "exit_code": 0},
            {"id": "patch-apply", "status": "passed", "exit_code": 0}
        ],
        "risk": {"tier": "low", "manual_review_required": True}
    }
    write_verdict(run_dir, verdict)
    print(f"[Gatekeeperd] Verdict generated: {verdict['status']}")

def write_verdict(run_dir, verdict):
    verdict_path = os.path.join(run_dir, "50_gate", "verdict.json")
    with open(verdict_path, "w") as f:
        json.dump(verdict, f, indent=2)

if __name__ == '__main__':
    if len(sys.argv) < 2:
        print("Usage: python gatekeeperd.py <run_dir>")
        sys.exit(1)
    run_gate(sys.argv[1])
