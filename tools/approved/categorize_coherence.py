import json

def categorize_coherence(pattern_string):
    # Simplified example: Extract coherence levels from pattern string and summarize
    coherence_levels = {
        "low": 0,
        "medium": 1,
        "high": 2
    }
    
    parts = pattern_string.split(',')
    summary = {"low": 0, "medium": 0, "high": 0}
    
    for part in parts:
        if 'coherence' in part:
            level = part.split(' ')[-1]
            summary[level] += 1
            
    result = json.dumps(summary)
    print(result)

categorize_coherence("Coherence(low), Coherence(medium), Coherence(high), Coherence(high)")