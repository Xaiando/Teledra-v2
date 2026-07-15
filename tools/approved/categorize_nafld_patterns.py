import json

def categorize_pattern(pattern_description):
    if "NAFLD" in pattern_description or "NASH" in pattern_description or "fatty liver" in pattern_description:
        return "NAFLD-Related"
    elif "fibrosis" in pattern_description or "cirrhosis" in pattern_description:
        return "Advanced Stages"
    else:
        return "Other"

if __name__ == "__main__":
    research_summary = """
    Nonalcoholic fatty liver disease (NAFLD) including nonalcoholic steatohepatitis (NASH), fibrosis, cirrhosis, and eventually hepatocellular carcinoma (HCC) has become the most common liver disease worldwide.
    """
    pattern_description = "A study on NAFLD progression to NASH"
    category = categorize_pattern(pattern_description)
    print(json.dumps({"Pattern": pattern_description, "Category": category}))