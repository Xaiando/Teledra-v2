import random

def generate_bilingual_pattern():
    languages = ["English", "Español", "Français", "Deutsch"]
    problems = [
        "What is 15 + 23?",
        "¿Cuánto es 47 - 19?",
        "Quelle est la somme de 68 et 12?",
        "Was ist die Differenz von 81 und 44?"
    ]
    
    random.shuffle(problems)
    pattern = ""
    for i, problem in enumerate(problems):
        language = languages[i % len(languages)]
        pattern += f"{language}: {problem}; "
        
    return pattern

print(generate_bilingual_pattern())