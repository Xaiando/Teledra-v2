import sys

def analyze_l_system(axiom, rules, iterations):
    current_string = axiom
    for _ in range(iterations):
        next_string = ""
        for char in current_string:
            if char in rules:
                next_string += rules[char]
            else:
                next_string += char
        current_string = next_string
    return current_string

if __name__ == "__main__":
    axiom = "F"
    rules = {"F": "FF+[+F-F-F]-[-F+F+F]"}
    iterations = 3
    pattern_str = analyze_l_system(axiom, rules, iterations)
    print("Generated L-system pattern:", pattern_str)