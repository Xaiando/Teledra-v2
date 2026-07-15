import json

# Simple aggregator function to demonstrate processing of data
def aggregate_results(data):
    results = []
    for item in data:
        result = {'id': item['id'], 'value': float(item['value'])}
        results.append(result)
    
    # Basic aggregation logic (average value)
    aggregated_value = sum(result['value'] for result in results) / len(results)
    print(f"Aggregated Value: {aggregated_value}")
    return aggregated_value

# Example input data
input_data = [
    {"id": 1, "value": 0.85},
    {"id": 2, "value": 0.90},
    {"id": 3, "value": 0.78}
]

aggregated_value = aggregate_results(input_data)
print(f"Final Aggregation: {aggregated_value}")