import json

def analyze_stance(text):
    # Simple analysis to print the length of the input text as a summary.
    summary = len(text)
    return f"Stance detected with {summary} characters."

if __name__ == "__main__":
    input_text = "This is a sample stance for testing."
    result = analyze_stance(input_text)
    print(result)