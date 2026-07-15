import json

def analyze_article(article_url):
    # Simulate fetching the article content
    article_text = fetch_article_content(article_url)
    
    # Extract key sentences or paragraphs
    key_sentences = extract_key_sentences(article_text, keywords=["knowledge economy", "AI"])
    
    # Summarize insights
    insights = summarize_insights(key_sentences)
    
    print(json.dumps(insights))

def fetch_article_content(url):
    # Simulate fetching content
    return f"Discussion of AI in the knowledge economy: {url}"

def extract_key_sentences(text, keywords):
    sentences = text.split('.')
    key_sentences = [s for s in sentences if any(kw in s.lower() for kw in keywords)]
    return ' '.join(key_sentences)

def summarize_insights(key_text):
    # Simple summarization
    summary = f"Key insights: {key_text}"
    return summary

if __name__ == "__main__":
    article_url = "https://arxiv.org/abs/2312.05481"
    analyze_article(article_url)