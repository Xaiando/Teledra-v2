import random

def analyze_f_ca(rule_set, iterations):
    result = ""
    for _ in range(iterations):
        cell_state = "X" if random.choice([0, 1]) == 1 else " "
        result += cell_state
    return result

print(analyze_f_ca("rule_set", 10))