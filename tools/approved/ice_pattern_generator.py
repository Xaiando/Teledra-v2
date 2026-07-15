import json

def generate_ice_pattern():
    pattern = {
        "type": "fractal_tree",
        "iterations": 5,
        "scale": 0.3,
        "angle": -45,
        "offsetX": 200,
        "offsetY": 100
    }
    return json.dumps(pattern, indent=4)

if __name__ == "__main__":
    print(generate_ice_pattern())