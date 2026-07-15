import json

def analyze_steiner(data):
    try:
        parsed_data = json.loads(data)
        print("Parsed Data:", parsed_data)
        # Simple analysis (print keys for now)
        print("Keys in data:", list(parsed_data.keys()))
    except Exception as e:
        print(f"Error parsing input: {e}")

if __name__ == "__main__":
    import sys
    if len(sys.argv) > 1:
        analyze_steiner(sys.argv[1])