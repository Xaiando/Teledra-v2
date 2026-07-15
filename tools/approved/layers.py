def generate_loop_structure(num_layers=4, layer_duration=10):
    layers = []
    for i in range(num_layers):
        start = 3*i + 1
        end = start + layer_duration - 1
        layers.append(f"({start} {end})")
    return ", ".join(layers)

print(generate_loop_structure())