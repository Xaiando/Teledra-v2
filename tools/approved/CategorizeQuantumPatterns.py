import json

def categorize_quantum_interactions(data):
    interactions = {
        "photon_detection": "[PHOTON, DETECTION]",
        "quantum_entanglement": "[ENTANGLEMENT, PHOTONS]",
        "superposition_measurement": "[SUPERPOSITION, MEASUREMENT]"
    }
    
    summary = []
    for interaction in data:
        if not isinstance(interaction, str):
            continue
        for key, value in interactions.items():
            if key.lower() in interaction.lower():
                summary.append(value)
                
    return json.dumps(summary)

if __name__ == "__main__":
    # Example NIST research data
    nist_data = "The NIST team has developed a new method for measuring the superposition of quantum states."
    pattern_summary = categorize_quantum_interactions(nist_data.split())
    print(pattern_summary)