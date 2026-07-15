import json

def categorize_hadronic_patterns(patterns):
    categorized = {
        "hadron_structure": [],
        "non_perturbative_features": []
    }
    
    for pattern in patterns:
        if "hadron" in pattern and "structure" in pattern:
            categorized["hadron_structure"].append(pattern)
        elif "tensor" in pattern and "non-perturbative" in pattern:
            categorized["non_perturbative_features"].append(pattern)
    
    return json.dumps(categorized, indent=4)

if __name__ == "__main__":
    input_patterns = [
        "Hadron structure reflects non-perturbative features.",
        "Tensor encodes crucial information about hadrons.",
        "Non-perturbative QCD features are encoded in the tensor."
    ]
    
    result = categorize_hadronic_patterns(input_patterns)
    print(result)