def analyze_table(html):
    # Extract the structure of the table from the HTML string.
    import re

    pattern = re.compile(r'<table.*?>((?:.|\n)*)</table>', re.DOTALL)
    match = pattern.search(html)
    if not match:
        return "No table found."

    rows, cols = 0, 0
    for line in match.group(1).split('\n'):
        fields = [field.strip() for field in line.split('<td')]
        rows += 1
        cols = max(cols, len(fields))

    # Determine if the table has headers.
    headers_present = '<th' in html

    return f"Table summary: {rows} rows, {cols} columns, {'has headers' if headers_present else 'no headers'}"

if __name__ == "__main__":
    test_html = """
    <table>
        <tr><td>Row 1, Column 1</td><td>Row 1, Column 2</td></tr>
        <tr><td>Row 2, Column 1</td></tr>
    </table>
    """
    print(analyze_table(test_html))