import re
from collections import Counter

def detect_bias(text):
    # Compile regular expressions for common copyrighted work indicators
    indicators = [
        re.compile(r"\b(Copyright \d{4})\b"),  # Copyright year
        re.compile(r"\b(creativecommons.org)\b"),  # CC license URL
        re.compile(r"\b(GPLv\d+|GPL-\d+)\b")  # GPL version
    ]

    # Count the number of indicators in the text
    indicator_counts = Counter()
    for indicator in indicators:
        matches = list(indicator.finditer(text))
        if matches:
            indicator_counts[indicator.pattern] += len(matches)

    # Print a summary of potential bias detection
    print(f"Potential bias detected: {', '.join([f'{count} x {pattern}' for pattern, count in indicator_counts.items()])}")

    return indicator_counts

# Example usage:
if __name__ == "__main__":
    text = "Here is a concise, source-backed factual note: 'Generative AI models have been trained on copyrighted works without the rightholders' permission.' (Source: arXiv.org)"
    detect_bias(text)