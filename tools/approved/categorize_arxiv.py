import json

def print_paper_info(paper):
    title = paper.get('title', 'Unknown Title')
    abstract = paper.get('abstract', 'No abstract available')
    category = paper.get('@category', 'N/A')
    
    print(f"Title: {title}")
    print("Abstract:")
    print(abstract)
    print(f"Categorized under: {category}")

if __name__ == "__main__":
    paper_data = {
        "title": "The Photon Sphere and Response Functions in Holography",
        "@category": "High Energy Physics - Theory"
    }
    
    print_paper_info(paper_data)