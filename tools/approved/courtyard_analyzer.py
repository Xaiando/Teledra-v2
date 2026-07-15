import numpy as np

def analyze_spatio_temporal(data):
    # Simple analysis for illustrative purposes
    mean_value = np.mean(data)
    max_value = np.max(data)
    min_value = np.min(data)

    print(f"Mean value: {mean_value}")
    print(f"Max value: {max_value}")
    print(f"Min value: {min_value}")

# Sample data simulation for demonstration purposes
data = np.random.rand(100, 100) * 10

analyze_spatio_temporal(data)