import json

def analyze_data(data):
    max_teach = max(data, key=lambda x: x['teaching_effectiveness'])
    min_teach = min(data, key=lambda x: x['teaching_effectiveness'])
    
    insights = {
        "best_material": max_teach["material"],
        "worst_material": min_teach["material"],
        "average_effectiveness": sum(x['teaching_effectiveness'] for x in data) / len(data)
    }
    return json.dumps(insights, indent=4)

# Smoke Test
data = [
    {"material": "Copper", "teaching_effectiveness": 0.95},
    {"material": "Niobium", "teaching_effectiveness": 0.85}
]
print(analyze_data(data))