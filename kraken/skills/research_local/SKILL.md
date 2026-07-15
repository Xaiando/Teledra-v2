---
name: research_local
timeout_s: 240
max_children: 3
harness:
---

# research_local — answer a question from the kingdom's own knowledge

**Input:** a question in plain language.

**What it does:** scores files in `D:\Teledra\knowledge\` against the question
(term overlap, cheap and offline), reads the top matches, and has qwen write a
short cited report. Citations are the actual file paths consulted.

**Output:** `vault/<job-id>-report.md` with a `## Sources` section listing only
files that were actually read. If the knowledge base has nothing relevant it
says so honestly instead of hallucinating — and may spawn ONE child job
suggesting a `research_web` follow-up (once that skill exists).

**Models:** qwen2.5:7b only.
