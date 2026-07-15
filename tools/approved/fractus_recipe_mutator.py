import random

SEED = 12
random.seed(SEED)

TYPES = ["mandala", "woven_web", "guilloche", "lissajous", "moire",
         "orbital_lace", "julia", "burning_ship", "newton", "tricorn"]
PALETTES = ["purple_haze", "electric_cyan", "neon_sunset", "emerald"]


def mutate():
    fractal = random.choice(TYPES)
    iterations = random.randint(160, 320)
    palette = random.choice(PALETTES)
    line = "--type " + fractal + " --iterations " + str(iterations) + " --palette " + palette
    if fractal == "julia":
        line += " --c-real " + str(round(random.uniform(-1.2, 1.2), 3))
        line += " --c-imag " + str(round(random.uniform(-1.2, 1.2), 3))
    return line


def main():
    recipes = [mutate() for _ in range(5)]
    for recipe in recipes:
        print(recipe)
    return recipes


if __name__ == "__main__":
    main()
