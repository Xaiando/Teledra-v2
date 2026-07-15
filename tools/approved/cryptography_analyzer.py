import sys

def analyze_cryptographic_risks(instruction):
    if "exploit" in instruction.lower():
        return f"Potential exploit detected: {instruction}"
    elif "vulnerability" in instruction.lower():
        return f"Possible vulnerability found: {instruction}"
    else:
        return "No immediate risks identified."

if __name__ == "__main__":
    input_instruction = sys.argv[1]
    print(analyze_cryptographic_risks(input_instruction))