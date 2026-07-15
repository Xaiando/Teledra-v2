import re

def extract_key_concepts(text):
    # Simple regex to find relevant patterns in the text
    pattern = r'\b(?:MCAD|Multi-teacher Cross-modal Alignment Distillation|image-text retrieval)\b'
    matches = re.findall(pattern, text)
    
    if matches:
        return ', '.join(matches)
    else:
        return "No key concepts found."

if __name__ == "__main__":
    research_paper = """Studied https://arxiv.org/abs/2310.19654: 
    'MCAD: Multi-teacher Cross-modal Alignment Distillation for efficient image-text retrieval is presented in the paper found at https://arxiv.org/abs/2310.19654.'"""
    
    key_concepts = extract_key_concepts(research_paper)
    print(f"Key Concepts: {key_concepts}")