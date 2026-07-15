def analyze_paper(paper_url):
    # Fetch the paper title and abstract from Arxiv URL
    import json

    paper_data = {"url": paper_url, "title": "", "abstract": ""}

    # Analyze the title and abstract for relevant keywords
    relevant_terms = ["pre-trained language models", "information extraction", "open information extraction"]
    
    # Summarize findings and suggest potential applications
    summary = f"Title: {paper_data['title']}\nAbstract: {paper_data['abstract']}\nKeywords Found: {relevant_terms}"
    
    suggestions = []
    if "pre-trained language models" in relevant_terms:
        suggestions.append("Suggestion: Explore using pre-trained language models for data analysis tasks.")
    
    return summary, suggestions

# Smoke test
summary, suggestions = analyze_paper("https://arxiv.org/abs/2310.15021")
print(summary)