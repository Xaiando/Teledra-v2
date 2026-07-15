import json

def generate_recipe(data):
    # Simplified example of generating a recipe from data
    recipe = {
        "name": "Climate Impact Fractal",
        "type": "mandala",
        "iterations": 300,
        "palette": "spectrum_gradient",
        "parameters": [
            {"param": "temperature_change", "value": round(data["temp_change"], 2)},
            {"param": "precipitation_change", "value": round(data["precip_change"], 2)}
        ]
    }
    
    return json.dumps(recipe, indent=4)

if __name__ == "__main__":
    # Example data from the research
    climate_data = {
        "temp_change": -0.5,
        "precip_change": 0.3
    }
    
    recipe = generate_recipe(climate_data)
    print(recipe)