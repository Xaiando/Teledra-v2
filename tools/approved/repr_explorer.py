import json

def show_repr(text):
    print(f"Original: {text}")
    print(f"Repr:     {repr(text)}")

if __name__ == "__main__":
    test_text = "Hello \t World\n"
    show_repr(test_text)
    print(json.dumps({"result": repr(test_text)}, indent=4))