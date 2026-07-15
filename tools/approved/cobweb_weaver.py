import json

def generate_cobweb(iterations=10):
    pattern = {
        "type": "cobweb",
        "iterations": iterations,
        "palette": ["red", "blue", "green"]
    }
    return json.dumps(pattern, indent=4)

if __name__ == "__main__":
    result = generate_cobweb()
    print(result)