import random

def vasorelaxation_simulation():
    # Simulate CBD's effect on vasorelaxation
    vasorelaxation_level = 0.5 + (random.random() * 0.2)
    print(f"Vasorelaxation level: {vasorelaxation_level:.2f}")

vasorelaxation_simulation()