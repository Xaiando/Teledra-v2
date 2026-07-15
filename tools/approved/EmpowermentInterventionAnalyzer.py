import re

def analyze_ui(ui_text):
    # Define regex to identify common dark pattern keywords
    dark_pattern_keywords = r'\b(excessive\.|predatory\.|coercive\.|confusing\.)'
    
    matches = re.findall(dark_pattern_keywords, ui_text)
    if not matches:
        return "No dark patterns detected."
    
    interventions = []
    for match in matches:
        interventions.append(f"Suggest clear and accessible alternatives to {match.strip('.')}.")
    
    return "\n".join(interventions)

if __name__ == "__main__":
    # Example UI text
    ui_text = """
    Your account will be suspended if you don't upgrade within 24 hours. Excessive fees for premium services.
    """
    print(analyze_ui(ui_text))