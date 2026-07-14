import json
import hashlib

def validate_work_order(work_order_path, contract_path):
    with open(work_order_path, 'r') as f:
        wo = json.load(f)
        
    # Cross-artifact hash consistency
    with open(contract_path, 'rb') as f:
        contract_hash = hashlib.sha256(f.read()).hexdigest()
        
    if wo['contract_hash'] != contract_hash:
        return False, "Contract hash mismatch"
        
    return True, "Valid"
