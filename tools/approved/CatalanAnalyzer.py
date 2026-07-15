import math

def catalan_number(n):
    return math.factorial(2 * n) // (math.factorial(n + 1) * math.factorial(n))

def analyze_catalan_numbers(n):
    result = [catalan_number(i) for i in range(n)]
    print("Fuss-Catalan numbers up to", n, ":", result)

analyze_catalan_numbers(10)