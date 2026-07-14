import json
from .common import validate_windows_path

def validate_task_contract(contract_path, manifest_path, workspace_root):
    with open(contract_path, 'r') as f:
        contract = json.load(f)
    with open(manifest_path, 'r') as f:
        manifest = json.load(f)
        
    epics = {e['epic_id'] for e in manifest.get('epics', [])}
    if contract.get('epic_id') not in epics:
        return False, "Epic ID not found in program manifest"
        
    for wu in contract.get('work_units', []):
        budget = wu.get('change_budget', {})
        if budget.get('max_files_changed', -1) < 0 or budget.get('max_added_lines', -1) < 0:
            return False, "Invalid budget arithmetic"
            
        for allowed in wu.get('allowed_paths', []):
            if not validate_windows_path(allowed, workspace_root): return False, f"Invalid Windows path: {allowed}"
            for forbidden in contract.get('forbidden_paths', []):
                if allowed.lower().startswith(forbidden.lower()):
                    return False, f"Allowed path {allowed} overlaps with forbidden path {forbidden}"
                    
        allowed_executables = {"python", "pytest", "npm", "node"}
        for cmd in wu.get('acceptance_commands', []):
            if cmd.get('executable') not in allowed_executables:
                return False, f"Forbidden executable: {cmd.get('executable')}"
                
    return True, "Valid"
