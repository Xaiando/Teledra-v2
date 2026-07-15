import json

def main():
    data = {
        "space_agencies": ["SpaceX", "Blue Origin", "Virgin Galactic"],
        "activities": ["launches", "payloads", "technology"]
    }
    
    report = f"Private space agencies like {', '.join(data['space_agencies'])} are focusing on activities such as {', '.join(data['activities'])}."
    
    print(json.dumps({"report": report}, indent=4))

main()