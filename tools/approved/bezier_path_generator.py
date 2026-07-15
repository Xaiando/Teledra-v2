import json

def generate_bezier_pattern():
    # Simple quadratic Bézier curve with control points (0, 0), (0.5, 1), and (1, 0)
    pattern = {
        "type": "quadratic",
        "points": [
            [0, 0],
            [0.5, 1],
            [1, 0]
        ]
    }
    return json.dumps(pattern)

if __name__ == "__main__":
    print(generate_bezier_pattern())