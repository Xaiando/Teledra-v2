import random

def crossover_pattern(pattern1, pattern2):
    midpoint = len(pattern1) // 2
    new_pattern = pattern1[:midpoint] + pattern2[midpoint:]
    return new_pattern

# Example usage:
pattern1 = "S(2,3)"
pattern2 = "F(4,5)"
mutation_result = crossover_pattern(pattern1, pattern2)
print(mutation_result)