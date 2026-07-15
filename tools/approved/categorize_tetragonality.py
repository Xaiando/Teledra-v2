import json

def generate_pattern_string(tetragonality_level):
    if tetragonality_level == 0:
        return "Orthorhombic {A}{B}{C}"
    elif tetragonality_level == 1:
        return "Tetragonal {A}2{B}2{C}2"
    else:
        return "High-Tetragonal {A}3{B}3{C}3"

def main():
    tetragonality_levels = [0, 1, 2]
    pattern_strings = [generate_pattern_string(level) for level in tetragonality_levels]
    
    print(json.dumps(pattern_strings))

if __name__ == "__main__":
    main()