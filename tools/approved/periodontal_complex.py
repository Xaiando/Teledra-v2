import random

def generate_periodontal_pattern():
    # Define basic colors representing different stages of the disease
    colors = ["#ff0000", "#ff8c00", "#ffff00", "#808000", "#00ff00"]
    
    # Generate a pattern with random iterations and color selection based on the research description
    pattern_str = f"{' '.join(random.choices(colors, k=10))} --iterations 300 --palette random"
    return pattern_str

if __name__ == "__main__":
    print(generate_periodontal_pattern())