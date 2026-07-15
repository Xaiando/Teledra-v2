import json
import random

def generate_wright_fisher_pattern(num_patterns=10):
    patterns = []
    
    for _ in range(num_patterns):
        # Simulate a simple Wright-Fisher distribution (mean=36, std_dev=8)
        pattern_params = {
            "type": "lissajous",
            "iterations": random.randint(25, 45),
            "scale": round(random.uniform(1.0, 3.0), 2),
            "palette": random.choice(["emerald", "azure"]),
            "offset_x": round(random.uniform(-10, 10), 2),
            "offset_y": round(random.uniform(-10, 10), 2)
        }
        patterns.append(pattern_params)
    
    print(json.dumps(patterns, indent=4))

if __name__ == "__main__":
    generate_wright_fisher_pattern()