import json

def analyze_geography(data):
    regions = {
        "Afrotropical": ["Djibouti", "Tanzania"],
        "Palaearctic": ["Egypt", "Kazakhstan"]
    }
    
    for region, locations in regions.items():
        print(f"{region}: {locations}")
        
if __name__ == "__main__":
    data = {
        "Afrotropical": ["Djibouti", "Tanzania"],
        "Palaearctic": ["Egypt", "Kazakhstan"]
    }
    analyze_geography(data)