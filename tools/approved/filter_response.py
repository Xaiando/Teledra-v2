import numpy as np

# Example input response data (replace with actual signal)
response_data = [0.5, 0.3, -0.2, 0.1]

# Calculate frequency response
freq_response = np.fft.fft(response_data)

# Extract magnitude and phase
magnitude = np.abs(freq_response)
phase = np.angle(freq_response)

print(f"Magnitude: {magnitude}")
print(f"Phase: {phase}")