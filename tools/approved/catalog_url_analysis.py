def analyze_url(url):
    if not url.startswith("http://") and not url.startswith("https://"):
        return "Invalid URL: Missing protocol."

    parts = url.split('/')
    scheme, netloc, path = parts[0], parts[2], '/' + '/'.join(parts[3:])
    
    if len(path.strip()) == 0:
        return f"Valid URL: {url}"
    
    return "Valid URL: " + url

print(analyze_url("https://example.com/path"))