import json

def analyze_starship_update(text):
    data = {
        "Company": "SpaceX",
        "UpdateSource": "Space.com",
        "Summary": "SpaceX is updating mission capabilities for its Starship spacecraft.",
        "KeyPoints": [
            "New mission profiles",
            "Advanced propulsion systems",
            "Enhanced thermal protection"
        ]
    }
    return json.dumps(data, indent=4)

print(analyze_starship_update("SpaceX is updating mission capabilities for its Starship spacecraft. (Source: Space.com)"))