print("Rheostat Settings Analyzer v1.0")
rheostat_settings = [3, 5, 7]  # Example settings

def analyze_rheostats(settings):
    print(f"Analyzing settings: {settings}")
    # Simple transformation logic for this example
    pattern_string = f"R{settings[0]}_H{settings[1]}_S{settings[2]}"
    return pattern_string

print(analyze_rheostats(rheostat_settings))