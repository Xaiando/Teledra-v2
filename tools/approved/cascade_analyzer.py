import re

def analyze_families(family_strings):
    pattern_strings = []
    for fs in family_strings:
        matches = re.findall(r'\b\w+\b', fs)
        if len(matches) > 1:
            pattern_strings.append(' '.join(matches))
    return pattern_strings

if __name__ == "__main__":
    input_families = [
        "F + [ F - F ] + F",
        "X -> X + Y F +",
        "Y -> F - X Y -",
        "F -> FF"
    ]
    print(analyze_families(input_families))