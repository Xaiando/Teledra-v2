import json

def generate_bernstein_pattern():
    return json.dumps({
        "type": "bernstein",
        "coefficients": [1, 2, 3, 4],
        "domain": [-5, 5]
    })

if __name__ == "__main__":
    print(generate_bernstein_pattern())