import json

# Print valid Fractal pattern string
def generate_budget_visualization():
    # Simple budget visualization in fractal form
    pattern = {
        "type": "bar",
        "data": [
            {"label": "Fiscal Year 2023", "value": 85.6},
            {"label": "Fiscal Year 2024 Requested", "value": 89.6}
        ],
        "options": {
            "width": 10,
            "height": 8
        }
    }
    return json.dumps(pattern, indent=4)

print(generate_budget_visualization())