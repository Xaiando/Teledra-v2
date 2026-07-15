import json  

def analyze_cosmic_ray(data):  
    # Example data processing (simplified)  
    if "anisotropy" in data and isinstance(data["anisotropy"], dict):  
        direction = data["anisotropy"].get("direction", "")  
        intensity = data["anisotropy"].get("intensity", 0.0)  

        # Generate a pattern recipe based on the anisotropy  
        pattern_recipe = {  
            "type": "line_pattern",  
            "parameters": {  
                "direction": direction,  
                "length": intensity * 10000,  # Convert to arbitrary units for visualization  
                "color": "#FF5733"  # Red color for dramatic effect  
            }  
        }  

        return json.dumps(pattern_recipe)  
    else:  
        return json.dumps({"error": "Invalid cosmic ray data"})  

# Example usage  
if __name__ == "__main__":  
    sample_data = {  
        "anisotropy": {  
            "direction": "north",  
            "intensity": 0.8  
        }  
    }  

    pattern_recipe = analyze_cosmic_ray(sample_data)  
    print(pattern_recipe)