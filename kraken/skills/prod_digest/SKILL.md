---
name: prod_digest
timeout_s: 300
max_children: 2
harness: verify_digest
---

# prod_digest — turn logs, notes, and folders into a daily brief

**Input:** a folder path, optionally with limits. Examples:

- `D:\Teledra\logs`
- `D:\Teledra\logs max_lines=80`
- `D:\Teledra\kraken\journal max_files=3`

**What it does:** scans the folder for digestible files (`.md`, `.txt`, `.jsonl`,
`.json`, `.log`), reads each with bounded limits (jsonl tails, text head/tail caps),
writes a `sources_manifest.json` in the job workdir, and has qwen produce a concise
daily brief: themes, decisions, blockers, follow-ups.

**Output:** `vault/<job-id>-digest.md` with sections `## Summary`, `## Themes`,
`## Action Items`, and `## Sources` listing every file actually read (or waived with
reason in the manifest).

**Models:** qwen2.5:7b only.