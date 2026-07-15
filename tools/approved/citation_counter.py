import json

def count_citations():
    data = {
        "journal": "Scientific Reports",
        "impact_factor": 3.9,
        "citation_count": 834000
    }
    print(json.dumps(data, indent=2))

count_citations()