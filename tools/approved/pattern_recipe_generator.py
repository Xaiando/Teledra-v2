import json

def generate_pattern(theme):
    # Example patterns based on National Geographic themes
    if "wildlife" in theme.lower():
        return '{"type": "lissajous", "iterations": 240, "palette": "emerald"}'
    elif "culture" in theme.lower():
        return '{"type": "sierpinski", "depth": 5}'
    else:
        return '{"type": "random", "seed": 123}'

if __name__ == "__main__":
    print(generate_pattern("wildlife"))