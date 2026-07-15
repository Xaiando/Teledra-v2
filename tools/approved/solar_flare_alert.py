import json

def main():
    # Sample JSON data representing solar flare information
    solar_data = {
        "flares": [
            {"time": "2023-10-05T14:30Z", "magnitude": 2.8, "type": "M"},
            {"time": "2023-10-06T12:00Z", "magnitude": 1.5, "type": "C"}
        ],
        "latest_cme": {
            "time": "2023-10-07T08:45Z",
            "speed": 900,
            "direction": "N"
        }
    }

    print(json.dumps(solar_data, indent=4))

if __name__ == "__main__":
    main()