import re
from collections import Counter

def analyze_text(text):
    # Count unique words and their frequencies
    word_counts = Counter(re.findall(r'\b\w+\b', text.lower()))
    
    # Check if copyrighted works are being trained on
    for word, count in word_counts.items():
        if count > 10:  # Heuristically detect common words in copyrighted texts
            print(f"Potential bias detected: '{word}' appears {count} times.")
            break

# Test the function with a sample text
sample_text = "Here's a concise, source-backed factual note..."
analyze_text(sample_text)