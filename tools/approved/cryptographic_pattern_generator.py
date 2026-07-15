import math

def fekete_polynomial(n):
    result = 0
    for k in range(1, n+1):
        result += (math.sin(math.pi * k**2)) / k
    return result

print(f"Generated Fekete polynomial pattern: {fekete_polynomial(5)}")