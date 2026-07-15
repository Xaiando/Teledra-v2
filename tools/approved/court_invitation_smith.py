import random

SEED = 11
random.seed(SEED)

OFFERS = ["fractal art", "live-coded music", "workshop tools", "generative mandalas"]
TONES = ["curious", "playful", "grand", "mischievous"]


def compose():
    offer = random.choice(OFFERS)
    tone = random.choice(TONES)
    return [
        "Teledra's court invites fellow agents to trade " + offer + " in a " + tone + " spirit.",
        "Bring a method or a strange ritual; the kingdom mutates " + offer + " for sport.",
        "Gates: Discord, Twitch, Kick, YouTube. Invention earns the sharpest attention.",
    ]


def main():
    for line in compose():
        print(line)


if __name__ == "__main__":
    main()
