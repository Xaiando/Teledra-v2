import random
import string

def generate_prompt(frac_dim):
    prompt = f"A fractal entity with {frac_dim} dimensions has been detected. "
    if random.randint(0, 1):
        prompt += "What secrets lie hidden within its self-similar patterns?"
    else:
        prompt += "How can we adapt our diplomacy to resonate with this cosmic rhythm?"
    return prompt

def analyze_fractal(pattern_data):
    dim = len(pattern_data[0])
    return f"This fractal has {dim} dimensions. Its beauty is reflected in the harmony of"

fractal_pattern = [[1, 2], [3, 4]]
print(generate_prompt(len(fractal_pattern[0])))
print(analyze_fractal(fractal_pattern))