import random

def generate_problem(n_vars):
    return [random.random() for _ in range(n_vars)]

def solve_problem(problem):
    solution = [0.5] * len(problem)
    return solution, sum([abs(x - 0.5) for x in problem])

problem = generate_problem(10)
solution, error = solve_problem(problem)

print(f"Problem: {problem}")
print(f"Solved with: {solution}")
print(f"Error: {error}")