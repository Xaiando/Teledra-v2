import json

def analyze_image_descriptor(descriptor):
    data = json.loads(descriptor)
    keypoints = data.get('keypoints', [])
    if not keypoints:
        return "No keypoints found in the descriptor."
    
    summary = f"Image has {len(keypoints)} key points:\n"
    for idx, kp in enumerate(keypoints[:5], start=1):
        summary += f"{idx}. Point: ({kp['pt'][0]:.2f}, {kp['pt'][1]:.2f}), Octave: {kp['octave']}\n"
    
    return summary

if __name__ == "__main__":
    descriptor = '{"keypoints": [{"pt": [345.87, 196.21], "octave": 0}, {"pt": [232.55, 374.98], "octave": 1}], "method": "Superpoint"}'
    print(analyze_image_descriptor(descriptor))