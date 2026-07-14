import json
def validate_program_manifest(manifest_path):
    with open(manifest_path, 'r') as f:
        data = json.load(f)
    
    epics = {e['epic_id'] for e in data.get('epics', [])}
    if len(epics) != len(data.get('epics', [])):
        return False, "Duplicate epic IDs found"
        
    dag = data.get('dependency_dag', [])
    for node in dag:
        if node['epic_id'] not in epics: return False, f"DAG node {node['epic_id']} not in epics"
        for dep in node['depends_on']:
            if dep not in epics: return False, f"Dependency {dep} not in epics"
            
    # DAG Acyclicity
    visited = set()
    path = set()
    graph = {n['epic_id']: n['depends_on'] for n in dag}
    
    def visit(vertex):
        if vertex in path: return False # cycle
        if vertex in visited: return True
        visited.add(vertex)
        path.add(vertex)
        for neighbour in graph.get(vertex, []):
            if not visit(neighbour): return False
        path.remove(vertex)
        return True
        
    for node in graph:
        if not visit(node): return False, "Cycle detected in DAG"
        
    return True, "Valid"
