import numpy as np

def analyze_signal(signal):
    # Simplified spectral analysis for demonstration
    frequency_domain = np.fft.fft(signal)
    
    magnitude = np.abs(frequency_domain)
    mean_magnitude = np.mean(magnitude)
    
    print(f"Signal Mean Magnitude: {mean_magnitude}")
    return f"magnitude:{mean_magnitude}"

# Example usage
signal = [1, 2, 3, 4, 5]
print(analyze_signal(signal))