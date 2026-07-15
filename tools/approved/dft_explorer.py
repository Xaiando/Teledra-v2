import numpy as np

def main():
    # Define a simple input signal
    t = np.linspace(0.0, 1.0, 500, endpoint=False)
    sig = np.sin(2 * np.pi * 5 * t) + np.cos(2 * np.pi * 7 * t)

    # Compute the DFT
    freq_domain = np.fft.rfft(sig)

    # Print the result
    print("DFT Coefficients:", freq_domain[:10])

if __name__ == "__main__":
    main()