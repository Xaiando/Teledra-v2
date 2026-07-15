---
name: research_web
timeout_s: 300
max_children: 2
harness:
---

# research_web â€” search the web and fetch sources

**Input:** A plain language question or search query.

**What it does:**
1. Generates search queries matching the question.
2. Queries DuckDuckGo Lite to find relevant links.
3. Fetches the top 3-4 unique informational source pages.
4. Extracts clean text from each page and saves them to the job's workdir.
5. Spawns a `research_synth` child job to compile the results.

**Output:** A list of raw text files and a manifest `sources.json` in the workdir.
