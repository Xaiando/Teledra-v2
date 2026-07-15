import numpy as np

def analyze_fft(data):
    fft_result = np.fft.fft(data)
    freqs = np.fft.fftfreq(len(data))
    return {"freqs": freqs, "fft_result": fft_result}

data = [1, 2, 3, 4, 5, 6, 7, 8]
result = analyze_fft(data)

print(f"Frequency Components: {result['freqs']}")
print(f"FFT Result: {result['fft_result']}")