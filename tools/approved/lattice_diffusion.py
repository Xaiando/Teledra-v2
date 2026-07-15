import random

def generate_lattice_pattern(width=10, height=10):
    pattern = []
    for y in range(height):
        row = []
        for x in range(width):
            # Simulate a simple lattice field using random values (0 or 1)
            value = random.choice([0, 1])
            row.append(str(value))
        pattern.append(' '.join(row) + '\n')
    return ''.join(pattern)

# Generate and print the pattern
print(generate_lattice_pattern())