---
name: introspect
timeout_s: 240
max_children: 3
harness: verify_introspect
---

# introspect — the taskforce audits its own history

**Input:** optional focus (a skill name, or empty / "all").

**What it does:** reads the operational record — every job verdict in
`journal/*.jsonl` and every forge lesson in `lessons/code_forge_lessons.jsonl` —
and aggregates it into failure patterns: which skills fail most, the recurring
failure reasons, honest-failure vs crash ratio, repair-cycle costs. Then qwen
turns those patterns into a **ranked improvement backlog**, each item backed by
concrete counts and example job ids, with a proposed fix.

**Output:** `vault/<job-id>-introspection.md` — a self-authored report the
operator (or the senior bench) can act on. This is how the taskforce points at
its own next improvement instead of waiting to be told.

The aggregation is deterministic (stdlib, no model); only the ranking/proposal
step uses qwen, so the evidence is always trustworthy even if the prose isn't.
