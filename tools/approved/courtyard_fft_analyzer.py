import numpy as np

def analyze_signal(signal):
    n = len(signal)
    frequencies = np.fft.fft(signal)
    freq_components = [(frequencies[k], k) for k in range(n)]
    return freq_components

signal = [1, 2, 3, 4, 3, 2, 1]
freq_components = analyze_signal(signal)

print("Frequency components of the signal:")
for component in freq_components:
    print(f"Component {component[0].real:.2f} at index {component[1]}")