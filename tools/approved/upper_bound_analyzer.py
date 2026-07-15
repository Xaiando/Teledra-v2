import math

def calculate_upper_bound(length):
    # Simple upper bound for demonstration purposes.
    return int(math.log2(length) * length)

length = 1024
upper_bound = calculate_upper_bound(length)
print(f"Upper Bound for Length {length}: {upper_bound}")