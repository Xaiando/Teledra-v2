import re
from collections import Counter

def analyze_text(text):
    # Tokenize the input text
    tokens = re.findall(r'\w+', text.lower())

    # Extract grammatical structures (nouns, verbs, adjectives)
    nouns = [token for token in tokens if token.isalpha()]
    verbs = [token for token in tokens if token.endswith('ing')]
    adjs = [token for token in tokens if token.endswith('ly')]

    # Count word frequencies
    freqs = Counter(tokens)

    # Print insights on structure and patterns
    print("Text Structure:")
    if len(nouns) > 3:
        print(f"Found {len(nouns)} nouns: {', '.join(nouns[:3])}...")
    if len(verbs) > 1:
        print(f"Detected {len(verbs)} verbs: {', '.join(verbs)}...")
    if len(adjs) > 2:
        print(f"Observed {len(adjs)} adjectives: {', '.join(adjs[:2])}...")

text = "Here is a concise, source-backed factual note..."
analyze_text(text)