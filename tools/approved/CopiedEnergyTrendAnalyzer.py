import json

def analyze_renewable_trends(data):
    total_renewable = 0
    total_fossil_fuels = 0
    
    for entry in data['trends']:
        if 'renewable' in entry:
            total_renewable += entry['renewable']
        if 'fossil_fuels' in entry:
            total_fossil_fuels += entry['fossil_fuels']
    
    renewable_percentage = (total_renewable / (total_renewable + total_fossil_fuels)) * 100
    print(f"Renewable Energy Percentage: {renewable_percentage:.2f}%")

# Example usage:
data = {
    'trends': [
        {'renewable': 45, 'fossil_fuels': 30},
        {'renewable': 50, 'fossil_fuels': 28}
    ]
}

analyze_renewable_trends(data)