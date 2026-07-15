import json

def generate_ca_pattern(rows=5, cols=5):
    pattern = [[0 for _ in range(cols)] for _ in range(rows)]
    
    # Simple rule: live cells become dead if they have exactly 2 neighbors.
    def apply_rule(cell, neighbors):
        return not (neighbors == 2)
    
    for row in range(1, rows - 1):
        for col in range(1, cols - 1):
            alive_neighbors = sum([pattern[row-1][col-1], pattern[row-1][col], pattern[row-1][col+1],
                                   pattern[row][col-1], 0, 
                                   pattern[row][col+1], pattern[row+1][col-1], pattern[row+1][col], 
                                   pattern[row+1][col+1]])
            pattern[row][col] = apply_rule(pattern[row][col], alive_neighbors)
    
    return json.dumps({'pattern': pattern})

print(generate_ca_pattern())