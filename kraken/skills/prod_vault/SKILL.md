---
name: prod_vault
timeout_s: 240
max_children: 0
harness: verify_digest
---

# prod_vault — distill a finished report into an evergreen vault note

**Input:** path to a finished vault report (relative to kraken root or absolute).
Examples:

- `vault/k-20260707-46d54b-report.md`
- `vault/k-20260707-abc123-digest.md`

**What it does:** reads the source report, extracts durable facts, decisions, and
references, and writes a compact evergreen note suitable for long-term retrieval.
The source path is recorded in the job workdir manifest for harness coverage checks.

**Output:** `vault/<source-stem>-evergreen.md` with sections `## Evergreen`,
`## Key Facts`, `## References`, and `## Source`.

**Models:** qwen2.5:7b only.