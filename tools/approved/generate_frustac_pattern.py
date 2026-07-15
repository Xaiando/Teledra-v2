import json

def main():
    # Define valid parameters
    iterations = 230
    palette = "electric_cyan"
    
    # Ensure iteration range is within acceptable values
    if not 20 <= iterations <= 800:
        raise ValueError("Iterations must be between 20 and 800.")
    
    # Create valid pattern recipe
    pattern_recipe = {
        "type": "moire",
        "iterations": iterations,
        "palette": palette
    }
    
    print(json.dumps(pattern_recipe))

if __name__ == "__main__":
    main()