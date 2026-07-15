import numpy as np

def generate_linspace(start, stop, num):
    return np.linspace(start, stop, num)

result = generate_linspace(0, 10, 5)
print(f"Linspace result: {result}")