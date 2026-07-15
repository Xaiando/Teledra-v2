import random

def generate_motto():
    mottos = [
        "Courage is not the absence of fear, but the triumph over it.",
        "Fear always lies. Faith and hope are eternal allies with courage.",
        "The bravest act is to be true to oneself in the face of fear.",
        "It takes great strength to face challenges head-on; let's embrace our courage today."
    ]
    
    return random.choice(mottos)

print(generate_motto())