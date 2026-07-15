import random

# Define a few basic pattern templates
templates = [
    "Mandelbrot(escape=10)",
    "Julia(z_start=-0.8+0.156j, escape=20)",
    "BurningShip(escape=30, palette='neon_sunset')",
    "Custom({x: 0.45 + y * 0.001, y: -0.0001 - x**2})"
]

def generate_recipe():
    return random.choice(templates)

print(generate_recipe())