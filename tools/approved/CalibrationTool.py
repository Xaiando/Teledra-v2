import math

def analyze_calibration(volatility_data):
    # Simple analysis for demonstration purposes
    results = {}
    
    if not volatility_data:
        print("No data provided.")
        return
    
    total_volatility = sum(volatility_data)
    mean_volatility = total_volatility / len(volatility_data)
    variance = sum((x - mean_volatility) ** 2 for x in volatility_data) / len(volatility_data)
    
    results['mean'] = mean_volatility
    results['variance'] = variance
    
    print(f"Mean Volatility: {mean_volatility}, Variance: {variance}")
    return results

# Example usage
if __name__ == "__main__":
    example_volatility_data = [0.1, 0.2, 0.3, 0.4]
    calibration_results = analyze_calibration(example_volatility_data)