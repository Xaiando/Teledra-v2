import json

def analyze_resummation_schemes(scheme_data):
    summary = ""
    # Example analysis logic (simplified)
    if "scheme_type" in scheme_data and scheme_data["scheme_type"] == "Bore":
        summary += "The Bore resummation scheme is used for high-electric-charge objects, improving mass limits."
    elif "scheme_type" in scheme_data and scheme_data["scheme_data"]["scheme_type"] == "Pade-Bore":
        summary += "The Pade-Bore resummation scheme combines advantages of both schemes for better precision."

    return summary

if __name__ == "__main__":
    # Example input (JSON)
    scheme_input = '{"scheme_type": "Bore", "data": {"charge": 10, "mass_limit_improvement": "5%"}}'
    result = analyze_resummation_schemes(json.loads(scheme_input))
    print(result)