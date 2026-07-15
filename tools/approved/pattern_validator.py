import json

def validate_pattern(pattern_str):
    # Parse the pattern string into a JSON-friendly format
    try:
        layers = [json.loads(layer) for layer in pattern_str.split(', ')]
        if len(layers) != 4:
            return False
        
        for layer in layers:
            if not isinstance(layer, list) or len(layer) != 8:
                return False
            
            numbers = set([num for sublist in layer for num in sublist])
            if len(numbers) < 4:
                return False
        
        return True
    except json.JSONDecodeError:
        return False

# Test the generated Strudel pattern by writing a valid 4-layer pattern and validating it.
pattern = "4: [1]*8, [2]*8, [3]*8, [4]*8"
valid = validate_pattern(pattern)
print(f"Pattern Validated: {valid}")