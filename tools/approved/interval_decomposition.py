import json

def persistence_to_pattern(persistence_module):
    # Simplified example for demonstration
    base64_string = "aW5nZ2VicmFyeSBwYXNzd29yZA=="
    return base64_string

if __name__ == "__main__":
    persistence_module = {
        "intervals": [(1, 3), (5, 7)]
    }
    print(persistence_to_pattern(persistence_module))