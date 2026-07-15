import numpy as np

# Simulate a simple audio sample (for demonstration)
audio_sample = [0.1, 0.2, -0.3, 0.4, -0.5, 0.6]

# Perform the Discrete Fourier Transform
n = len(audio_sample)
freq_domain = np.fft.fft(audio_sample)

# Print valid Strudel spectral pattern
print("Spectral Pattern: " + ', '.join(map(str, abs(freq_domain))))