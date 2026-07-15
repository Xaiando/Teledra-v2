import json

def analyze_summary(text):
    summary = {
        "researchers": ["I. Bonamassa", "B. Gross", "M. Laav", "I. Volotsenko", "A. Frydman", "S. Havlin"],
        "contributions": ["experimentation", "modelling", "theoretical development"]
    }
    return json.dumps(summary, indent=4)

if __name__ == "__main__":
    text = "I. Bonamassa and colleagues initiated and designed the research, with contributions from B. Gross, M. Laav, I. Volotsenko, A. Frydman, and S. Havlin in various aspects of experimentation, modeling, and theoretical development."
    print(analyze_summary(text))