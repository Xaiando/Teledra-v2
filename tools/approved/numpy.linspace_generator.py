import numpy as np

def generate_linspace_sequence(start=0.0, stop=1.0, num=50):
    sequence = np.linspace(start, stop, num)
    return ', '.join(map(str, sequence))

if __name__ == '__main__':
    print(generate_linspace_sequence())