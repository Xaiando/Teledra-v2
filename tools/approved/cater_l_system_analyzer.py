from typing import List

def analyze_l_system(axiom: str, rules: dict, iterations: int) -> str:
    current_pattern = axiom
    for _ in range(iterations):
        new_pattern = ""
        for char in current_pattern:
            if char in rules:
                new_pattern += rules[char]
            else:
                new_pattern += char
        current_pattern = new_pattern
    return current_pattern

if __name__ == "__main__":
    axiom = "F"
    rules = {"F": "FF+[+F-F]-[-F+F]"}
    iterations = 3
    result = analyze_l_system(axiom, rules, iterations)
    print(result)