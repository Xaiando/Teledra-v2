import json

def generate_schema():
    schema = {
        "mcp": [
            {"name": "TeledraTool", "type": "tool", "description": "A tool created by the Queen of Teledra"},
            {"name": "MCPArtifact", "type": "artifact", "description": "An artifact shared within the MCP community"}
        ]
    }
    
    print(json.dumps(schema, indent=4))

generate_schema()