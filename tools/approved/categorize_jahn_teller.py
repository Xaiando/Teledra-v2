import numpy as np

def categorize_effect(fermion_data):
    # Simplified model to detect Jahn-Teller effect in 1D fermions
    if len(fermion_data) < 3:
        return "Not enough data"
    
    max_gap = np.max(np.diff(fermion_data))
    min_gap = np.min(np.diff(fermion_data))
    
    if (max_gap - min_gap) > 0.1:  # Arbitrary threshold
        return "Pseudo Jahn-Teller effect detected"
    else:
        return "No significant effect"

if __name__ == "__main__":
    sample_data = [1, 2, 3, 4, 6, 7, 8]
    result = categorize_effect(sample_data)
    print(f"Jahn-Teller Effect: {result}")