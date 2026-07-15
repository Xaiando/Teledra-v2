import json

def analyze_project(project):
    summary = f"Project Title: {project['title']}\n"
    summary += f"Description: {project['description']}\n"
    impact_metric = project.get('impact', {}).get('metric', 'N/A')
    if impact_metric:
        summary += f"Impact Metric: {impact_metric}\n"
    else:
        summary += "Impact Metric: Not specified\n"
    
    output_file = "climate_impact_summary.txt"
    with open(output_file, "w") as file:
        file.write(summary)
    
    return summary

project_data = {
    'title': 'Renewable Energy in Urban Areas',
    'description': 'A project to implement solar panels and wind turbines in city centers.',
    'impact': {'metric': '30% reduction in local CO2 emissions'}
}

print(analyze_project(project_data))