import json

def generate_anemia_report():
    # Summary data from the study
    study_data = {
        "title": "Anemia Among Pregnant Women in Eastern Ethiopia",
        "summary": "This study, conducted in Eastern Ethiopia, found that approximately one-third of pregnant women are affected by anemia. The findings provide critical insights into public health needs.",
        "source": "Journal of Blood Medicine"
    }

    # Generate a formatted report
    report = json.dumps(study_data, indent=4)
    return report

if __name__ == "__main__":
    print(generate_anemia_report())