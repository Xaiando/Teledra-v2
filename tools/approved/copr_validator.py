def validate_copr_pattern(pattern):
    if not isinstance(pattern, str) or len(pattern.split()) != 3:
        return False
    parts = pattern.split()
    for part in parts:
        try:
            int(part)
        except ValueError:
            return False
    return True

if __name__ == "__main__":
    test_pattern = "12 34 56"
    print(f"Valid COPR pattern: {validate_copr_pattern(test_pattern)}")