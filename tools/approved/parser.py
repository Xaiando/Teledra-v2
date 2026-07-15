import re

def extract_parameters(text):
    pattern = r'(-?\d+),\s*(-?\d+),\s*([-+\d.]+)'
    match = re.search(pattern, text)
    if match:
        a, b, c = map(float, match.groups())
        return f"a={a}, b={b}, c={c}"
    else:
        return "No parameters found"

print(extract_parameters("(-2), (3), (1.5)"))