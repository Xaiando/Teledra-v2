import os
from collections import Counter

def load_pattern(path):
    with open(path, 'r') as file:
        return file.read()

def analyze_patterns(patterns_dir='patterns'):
    pattern_files = [f for f in os.listdir(patterns_dir) if f.endswith('.txt')]
    if len(pattern_files) < 10:
        print("Not enough patterns to analyze.")
        return

    all_textures = []
    mixed_media_patterns = []

    for filename in pattern_files[-10:]:
        path = os.path.join(patterns_dir, filename)
        content = load_pattern(path)
        if 'texture' in content.lower():
            all_textures.append(filename)

    if not all_textures:
        print("No patterns with textures found.")
        return

    texture_counts = Counter(all_textures)
    top_texture_patterns = [p for p, c in texture_counts.most_common(5)]
    mixed_media_patterns.extend(top_texture_patterns)

    print(f"Top 5 patterns incorporating mixed media textures: {mixed_media_patterns}")