import random

def generate_caterpillar_layout(num_segments, min_length=10, max_length=50):
    layout = []
    current_position = 0
    for _ in range(num_segments):
        length = random.randint(min_length, max_length)
        layout.append((current_position, current_position + length))
        current_position += length
    return layout

def mutate_layout(layout, mutation_rate=0.2):
    mutated_layout = []
    for start, end in layout:
        if random.random() < mutation_rate:
            new_start = random.randint(start - 10, start + 10)
            new_end = random.randint(end - 5, end + 5)
            if new_start < new_end:
                mutated_layout.append((new_start, new_end))
    return mutated_layout

def main():
    layout = generate_caterpillar_layout(8)
    print("Generated Layout:", layout)
    mutated_layout = mutate_layout(layout)
    print("Mutated Layout:", mutated_layout)

if __name__ == "__main__":
    main()