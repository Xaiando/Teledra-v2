import json

def summarize_water_data(data):
    if not data:
        return "No water quality data to analyze."
    
    # Basic metrics from the data
    total_samples = len(data)
    average_ph = sum(sample['ph'] for sample in data) / total_samples
    average_turbidity = sum(sample['turbidity'] for sample in data) / total_samples
    
    summary = f"Summary of {total_samples} water quality samples:\n" \
              f"Average pH: {average_ph:.2f}\n" \
              f"Average Turbidity: {average_turbidity:.2f}"

    return summary

# Example usage
if __name__ == "__main__":
    # Sample data (replace with actual data collection)
    sample_data = [
        {"ph": 7.5, "turbidity": 150},
        {"ph": 8.0, "turbidity": 200},
        {"ph": 6.9, "turbidity": 100}
    ]
    
    print(summarize_water_data(sample_data))