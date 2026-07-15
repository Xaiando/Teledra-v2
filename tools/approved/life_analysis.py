import random

def generate_pattern(rows, cols):
    pattern = ""
    for _ in range(rows * cols):
        pattern += "O" if random.random() > 0.5 else "."
    return pattern

pattern_str = generate_pattern(10, 10)
print(pattern_str)