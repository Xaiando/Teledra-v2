import json

# Define a simple pattern generator based on the study findings
def generate_copd_pattern():
    # Mock polymorphism data (for demonstration purposes)
    polymorphisms = {
        "IL1B": ["rs1946518", "rs2307042"],
        "IL1RN": ["rs2071627"]
    }
    
    pattern_string = ""
    for gene, variants in polymorphisms.items():
        for variant in variants:
            # Generate a simple pattern string based on the gene and variant
            pattern_string += f"Gene: {gene}, Variant: {variant}"
    
    return pattern_string

# Print the generated pattern as a summary
print(generate_copd_pattern())