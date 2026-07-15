import random

def generate_pattern(width=100, height=50):
    pattern = []
    for _ in range(height):
        row = [random.choice([' ', '#']) for _ in range(width)]
        pattern.append(''.join(row))
    return '\n'.join(pattern)

print(generate_pattern())