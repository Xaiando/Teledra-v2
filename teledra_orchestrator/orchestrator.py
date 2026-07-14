import os
import json
import uuid
import datetime

class TeledraOrchestrator:
    def __init__(self, run_dir, transition_table_path):
        self.run_dir = run_dir
        with open(transition_table_path, 'r') as f:
            self.transitions = json.load(f)
            
        self.program_state = "SPEC_RECEIVED"
        self.work_state = "RECEIVED"

    def _transition(self, lifecycle_type, current_state, new_state):
        allowed = self.transitions[lifecycle_type].get(current_state, [])
        if new_state not in allowed:
            raise ValueError(f"Illegal transition: {current_state} -> {new_state} in {lifecycle_type}")
        return new_state

    def transition_program(self, new_state):
        self.program_state = self._transition("program_lifecycle", self.program_state, new_state)
        print(f"Program state: {self.program_state}")

    def transition_work(self, new_state):
        self.work_state = self._transition("work_unit_lifecycle", self.work_state, new_state)
        print(f"Work state: {self.work_state}")

    def validate_authorization_receipt(self, receipt_path, draft_hash, expected_type):
        with open(receipt_path, 'r') as f:
            receipt = json.load(f)
            
        if receipt["artifact_type"] != expected_type:
            raise ValueError(f"Receipt type mismatch: {receipt['artifact_type']} != {expected_type}")
            
        if receipt["artifact_sha256"] != draft_hash:
            raise ValueError("Draft hash does not match authorization receipt hash")
            
        if receipt["decision"] != "AUTHORIZE_LOCK":
            raise ValueError("Receipt decision is not AUTHORIZE_LOCK")
            
        return True

    def log_attempt(self, attempt_number, patch_content):
        # Attempt numbering mapping (0 = builder, 1 = repair 1, 2 = repair 2)
        if attempt_number == 0:
            folder = "attempt-00-builder"
        elif attempt_number == 1:
            folder = "attempt-01-repair"
        elif attempt_number == 2:
            folder = "attempt-02-repair"
        else:
            raise ValueError("Maximum repair attempts (2) exceeded.")
            
        attempt_dir = os.path.join(self.run_dir, folder)
        os.makedirs(attempt_dir, exist_ok=True)
        with open(os.path.join(attempt_dir, "patch.diff"), "w") as f:
            f.write(patch_content)
        
if __name__ == '__main__':
    # Initial dummy run
    orc = TeledraOrchestrator(r"d:\Teledra\.teledra\runs\latest", r"d:\Teledra\.teledra\transition_table.draft.json")
    orc.transition_program("MANIFEST_DRAFTING")
