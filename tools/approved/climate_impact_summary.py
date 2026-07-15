import json

def summarize_climate_data():
    print("Climate change impacts on Sierra Nevada mountains:")
    with open('sierra_nevada.json', 'w') as f:
        # Simulate writing to file (no actual I/O)
        f.write(json.dumps({"mountain_1": "melting glaciers", "mountain_2": "increased precipitation"}))
    
    print("Summary written to sierra_nevada.json")

summarize_climate_data()