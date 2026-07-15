import re

def analyze_rhythm(pattern_str):
    # Simple validation (just to prove it's running)
    if re.match(r'^\s*\[(?:[0-9/\s]+\]\s*)+\]', pattern_str.strip()):
        return f"Valid Strudel pattern: {pattern_str}"
    else:
        return "Invalid rhythm pattern. Try again!"

print(analyze_rhythm("[2/4 1/8 3/8]"))