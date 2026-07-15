import json

def analyze_lang_splat(text):
    # Example analysis (simplified)
    words = text.split()
    pattern_string = " ".join(words[:5])
    return f"Pattern: {pattern_string}"

text_input = "LangSplat: 3D Language Gaussian Splatting is published in Computer Science > Computer Vision and Pattern Recognition."
print(analyze_lang_splat(text_input))