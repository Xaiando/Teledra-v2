import random

# Example input pattern string
pattern = "F++F+F--F++F"

def mutate_pattern(pattern):
    # Define mutation operations
    mutations = [
        ("F", "f"),  # Change F to f (lowercase)
        ("+", "+-"), # Add a - between adjacent +s
        ("-","+-"),  # Add a + between adjacent -s
    ]
    
    new_pattern = ""
    i = 0
    while i < len(pattern):
        if random.random() < 0.1:  # 10% chance of mutation
            op, replacement = random.choice(mutations)
            start = pattern[i:i+len(op)]
            if start == op:
                new_pattern += replacement
                i += len(replacement)
            else:
                new_pattern += start
        else:
            new_pattern += pattern[i]
            i += 1
    
    return new_pattern

mutated_pattern = mutate_pattern(pattern)

print(mutated_pattern)