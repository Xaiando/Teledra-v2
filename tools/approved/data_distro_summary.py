import random
from collections import Counter

# Sample data from Query.ai
data_sources = ["SIEMs", "Data Lakes", "Endpoint", "Identity", "Network Tools", "IT Systems"]
data_samples = [
    {"source": "SIEMs", "count": 10},
    {"source": "Data Lakes", "count": 15},
    {"source": "Endpoint", "count": 8},
    {"source": "Identity", "count": 12},
    {"source": "Network Tools", "count": 18},
    {"source": "IT Systems", "count": 20}
]

# Analyze the distribution
data_distro = [sample["count"] for sample in data_samples]
print("Data Distribution:")
for source, count in zip(data_sources, data_distro):
    print(f"{source}: {count}")

# Generate a random summary
summary_counts = Counter(data_distro)
most_common_source = max(summary_counts, key=summary_counts.get)
print(f"\nMost Common Source: {most_common_source} ({random.choice([x for x in range(1, 100)])}% of total data)")