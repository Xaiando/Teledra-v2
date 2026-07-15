import json

def analyze_camera_data(data):
    # Simple analysis to identify under-calibrated cameras
    calibration_issues = []
    for cam_info in data['cameras']:
        if cam_info['calibration']['confidence'] < 0.5:
            calibration_issues.append(cam_info)
    
    if len(calibration_issues) > 0:
        print(json.dumps({
            "issue": f"Detected {len(calibration_issues)} under-calibrated camera(s).",
            "suggestions": [
                "Calibrate more frequently.",
                "Use higher-precision lenses."
            ]
        }))
    else:
        print(json.dumps({"status": "All cameras calibrated."}))

# Example usage
data = {
    "cameras": [
        {"calibration": {"confidence": 0.8}},
        {"calibration": {"confidence": 0.2}},
        {"calibration": {"confidence": 0.9}}
    ]
}
analyze_camera_data(data)