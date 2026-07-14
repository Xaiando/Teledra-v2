import os

val_dir = r"d:\Teledra\teledra_orchestrator\validators"
os.makedirs(val_dir, exist_ok=True)

# Common Path Validator
common_validator = '''import os
import re

def validate_windows_path(path_str, workspace_root):
    # Reject relative traversing
    if ".." in path_str: return False
    # Reject UNC paths
    if path_str.startswith("\\\\\\\\") or path_str.startswith("//"): return False
    # Reject device paths
    if path_str.startswith("\\\\?\\") or path_str.startswith("\\\\.\\"): return False
    if "GLOBALROOT" in path_str: return False
    # Reject alternate data streams
    if ":" in path_str and not re.match(r"^[a-zA-Z]:\\\\", path_str): return False
    
    # Reject trailing spaces/dots
    if path_str.endswith(" ") or path_str.endswith("."): return False
    
    # Check for reserved names (CON, PRN, AUX, etc.)
    reserved = {"CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"}
    basename = os.path.basename(path_str).split('.')[0].upper()
    if basename in reserved: return False
    
    # Runtime containment check
    try:
        resolved = os.path.realpath(path_str)
        base = os.path.realpath(workspace_root)
        if not resolved.startswith(base): return False
    except:
        return False
        
    return True
'''

with open(os.path.join(val_dir, "common.py"), "w") as f:
    f.write(common_validator)

# validate_program_manifest.py
val_prog = '''import json
def validate_program_manifest(manifest_path):
    with open(manifest_path, 'r') as f:
        data = json.load(f)
    
    epics = {e['epic_id'] for e in data.get('epics', [])}
    if len(epics) != len(data.get('epics', [])):
        return False, "Duplicate epic IDs found"
        
    dag = data.get('dependency_dag', [])
    for node in dag:
        if node['epic_id'] not in epics: return False, f"DAG node {node['epic_id']} not in epics"
        for dep in node['depends_on']:
            if dep not in epics: return False, f"Dependency {dep} not in epics"
            
    # DAG Acyclicity
    visited = set()
    path = set()
    graph = {n['epic_id']: n['depends_on'] for n in dag}
    
    def visit(vertex):
        if vertex in path: return False # cycle
        if vertex in visited: return True
        visited.add(vertex)
        path.add(vertex)
        for neighbour in graph.get(vertex, []):
            if not visit(neighbour): return False
        path.remove(vertex)
        return True
        
    for node in graph:
        if not visit(node): return False, "Cycle detected in DAG"
        
    return True, "Valid"
'''
with open(os.path.join(val_dir, "validate_program_manifest.py"), "w") as f:
    f.write(val_prog)

# validate_task_contract.py
val_task = '''import json
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
'''
with open(os.path.join(val_dir, "validate_task_contract.py"), "w") as f:
    f.write(val_task)

print("Validators updated")
