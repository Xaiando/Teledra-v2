import json

def analyze_ptperf_results(results):
    print(json.dumps(results, indent=4))

# Example usage:
results = {
    "transport": "meek",
    "performance": 85,
    "reasons": ["data integrity", "latency"]
}
analyze_ptperf_results(results)