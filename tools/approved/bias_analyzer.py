import re

def summarize_paper(paper_text):
    # Extract key points from the paper text
    key_points = re.findall(r'Key point: (.+)', paper_text)
    
    # Generate prompts based on extracted key points
    prompts = [f"Explore {point} in bias evaluation processes." for point in key_points]
    
    return "\n".join(prompts)

if __name__ == "__main__":
    abstract = """
    Key point: The framework proposes a method to address bias in evaluation by optimizing decision-making processes.
    Key point: It suggests using optimization techniques to identify and mitigate biased evaluations.
    """
    print(summarize_paper(abstract))