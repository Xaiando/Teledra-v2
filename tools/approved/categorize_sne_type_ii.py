import json

def categorize_sn_light_curve(light_curve_data):
    # Example light curve data structure for demonstration:
    # {"time": [0, 1, 2], "flux": [1.2, 1.5, 1.8], "units": "Jansky"}
    
    if not isinstance(light_curve_data, dict) or 'time' not in light_curve_data or 'flux' not in light_curve_data:
        return "Invalid data format."
    
    time = light_curve_data['time']
    flux = light_curve_data['flux']
    
    # Simplified categorization logic (for demonstration purposes)
    if all(f > 1.5 for f in flux):
        return "High-luminosity Type II Supernova"
    elif any(f < 0.8 for f in flux):
        return "Low-luminosity Type II Supernova"
    else:
        return "Moderate-luminosity Type II Supernova"

def main():
    data = {
        "time": [0, 1, 2],
        "flux": [1.2, 1.5, 1.8],
        "units": "Jansky"
    }
    
    result = categorize_sn_light_curve(data)
    print(f"Type II SN categorization: {result}")

if __name__ == "__main__":
    main()