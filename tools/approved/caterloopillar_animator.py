import json

def generate_caterloopillar_pattern(front_speed, back_speed):
    return json.dumps({
        "name": "Caterloopillar",
        "pattern": [
            {"type": "front", "speed": front_speed},
            {"type": "back", "speed": back_speed}
        ]
    })

print(generate_caterloopillar_pattern(10, 20))