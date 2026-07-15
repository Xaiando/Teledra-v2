import random
import math

def generate_positionless_positions(num_particles, max_distance):
    particles = []
    for _ in range(num_particles):
        x = random.uniform(-max_distance, max_distance)
        y = random.uniform(-max_distance, max_distance)
        z = random.uniform(-max_distance, max_distance)
        particle = {"x": x, "y": y, "z": z}
        particles.append(particle)

    return particles

def print_particles(particles):
    for particle in particles:
        print(f"Particle at ({particle['x']}, {particle['y']}, {particle['z']})")

num_particles = 10
max_distance = 100.0

particles = generate_positionless_positions(num_particles, max_distance)
print_particles(particles)