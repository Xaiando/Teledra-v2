import re

def categorize_sjsmith(input_text):
    summary = []
    # Patterns to match specific research areas
    patterns = {
        'Fourier': r'Fourier',
        'Wavelets': r'wavelets|wavelet',
        'Digital Signal Processing': r'Digital Signal Processing|DSP'
    }
    
    for category, pattern in patterns.items():
        matches = re.findall(pattern, input_text, re.IGNORECASE)
        if matches:
            summary.append(f"Contributions to {category}: {len(matches)}")
    return '\n'.join(summary)

print(categorize_sjsmith("Julius Orion Smith III is a Professor Emeritus of Music and by courtesy Electrical Engineering at Stanford University. [Source: ccrma.stanford.edu/~jos/]"))