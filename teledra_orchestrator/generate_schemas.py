import json
import os

schema_dir = r"d:\Teledra\teledra_orchestrator\schema\v1"
os.makedirs(schema_dir, exist_ok=True)

# 1. Program Manifest Schema
program_manifest = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "title": "Program Manifest",
    "type": "object",
    "additionalProperties": False,
    "required": ["manifest_version", "project_name", "epics", "dependency_dag", "global_invariants", "deferred_features", "definition_of_completion"],
    "properties": {
        "manifest_version": {"type": "string"},
        "project_name": {"type": "string"},
        "epics": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": ["epic_id", "description"],
                "properties": {
                    "epic_id": {"type": "string", "pattern": "^[a-zA-Z0-9_-]+$"},
                    "description": {"type": "string"}
                }
            }
        },
        "dependency_dag": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": ["epic_id", "depends_on"],
                "properties": {
                    "epic_id": {"type": "string", "pattern": "^[a-zA-Z0-9_-]+$"},
                    "depends_on": {"type": "array", "items": {"type": "string", "pattern": "^[a-zA-Z0-9_-]+$"}}
                }
            }
        },
        "global_invariants": {"type": "array", "items": {"type": "string"}},
        "deferred_features": {"type": "array", "items": {"type": "string"}},
        "definition_of_completion": {"type": "array", "items": {"type": "string"}}
    }
}

# 2. Task Contract Schema
task_contract = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "title": "Task Contract",
    "type": "object",
    "additionalProperties": False,
    "required": ["contract_version", "epic_id", "work_units", "forbidden_paths", "non_goals"],
    "properties": {
        "contract_version": {"type": "string"},
        "epic_id": {"type": "string", "pattern": "^[a-zA-Z0-9_-]+$"},
        "forbidden_paths": {"type": "array", "items": {"type": "string"}},
        "non_goals": {"type": "array", "items": {"type": "string"}},
        "work_units": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": ["work_unit_id", "description", "allowed_paths", "change_budget", "acceptance_commands"],
                "properties": {
                    "work_unit_id": {"type": "string", "pattern": "^[a-zA-Z0-9_-]+$"},
                    "description": {"type": "string"},
                    "allowed_paths": {"type": "array", "items": {"type": "string"}},
                    "change_budget": {
                        "type": "object",
                        "additionalProperties": False,
                        "required": ["max_files_changed", "max_added_lines", "max_total_diff_lines", "max_binary_files", "max_new_dependencies"],
                        "properties": {
                            "max_files_changed": {"type": "integer", "minimum": 0},
                            "max_added_lines": {"type": "integer", "minimum": 0},
                            "max_total_diff_lines": {"type": "integer", "minimum": 0},
                            "max_binary_files": {"type": "integer", "minimum": 0},
                            "max_new_dependencies": {"type": "integer", "minimum": 0}
                        }
                    },
                    "acceptance_commands": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": False,
                            "required": ["executable", "arguments", "working_directory", "timeout_seconds", "expected_exit_codes", "network"],
                            "properties": {
                                "executable": {"type": "string"},
                                "arguments": {"type": "array", "items": {"type": "string"}},
                                "working_directory": {"type": "string"},
                                "timeout_seconds": {"type": "integer", "minimum": 1},
                                "expected_exit_codes": {"type": "array", "items": {"type": "integer"}},
                                "network": {"type": "string", "enum": ["deny", "allow"]}
                            }
                        }
                    }
                }
            }
        }
    }
}

# 3. Work Order Schema
work_order = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "title": "Work Order",
    "type": "object",
    "additionalProperties": False,
    "required": ["work_order_version", "contract_hash", "work_unit_id", "base_commit", "allowed_paths", "forbidden_paths", "change_budget", "acceptance_commands", "agent_registry_hash", "shadow_mode"],
    "properties": {
        "work_order_version": {"type": "string"},
        "contract_hash": {"type": "string", "pattern": "^[a-fA-F0-9]{64}$"},
        "work_unit_id": {"type": "string", "pattern": "^[a-zA-Z0-9_-]+$"},
        "base_commit": {"type": "string", "pattern": "^[a-fA-F0-9]{40}$"},
        "allowed_paths": {"type": "array", "items": {"type": "string"}},
        "forbidden_paths": {"type": "array", "items": {"type": "string"}},
        "change_budget": task_contract["properties"]["work_units"]["items"]["properties"]["change_budget"],
        "acceptance_commands": task_contract["properties"]["work_units"]["items"]["properties"]["acceptance_commands"],
        "agent_registry_hash": {"type": "string", "pattern": "^[a-fA-F0-9]{64}$"},
        "shadow_mode": {"type": "boolean"}
    }
}

# 4. Authorization Receipt Schema
authorization_receipt = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "title": "Authorization Receipt",
    "type": "object",
    "additionalProperties": False,
    "required": ["authorization_version", "artifact_type", "artifact_id", "artifact_sha256", "schema_sha256", "decision", "approver_id", "author_id", "authorized_at", "comments"],
    "properties": {
        "authorization_version": {"type": "string"},
        "artifact_type": {"type": "string", "enum": ["task_contract", "program_manifest", "agent_registry", "increment"]},
        "artifact_id": {"type": "string"},
        "artifact_sha256": {"type": "string", "pattern": "^[a-fA-F0-9]{64}$"},
        "schema_sha256": {"type": "string", "pattern": "^[a-fA-F0-9]{64}$"},
        "decision": {"type": "string", "enum": ["AUTHORIZE_LOCK"]},
        "approver_id": {"type": "string"},
        "author_id": {"type": "string"},
        "authorized_at": {"type": "string", "format": "date-time"},
        "comments": {"type": "string"}
    }
}

# 5. Agent Registry Schema
agent_registry = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "title": "Agent Registry",
    "type": "object",
    "additionalProperties": False,
    "required": ["registry_version", "agents"],
    "properties": {
        "registry_version": {"type": "string"},
        "agents": {
            "type": "object",
            "additionalProperties": {
                "type": "object",
                "additionalProperties": False,
                "required": ["logical_id", "role", "model", "model_version", "prompt_version", "allowed_read_scopes", "allowed_write_scopes", "tool_permissions", "replacement_history", "can_authorize", "can_lock", "can_gate", "can_merge", "can_push"],
                "properties": {
                    "logical_id": {"type": "string"},
                    "role": {"type": "string"},
                    "model": {"type": "string"},
                    "model_version": {"type": "string"},
                    "prompt_version": {"type": "string"},
                    "allowed_read_scopes": {"type": "array", "items": {"type": "string"}},
                    "allowed_write_scopes": {"type": "array", "items": {"type": "string"}},
                    "tool_permissions": {"type": "array", "items": {"type": "string"}},
                    "replacement_history": {"type": "array", "items": {"type": "string"}},
                    "can_authorize": {"type": "boolean"},
                    "can_lock": {"type": "boolean"},
                    "can_gate": {"type": "boolean"},
                    "can_merge": {"type": "boolean"},
                    "can_push": {"type": "boolean"}
                }
            }
        }
    }
}

# 6. Transition Table Schema
transition_table = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "title": "Transition Table",
    "type": "object",
    "additionalProperties": False,
    "required": ["table_version", "program_lifecycle", "work_unit_lifecycle"],
    "properties": {
        "table_version": {"type": "string"},
        "program_lifecycle": {
            "type": "object",
            "additionalProperties": {"type": "array", "items": {"type": "string", "pattern": "^[A-Z_]+$"}}
        },
        "work_unit_lifecycle": {
            "type": "object",
            "additionalProperties": {"type": "array", "items": {"type": "string", "pattern": "^[A-Z_]+$"}}
        }
    }
}

# 7. Gate Verdict Schema
gate_verdict = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "title": "Gate Verdict",
    "type": "object",
    "additionalProperties": False,
    "required": ["verdict", "reason_code", "details"],
    "properties": {
        "verdict": {"type": "string", "enum": ["APPROVE", "REWORK", "REJECT", "ESCALATE", "INFRASTRUCTURE_ERROR"]},
        "reason_code": {"type": "string"},
        "details": {"type": "string"}
    }
}

schemas = {
    "program_manifest.schema.json": program_manifest,
    "task_contract.schema.json": task_contract,
    "work_order.schema.json": work_order,
    "authorization_receipt.schema.json": authorization_receipt,
    "agent_registry.schema.json": agent_registry,
    "transition_table.schema.json": transition_table,
    "gate_verdict.schema.json": gate_verdict
}

for filename, schema in schemas.items():
    with open(os.path.join(schema_dir, filename), "w") as f:
        json.dump(schema, f, indent=2)

print("Schemas written to", schema_dir)
