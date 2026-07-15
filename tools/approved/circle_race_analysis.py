import json

def analyze_race_pattern(pattern_str):
    # Example pattern string from arXiv paper
    # pattern_str = '{"frames": [{"x": 10, "y": 20}, {"x": 30, "y": 40}])'
    
    try:
        parsed_pattern = json.loads(pattern_str)
        print(f"Pattern analyzed: {parsed_pattern}")
        return f"{'-'*80}\nRace analysis complete. Generated pattern:\n{json.dumps(parsed_pattern, indent=2)}"
    except Exception as e:
        return f"Error analyzing pattern: {str(e)}"

if __name__ == "__main__":
    example_pattern = '{"frames": [{"x": 10, "y": 20}, {"x": 30, "y": 40}]}'
    print(analyze_race_pattern(example_pattern))