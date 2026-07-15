import json

def analyze_material(material_data):
    material_type = material_data.get('type')
    if material_type == 'magnetic_weyl':
        print("Electric quadrupole second harmonic generation reveals dual magnetic orders.")
        pattern_str = "M:2|Q:1|T:Weyl"
        return json.dumps({"pattern": pattern_str, "orders": ["dual"]})
    else:
        print(f"Material type {material_type} not analyzed.")
        return None

# Example input
material_data = {
    'type': 'magnetic_weyl',
    'details': 'Electric quadrupole second harmonic generation revealing dual magnetic orders in a magnetic Weyl semimetal.'
}
result = analyze_material(material_data)
print(result)