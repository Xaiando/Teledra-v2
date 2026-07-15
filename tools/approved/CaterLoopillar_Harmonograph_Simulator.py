import math

def lissajous_curve(t, a1=1, b1=1, p1=1, a2=1, b2=1, p2=1):
    x = (a1 * math.sin(2 * math.pi * t * p1 + 0)) - (a2 * math.cos(2 * math.pi * t * p2))
    y = (b1 * math.sin(2 * math.pi * t * b1)) - (b2 * math.cos(2 * math.pi * t * p2 + 0))
    return x, y

def main():
    t_values = [i / 100 for i in range(100)]
    points = [(lissajous_curve(t)[0], lissajous_curve(t)[1]) for t in t_values]
    
    print("Generated Lissajous curve points:")
    for point in points:
        print(f"({point[0]:.4f}, {point[1]:.4f})")

if __name__ == "__main__":
    main()