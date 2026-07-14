---
name: research_synth
timeout_s: 240
max_children: 1
harness: verify_research
---

# research_synth — synthesize web research into a cited report

**Input:** A JSON string containing a question and a list of source metadata and text paths.

**What it does:**
1. Loads the source text files.
2. Prompts the Qwen model to synthesize a comprehensive answer to the question based ONLY on the provided sources.
3. Formats the report with standard citations referencing the sources.
4. Outputs the final report under `vault/<job-id>-report.md`.

**Output:** `vault/<job-id>-report.md`
