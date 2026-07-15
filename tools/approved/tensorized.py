import json

def generate_pattern_string():
    # Example pattern string structure for Strudel patterns
    pattern = {
        "type": "tensorized_pauli",
        "dimensions": 4,
        "decomposition": [
            {"operation": "X", "axis": 0},
            {"operation": "Z", "axis": 1}
        ]
    }
    
    # Convert the pattern dictionary to a JSON string
    pattern_string = json.dumps(pattern, indent=2)
    return pattern_string

print(generate_pattern_string())