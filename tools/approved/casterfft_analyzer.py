import numpy as np

def analyze_fft(signal):
    fft_result = np.fft.fft(signal)
    magnitude_spectrum = np.abs(fft_result)
    
    print("FFT Magnitude Spectrum:", magnitude_spectrum)

analyze_fft(np.random.rand(10))