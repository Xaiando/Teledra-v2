import random

def generate_pattern(length=10):
    base_pattern = "F-G+F+G-F"
    return ''.join(random.choices(base_pattern, k=length))

print(generate_pattern())