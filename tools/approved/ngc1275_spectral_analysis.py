import json

def analyze_ngc1275_data(spectrum):
    # Simplified analysis of spectral data (example)
    wavelengths = [300, 400, 500, 600, 700]
    intensities = [spectrum[0], spectrum[1], spectrum[2], spectrum[3], spectrum[4]]
    
    # Find the max intensity and corresponding wavelength
    max_intensity = max(intensities)
    max_wavelength = wavelengths[intensities.index(max_intensity)]
    
    summary = f"Max Intensity at: {max_wavelength} nm, with value: {max_intensity}"
    return summary

# Example usage:
spectrum_data = [10, 25, 45, 65, 80]  # Example spectrum data
print(analyze_ngc1275_data(spectrum_data))