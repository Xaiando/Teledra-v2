import random
from typing import List

# Define the probability distribution for the simulation
probabilities: List[float] = [0.4, 0.3, 0.2, 0.1]

def simulate_turbulence(steps: int) -> None:
    """Simulates a turbulent system using quantum-inspired computing.
    
    Args:
        steps (int): Number of iterations to run the simulation for.
    """
    print("Initializing turbulence simulator...")
    for _ in range(steps):
        # Generate random probabilities
        random_probabilities: List[float] = [random.random() for _ in range(len(probabilities))]
        
        # Calculate the weighted sum based on the probabilities
        total_weight: float = sum(p * r for p, r in zip(probabilities, random_probabilities))
        
        # Print the simulation results
        print(f"Step {_+1}: {total_weight:.2f}")
    
    print("Turbulence simulator finished.")

# Run the simulation with 10 steps
simulate_turbulence(10)