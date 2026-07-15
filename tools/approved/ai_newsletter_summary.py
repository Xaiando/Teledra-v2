import json

def summarize_article(article_json):
    title = article_json.get('title', 'Untitled')
    summary = article_json.get('summary', '')
    category = article_json.get('category', 'Uncategorized')
    
    return f"Article: {title}\nSummary: {summary}\nCategory: {category}"

if __name__ == "__main__":
    sample_article = {
        "title": "Weekly AI Newsletter",
        "summary": "A round-up of the latest developments in artificial intelligence.",
        "category": "AI Trends"
    }
    
    print(summarize_article(sample_article))