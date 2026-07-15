import random

def generate_pattern():
    patterns = ["@circle", "@square", "@triangle", "@wave"]
    return random.choice(patterns)

print(generate_pattern())