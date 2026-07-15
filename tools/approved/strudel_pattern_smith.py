import random

SEED = 13
random.seed(SEED)

DRUMS = ["bd ~ sn ~", "bd*2 ~ sn ~", "bd ~ ~ sn", "bd sn ~ sn"]
HATS = ["hh*2", "hh*4", "hh*3 ~", "~ hh*2"]
BASSLINES = ["c2 eb2 g2 bb2", "a1 e2 g2 d2", "d2 a2 f2 c3", "g1 d2 bb2 f2"]
WAVES = ["triangle", "sawtooth", "square", "sine"]


def smith():
    drum = random.choice(DRUMS)
    hat = random.choice(HATS)
    bass = random.choice(BASSLINES)
    wave = random.choice(WAVES)
    return (
        "stack(\n"
        '  s("' + drum + " " + hat + '").gain(0.5),\n'
        '  note("' + bass + '").s("' + wave + '").gain(0.35).slow(1.5)\n'
        ")"
    )


def main():
    pattern = smith()
    print(pattern)
    return pattern


if __name__ == "__main__":
    main()
