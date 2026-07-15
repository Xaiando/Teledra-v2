import json

def validate_fractus_arguments(args):
    valid_types = ["moire", "vortex", "spiral"]
    
    if not isinstance(args, dict) or "type" not in args or "iterations" not in args or "palette" not in args:
        return False
    
    if args["type"] not in valid_types:
        print(f"Invalid type: {args['type']}")
        return False
    
    try:
        iterations = int(args["iterations"])
        palette = args["palette"]
        
        # Simple validation for now; more complex checks can be added later.
        if iterations < 0 or len(palette) == 0:
            print(f"Invalid value: {args}")
            return False
    except ValueError as e:
        print(f"ValueError: {e}")
        return False
    
    print("Valid arguments")
    return True

# Example usage
if __name__ == "__main__":
    args = {"type": "moire", "--iterations": 230, "--palette": "electric_cyan"}
    if validate_fractus_arguments(args):
        print(json.dumps({"valid_args": args}))