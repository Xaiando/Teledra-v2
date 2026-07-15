import random

def generate_l_system(axiom, rules, iterations):
    current_string = axiom
    for _ in range(iterations):
        temp_string = ""
        for char in current_string:
            temp_string += rules.get(char, char)
        current_string = temp_string
    return current_string

def mutate_l_system(system_str, mutation_rate=0.1):
    new_str = []
    for char in system_str:
        if random.random() < mutation_rate:
            new_str.append(random.choice('FX+[-]+'))
        else:
            new_str.append(char)
    return ''.join(new_str)

if __name__ == "__main__":
    axiom = "X"
    rules = {"X": "F-[[X]+X]+F[+FX]-X", "F": "FF"}
    iterations = 6
    system = generate_l_system(axiom, rules, iterations)
    print("Generated L-system: ", system)
    mutated_system = mutate_l_system(system)
    print("Mutated L-system: ", mutated_system)