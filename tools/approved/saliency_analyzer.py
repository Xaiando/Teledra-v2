import numpy as np

def analyze_saliency(pattern_string):
    # Dummy dataset for simplicity (replace with actual pattern data)
    parameters = [3, 5, 7, 10]
    
    # Simulate saliency scores based on the parameter values
    saliency_scores = np.array([param ** 2 for param in parameters])
    
    max_saliency = np.max(saliency_scores)
    important_params = [params for params, score in zip(parameters, saliency_scores) if score == max_saliency]
    
    result = f"Most important parameter: {important_params[0]}"
    return result

print(analyze_saliency("example_pattern_string"))