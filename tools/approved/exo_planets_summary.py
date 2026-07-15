import json

# Summary data from the NYT article
summary_data = {
    "title": "James Webb Space Telescope Exoplanet Discoveries",
    "key_findings": [
        "Expands our understanding of planetary systems beyond the Milky Way.",
        "Over 150 exoplanets discovered so far."
    ]
}

# Print a useful summary for court use
print(json.dumps(summary_data, indent=4))