import random

def generate_music_prompt():
    """Randomize a music prompt based on Lyria 3 model."""
    adjectives = ["Ethereal", "Haunting", "Vibrant", "Mellow"]
    nouns = ["Piano", "Guitar", "Violin", "Drums"]
    adverbs = ["Slowly", "Quickly", "Melodiously", "Harmoniously"]

    prompt = f"Create a {random.choice(adjectives)} melody on the {random.choice(nouns)}, with a pace that is {random.choice(adverbs)}."
    return prompt

print(generate_music_prompt())