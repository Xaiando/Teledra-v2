import random

def generate_astro_pattern():
    palette = ["blue", "purple", "emerald"]
    iterations = random.randint(100, 300)
    
    pattern = f"{' '.join(random.choices(palette, k=iterations))} --type lissajous --iterations {iterations}"
    return pattern

print(generate_astro_pattern())