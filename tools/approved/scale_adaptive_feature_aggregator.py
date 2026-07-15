import json

def generate_pattern():
    pattern = {
        "type": "scale_adaptive",
        "description": "Efficient Space-Time Video Super-Resolution Pattern",
        "params": {
            "scale_factor": 2,
            "adaptive_aggregation": True
        }
    }
    print(json.dumps(pattern, indent=4))

generate_pattern()