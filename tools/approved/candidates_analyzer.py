import json

def analyze_promise_methods(input_data):
    # Load the input data which is expected to be in JSON format
    data = json.loads(input_data)
    
    # Extract relevant information from the data
    methods = data.get('methods', [])
    summary = []
    
    for method in methods:
        title = method.get('title', '')
        description = method.get('description', '')
        
        if 'PROMSIS' in title or 'TOPSIS' in title or 'PROMETHEE' in title:
            summary.append(f"Title: {title}\nDescription: {description}")
    
    # Print the summary
    for entry in summary:
        print(entry)

# Example input JSON data
input_data = """
{
    "methods": [
        {"title": "Method A", "description": "A method using PROMSIS."},
        {"title": "Method B", "description": "B method not related to PROMSIS."}
    ]
}
"""

analyze_promise_methods(input_data)