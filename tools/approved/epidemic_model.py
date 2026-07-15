import random

def simulate_pandemic(population_size=1000, infection_rate=0.05, recovery_rate=0.1):
    infected = 2
    recovered = 0
    susceptible = population_size - infected - recovered
    
    steps = 100
    for step in range(steps):
        new_infections = int(infected * infection_rate * (susceptible / population_size))
        infections_removed = min(new_infections, infected * recovery_rate)
        
        infected -= infections_removed
        susceptible += new_infections - infections_removed
        recovered += infections_removed
    
    return {"infected": infected, "recovered": recovered}

result = simulate_pandemic()
print(f"Final State: Infected: {result['infected']}, Recovered: {result['recovered']}")