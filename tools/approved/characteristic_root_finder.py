import numpy as np

def find_roots(coeffs):
    roots = np.roots(coeffs)
    return [round(root.real, 4) for root in roots]

if __name__ == "__main__":
    coeffs = [-1, -2, -1]  # Example: x^2 + 2x + 1
    print(find_roots(coeffs))