import json

def generate_strudel_pattern():
    return json.dumps({
        "layers": [
            {"type": "random", "color": "#FF0000"},
            {"type": "random", "color": "#00FF00"},
            {"type": "random", "color": "#0000FF"},
            {"type": "random", "color": "#FFFF00"},
            {"type": "random", "color": "#FFA500"}
        ]
    })

def generate_fractal_pattern():
    return json.dumps({
        "type": "guilloche",
        "iterations": 300,
        "palette": ["#FFFFFF"]
    })

print("Strudel pattern:", generate_strudel_pattern())
print("Fractal pattern:", generate_fractal_pattern())