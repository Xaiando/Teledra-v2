import random

def generate_hypergraph(num_nodes, num_edges):
    graph = {node: set() for node in range(1, num_nodes + 1)}
    for _ in range(num_edges):
        a, b, c = sorted(random.sample(range(1, num_nodes + 1), 3))
        if (a, b) not in graph[c]:
            graph[a].add((b, c))
            graph[b].add((c, a))
            graph[c].add((a, b))
    return graph

def analyze_hypergraph(graph):
    print("Hypergraph analyzed: ", {k: sorted(v) for k, v in graph.items()})

# Generate and analyze a random hypergraph
analyze_hypergraph(generate_hypergraph(5, 10))