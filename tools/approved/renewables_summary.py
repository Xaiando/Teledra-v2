import json

def print_renewable_trends():
    report = {
        "title": "Renewable Energy Growth Dwarfs Fossil Fuels",
        "summary": "According to the latest Reuters report, renewable energy is outpacing fossil fuels at an astonishing rate. This shift signals a significant transformation in global energy landscapes.",
        "key_points": [
            "Wind and solar are leading the charge",
            "Government policies and investments are driving growth"
        ]
    }
    print(json.dumps(report, indent=4))

print_renewable_trends()