import re

def categorize_snippet(snippet):
    keywords = ['Julius', 'Smith', 'Stanford', 'Music', 'Engineering']
    summary = ""
    
    for keyword in keywords:
        if re.search(keyword, snippet):
            summary += f"Key Research: {keyword}\n"
            
    return summary

if __name__ == "__main__":
    input_snippet = "Julius Orion Smith III is a Professor Emeritus of Music and by courtesy Electrical Engineering at Stanford University. [Source: ccrma.stanford.edu/~jos/]"
    print(categorize_snippet(input_snippet))