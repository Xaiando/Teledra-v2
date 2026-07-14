import os
import json
import traceback
from validators.common import validate_windows_path
from validators.validate_program_manifest import validate_program_manifest
from validators.validate_task_contract import validate_task_contract
from orchestrator import TeledraOrchestrator

tmp_dir = r"d:\Teledra\teledra_orchestrator\tmp_tests"
os.makedirs(tmp_dir, exist_ok=True)

def test_cyclic_dependency_graph():
    manifest = {
        "manifest_version": "1.0",
        "project_name": "Test",
        "epics": [{"epic_id": "a", "description": ""}, {"epic_id": "b", "description": ""}],
        "dependency_dag": [
            {"epic_id": "a", "depends_on": ["b"]},
            {"epic_id": "b", "depends_on": ["a"]}
        ],
        "global_invariants": [],
        "deferred_features": [],
        "definition_of_completion": []
    }
    manifest_path = os.path.join(tmp_dir, "manifest_cycle.json")
    with open(manifest_path, 'w') as f:
        json.dump(manifest, f)
    valid, msg = validate_program_manifest(manifest_path)
    assert not valid
    assert "Cycle detected" in msg

def test_missing_dependency_ids():
    manifest = {
        "manifest_version": "1.0",
        "project_name": "Test",
        "epics": [{"epic_id": "a", "description": ""}],
        "dependency_dag": [{"epic_id": "a", "depends_on": ["b"]}],
        "global_invariants": [],
        "deferred_features": [],
        "definition_of_completion": []
    }
    manifest_path = os.path.join(tmp_dir, "manifest_missing.json")
    with open(manifest_path, 'w') as f:
        json.dump(manifest, f)
    valid, msg = validate_program_manifest(manifest_path)
    assert not valid
    assert "Dependency b not in epics" in msg

def test_duplicate_ids():
    manifest = {
        "manifest_version": "1.0",
        "project_name": "Test",
        "epics": [{"epic_id": "a", "description": ""}, {"epic_id": "a", "description": ""}],
        "dependency_dag": [],
        "global_invariants": [],
        "deferred_features": [],
        "definition_of_completion": []
    }
    manifest_path = os.path.join(tmp_dir, "manifest_dup.json")
    with open(manifest_path, 'w') as f:
        json.dump(manifest, f)
    valid, msg = validate_program_manifest(manifest_path)
    assert not valid
    assert "Duplicate epic IDs" in msg

def test_prohibited_windows_paths():
    base = r"C:\workspace"
    assert not validate_windows_path(r"C:\workspace\file.txt:stream", base)
    assert not validate_windows_path(r"\\server\share", base)
    assert not validate_windows_path(r"\\?\C:\path", base)
    assert not validate_windows_path(r"\\.\device", base)
    assert not validate_windows_path(r"C:\workspace\GLOBALROOT\a", base)
    assert not validate_windows_path(r"C:\workspace\..\escape", base)
    assert not validate_windows_path(r"C:\workspace\CON", base)
    assert not validate_windows_path(r"C:\workspace\file ", base)
    assert not validate_windows_path(r"C:\workspace\file.", base)

def test_allowed_forbidden_overlap():
    contract = {
        "contract_version": "1.0",
        "epic_id": "a",
        "forbidden_paths": [r"C:\workspace\secret"],
        "non_goals": [],
        "work_units": [{
            "work_unit_id": "wu1",
            "description": "",
            "allowed_paths": [r"C:\workspace\secret\file.txt"],
            "change_budget": {"max_files_changed": 1, "max_added_lines": 10, "max_total_diff_lines": 10, "max_binary_files": 0, "max_new_dependencies": 0},
            "acceptance_commands": []
        }]
    }
    manifest = {"epics": [{"epic_id": "a", "description": ""}]}
    c_path = os.path.join(tmp_dir, "contract.json")
    m_path = os.path.join(tmp_dir, "manifest.json")
    with open(c_path, 'w') as f:
        json.dump(contract, f)
    with open(m_path, 'w') as f:
        json.dump(manifest, f)
    
    valid, msg = validate_task_contract(c_path, m_path, r"C:\workspace")
    assert not valid
    assert "overlaps with forbidden path" in msg

def test_unknown_transitions():
    tt = {"table_version": "1.0", "program_lifecycle": {"A": ["B"]}, "work_unit_lifecycle": {"X": ["Y"]}}
    tt_path = os.path.join(tmp_dir, "tt.json")
    with open(tt_path, 'w') as f:
        json.dump(tt, f)
    orc = TeledraOrchestrator(tmp_dir, tt_path)
    orc.program_state = "A"
    
    try:
        orc.transition_program("C")
        assert False, "Should have thrown ValueError"
    except ValueError as e:
        assert "Illegal transition" in str(e)

def test_self_authorization():
    tt = {"table_version": "1.0", "program_lifecycle": {}, "work_unit_lifecycle": {}}
    tt_path = os.path.join(tmp_dir, "tt.json")
    with open(tt_path, 'w') as f:
        json.dump(tt, f)
    orc = TeledraOrchestrator(tmp_dir, tt_path)
    
    receipt = {
        "artifact_type": "task_contract",
        "artifact_sha256": "dummy",
        "decision": "AUTHORIZE_LOCK",
        "author_id": "user1",
        "approver_id": "user1"
    }
    r_path = os.path.join(tmp_dir, "receipt.json")
    with open(r_path, 'w') as f:
        json.dump(receipt, f)
    
    try:
        orc.validate_authorization_receipt(r_path, "dummy", "task_contract")
        assert False, "Should have thrown ValueError"
    except ValueError as e:
        assert "Author and approver cannot be the same" in str(e)

def test_old_artifact_hash():
    tt = {"table_version": "1.0", "program_lifecycle": {}, "work_unit_lifecycle": {}}
    tt_path = os.path.join(tmp_dir, "tt.json")
    with open(tt_path, 'w') as f:
        json.dump(tt, f)
    orc = TeledraOrchestrator(tmp_dir, tt_path)
    
    receipt = {
        "artifact_type": "task_contract",
        "artifact_sha256": "old_hash",
        "decision": "AUTHORIZE_LOCK",
        "author_id": "user1",
        "approver_id": "user2"
    }
    r_path = os.path.join(tmp_dir, "receipt.json")
    with open(r_path, 'w') as f:
        json.dump(receipt, f)
    
    try:
        orc.validate_authorization_receipt(r_path, "new_hash", "task_contract")
        assert False, "Should have thrown ValueError"
    except ValueError as e:
        assert "Draft hash does not match" in str(e)

def test_third_repair_attempt():
    tt = {"table_version": "1.0", "program_lifecycle": {}, "work_unit_lifecycle": {}}
    tt_path = os.path.join(tmp_dir, "tt.json")
    with open(tt_path, 'w') as f:
        json.dump(tt, f)
    orc = TeledraOrchestrator(tmp_dir, tt_path)
    
    orc.log_attempt(0, "patch0")
    orc.log_attempt(1, "patch1")
    orc.log_attempt(2, "patch2")
    
    try:
        orc.log_attempt(3, "patch3")
        assert False, "Should have thrown ValueError"
    except ValueError as e:
        assert "Maximum repair attempts (2) exceeded" in str(e)

if __name__ == '__main__':
    tests = [
        test_cyclic_dependency_graph,
        test_missing_dependency_ids,
        test_duplicate_ids,
        test_prohibited_windows_paths,
        test_allowed_forbidden_overlap,
        test_unknown_transitions,
        test_self_authorization,
        test_old_artifact_hash,
        test_third_repair_attempt
    ]
    
    passed = 0
    for t in tests:
        try:
            t()
            print(f"PASS: {t.__name__}")
            passed += 1
        except Exception as e:
            print(f"FAIL: {t.__name__}")
            traceback.print_exc()
            
    print(f"\\nTotal: {len(tests)}, Passed: {passed}")
    if passed != len(tests):
        exit(1)
