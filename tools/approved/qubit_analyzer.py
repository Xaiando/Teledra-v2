import random

def generate_qubit_statistics(num_qubits):
    entangled = 0
    correlated = 0
    for _ in range(num_qubits):
        if random.random() < 0.5:  # 50% chance of entanglement
            entangled += 1
        else:
            correlated += 1
    return {"entangled": entangled, "correlated": correlated}

def print_qubit_stats(stats):
    print(f"Entangled qubits: {stats['entangled']}")
    print(f"Correlated qubits: {stats['correlated']}")

num_qubits = 24  # according to the research note
stats = generate_qubit_statistics(num_qubits)
print_qubit_stats(stats)