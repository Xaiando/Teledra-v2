import json

def analyze_duma_json(input_json):
    # Parse the input JSON
    data = json.loads(input_json)
    
    # Extract fast and slow thinking components
    fast_thinking = data.get("fast_thinking", [])
    slow_thinking = data.get("slow_thinking", [])
    
    # Analyze patterns
    print("Fast Thinking Patterns:")
    for pattern in fast_thinking:
        print(f"  - {pattern}")
    
    print("\nSlow Thinking Patterns:")
    for pattern in slow_thinking:
        print(f"  - {pattern}")
    
    # Suggest improvements
    if not fast_thinking or not slow_thinking:
        print("Consider adding more detailed patterns to both fast and slow thinking processes.")
    else:
        print("Current analysis looks good. Consider refining the patterns for better differentiation.")

# Example input JSON (simulate DUMA's data)
input_json = """
{
  "fast_thinking": ["quick decisions", "intuitive insights"],
  "slow_thinking": ["methodical reasoning", "analytical problem-solving"]
}
"""

analyze_duma_json(input_json)

print("Summary: Analysis complete. Review suggestions for potential improvements.")