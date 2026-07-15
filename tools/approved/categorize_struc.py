import sys

def main():
    # Simulate input data (for testing purposes)
    input_data = "exciton_geometric_structure_details_here"

    # Categorize the structure
    categories = {
        'square': 'Regular lattice',
        'hexagonal': 'Hexagonal lattice',
        'rhombic': 'Rhombic lattice'
    }

    for key in categories:
        if key in input_data.lower():
            print(f"Found {categories[key]}")
            break

if __name__ == "__main__":
    main()