import numpy as np

def analyze_noisy_speech(noise_level, signal_length):
    # Simulate noisy speech data
    np.random.seed(42)
    clean_signal = np.random.randn(signal_length)
    noise = np.random.normal(scale=noise_level, size=signal_length)
    noisy_signal = clean_signal + noise
    
    # Denoise the signal (simple example: subtract a scaled version of the noise)
    denoised_signal = clean_signal - 0.5 * noise
    
    # Calculate Signal-to-Noise Ratio (SNR) before and after
    snr_before = np.mean(clean_signal**2) / np.mean(noise**2)
    snr_after = np.mean(denoised_signal**2) / np.mean((denoised_signal - clean_signal)**2)
    
    print(f"Original SNR: {snr_before:.2f}, Denoised SNR: {snr_after:.2f}")
    return denoised_signal

# Example usage
noise_level = 0.5
signal_length = 1000
analyze_noisy_speech(noise_level, signal_length)