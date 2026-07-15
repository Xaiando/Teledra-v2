import collections
from typing import List

def analyze_quantum_patterns(numbers: List[int]) -> None:
    pattern_counts = collections.Counter(map(lambda x: str(x)[:2], numbers))
    sorted_patterns = sorted(pattern_counts.items(), key=lambda x: x[1], reverse=True)
    print("Top 3 most frequent patterns:")
    for pattern, count in sorted_patterns[:3]:
        print(f"{pattern}: {count} occurrences")

if __name__ == "__main__":
    analyze_quantum_patterns([1, 2, 3, 4, 5, 6, 7, 8, 9, 1, 2, 3, 4, 5])