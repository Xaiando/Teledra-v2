import math

def calculate_persistence_length(radius):
    """
    Calculates the persistence length of a circular DNA strand.
    
    Parameters:
        radius (float): The radius of the circular DNA strand.
        
    Returns:
        float: The estimated persistence length.
    """
    k_b = 1.380649e-23  # Boltzmann constant in J/K
    t_0 = 5.77 * (1e-12)  # Reference temperature in seconds
    l_p = radius * math.sqrt(k_b * t_0 / (0.0018))  # Persistence length calculation
    
    return l_p

if __name__ == "__main__":
    print(f"Circular persistence length: {calculate_persistence_length(5e-9)} meters")