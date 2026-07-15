import json

# Define the Lévy curve parameters
axiom = "F F"
rules = {"F": "-F"}
angle = 45
iterations = 4

def levy_curve(axiom, rules, angle, iterations):
    current_rule = axiom
    for _ in range(iterations):
        current_rule = ''.join(rules.get(char, char) for char in current_rule)
    
    return current_rule

# Generate the Lévy curve pattern
pattern = levy_curve(axiom, rules, angle, iterations)
print(f"Generated Lévy curve pattern: {pattern}")

# Analyze the pattern (simple example: count 'F' characters)
analysis = {"length": len(pattern), "F_count": pattern.count('F')}
print(json.dumps(analysis, indent=2))