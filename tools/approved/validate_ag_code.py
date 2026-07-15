import re

def is_valid_ag_code_pattern(pattern):
    # Simple validation logic based on the pattern's structure
    if len(pattern) < 10 or not re.match(r'^[\dA-Za-z]+$', pattern):
        return False
    return True

if __name__ == "__main__":
    test_patterns = ["AG1234", "xyz98765", "1A2B3C"]
    for pattern in test_patterns:
        result = is_valid_ag_code_pattern(pattern)
        print(f"Pattern '{pattern}' is {'valid' if result else 'invalid'}")