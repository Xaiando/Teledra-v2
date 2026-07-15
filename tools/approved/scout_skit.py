import random

def generate_skit():
    skits = [
        "Skits were one of my favorite parts of Scouting! (Source: wackyscouter.org)",
        "In Scouting, we share stories and laughter. Today's tale? One of our most cherished activities!",
        "Every scout knows that skits are the heart of our adventures!"
    ]
    
    return random.choice(skits)

print(generate_skit())