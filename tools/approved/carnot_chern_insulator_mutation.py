import json

def main():
    print(json.dumps({
        "mutation": {
            "type": "chern-insulator",
            "parameters": {
                "twist_angle": 30,
                "layer_distance": 1.54,
                "interaction_strength": 2.0
            }
        }
    }))

if __name__ == "__main__":
    main()