import json

def generate_circular_filter(radius):
    """Generate a circular filter pattern."""
    if radius <= 0:
        return "Error: Radius must be positive."
    
    # Generate a simple circular filter pattern
    pattern = {
        "type": "circular",
        "radius": radius,
        "data": [
            {"x": i, "y": j} for i in range(-radius, radius + 1) for j in range(-radius, radius + 1)
                         if (i ** 2 + j ** 2 <= radius ** 2)
        ]
    }
    
    return json.dumps(pattern)

if __name__ == "__main__":
    radius = 5
    print(generate_circular_filter(radius))