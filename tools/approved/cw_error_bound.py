import json

def print_summary(result):
    print(f"Error Bound Analysis: {result}")

if __name__ == "__main__":
    result = {"bound": 0.15, "method": "Boosting Approach", "source": "https://arxiv.org/abs/2310.14890"}
    print_summary(result)