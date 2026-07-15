import json

# Function to generate a basic C2FPL pattern recipe string.
def generate_c2fpl_recipe():
    # Define the core elements of the C2FPL recipe.
    recipe = {
        "type": "anomaly_detection",
        "algorithm": "c2fpl",
        "parameters": {
            "coarse_to_fine_steps": 5,
            "pseudo_labeling_iterations": 10
        }
    }

    # Convert the dictionary to a JSON string.
    recipe_json = json.dumps(recipe, indent=4)

    return recipe_json

# Print the generated recipe.
print(generate_c2fpl_recipe())