import re

def analyze_axion_models(text):
    axions = []
    for line in text.split('\n'):
        if 'axion' in line.lower():
            match = re.search(r'(axion\w+)', line)
            if match:
                axions.append(match.group(1))
    return axions

if __name__ == "__main__":
    sample_text = """Studied https://arxiv.org/abs/2310.16087: 'Extending preferred axion models via heavy-quark induced early matter domination [High Energy Physics - Phenomenology].'"""
    print(analyze_axion_models(sample_text))