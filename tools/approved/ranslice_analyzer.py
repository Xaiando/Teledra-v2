import numpy as np

def analyze_ran_slicing(data):
    # Simple analysis for demonstration
    mean = np.mean(data)
    std_dev = np.std(data)
    result = f"Mean: {mean}, Std Dev: {std_dev}"
    return result

print(analyze_ran_slicing(np.random.rand(10)))