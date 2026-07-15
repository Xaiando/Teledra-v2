import json

def generate_pattern_recipe(material_properties):
    # Example material property data
    material_property_data = {
        "dielectric_constant": 10.5,
        "ferroelectric_threshold": 250e-9,
        "thickness_mm": 0.2
    }
    
    # Map properties to pattern recipe strings
    recipe_strings = []
    for key, value in material_property_data.items():
        if key == "dielectric_constant":
            recipe_strings.append(f"DielectricConst({value})")
        elif key == "ferroelectric_threshold":
            recipe_strings.append(f"FerroelectricThreshold({value})")
        elif key == "thickness_mm":
            recipe_strings.append(f"ThicknessMM({value})")

    return ", ".join(recipe_strings)

if __name__ == "__main__":
    material_properties = {
        "dielectric_constant": 10.5,
        "ferroelectric_threshold": 250e-9,
        "thickness_mm": 0.2
    }
    print(generate_pattern_recipe(material_properties))