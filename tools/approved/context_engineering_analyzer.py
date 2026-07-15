import json

def analyze_token_utility(tokens):
    token_counts = {}
    for token in tokens:
        if token not in token_counts:
            token_counts[token] = 1
        else:
            token_counts[token] += 1
    
    sorted_tokens = dict(sorted(token_counts.items(), key=lambda item: item[1], reverse=True))
    
    return json.dumps(sorted_tokens, indent=4)

print(analyze_token_utility(["context", "state", "optimization", "constraints", "agentic", "AI"]))