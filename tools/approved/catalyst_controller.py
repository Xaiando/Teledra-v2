import json

def analyze_shifts(data):
    with open('phosphorene_barriers.json', 'w') as f:
        json.dump(data, f)
    shift_summary = "Shifted parameters calculated: barriers={}, wells={}".format(
        data.get('barrier_count'), data.get('well_count'))
    return shift_summary

# Example usage
data = {
    'barrier_count': 3,
    'well_count': 2,
    'shift_results': {'barrier_1': [0.5, -0.3], 'barrier_2': [-0.7, 0.2], 'barrier_3': [0.4, -0.6]}
}
print(analyze_shifts(data))