import json

def analyze_rotation_effect(data):
    # Example data structure
    result = {
        "rotation_angle": 120,
        "thermodynamic_change": 5.2,
        "energy_shift": -3.1
    }
    
    print(json.dumps(result))

# Test the tool
analyze_rotation_effect({
    "rotation_angle": 120,
    "thermodynamic_change": 5.2,
    "energy_shift": -3.1
})